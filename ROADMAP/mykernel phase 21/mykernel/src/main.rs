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
    println!("Phase 21: Virtio Network Driver");

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

    println!("[ok] Kernel initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("=== Phase 21: Virtio Network Driver ===");

        // Initialize drivers (including virtio-net)
        mykernel::drivers::init();

        // Demo network operations
        demo_network();

        println!("");
        println!("=== Phase 21 Complete ===");
        println!("To test with networking:");
        println!("  cargo bootimage");
        println!("  qemu-system-x86_64 ... -netdev user,id=net0 -device virtio-net-pci,netdev=net0");
        println!("");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_network() {
    use mykernel::drivers::virtio_net;
    use mykernel::drivers::virtio_net::{
        build_ethernet_frame, build_arp_reply, internet_checksum,
        ETH_P_ARP, ETH_P_IP,
    };

    println!("");

    // Check if virtio-net device is present
    match virtio_net::get_mac() {
        Some(mac) => {
            println!("[ok] virtio-net found!");
            println!("     MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

            // Try to send a test packet (ARP announcement)
            let broadcast = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
            let our_ip = [10, 0, 2, 15]; // QEMU default user-mode IP

            // Build gratuitous ARP (announce our MAC/IP)
            let arp_frame = build_arp_reply(mac, our_ip, broadcast, our_ip);
            match virtio_net::send_packet(&arp_frame) {
                Ok(()) => println!("[ok] ARP announcement sent ({} bytes)", arp_frame.len()),
                Err(e) => println!("[!] Send failed: {}", e),
            }

            // Poll for received packets
            println!("Polling for received packets...");
            let mut received = 0;
            for _ in 0..1000 {
                if let Some(pkt) = virtio_net::recv_packet() {
                    received += 1;
                    if let Some(ethertype) = pkt.ethertype() {
                        match ethertype {
                            ETH_P_ARP => println!("  [rx] ARP packet ({} bytes)", pkt.data.len()),
                            ETH_P_IP  => println!("  [rx] IPv4 packet ({} bytes)", pkt.data.len()),
                            t => println!("  [rx] Ethertype={:#x} ({} bytes)", t, pkt.data.len()),
                        }
                    }
                }
                core::hint::spin_loop();
            }
            if received == 0 {
                println!("  (no packets received — normal without active network)");
            }
        }
        None => {
            println!("[!] No virtio-net device found");
            println!("    Start QEMU with: -netdev user,id=net0 -device virtio-net-pci,netdev=net0");
        }
    }

    // Demo: packet building (works without hardware)
    println!("");
    println!("Packet building demo:");
    let src_mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    let dst_mac = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let payload = b"Hello from MyKernel!";
    let frame = build_ethernet_frame(dst_mac, src_mac, 0x0800, payload);
    println!("  Ethernet frame: {} bytes", frame.len());
    println!("  dst: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        frame[0], frame[1], frame[2], frame[3], frame[4], frame[5]);
    println!("  src: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        frame[6], frame[7], frame[8], frame[9], frame[10], frame[11]);
    println!("  ethertype: {:#04x}", ((frame[12] as u16) << 8) | frame[13] as u16);

    // Demo: checksum
    let data = &[0x45, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x40, 0x01,
                 0x00, 0x00, 0x0a, 0x00, 0x02, 0x0f, 0x0a, 0x00, 0x02, 0x01u8];
    let cksum = internet_checksum(data);
    println!("  IP checksum example: {:#06x}", cksum);

    serial_println!("[net] Demo complete");
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
fn test_ethernet_frame_builder() {
    use mykernel::drivers::virtio_net::build_ethernet_frame;
    let src = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    let dst = [0xFF; 6];
    let payload = b"test";
    let frame = build_ethernet_frame(dst, src, 0x0800, payload);
    assert_eq!(frame.len(), 14 + 4); // ETH_HLEN + payload
    assert_eq!(&frame[0..6], &dst);
    assert_eq!(&frame[6..12], &src);
    assert_eq!(frame[12], 0x08);
    assert_eq!(frame[13], 0x00);
    serial_println!("[test] Ethernet frame builder OK");
}

#[test_case]
fn test_internet_checksum() {
    use mykernel::drivers::virtio_net::internet_checksum;
    // Known-good IP header checksum test
    // IP header with checksum=0, result should match known value
    let hdr = [0x45u8, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x00,
               0x40, 0x01, 0x00, 0x00, 0x0a, 0x00, 0x02, 0x0f,
               0x0a, 0x00, 0x02, 0x01];
    let cksum = internet_checksum(&hdr);
    // Fill checksum back and verify
    let mut hdr2 = hdr;
    hdr2[10] = (cksum >> 8) as u8;
    hdr2[11] = cksum as u8;
    assert_eq!(internet_checksum(&hdr2), 0); // Should be 0 (valid)
    serial_println!("[test] Internet checksum OK: {:#06x}", cksum);
}

#[test_case]
fn test_arp_builder() {
    use mykernel::drivers::virtio_net::build_arp_reply;
    let sender_mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    let sender_ip  = [10, 0, 2, 15];
    let target_mac = [0xFF; 6];
    let target_ip  = [10, 0, 2, 1];
    let frame = build_arp_reply(sender_mac, sender_ip, target_mac, target_ip);
    // 14 (eth) + 28 (arp) = 42
    assert_eq!(frame.len(), 42);
    // ARP opcode = 2 (reply) at offset 14+6 = 20
    assert_eq!(frame[20], 0x00);
    assert_eq!(frame[21], 0x02);
    serial_println!("[test] ARP builder OK: {} bytes", frame.len());
}

#[test_case]
fn test_pci_scan_for_net() {
    use mykernel::drivers::pci::scan_pci_bus;
    use mykernel::drivers::virtio_net::VIRTIO_NET_DEVICE_ID;
    use mykernel::drivers::virtio::VIRTIO_VENDOR_ID;
    let devices = scan_pci_bus();
    let found = devices.iter().any(|d|
        d.vendor_id == VIRTIO_VENDOR_ID && d.device_id == VIRTIO_NET_DEVICE_ID);
    serial_println!("[test] virtio-net in PCI scan: {}", found);
    // Don't assert - device may not be present
}
