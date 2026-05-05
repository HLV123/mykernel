#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(naked_functions)]
#![test_runner(mykernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use mykernel::{println, serial_println};
use mykernel::memory::BootInfoFrameAllocator;
use mykernel::task::{executor::Executor, Task};
use x86_64::VirtAddr;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 22: TCP/IP Stack");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };
    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    mykernel::fs::init();
    mykernel::process::set_phys_mem_offset(boot_info.physical_memory_offset);
    mykernel::usermode::init_syscalls();
    mykernel::drivers::init();
    mykernel::net::init();

    println!("[ok] Kernel + Network stack initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        let ip = mykernel::net::our_ip();
        let mac = mykernel::net::our_mac();

        println!("");
        println!("=== Phase 22: TCP/IP Stack ===");
        println!("  MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
        println!("  IP:  {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
        println!("");
        println!("Network layers ready:");
        println!("  [ok] Ethernet framing");
        println!("  [ok] ARP request/reply + cache");
        println!("  [ok] IPv4 parse + build");
        println!("  [ok] ICMP echo reply (ping responder)");
        println!("  [ok] UDP echo server (port 7)");
        println!("  [ok] TCP state machine (SYN/ACK/FIN)");
        println!("");

        if mykernel::drivers::virtio_net::get_mac().is_some() {
            println!("Network active! Test with:");
            println!("  ping 10.0.2.15        # ICMP echo");
            println!("  nc -u 10.0.2.15 7     # UDP echo");
            println!("");
            println!("Polling for incoming packets...");
            // Poll network for a bit
            for _ in 0..10_000 {
                mykernel::net::poll();
                core::hint::spin_loop();
            }
        } else {
            println!("Start QEMU with networking:");
            println!("  -netdev user,id=net0 -device virtio-net-pci,netdev=net0");
        }

        println!("");
        println!("=== Phase 22 Complete ===");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[PANIC] {}", info);
    serial_println!("[PANIC] {}", info);
    mykernel::hlt_loop()
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! { mykernel::test_panic_handler(info) }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test_case]
fn test_breakpoint_exception() { x86_64::instructions::interrupts::int3(); }

#[test_case]
fn test_ipv4_build_parse() {
    use mykernel::net::ip;
    let src = [10, 0, 2, 15u8];
    let dst = [10, 0, 2,  1u8];
    let payload = b"hello";
    let pkt = ip::build_ipv4(src, dst, ip::PROTO_UDP, payload);
    assert!(pkt.len() >= ip::IP_HDR_LEN + payload.len());
    let (hdr, data) = ip::parse_ipv4(&pkt).expect("parse failed");
    assert!(hdr.is_valid());
    assert_eq!(hdr.src_ip, src);
    assert_eq!(hdr.dst_ip, dst);
    assert_eq!(hdr.protocol, ip::PROTO_UDP);
    assert_eq!(data, payload.as_slice());
    serial_println!("[test] IPv4 build+parse OK: {} bytes", pkt.len());
}

#[test_case]
fn test_udp_build_parse() {
    use mykernel::net::udp;
    use mykernel::net::ip;
    let src_ip = [10, 0, 2, 15u8];
    let dst_ip = [10, 0, 2,  1u8];
    let msg = b"udp test";
    let pkt = udp::build_udp(src_ip, dst_ip, 12345, 7, msg);
    // Parse IP layer
    let (_, ip_payload) = ip::parse_ipv4(&pkt).unwrap();
    let (udp_hdr, udp_payload) = udp::parse_udp(ip_payload).unwrap();
    assert_eq!(udp_hdr.src_port(), 12345);
    assert_eq!(udp_hdr.dst_port(), 7);
    assert_eq!(udp_payload, msg.as_slice());
    serial_println!("[test] UDP build+parse OK: sport={} dport={}",
        udp_hdr.src_port(), udp_hdr.dst_port());
}

#[test_case]
fn test_tcp_handshake() {
    use mykernel::net::tcp;
    // Build a SYN packet
    let syn = tcp::build_tcp(
        [10,0,2,1], [10,0,2,15],
        54321, 80,
        0x1000, 0,
        65535, tcp::TCP_SYN,
        &[],
    );
    assert!(syn.len() >= 20 + 20); // IP + TCP
    // Parse it back
    use mykernel::net::ip;
    let (_, tcp_payload) = ip::parse_ipv4(&syn).unwrap();
    let (tcp_hdr, _) = tcp::parse_tcp(tcp_payload).unwrap();
    assert!(tcp_hdr.has_syn());
    assert!(!tcp_hdr.has_ack());
    assert_eq!(tcp_hdr.src_port(), 54321);
    serial_println!("[test] TCP SYN packet OK: {} bytes", syn.len());
}

#[test_case]
fn test_tcp_state_machine() {
    use mykernel::net::tcp::{TcpConn, TcpState, TcpHdr, TCP_SYN};
    let our_ip = [10, 0, 2, 15u8];
    let mut conn = TcpConn::new(our_ip, 80);
    conn.state = TcpState::Listen;

    // Simulate incoming SYN
    let syn_hdr = TcpHdr {
        src_port:  54321u16.to_be_bytes(),
        dst_port:  80u16.to_be_bytes(),
        seq_num:   0x1000u32.to_be_bytes(),
        ack_num:   [0;4],
        data_off:  0x50,
        flags:     TCP_SYN,
        window:    65535u16.to_be_bytes(),
        checksum:  [0;2],
        urgent:    [0;2],
    };
    let client_ip = [10, 0, 2, 1u8];
    let reply = conn.process(client_ip, &syn_hdr, &[]);
    assert!(reply.is_some(), "SYN should produce SYN-ACK");
    assert_eq!(conn.state, TcpState::SynReceived);
    serial_println!("[test] TCP state machine: Listen→SynReceived OK");
}

#[test_case]
fn test_arp_cache() {
    use mykernel::net::arp;
    let ip  = [10, 0, 2, 1u8];
    let mac = [0x52, 0x54, 0x00, 0xAA, 0xBB, 0xCCu8];
    arp::cache_insert(ip, mac);
    let found = arp::cache_lookup(&ip);
    assert_eq!(found, Some(mac));
    let miss = arp::cache_lookup(&[1,2,3,4]);
    assert_eq!(miss, None);
    serial_println!("[test] ARP cache OK");
}
