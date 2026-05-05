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
use mykernel::process::{AddressSpace, Process};
use x86_64::{registers::control::Cr3, VirtAddr};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 12: Virtual Address Space");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    mykernel::usermode::init_syscalls();
    mykernel::process::set_phys_mem_offset(boot_info.physical_memory_offset);

    println!("[ok] Heap, syscalls initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        let (kernel_l4_frame, _) = Cr3::read();
        let kernel_l4_vaddr = phys_mem_offset + kernel_l4_frame.start_address().as_u64();
        let kernel_l4_table = unsafe {
            &*(kernel_l4_vaddr.as_ptr::<x86_64::structures::paging::PageTable>())
        };

        // Process A: dung L4[0] space (0x400000)
        let mut addr_space_a = AddressSpace::new(
            &mut frame_allocator, phys_mem_offset, kernel_l4_table);
        mykernel::user_program::setup_user_memory_in(
            &mut addr_space_a, &mut frame_allocator, phys_mem_offset,
            b"Hello from Process A!\n          ",
        );
        let proc_a = Process::new(
            mykernel::user_program::USER_CODE_ADDR,
            mykernel::user_program::USER_STACK_ADDR as u64,
            addr_space_a,
        );
        let pid_a = mykernel::process::add_process(proc_a);
        println!("[ok] Process A: PID={}", pid_a.as_u64());

        // Process B: dung L4[1] space (0x8000400000)
        let mut addr_space_b = AddressSpace::new(
            &mut frame_allocator, phys_mem_offset, kernel_l4_table);
        mykernel::user_program::setup_user_memory_b(
            &mut addr_space_b, &mut frame_allocator, phys_mem_offset,
            b"Hello from Process B!\n          ",
        );
        let proc_b = Process::new(
            mykernel::user_program::USER_CODE_ADDR_B,
            mykernel::user_program::USER_STACK_ADDR_B as u64,
            addr_space_b,
        );
        let pid_b = mykernel::process::add_process(proc_b);
        println!("[ok] Process B: PID={}", pid_b.as_u64());

        println!("");
        println!("Activating Process A and running...");
        serial_println!("[proc] Switching to Process A CR3...");

        let cr3_a = mykernel::process::PROCESSES.lock()[0]
            .address_space.l4_frame.start_address().as_u64();
        serial_println!("[proc] Process A CR3 = {:#x}", cr3_a);
        mykernel::process::PROCESSES.lock()[0].address_space.activate();
        serial_println!("[proc] CR3 switched OK");

        mykernel::usermode::enter_user_mode_with_stack(
            mykernel::user_program::USER_CODE_ADDR,
            mykernel::user_program::USER_STACK_ADDR as u64,
        )
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
fn panic(info: &PanicInfo) -> ! {
    mykernel::test_panic_handler(info)
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

#[test_case]
fn test_address_space_creation() {
    use x86_64::registers::control::Cr3;
    let (frame, _) = Cr3::read();
    assert!(frame.start_address().as_u64() > 0);
}
