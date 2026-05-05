#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(naked_functions)]
#![test_runner(mykernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use mykernel::println;
use mykernel::serial_println;
use mykernel::memory::BootInfoFrameAllocator;
use mykernel::task::{executor::Executor, Task};
use x86_64::VirtAddr;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 14: VFS Layer");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    // Khởi tạo VFS — mount ramfs + devfs
    mykernel::fs::init();

    println!("[ok] VFS initialized");
    println!("[ok] Mounted: / (ramfs), /dev (devfs)");

    // Demo VFS operations
    demo_vfs();

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("Starting shell with VFS support...");
        println!("New commands: ls, cat, write");
        println!("");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_vfs() {
    use mykernel::fs::{open, create, stat, readdir, mkdir, OpenFlags, FileType};

    println!("");
    println!("=== VFS Demo ===");

    // List root directory
    match readdir("/") {
        Ok(entries) => {
            println!("ls /:");
            for e in &entries {
                let type_char = match e.file_type {
                    FileType::Directory => 'd',
                    _ => '-',
                };
                println!("  {}{} {}", type_char, e.name, e.size);
            }
        }
        Err(e) => println!("ls / error: {:?}", e),
    }

    // Read /etc/motd
    match open("/etc/motd", OpenFlags::READ) {
        Ok(file) => {
            let mut buf = [0u8; 64];
            match file.lock().read(&mut buf) {
                Ok(n) => {
                    let s = core::str::from_utf8(&buf[..n]).unwrap_or("(invalid utf8)");
                    println!("cat /etc/motd: {}", s.trim());
                }
                Err(e) => println!("read error: {:?}", e),
            }
        }
        Err(e) => println!("open /etc/motd error: {:?}", e),
    }

    // Write a new file
    match create("/tmp/hello.txt") {
        Ok(file) => {
            let _ = file.lock().write(b"Hello from kernel!\n");
            println!("write /tmp/hello.txt: OK");
        }
        Err(e) => println!("create error: {:?}", e),
    }

    // Read it back
    match open("/tmp/hello.txt", OpenFlags::READ) {
        Ok(file) => {
            let mut buf = [0u8; 32];
            match file.lock().read(&mut buf) {
                Ok(n) => {
                    let s = core::str::from_utf8(&buf[..n]).unwrap_or("(err)");
                    println!("cat /tmp/hello.txt: {}", s.trim());
                }
                Err(e) => println!("read error: {:?}", e),
            }
        }
        Err(e) => println!("open error: {:?}", e),
    }

    // Test /dev/zero
    match open("/dev/zero", OpenFlags::READ) {
        Ok(file) => {
            let mut buf = [0xffu8; 4];
            let _ = file.lock().read(&mut buf);
            println!("/dev/zero read: {:?} (all zeros)", &buf);
        }
        Err(e) => println!("/dev/zero error: {:?}", e),
    }

    println!("=== VFS Demo Done ===");
    println!("");

    serial_println!("[vfs] Demo completed successfully");
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
fn panic(info: &PanicInfo) -> ! {
    mykernel::test_panic_handler(info)
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

#[test_case]
fn test_vfs_ramfs_write_read() {
    mykernel::fs::init();
    use mykernel::fs::{create, open, OpenFlags};

    let file = create("/test_file").expect("create failed");
    file.lock().write(b"test data").expect("write failed");
    drop(file);

    let file = open("/test_file", OpenFlags::READ).expect("open failed");
    let mut buf = [0u8; 16];
    let n = file.lock().read(&mut buf).expect("read failed");
    assert_eq!(&buf[..n], b"test data");
    serial_println!("[test] ramfs write+read OK");
}

#[test_case]
fn test_vfs_devnull() {
    mykernel::fs::init();
    use mykernel::fs::{open, OpenFlags};

    let file = open("/dev/null", OpenFlags::WRITE).expect("open /dev/null failed");
    let n = file.lock().write(b"discard this").expect("write failed");
    assert_eq!(n, 12);
    serial_println!("[test] /dev/null OK");
}

#[test_case]
fn test_vfs_stat() {
    mykernel::fs::init();
    use mykernel::fs::{stat, FileType};

    let s = stat("/etc/hostname").expect("stat failed");
    assert_eq!(s.file_type, FileType::RegularFile);
    assert!(s.size > 0);
    serial_println!("[test] stat /etc/hostname OK: size={}", s.size);
}
