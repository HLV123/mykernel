#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(naked_functions)]
#![test_runner(mykernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use mykernel::{print, println, serial_println};
use mykernel::memory::BootInfoFrameAllocator;
use mykernel::task::{executor::Executor, Task};
use x86_64::VirtAddr;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 15: initramfs (CPIO)");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    mykernel::fs::init();
    println!("[ok] VFS + initramfs loaded");

    demo_initramfs();

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("Shell ready. Try: ls /, ls /bin, cat /etc/motd");
        println!("");
        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_initramfs() {
    use mykernel::fs::{readdir, open, stat, OpenFlags, FileType};

    println!("");
    println!("=== initramfs Contents ===");

    for dir in &["/", "/bin", "/etc"] {
        match readdir(dir) {
            Ok(entries) if !entries.is_empty() => {
                println!("{}:", dir);
                for e in &entries {
                    let t = match e.file_type { FileType::Directory => 'd', _ => '-' };
                    println!("  {}{:<20} {:>6} bytes", t, e.name, e.size);
                }
            }
            _ => {}
        }
    }

    println!("");
    if let Ok(f) = open("/etc/motd", OpenFlags::READ) {
        let mut buf = [0u8; 128];
        if let Ok(n) = f.lock().read(&mut buf) {
            let s = core::str::from_utf8(&buf[..n]).unwrap_or("");
            print!("{}", s);
        }
    }

    for path in ["/bin/init", "/bin/hello", "/README"] {
        if let Ok(s) = stat(path) {
            println!("stat {}: {} bytes", path, s.size);
        }
    }

    println!("=== End initramfs ===");
    serial_println!("[initramfs] Demo OK");
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

#[test_case]
fn test_breakpoint_exception() { x86_64::instructions::interrupts::int3(); }

#[test_case]
fn test_cpio_builder() {
    use mykernel::fs::initramfs::{CpioBuilder, CpioIterator};
    let archive = CpioBuilder::new()
        .add_file("hello.txt", b"hello world")
        .add_dir("mydir")
        .build();
    let mut found = false;
    let mut count = 0;
    for entry in CpioIterator::new(&archive) {
        if entry.name == "hello.txt" { assert_eq!(entry.data, b"hello world"); found = true; }
        count += 1;
    }
    assert!(found);
    assert!(count >= 2);
    serial_println!("[test] CPIO OK: {} entries", count);
}

#[test_case]
fn test_initramfs_load() {
    mykernel::fs::init();
    use mykernel::fs::{open, stat, OpenFlags, FileType};
    let s = stat("/etc/os-release").expect("stat failed");
    assert_eq!(s.file_type, FileType::RegularFile);
    assert!(s.size > 0);
    let f = open("/etc/os-release", OpenFlags::READ).expect("open failed");
    let mut buf = [0u8; 64];
    let n = f.lock().read(&mut buf).expect("read failed");
    let content = core::str::from_utf8(&buf[..n]).unwrap_or("");
    assert!(content.contains("MyKernel"));
    serial_println!("[test] initramfs load OK");
}

#[test_case]
fn test_initramfs_dirs() {
    mykernel::fs::init();
    use mykernel::fs::readdir;
    let entries = readdir("/bin").expect("readdir /bin failed");
    let names: alloc::vec::Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"hello"));
    assert!(names.contains(&"init"));
    serial_println!("[test] initramfs dirs OK");
}
