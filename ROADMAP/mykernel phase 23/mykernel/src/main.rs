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
    println!("Phase 23: Socket API");

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

    println!("[ok] Kernel + Socket API initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("=== Phase 23: Socket API ===");
        demo_socket_api();
        println!("");
        println!("=== Phase 23 Complete ===");
        println!("POSIX Socket API ready!");
        println!("");
        println!("To test with networking:");
        println!("  qemu ... -netdev user,id=net0 -device virtio-net-pci,netdev=net0");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_socket_api() {
    use mykernel::net::socket::*;

    println!("Testing Socket API:");

    // Test 1: Create TCP socket
    let tcp_fd = sys_socket(AF_INET, SOCK_STREAM, 0);
    assert!(tcp_fd > 0, "socket() failed: {}", tcp_fd);
    println!("  [ok] socket(AF_INET, SOCK_STREAM) = fd {}", tcp_fd);

    // Test 2: Create UDP socket
    let udp_fd = sys_socket(AF_INET, SOCK_DGRAM, 0);
    assert!(udp_fd > 0, "socket() failed: {}", udp_fd);
    println!("  [ok] socket(AF_INET, SOCK_DGRAM) = fd {}", udp_fd);

    // Test 3: Bind TCP socket to port 8080
    let addr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   8080u16.to_be_bytes(),
        sin_addr:   [0, 0, 0, 0],
        _pad:       [0; 8],
    };
    let ret = sys_bind(tcp_fd as usize, &addr as *const _ as u64, 16);
    assert_eq!(ret, 0, "bind() failed: {}", ret);
    println!("  [ok] bind(fd={}, port=8080) = {}", tcp_fd, ret);

    // Test 4: Listen on TCP socket
    let ret = sys_listen(tcp_fd as usize, 5);
    assert_eq!(ret, 0, "listen() failed: {}", ret);
    println!("  [ok] listen(fd={}) = {}", tcp_fd, ret);

    // Test 5: Bind UDP socket to port 9090
    let udp_addr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   9090u16.to_be_bytes(),
        sin_addr:   [0, 0, 0, 0],
        _pad:       [0; 8],
    };
    let ret = sys_bind(udp_fd as usize, &udp_addr as *const _ as u64, 16);
    assert_eq!(ret, 0);
    println!("  [ok] bind(fd={}, port=9090) = {}", udp_fd, ret);

    // Test 6: setsockopt SO_REUSEADDR
    let val: i32 = 1;
    let ret = sys_setsockopt(tcp_fd as usize, SOL_SOCKET, SO_REUSEADDR,
        &val as *const _ as u64, 4);
    assert_eq!(ret, 0);
    println!("  [ok] setsockopt(SO_REUSEADDR) = {}", ret);

    // Test 7: getsockname
    let mut name = SockaddrIn::default();
    let ret = sys_getsockname(tcp_fd as usize, &mut name as *mut _ as u64, 0);
    assert_eq!(ret, 0);
    println!("  [ok] getsockname: port={}", name.port());

    // Test 8: Duplicate bind should fail (port in use)
    let addr2 = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   8080u16.to_be_bytes(),
        sin_addr:   [0, 0, 0, 0],
        _pad:       [0; 8],
    };
    let tcp_fd2 = sys_socket(AF_INET, SOCK_STREAM, 0);
    let ret = sys_bind(tcp_fd2 as usize, &addr2 as *const _ as u64, 16);
    assert_eq!(ret, EADDRINUSE, "Expected EADDRINUSE, got {}", ret);
    println!("  [ok] Duplicate bind correctly returns EADDRINUSE");
    sys_close_socket(tcp_fd2 as usize);

    // Test 9: Close sockets
    let ret = sys_close_socket(tcp_fd as usize);
    assert_eq!(ret, 0);
    let ret = sys_close_socket(udp_fd as usize);
    assert_eq!(ret, 0);
    println!("  [ok] close() sockets OK");

    // Test 10: is_socket_fd
    assert!(!is_socket_fd(1));
    assert!(!is_socket_fd(99));
    println!("  [ok] is_socket_fd() check OK");

    println!("");
    println!("Socket API summary:");
    println!("  socket()     — create TCP/UDP socket");
    println!("  bind()       — bind to local port");
    println!("  listen()     — mark as server (TCP)");
    println!("  accept()     — accept new connection");
    println!("  connect()    — initiate connection");
    println!("  send/recv()  — data transfer");
    println!("  sendto/recvfrom() — UDP with address");
    println!("  setsockopt() — set socket options");
    println!("  getsockname/getpeername() — get addresses");
    println!("  shutdown()   — half-close connection");
    println!("  close()      — free socket");

    serial_println!("[phase23] Socket API demo complete");
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
fn test_socket_create() {
    use mykernel::net::socket::*;
    let fd = sys_socket(AF_INET, SOCK_STREAM, 0);
    assert!(fd >= 100, "socket fd should be >= 100, got {}", fd);
    assert!(is_socket_fd(fd as usize));
    sys_close_socket(fd as usize);
    assert!(!is_socket_fd(fd as usize));
    serial_println!("[test] socket create/close OK fd={}", fd);
}

#[test_case]
fn test_socket_bind_listen() {
    use mykernel::net::socket::*;
    let fd = sys_socket(AF_INET, SOCK_STREAM, 0);
    assert!(fd > 0);

    let addr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   7777u16.to_be_bytes(),
        sin_addr:   [0; 4],
        _pad:       [0; 8],
    };
    assert_eq!(sys_bind(fd as usize, &addr as *const _ as u64, 16), 0);
    assert_eq!(sys_listen(fd as usize, 5), 0);
    sys_close_socket(fd as usize);
    serial_println!("[test] socket bind+listen OK on port 7777");
}

#[test_case]
fn test_socket_udp() {
    use mykernel::net::socket::*;
    let fd = sys_socket(AF_INET, SOCK_DGRAM, 0);
    assert!(fd > 0);
    let addr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   5555u16.to_be_bytes(),
        sin_addr:   [0; 4],
        _pad:       [0; 8],
    };
    assert_eq!(sys_bind(fd as usize, &addr as *const _ as u64, 16), 0);
    let mut name = SockaddrIn::default();
    assert_eq!(sys_getsockname(fd as usize, &mut name as *mut _ as u64, 0), 0);
    assert_eq!(name.port(), 5555);
    sys_close_socket(fd as usize);
    serial_println!("[test] UDP socket bind+getsockname OK");
}

#[test_case]
fn test_socket_eaddrinuse() {
    use mykernel::net::socket::*;
    let fd1 = sys_socket(AF_INET, SOCK_STREAM, 0);
    let fd2 = sys_socket(AF_INET, SOCK_STREAM, 0);
    let addr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   4444u16.to_be_bytes(),
        sin_addr:   [0; 4],
        _pad:       [0; 8],
    };
    assert_eq!(sys_bind(fd1 as usize, &addr as *const _ as u64, 16), 0);
    assert_eq!(sys_bind(fd2 as usize, &addr as *const _ as u64, 16), EADDRINUSE);
    sys_close_socket(fd1 as usize);
    sys_close_socket(fd2 as usize);
    serial_println!("[test] EADDRINUSE on duplicate port OK");
}

#[test_case]
fn test_socket_setsockopt() {
    use mykernel::net::socket::*;
    let fd = sys_socket(AF_INET, SOCK_STREAM, 0);
    let val: i32 = 1;
    let ret = sys_setsockopt(fd as usize, SOL_SOCKET, SO_REUSEADDR,
        &val as *const _ as u64, 4);
    assert_eq!(ret, 0);
    sys_close_socket(fd as usize);
    serial_println!("[test] setsockopt SO_REUSEADDR OK");
}
