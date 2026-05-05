// MyKernel — library crate
//
// All kernel subsystems are organised as modules here and re-exported
// so that both main.rs and the integration tests can use them via the
// `mykernel::` namespace.

#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![feature(naked_functions)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use core::panic::PanicInfo;

// ---------------------------------------------------------------------------
// Kernel subsystem modules
// ---------------------------------------------------------------------------

/// Linked-list heap allocator — enables Box, Vec, Arc in the kernel.
pub mod allocator;

/// Local APIC and I/O APIC drivers (replaces legacy 8259 PIC).
pub mod apic;

/// Hardware device drivers: PCI scanner, virtio-blk, virtio-net.
pub mod drivers;

/// ELF64 binary loader — parses headers and maps PT_LOAD segments.
pub mod elf_loader;

/// Virtual Filesystem: VFS trait, RamFS, DevFS, initramfs, FAT32.
pub mod fs;

/// Global Descriptor Table — kernel/user segments, TSS.
pub mod gdt;

/// Interrupt Descriptor Table + hardware interrupt handlers.
pub mod interrupts;

/// Physical frame allocator + page-table mapper.
pub mod memory;

/// TCP/IP network stack: ARP, IPv4, ICMP, UDP, TCP, POSIX sockets.
pub mod net;

/// Process / address-space management (virtual memory per process).
pub mod process;

/// Preemptive round-robin scheduler with context switching.
pub mod scheduler;

/// Security subsystem: CSPRNG, stack canary, KASLR, capabilities.
pub mod security;

/// Serial port (UART) driver — used for logging and test output.
pub mod serial;

/// Interactive kernel shell.
pub mod shell;

/// Symmetric Multi-Processing: ACPI MADT parser, AP boot, CPU table.
pub mod smp;

/// SMP-safe synchronisation: SpinLock, RwLock, SeqLock, Once, PerCpu.
pub mod sync;

/// Linux-compatible syscall table (40 calls, x86_64 ABI).
pub mod syscall;

/// Async task executor and keyboard reader.
pub mod task;

/// Ring-3 user mode: SYSCALL/SYSRET handler, enter_user_mode.
pub mod usermode;

/// VGA text-mode output driver (80×25, 16 colours).
pub mod vga_buffer;

// ---------------------------------------------------------------------------
// Top-level kernel initialisation
// ---------------------------------------------------------------------------

/// Initialise the minimum set of CPU structures required for safe operation.
/// Must be called before any interrupts are enabled.
///
/// Steps:
///   1. Load the GDT (segment descriptors for kernel/user code+data, TSS)
///   2. Install the IDT (exception and interrupt handlers)
///   3. Initialise and unmask the 8259 PIC
///   4. Enable CPU interrupts (STI)
pub fn init() {
    gdt::init();
    interrupts::init_idt();
    // SAFETY: called once during boot before any concurrency.
    unsafe { interrupts::PICS.lock().initialize() };
    x86_64::instructions::interrupts::enable();
}

/// Halt the CPU in a loop — used as the final fallback after a panic.
pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Trait implemented for every `fn()` so that test functions can be stored
/// in a `&[&dyn Testable]` slice and dispatched uniformly.
pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T where T: Fn() {
    fn run(&self) {
        // Print the fully-qualified function name over serial before running.
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

/// Custom test runner — called by the `test_main` harness.
/// Runs each test inside QEMU and exits with a success code when done.
pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    // Signal success to QEMU via the isa-debug-exit device.
    x86_64::instructions::interrupts::disable();
    exit_qemu(QemuExitCode::Success);
}

/// Panic handler for test mode — print the failure and exit QEMU.
pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]");
    serial_println!("Error: {}", info);
    x86_64::instructions::interrupts::disable();
    exit_qemu(QemuExitCode::Failed);
    loop {}
}

/// Exit codes mapped to the isa-debug-exit I/O port values.
/// QEMU maps exit code N to process exit code 2*N+1, so:
///   Success (0x10) → process exit 33 (matches test-success-exit-code in Cargo.toml)
///   Failed  (0x11) → process exit 35
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed  = 0x11,
}

/// Write to the isa-debug-exit I/O port to terminate QEMU with a status code.
pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;
    // SAFETY: this port is only connected to the debug-exit device.
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

// ---------------------------------------------------------------------------
// Test-mode entry point and panic handler
// ---------------------------------------------------------------------------

/// Entry point when running `cargo test` (lib tests).
/// Mirrors kernel_main but skips the interactive shell.
#[cfg(test)]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    init();
    test_main();
    hlt_loop()
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}
