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
use mykernel::elf_loader;
use x86_64::{registers::control::Cr3, VirtAddr};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 13: ELF Loader");

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

    println!("[ok] Kernel initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        // Lấy kernel L4 table
        let (kernel_l4_frame, _) = Cr3::read();
        let kernel_l4_vaddr = phys_mem_offset + kernel_l4_frame.start_address().as_u64();
        let kernel_l4_table = unsafe {
            &*(kernel_l4_vaddr.as_ptr::<x86_64::structures::paging::PageTable>())
        };

        // Tạo ELF binary trong memory (minimal test binary)
        println!("[ok] Building test ELF binary...");
        let elf_data = elf_loader::create_test_elf();
        println!("[ok] ELF binary: {} bytes", elf_data.len());
        serial_println!("[elf] Binary created: {} bytes", elf_data.len());

        // Tạo address space cho process
        let mut addr_space = AddressSpace::new(
            &mut frame_allocator, phys_mem_offset, kernel_l4_table);

        // Load ELF vào address space
        println!("[ok] Loading ELF into address space...");
        let loaded = elf_loader::load_elf(
            &elf_data,
            &mut addr_space,
            &mut frame_allocator,
            phys_mem_offset,
        ).expect("ELF load failed");

        println!("[ok] ELF loaded:");
        println!("     Entry point: {:#x}", loaded.entry_point);
        println!("     Stack top:   {:#x}", loaded.stack_top);

        // Tạo process và activate address space
        let process = Process::new(
            loaded.entry_point,
            loaded.stack_top,
            addr_space,
        );
        let pid = mykernel::process::add_process(process);
        println!("[ok] Process created: PID={}", pid.as_u64());

        println!("");
        println!("Activating process address space...");
        println!("Running ELF binary in Ring 3:");
        println!("");

        mykernel::process::PROCESSES.lock()[0].address_space.activate();
        serial_println!("[proc] Address space activated, entering ring 3");

        mykernel::usermode::enter_user_mode_with_stack(
            loaded.entry_point,
            loaded.stack_top,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

#[test_case]
fn test_elf_header_parse() {
    let elf = elf_loader::create_test_elf();
    // Verify ELF magic
    assert_eq!(&elf[0..4], &[0x7f, b'E', b'L', b'F']);
    // Verify it's 64-bit
    assert_eq!(elf[4], 2);
    // Verify little-endian
    assert_eq!(elf[5], 1);
    // Verify x86_64 (e_machine at offset 18 = 0x3e = 62)
    assert_eq!(elf[18], 62);
    assert_eq!(elf[19], 0);
    serial_println!("[test] ELF header valid");
}

#[test_case]
fn test_elf_program_headers() {
    let elf = elf_loader::create_test_elf();
    // e_phnum at offset 56 = 2
    let phnum = u16::from_le_bytes([elf[56], elf[57]]);
    assert_eq!(phnum, 2);
    // First phdr at offset 0x40, p_type = PT_LOAD = 1
    let p_type = u32::from_le_bytes([elf[0x40], elf[0x41], elf[0x42], elf[0x43]]);
    assert_eq!(p_type, 1);
    serial_println!("[test] ELF program headers valid");
}
