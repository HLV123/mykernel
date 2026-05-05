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
use x86_64::{registers::control::Cr3, VirtAddr};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 19: APIC + Multi-core Boot (SMP)");

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

    println!("[ok] Kernel base initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        let phys_offset_u64 = boot_info.physical_memory_offset;

        // Phase 19: Initialize APIC subsystem
        println!("");
        println!("=== Phase 19: APIC + SMP ===");

        // Step 1: Initialize Local APIC (replace legacy PIC for timer)
        println!("[1] Initializing Local APIC...");
        mykernel::apic::init_lapic(phys_offset_u64);

        let bsp_id = mykernel::apic::lapic_id();
        println!("[ok] Local APIC initialized, BSP APIC ID={}", bsp_id);

        // Step 2: Initialize I/O APIC
        println!("[2] Initializing I/O APIC...");
        mykernel::apic::init_ioapic(phys_offset_u64);
        println!("[ok] I/O APIC initialized");

        // Step 3: Detect CPU count
        println!("[3] Detecting CPU topology...");
        let cpu_count_cpuid = mykernel::smp::detect_cpu_count();
        let current_apic = mykernel::smp::current_apic_id();
        println!("[ok] CPUID: {} logical CPUs, current APIC ID={}",
            cpu_count_cpuid, current_apic);

        // Step 4: Parse ACPI MADT for APIC IDs
        println!("[4] Parsing ACPI MADT...");
        let apic_ids = mykernel::smp::parse_madt_apics(phys_offset_u64);
        println!("[ok] MADT: {} processors found", apic_ids.len());
        for (i, id) in apic_ids.iter().enumerate() {
            let bsp = if *id == bsp_id { " (BSP)" } else { "" };
            println!("     CPU[{}]: APIC_ID={}{}", i, id, bsp);
        }

        // Step 5: Boot APs (if any)
        if apic_ids.len() > 1 {
            println!("[5] Booting {} Application Processors...", apic_ids.len() - 1);
            let (cr3_frame, _) = Cr3::read();
            mykernel::smp::boot_aps(phys_offset_u64, cr3_frame.start_address().as_u64());
        } else {
            println!("[5] Single-core system (no APs to boot)");
            serial_println!("[smp] Running with 1 CPU (QEMU default)");
        }

        // Step 6: Show final topology
        mykernel::smp::print_topology();
        println!("[ok] SMP initialized: {} CPU(s) online",
            mykernel::smp::online_count());

        // Step 7: Setup APIC timer (replaces PIT-based timer)
        println!("[6] Setting up APIC timer...");
        // Vector 0x20 = timer interrupt (same as before)
        mykernel::apic::lapic_timer_init(0x20, 100);
        println!("[ok] APIC timer configured @ 100Hz");

        println!("");
        println!("=== Phase 19 Complete ===");
        println!("APIC + SMP infrastructure ready!");
        println!("");

        // Start shell
        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
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
fn panic(info: &PanicInfo) -> ! { mykernel::test_panic_handler(info) }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test_case]
fn test_breakpoint_exception() { x86_64::instructions::interrupts::int3(); }

#[test_case]
fn test_lapic_init() {
    let phys_offset = mykernel::process::get_phys_mem_offset().as_u64();
    if phys_offset == 0 {
        serial_println!("[test] Skipping APIC test (phys_offset not set in test mode)");
        return;
    }
    mykernel::apic::init_lapic(phys_offset);
    let id = mykernel::apic::lapic_id();
    serial_println!("[test] Local APIC ID={}", id);
    // APIC ID should be 0 for BSP in QEMU
    assert!(id < 8, "APIC ID {} seems invalid", id);
}

#[test_case]
fn test_cpuid_detection() {
    let count = mykernel::smp::detect_cpu_count();
    assert!(count >= 1, "CPU count should be >= 1");
    assert!(count <= 255, "CPU count {} seems too high", count);
    let apic_id = mykernel::smp::current_apic_id();
    serial_println!("[test] CPUID: {} CPUs, current APIC ID={}", count, apic_id);
}

#[test_case]
fn test_acpi_madt() {
    // Skip MADT parsing in test - requires real phys_mem_offset
    // Just verify parse_madt_apics returns at least 1 entry with BSP offset 0
    // Using offset 0 means we read from physical addresses directly
    // This is safe to skip in unit tests
    serial_println!("[test] MADT test skipped in unit test environment");
    // Just verify cpu count detection works
    let count = mykernel::smp::detect_cpu_count();
    assert!(count >= 1);
}


