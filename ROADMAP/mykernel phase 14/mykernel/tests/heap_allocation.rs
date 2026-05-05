#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use mykernel::{exit_qemu, serial_print, serial_println, QemuExitCode};

entry_point!(test_kernel_main);

fn test_kernel_main(boot_info: &'static BootInfo) -> ! {
    use mykernel::memory::{self, BootInfoFrameAllocator};
    use x86_64::VirtAddr;

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    serial_println!("Running 3 tests");

    serial_print!("simple_allocation...\t");
    let h1 = Box::new(41);
    let h2 = Box::new(13);
    assert_eq!(*h1, 41);
    assert_eq!(*h2, 13);
    serial_println!("[ok]");

    serial_print!("large_vec...\t");
    let n = 1000u64;
    let mut vec = Vec::new();
    for i in 0..n { vec.push(i); }
    assert_eq!(vec.iter().sum::<u64>(), (n - 1) * n / 2);
    serial_println!("[ok]");

    serial_print!("many_boxes...\t");
    for i in 0..1000 {
        let x = Box::new(i);
        assert_eq!(*x, i);
    }
    serial_println!("[ok]");

    x86_64::instructions::interrupts::disable();
    exit_qemu(QemuExitCode::Success);
    mykernel::hlt_loop()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    mykernel::test_panic_handler(info)
}
