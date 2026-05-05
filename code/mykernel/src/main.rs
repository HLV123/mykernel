// MyKernel — Bare-metal x86_64 OS kernel written in Rust
//
// Entry point: kernel_main() is called by the bootloader after setting up
// a basic environment (64-bit mode, page tables, stack).
//
// Boot sequence:
//   1. GDT + IDT + PIC  — CPU tables and interrupt controller
//   2. Memory           — physical frame allocator + virtual address mapper
//   3. Heap             — dynamic allocator (Box, Vec, Arc, etc.)
//   4. Filesystem       — VFS + initramfs + devfs
//   5. Drivers          — virtio-blk + virtio-net
//   6. Network stack    — ARP, IPv4, ICMP, UDP, TCP, Socket API
//   7. Security         — stack canary, KASLR, pointer validation, capabilities
//   8. SMP              — APIC init, multi-core detection
//   9. Shell            — interactive user interface

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

// Register kernel_main as the bootloader entry point.
// The bootloader calls this after setting up 64-bit mode and basic paging.
entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // ------------------------------------------------------------------
    // Step 1: CPU tables
    // GDT  — segment descriptors (kernel/user code + data, TSS)
    // IDT  — exception and interrupt handlers
    // PIC  — 8259 programmable interrupt controller (timer, keyboard, …)
    // ------------------------------------------------------------------
    mykernel::init();

    // ------------------------------------------------------------------
    // Step 2: Virtual memory
    // Map all physical RAM into the kernel's higher-half virtual space.
    // Build a FrameAllocator to hand out physical 4-KiB pages on demand.
    // ------------------------------------------------------------------
    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    // ------------------------------------------------------------------
    // Step 3: Kernel heap
    // Carve out a 512 KiB virtual region for the heap so that Box, Vec,
    // Arc, String, and BTreeMap all work in the kernel.
    // ------------------------------------------------------------------
    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap initialization failed");

    // ------------------------------------------------------------------
    // Step 4: Filesystem layer
    // Mount an in-memory filesystem hierarchy:
    //   /         — RamFS (read-write in RAM)
    //   /dev      — DevFS (null, zero, serial pseudo-devices)
    //   /         — initramfs CPIO archive unpacked on top of RamFS
    //               provides /bin, /etc, /README, etc.
    // ------------------------------------------------------------------
    mykernel::fs::init();

    // ------------------------------------------------------------------
    // Step 5: Process infrastructure
    // Store the physical-to-virtual offset so the process module can
    // translate physical frame addresses when building user page tables.
    // ------------------------------------------------------------------
    mykernel::process::set_phys_mem_offset(boot_info.physical_memory_offset);

    // ------------------------------------------------------------------
    // Step 6: Syscall interface (Linux x86_64 ABI)
    // Enable the SYSCALL / SYSRET fast-path via IA32_LSTAR / IA32_STAR
    // MSRs.  Install a 40-entry dispatch table that mirrors the Linux
    // x86_64 ABI (read=0, write=1, open=2, close=3, mmap=9, …).
    // ------------------------------------------------------------------
    mykernel::usermode::init_syscalls();

    // ------------------------------------------------------------------
    // Step 7: Hardware drivers
    // Scan the PCI bus for virtio devices:
    //   virtio-blk (device 0x1001) — block storage
    //   virtio-net (device 0x1000) — ethernet NIC
    // ------------------------------------------------------------------
    mykernel::drivers::init();

    // ------------------------------------------------------------------
    // Step 8: Network stack
    // If virtio-net is present, configure the stack and bring it up.
    // Default IP: 10.0.2.15 (QEMU user-mode network).
    // Supported protocols: ARP, IPv4, ICMP, UDP, TCP, POSIX sockets.
    // ------------------------------------------------------------------
    mykernel::net::init();

    // ------------------------------------------------------------------
    // Step 9: Security subsystem
    //   • xoshiro256** CSPRNG seeded from RDTSC
    //   • Stack canary initialised with RDTSC-derived value
    //   • KASLR offset computed at boot time
    //   • CR4.SMEP / CR4.SMAP / CR4.UMIP enabled when the CPU supports them
    //   • Hardened security policy applied (no raw sockets, kptr_restrict, …)
    // ------------------------------------------------------------------
    mykernel::security::init();

    // ------------------------------------------------------------------
    // Step 10: APIC and SMP
    // Initialise the BSP's Local APIC (replaces the legacy PIC for
    // timer delivery) and the I/O APIC.  Parse ACPI MADT to discover
    // Application Processors; send INIT + SIPI IPIs to boot them.
    // ------------------------------------------------------------------
    let phys_offset_u64 = boot_info.physical_memory_offset;
    mykernel::apic::init_lapic(phys_offset_u64);
    mykernel::apic::init_ioapic(phys_offset_u64);

    // ------------------------------------------------------------------
    // Test harness path
    // When the crate is compiled for testing (`cargo test`), jump into
    // the generated test runner instead of the interactive shell.
    // ------------------------------------------------------------------
    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    // ------------------------------------------------------------------
    // Normal boot path — interactive shell
    // Print a status banner, then hand control to the async shell task.
    // ------------------------------------------------------------------
    #[cfg(not(test))]
    {
        print_banner();

        // Spawn the shell as an async task.  The executor cooperatively
        // drives all pending futures; it sleeps the CPU with HLT when
        // there is nothing to do, waking on the next hardware interrupt.
        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

// ---------------------------------------------------------------------------
// Boot banner
// ---------------------------------------------------------------------------

/// Print a one-time banner summarising the state of all subsystems.
/// Called only in normal (non-test) boot mode.
#[cfg(not(test))]
fn print_banner() {
    println!("");
    println!("  __  __       _  __                    _");
    println!(" |  \\/  |_   _| |/ /___ _ __ _ __   ___| |");
    println!(" | |\\/| | | | | ' // _ \\ '__| '_ \\ / _ \\ |");
    println!(" | |  | | |_| | . \\  __/ |  | | | |  __/ |");
    println!(" |_|  |_|\\__, |_|\\_\\___|_|  |_| |_|\\___|_|");
    println!("          |___/");
    println!("");
    println!("  Bare-metal OS kernel  |  Rust  |  x86_64");
    println!("");

    // --- CPU and memory ---
    println!("  [ok] GDT / IDT / PIC");
    println!("  [ok] Virtual memory + heap");
    println!("  [ok] Filesystem  (VFS + initramfs)");
    println!("  [ok] Syscalls    (Linux x86_64 ABI, 40 calls)");

    // --- Block device ---
    if let Some(sectors) = mykernel::drivers::virtio_blk::num_sectors() {
        println!("  [ok] virtio-blk  ({} MiB)", sectors * 512 / 1024 / 1024);
    } else {
        println!("  [ ] virtio-blk   not found");
        println!("      tip: add -drive format=raw,file=disk.img,if=virtio");
    }

    // --- Network device ---
    if let Some(mac) = mykernel::drivers::virtio_net::get_mac() {
        let ip = mykernel::net::our_ip();
        println!("  [ok] virtio-net  MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  \
                  IP {}.{}.{}.{}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
            ip[0], ip[1], ip[2], ip[3]);
        println!("       ping 10.0.2.15 from host to reach this kernel");
    } else {
        println!("  [ ] virtio-net   not found");
        println!("      tip: add -netdev user,id=n0 -device virtio-net-pci,netdev=n0");
    }

    // --- Security ---
    let audit = mykernel::security::audit();
    println!("  [ok] Security    score {}/100  (SMEP={} SMAP={} NX={} canary={})",
        audit.score(),
        audit.smep_enabled,
        audit.smap_enabled,
        audit.nx_enabled,
        audit.canary_set);

    // --- SMP ---
    let bsp_id = mykernel::apic::lapic_id();
    let cpu_count = mykernel::smp::cpu_count();
    println!("  [ok] APIC        BSP ID={}  {} CPU(s) detected", bsp_id, cpu_count);

    println!("");
    println!("  Type 'help' for available commands.");
    println!("");
}

// ---------------------------------------------------------------------------
// Panic handlers
// ---------------------------------------------------------------------------

/// Production panic handler — print the message and halt.
/// The kernel never returns from a panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("");
    println!("--- KERNEL PANIC ---");
    println!("{}", info);
    serial_println!("KERNEL PANIC: {}", info);
    mykernel::hlt_loop()
}

/// Test panic handler — signal test failure to QEMU and exit.
#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    mykernel::test_panic_handler(info)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// CPU exception handling: a breakpoint must be caught without crashing.
#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

/// Full boot smoke test: reaching this line means every init step succeeded.
#[test_case]
fn test_full_boot() {
    serial_println!("[test] full boot ok");
}
