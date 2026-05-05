#![no_std]
#![no_main]
extern crate alloc;

use alloc::boxed::Box;
use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use mykernel::{exit_qemu, serial_println, QemuExitCode};

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

    // Verify heap works
    let x = Box::new(42u32);
    assert_eq!(*x, 42);

    serial_println!("[basic_boot] heap allocation OK");
    exit_qemu(QemuExitCode::Success);
    mykernel::hlt_loop()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    mykernel::test_panic_handler(info)
}
