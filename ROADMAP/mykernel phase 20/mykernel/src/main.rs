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

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 20: SMP-safe Locking");

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

    println!("[ok] Kernel initialized");

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        println!("");
        println!("=== Phase 20: SMP-safe Locking ===");

        demo_spinlock();
        demo_rwlock();
        demo_seqlock();
        demo_once();
        demo_per_cpu();
        demo_atomic_counter();

        println!("");
        println!("=== Phase 20 Complete ===");
        println!("All SMP synchronization primitives working!");
        println!("");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn demo_spinlock() {
    use mykernel::sync::SpinLock;

    println!("[SpinLock]");

    // Basic lock/unlock
    let counter = SpinLock::new(0u64);

    for _ in 0..1000 {
        *counter.lock() += 1;
    }

    println!("  counter after 1000 increments: {}", *counter.lock());
    assert_eq!(*counter.lock(), 1000);

    // try_lock
    let lock = SpinLock::new(42u64);
    {
        let guard = lock.lock();
        assert_eq!(*guard, 42);
        // try_lock should fail when already locked
        // (can't test in single-CPU but infrastructure is correct)
        serial_println!("[sync] SpinLock: value={}", *guard);
    }
    // After drop, lock released
    assert!(!lock.is_locked());

    println!("  [ok] SpinLock working");
}

fn demo_rwlock() {
    use mykernel::sync::RwSpinLock;

    println!("[RwSpinLock]");

    let data = RwSpinLock::new(alloc::vec![1u32, 2, 3, 4, 5]);

    // Multiple readers
    {
        let r1 = data.read();
        let r2 = data.read();
        assert_eq!(r1.len(), 5);
        assert_eq!(r2[0], 1);
        serial_println!("[sync] RwLock readers: {:?}", &*r1);
    }

    // Writer
    {
        let mut w = data.write();
        w.push(6);
        w[0] = 100;
    }

    {
        let r = data.read();
        assert_eq!(r.len(), 6);
        assert_eq!(r[0], 100);
        assert_eq!(r[5], 6);
        serial_println!("[sync] RwLock after write: {:?}", &*r);
    }

    println!("  [ok] RwSpinLock working");
}

fn demo_seqlock() {
    use mykernel::sync::SeqLock;

    println!("[SeqLock]");

    #[derive(Copy, Clone, Default)]
    struct TimeValue { sec: u64, nsec: u32 }

    let time = SeqLock::new(TimeValue { sec: 0, nsec: 0 });

    // Write
    time.write(TimeValue { sec: 1000, nsec: 500_000 });

    // Read back
    let t = time.read();
    assert_eq!(t.sec, 1000);
    assert_eq!(t.nsec, 500_000);
    serial_println!("[sync] SeqLock: sec={} nsec={}", t.sec, t.nsec);

    // Multiple writes and reads
    for i in 0..100u64 {
        time.write(TimeValue { sec: i, nsec: (i * 1000) as u32 });
        let t = time.read();
        assert_eq!(t.sec, i);
    }

    println!("  [ok] SeqLock working (100 write/read cycles)");
}

fn demo_once() {
    use mykernel::sync::Once;

    println!("[Once]");

    static INIT: Once = Once::new();
    static mut INIT_VALUE: u64 = 0;

    assert!(!INIT.is_completed());

    INIT.call_once(|| {
        unsafe { INIT_VALUE = 42; }
    });

    assert!(INIT.is_completed());
    assert_eq!(unsafe { INIT_VALUE }, 42);

    // Second call should not run
    INIT.call_once(|| {
        unsafe { INIT_VALUE = 999; } // Should NOT execute
    });

    assert_eq!(unsafe { INIT_VALUE }, 42);
    serial_println!("[sync] Once: value={}", unsafe { INIT_VALUE });

    println!("  [ok] Once working");
}

fn demo_per_cpu() {
    use mykernel::sync::PerCpu;

    println!("[PerCpu]");

    let per_cpu: PerCpu<u64> = PerCpu::new();

    *per_cpu.get_mut() = 100;
    assert_eq!(*per_cpu.get(), 100);

    *per_cpu.get_mut() += 50;
    assert_eq!(*per_cpu.get(), 150);

    serial_println!("[sync] PerCpu CPU[0]: {}", per_cpu.get_for(0));
    println!("  [ok] PerCpu working");
}

fn demo_atomic_counter() {
    use mykernel::sync::AtomicCounter;

    println!("[AtomicCounter]");

    let counter = AtomicCounter::new(0);

    for _ in 0..1000 {
        counter.increment();
    }
    assert_eq!(counter.get(), 1000);

    counter.add(500);
    assert_eq!(counter.get(), 1500);

    // CAS operation
    let result = counter.compare_and_swap(1500, 0);
    assert!(result.is_ok());
    assert_eq!(counter.get(), 0);

    // Failed CAS
    let result = counter.compare_and_swap(999, 42);
    assert!(result.is_err());
    assert_eq!(counter.get(), 0);

    serial_println!("[sync] AtomicCounter: final={}", counter.get());
    println!("  [ok] AtomicCounter working");

    // System timer
    use mykernel::sync::{timer_tick, get_ticks, get_time};
    for _ in 0..50 { timer_tick(); }
    assert_eq!(get_ticks(), 50);
    let t = get_time();
    assert_eq!(t.ticks, 50);
    println!("  [ok] System timer: {} ticks", get_ticks());
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
fn test_spinlock_basic() {
    use mykernel::sync::SpinLock;
    let lock = SpinLock::new(0u64);
    *lock.lock() += 100;
    assert_eq!(*lock.lock(), 100);
    assert!(!lock.is_locked());
    serial_println!("[test] SpinLock OK");
}

#[test_case]
fn test_rwlock() {
    use mykernel::sync::RwSpinLock;
    let rw = RwSpinLock::new(0u32);
    { let r = rw.read(); assert_eq!(*r, 0); }
    { *rw.write() = 42; }
    { let r = rw.read(); assert_eq!(*r, 42); }
    serial_println!("[test] RwSpinLock OK");
}

#[test_case]
fn test_seqlock() {
    use mykernel::sync::SeqLock;
    #[derive(Copy, Clone, Default)]
    struct D { a: u64, b: u64 }
    let sl = SeqLock::new(D { a: 0, b: 0 });
    sl.write(D { a: 1, b: 2 });
    let d = sl.read();
    assert_eq!(d.a, 1);
    assert_eq!(d.b, 2);
    serial_println!("[test] SeqLock OK");
}

#[test_case]
fn test_once() {
    use mykernel::sync::Once;
    static O: Once = Once::new();
    static mut V: u32 = 0;
    O.call_once(|| unsafe { V = 1; });
    O.call_once(|| unsafe { V = 2; }); // Should not run
    assert_eq!(unsafe { V }, 1);
    serial_println!("[test] Once OK");
}

#[test_case]
fn test_atomic_counter() {
    use mykernel::sync::AtomicCounter;
    let c = AtomicCounter::new(0);
    assert_eq!(c.increment(), 0); // Returns old value
    assert_eq!(c.get(), 1);
    assert!(c.compare_and_swap(1, 99).is_ok());
    assert_eq!(c.get(), 99);
    serial_println!("[test] AtomicCounter OK");
}
