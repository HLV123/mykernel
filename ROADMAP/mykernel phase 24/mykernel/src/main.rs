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
    println!("Phase 24: Security Hardening");

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
    mykernel::drivers::init();
    mykernel::net::init();

    // Initialize security subsystem
    mykernel::security::init();

    println!("[ok] All subsystems initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("╔══════════════════════════════════════╗");
        println!("║    Phase 24: Security Hardening       ║");
        println!("╚══════════════════════════════════════╝");

        demo_security();
        print_final_summary();

        println!("");
        println!("MyKernel is fully operational!");
        println!("Starting interactive shell...");
        println!("");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_security() {
    use mykernel::security::*;

    println!("");
    println!("[ Security Features ]");
    println!("");

    // 1. Stack Canary
    let canary = get_stack_canary();
    println!("Stack Canary:     {:#018x}", canary);
    assert!(canary != 0, "Canary should be non-zero");
    assert!(check_stack_canary(canary), "Canary check should pass");
    println!("  [ok] Stack canary initialized and verified");

    // 2. KASLR
    let offset = kaslr_offset();
    println!("KASLR Offset:     +{:#x}", offset);
    println!("  [ok] KASLR offset calculated");

    // 3. Entropy
    let r1 = get_random_u64();
    let r2 = get_random_u64();
    assert_ne!(r1, r2, "Random values should differ");
    println!("CSPRNG:           {:#018x}", r1);
    println!("  [ok] Entropy pool functional");

    // 4. Pointer Validation
    println!("");
    println!("[ Pointer Validation ]");
    // Valid user pointer
    let valid = validate_user_ptr(0x1000, 64);
    println!("  0x1000 (user):  {}", if valid { "ALLOWED" } else { "BLOCKED" });
    // Kernel pointer → should be rejected
    let kernel_ptr = validate_user_ptr(0xFFFF_8000_0000_0000, 1);
    println!("  0xFFFF...(kern):  {}", if kernel_ptr { "ALLOWED" } else { "BLOCKED" });
    // Null pointer → should be rejected
    let null_ptr = validate_user_ptr(0, 1);
    println!("  0x0 (null):     {}", if null_ptr { "ALLOWED" } else { "BLOCKED" });
    assert!(valid, "Valid user ptr should pass");
    assert!(!kernel_ptr, "Kernel ptr should be rejected");
    assert!(!null_ptr, "Null ptr should be rejected");
    println!("  [ok] User pointer validation working");

    // 5. Capabilities
    println!("");
    println!("[ Capability System ]");
    let root_caps = Capabilities::root();
    let user_caps = Capabilities::none();
    println!("  root can bind port 80:  {}", root_caps.can_bind_port(80));
    println!("  user can bind port 80:  {}", user_caps.can_bind_port(80));
    println!("  user can bind port 8080:{}", user_caps.can_bind_port(8080));
    assert!(root_caps.can_bind_port(80));
    assert!(!user_caps.can_bind_port(80));
    assert!(user_caps.can_bind_port(8080));
    println!("  [ok] Capability checks working");

    // 6. Security Audit
    println!("");
    println!("[ Security Audit ]");
    let audit = audit();
    let score = audit.score();

    let check = |enabled: bool| if enabled { "[✓]" } else { "[ ]" };
    println!("  {} SMEP (no exec user pages in kernel)",  check(audit.smep_enabled));
    println!("  {} SMAP (no access user pages in kernel)",check(audit.smap_enabled));
    println!("  {} NX/XD (no-execute data pages)",        check(audit.nx_enabled));
    println!("  {} Stack Canary",                          check(audit.canary_set));
    println!("  {} KASLR",                                 check(audit.kaslr_active));
    println!("  {} RDRAND (hardware RNG)",                 check(audit.rdrand_available));
    println!("  {} Hardened Policy",                        check(audit.hardened_policy));
    println!("");
    println!("  Security Score: {}/100", score);

    if score >= 60 {
        println!("  [ok] Security posture: GOOD");
    } else if score >= 40 {
        println!("  [!] Security posture: FAIR");
    } else {
        println!("  [!] Security posture: WEAK (some features not supported by CPU)");
    }

    serial_println!("[phase24] Security demo complete, score={}/100", score);
}

fn print_final_summary() {
    println!("");
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║            MyKernel — Complete Build Summary              ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Phase  1-3:  Freestanding binary, VGA, serial           ║");
    println!("║  Phase  4-6:  Exceptions, interrupts, paging             ║");
    println!("║  Phase  7-9:  Heap, async executor, keyboard shell       ║");
    println!("║  Phase 10-11: Preemptive scheduler, Ring 3 user mode     ║");
    println!("║  Phase 12-13: Virtual address spaces, ELF loader         ║");
    println!("║  Phase 14-15: VFS layer, initramfs (CPIO)                ║");
    println!("║  Phase 16-17: Virtio block driver, FAT32 filesystem      ║");
    println!("║  Phase    18: 40 Linux-compatible syscalls               ║");
    println!("║  Phase    19: APIC + SMP multi-core boot                 ║");
    println!("║  Phase    20: SMP-safe locking (SpinLock/RwLock/SeqLock) ║");
    println!("║  Phase    21: Virtio network driver                      ║");
    println!("║  Phase    22: TCP/IP stack (ARP/ICMP/UDP/TCP)            ║");
    println!("║  Phase    23: POSIX Socket API                           ║");
    println!("║  Phase    24: Security hardening (SMEP/SMAP/NX/KASLR)   ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  ~3500 lines of Rust | 24 phases | bare-metal x86_64     ║");
    println!("╚══════════════════════════════════════════════════════════╝");
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
fn test_stack_canary() {
    use mykernel::security::*;
    init_stack_canary();
    let canary = get_stack_canary();
    assert!(canary != 0);
    assert!(check_stack_canary(canary));
    assert!(!check_stack_canary(canary ^ 1)); // Flipped bit = corrupted
    serial_println!("[test] Stack canary OK: {:#x}", canary);
}

#[test_case]
fn test_pointer_validation() {
    use mykernel::security::*;
    // Valid user pointers
    assert!(validate_user_ptr(0x1000, 1));
    assert!(validate_user_ptr(0x1000, 4096));
    assert!(validate_user_ptr(0x7FFF_FFFF_0000, 100));
    // Invalid: null
    assert!(!validate_user_ptr(0, 1));
    // Invalid: kernel space
    assert!(!validate_user_ptr(0xFFFF_8000_0000_0000, 1));
    assert!(!validate_user_ptr(0xFFFF_FFFF_FFFF_FFFF, 1));
    // Invalid: overflow
    assert!(!validate_user_ptr(0x7FFF_FFFF_FF00, 0x1000));
    serial_println!("[test] Pointer validation OK");
}

#[test_case]
fn test_capabilities() {
    use mykernel::security::*;
    let mut caps = Capabilities::root();
    assert!(caps.has(CAP_SYS_ADMIN));
    assert!(caps.has(CAP_NET_BIND));
    assert!(caps.can_bind_port(80));

    // Drop a capability
    caps.drop(CAP_NET_BIND);
    assert!(!caps.has(CAP_NET_BIND));
    assert!(!caps.can_bind_port(80));
    assert!(caps.can_bind_port(8080));

    // Unprivileged
    let user = Capabilities::none();
    assert!(!user.has(CAP_SYS_ADMIN));
    assert!(!user.can_bind_port(1));
    assert!(user.can_bind_port(1024));

    serial_println!("[test] Capabilities OK");
}

#[test_case]
fn test_csprng() {
    use mykernel::security::*;
    init_entropy();

    // Generate 8 random values, all should be different
    let vals: [u64; 8] = core::array::from_fn(|_| get_random_u64());
    let mut unique = true;
    for i in 0..8 {
        for j in (i+1)..8 {
            if vals[i] == vals[j] { unique = false; }
        }
    }
    assert!(unique, "Random values should be unique");

    // fill_random test
    let mut buf = [0u8; 16];
    fill_random(&mut buf);
    let all_zero = buf.iter().all(|&b| b == 0);
    assert!(!all_zero, "Random buffer should not be all zeros");

    serial_println!("[test] CSPRNG OK");
}

#[test_case]
fn test_security_audit() {
    use mykernel::security::*;
    init_entropy();
    init_stack_canary();
    init_kaslr();
    let audit = audit();
    // These should always be true after init
    assert!(audit.canary_set, "Canary should be set");
    assert!(audit.kaslr_active, "KASLR should be active");
    assert!(audit.nx_enabled, "NX should be enabled");
    let score = audit.score();
    assert!(score >= 30, "Security score should be >= 30, got {}", score);
    serial_println!("[test] Security audit OK, score={}/100", score);
}
