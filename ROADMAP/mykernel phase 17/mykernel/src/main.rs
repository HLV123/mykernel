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
    println!("Phase 17: FAT32 Filesystem");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    mykernel::fs::init();
    println!("[ok] VFS + initramfs");

    // Táº¡o FAT32 test image vÃ  mount táº¡i /mnt
    let fat32_img = mykernel::fs::fat32::create_test_fat32_image();
    println!("[ok] FAT32 image: {} bytes", fat32_img.len());
    mykernel::fs::mount_fat32_image(fat32_img, "/mnt");
    println!("[ok] FAT32 mounted at /mnt");

    demo_fat32();

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("Shell: try 'ls /mnt', 'cat /mnt/HELLO.TXT', 'ls /mnt/DOCS'");
        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_fat32() {
    use mykernel::fs::{readdir, open, stat, OpenFlags, FileType};

    println!("");
    println!("=== FAT32 Demo ===");

    // List root of FAT32
    match readdir("/mnt") {
        Ok(entries) => {
            println!("ls /mnt:");
            for e in &entries {
                let t = match e.file_type { FileType::Directory => 'd', _ => '-' };
                println!("  {}{:<20} {:>8} bytes", t, e.name, e.size);
            }
        }
        Err(e) => println!("ls /mnt error: {:?}", e),
    }

    // Read HELLO.TXT
    println!("");
    match open("/mnt/HELLO.TXT", OpenFlags::READ) {
        Ok(f) => {
            let mut buf = [0u8; 128];
            match f.lock().read(&mut buf) {
                Ok(n) => {
                    let s = core::str::from_utf8(&buf[..n]).unwrap_or("(err)");
                    print!("cat /mnt/HELLO.TXT:\n{}", s);
                }
                Err(e) => println!("read error: {:?}", e),
            }
        }
        Err(e) => println!("open /mnt/HELLO.TXT error: {:?}", e),
    }

    // List /mnt/DOCS
    println!("");
    match readdir("/mnt/DOCS") {
        Ok(entries) => {
            println!("ls /mnt/DOCS:");
            for e in &entries {
                let t = match e.file_type { FileType::Directory => 'd', _ => '-' };
                println!("  {}{:<20} {:>8} bytes", t, e.name, e.size);
            }
        }
        Err(e) => println!("ls /mnt/DOCS error: {:?}", e),
    }

    // Read /mnt/DOCS/README.TXT
    match open("/mnt/DOCS/README.TXT", OpenFlags::READ) {
        Ok(f) => {
            let mut buf = [0u8; 128];
            match f.lock().read(&mut buf) {
                Ok(n) => {
                    let s = core::str::from_utf8(&buf[..n]).unwrap_or("(err)");
                    print!("cat /mnt/DOCS/README.TXT:\n{}", s);
                }
                Err(e) => println!("read error: {:?}", e),
            }
        }
        Err(e) => println!("open error: {:?}", e),
    }

    // Stat
    if let Ok(s) = stat("/mnt/HELLO.TXT") {
        println!("stat /mnt/HELLO.TXT: {} bytes", s.size);
    }

    println!("=== End FAT32 ===");
    serial_println!("[fat32] Demo OK");
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
fn test_fat32_mount() {
    mykernel::fs::init();
    let img = mykernel::fs::fat32::create_test_fat32_image();
    assert!(img.len() == 256 * 512);
    // Check BPB signature
    assert_eq!(img[510], 0x55);
    assert_eq!(img[511], 0xAA);
    // Check FS type string
    assert_eq!(&img[82..90], b"FAT32   ");
    serial_println!("[test] FAT32 image structure OK");
}

#[test_case]
fn test_fat32_readdir() {
    mykernel::fs::init();
    let img = mykernel::fs::fat32::create_test_fat32_image();
    mykernel::fs::mount_fat32_image(img, "/mnt");

    use mykernel::fs::readdir;
    let entries = readdir("/mnt").expect("readdir /mnt failed");
    serial_println!("[test] /mnt has {} entries", entries.len());
    // Should have HELLO.TXT and DOCS
    let names: alloc::vec::Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case("HELLO.TXT")),
        "HELLO.TXT not found in {:?}", names);
    serial_println!("[test] FAT32 readdir OK");
}

#[test_case]
fn test_fat32_read_file() {
    mykernel::fs::init();
    let img = mykernel::fs::fat32::create_test_fat32_image();
    mykernel::fs::mount_fat32_image(img, "/mnt2");

    use mykernel::fs::{open, OpenFlags};
    let f = open("/mnt2/HELLO.TXT", OpenFlags::READ).expect("open failed");
    let mut buf = [0u8; 64];
    let n = f.lock().read(&mut buf).expect("read failed");
    assert!(n > 0, "empty file");
    let content = core::str::from_utf8(&buf[..n]).unwrap_or("");
    assert!(content.contains("FAT32"), "content: {}", content);
    serial_println!("[test] FAT32 read file OK: {} bytes", n);
}
