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
    println!("Phase 16: Virtio Block Driver");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    // Khá»Ÿi táº¡o VFS + initramfs
    mykernel::fs::init();
    println!("[ok] VFS + initramfs");

    // Khá»Ÿi táº¡o drivers (bao gá»“m virtio-blk)
    mykernel::drivers::init();

    // Demo
    demo_block_driver();

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("Shell ready (new command: disk)");
        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_block_driver() {
    use mykernel::drivers::{num_sectors, read_sector, SECTOR_SIZE};

    println!("");
    println!("=== Virtio Block Driver ===");

    match num_sectors() {
        Some(n) => {
            println!("[ok] Disk found: {} sectors ({} MiB)",
                n, n * 512 / 1024 / 1024);

            // Read sector 0 (MBR/boot sector)
            let mut buf = [0u8; SECTOR_SIZE];
            match read_sector(0, &mut buf) {
                Ok(()) => {
                    println!("[ok] Sector 0 read successfully");
                    // Show first 16 bytes as hex
                    print!("     First 16 bytes: ");
                    for i in 0..16 {
                        print!("{:02x} ", buf[i]);
                    }
                    println!("");

                    // Check MBR signature
                    if buf[510] == 0x55 && buf[511] == 0xAA {
                        println!("[ok] MBR signature found (0x55AA)");
                    } else {
                        println!("[?] No MBR signature (raw disk or blank)");
                    }
                }
                Err(e) => println!("[!] Read sector 0 failed: {}", e),
            }
        }
        None => {
            println!("[!] No virtio-blk device found");
            println!("    To enable: add -drive format=raw,file=disk.img,if=virtio");
            println!("    Create test disk: qemu-img create -f raw disk.img 64M");
            serial_println!("[virtio-blk] Demo: no device (expected without disk image)");
        }
    }

    println!("=== End Block Driver ===");
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
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

#[test_case]
fn test_pci_scan() {
    // PCI scan khÃ´ng crash â€” Ä‘Ã¢y lÃ  smoke test
    use mykernel::drivers::pci::scan_pci_bus;
    let devices = scan_pci_bus();
    serial_println!("[test] PCI scan found {} devices", devices.len());
    // In cÃ¡c devices tÃ¬m Ä‘Æ°á»£c
    for d in &devices {
        serial_println!("[test]   {:04x}:{:04x} at {:02x}:{:02x}.{}",
            d.vendor_id, d.device_id, d.bus, d.dev, d.func);
    }
    // QEMU luÃ´n cÃ³ Ã­t nháº¥t 1 PCI device (PIIX3/PIIX4)
    assert!(devices.len() > 0, "no PCI devices found");
}

#[test_case]
fn test_virtio_blk_optional() {
    // Test nÃ y pass dÃ¹ cÃ³ hay khÃ´ng cÃ³ virtio-blk device
    use mykernel::drivers::virtio_blk;

    // Init driver (may or may not find device)
    let found = virtio_blk::init();
    serial_println!("[test] virtio-blk found: {}", found);

    if found {
        let n = virtio_blk::num_sectors().unwrap_or(0);
        serial_println!("[test] Disk: {} sectors", n);

        if n > 0 {
            let mut buf = [0u8; mykernel::drivers::SECTOR_SIZE];
            match virtio_blk::read_sector(0, &mut buf) {
                Ok(()) => serial_println!("[test] Read sector 0 OK"),
                Err(e) => serial_println!("[test] Read sector 0 error: {}", e),
            }
        }
    }
    // Always pass â€” virtio-blk is optional
}

