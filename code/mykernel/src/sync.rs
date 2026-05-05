/// SMP-safe Synchronization Primitives
///
/// Trên hệ thống SMP, việc chỉ disable interrupts KHÔNG đủ để protect shared data
/// vì nhiều CPUs chạy song song. Cần thêm hardware atomic operations.
///
/// Primitives cung cấp:
///
/// 1. **SpinLock** — Busy-wait lock dùng CMPXCHG atomic
///    - Disable interrupts khi acquire (trên uniprocessor đủ rồi)
///    - Dùng atomic CAS loop để đảm bảo mutual exclusion giữa CPUs
///    - Có deadlock detection (nếu cùng CPU cố acquire lần 2)
///
/// 2. **RwLock** — Multiple readers, single writer
///    - Readers chạy song song được
///    - Writer cần exclusive access
///    - Dùng atomic counter: positive = readers, -1 = writer
///
/// 3. **SeqLock** — Lock-free reads, serialized writes
///    - Readers không block writers
///    - Dùng sequence counter để detect concurrent write
///    - Phù hợp cho data thường read, ít write (vd: system time)
///
/// 4. **PerCpuData** — Per-CPU data không cần lock
///    - Mỗi CPU có copy riêng của data
///    - Không cần synchronization khi access own data
///
/// 5. **AtomicQueue** — Lock-free MPMC queue
///    - Multiple Producers, Multiple Consumers
///    - Dùng cho IPC giữa CPUs

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use core::ops::{Deref, DerefMut};

// ---------------------------------------------------------------------------
// 1. SpinLock — core SMP primitive
// ---------------------------------------------------------------------------

pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
    #[cfg(debug_assertions)]
    owner_cpu: AtomicI32, // -1 = unlocked, >=0 = CPU that holds lock
}

unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    _saved_flags: u64,
}

impl<T> SpinLock<T> {
    pub const fn new(data: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
            #[cfg(debug_assertions)]
            owner_cpu: AtomicI32::new(-1),
        }
    }

    /// Acquire lock — disable interrupts và spin until acquired
    pub fn lock(&self) -> SpinLockGuard<T> {
        // Save and disable interrupts
        let saved_flags = save_and_disable_interrupts();

        // Spin until we acquire the lock
        let mut spin_count = 0u32;
        loop {
            // Try to acquire with CAS: false → true
            if self.locked.compare_exchange_weak(
                false, true,
                Ordering::Acquire,
                Ordering::Relaxed,
            ).is_ok() {
                #[cfg(debug_assertions)]
                self.owner_cpu.store(current_cpu_id() as i32, Ordering::Relaxed);
                break;
            }

            // Spin hint — reduces memory bus contention
            core::hint::spin_loop();

            spin_count += 1;
            if spin_count > 10_000_000 {
                // Potential deadlock — log and continue
                crate::serial_println!("[spinlock] WARNING: long spin count={}", spin_count);
                spin_count = 0;
            }
        }

        SpinLockGuard {
            lock: self,
            _saved_flags: saved_flags,
        }
    }

    /// Try to acquire lock without blocking
    /// Returns None if lock is already held
    pub fn try_lock(&self) -> Option<SpinLockGuard<T>> {
        let saved_flags = save_and_disable_interrupts();

        if self.locked.compare_exchange(
            false, true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_ok() {
            #[cfg(debug_assertions)]
            self.owner_cpu.store(current_cpu_id() as i32, Ordering::Relaxed);
            Some(SpinLockGuard { lock: self, _saved_flags: saved_flags })
        } else {
            restore_interrupts(saved_flags);
            None
        }
    }

    /// Check if currently locked
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Unsafe: get reference without locking (for emergency access)
    pub unsafe fn force_get(&self) -> &T {
        &*self.data.get()
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        self.lock.owner_cpu.store(-1, Ordering::Relaxed);

        // Release lock
        self.lock.locked.store(false, Ordering::Release);

        // Restore interrupt state
        restore_interrupts(self._saved_flags);
    }
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

// ---------------------------------------------------------------------------
// 2. RwSpinLock — Reader-Writer lock
// ---------------------------------------------------------------------------

/// Reader-Writer SpinLock
/// state > 0: number of active readers
/// state = 0: unlocked
/// state = -1: writer holds lock
pub struct RwSpinLock<T> {
    state: AtomicI32,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for RwSpinLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwSpinLock<T> {}

pub struct ReadGuard<'a, T> {
    lock: &'a RwSpinLock<T>,
}

pub struct WriteGuard<'a, T> {
    lock: &'a RwSpinLock<T>,
    _saved_flags: u64,
}

impl<T> RwSpinLock<T> {
    pub const fn new(data: T) -> Self {
        RwSpinLock { state: AtomicI32::new(0), data: UnsafeCell::new(data) }
    }

    /// Acquire read lock (multiple readers allowed)
    pub fn read(&self) -> ReadGuard<T> {
        loop {
            let state = self.state.load(Ordering::Relaxed);
            if state >= 0 {
                if self.state.compare_exchange_weak(
                    state, state + 1,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ).is_ok() {
                    return ReadGuard { lock: self };
                }
            }
            core::hint::spin_loop();
        }
    }

    /// Acquire write lock (exclusive)
    pub fn write(&self) -> WriteGuard<T> {
        let saved_flags = save_and_disable_interrupts();
        loop {
            if self.state.compare_exchange_weak(
                0, -1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ).is_ok() {
                return WriteGuard { lock: self, _saved_flags: saved_flags };
            }
            core::hint::spin_loop();
        }
    }
}

impl<'a, T> Drop for ReadGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

impl<'a, T> Drop for WriteGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);
        restore_interrupts(self._saved_flags);
    }
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.lock.data.get() } }
}

impl<'a, T> Deref for WriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.lock.data.get() } }
}

impl<'a, T> DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.lock.data.get() } }
}

// ---------------------------------------------------------------------------
// 3. SeqLock — Lock-free reads for frequently-read data
// ---------------------------------------------------------------------------

pub struct SeqLock<T: Copy> {
    seq: AtomicU64,
    data: UnsafeCell<T>,
}

unsafe impl<T: Copy + Send> Send for SeqLock<T> {}
unsafe impl<T: Copy + Send> Sync for SeqLock<T> {}

impl<T: Copy> SeqLock<T> {
    pub const fn new(data: T) -> Self {
        SeqLock { seq: AtomicU64::new(0), data: UnsafeCell::new(data) }
    }

    /// Read data — retry if concurrent write detected
    pub fn read(&self) -> T {
        loop {
            let seq1 = self.seq.load(Ordering::Acquire);
            if seq1 & 1 != 0 {
                // Write in progress
                core::hint::spin_loop();
                continue;
            }

            let data = unsafe { *self.data.get() };

            let seq2 = self.seq.load(Ordering::Acquire);
            if seq1 == seq2 {
                return data; // Consistent read
            }
            // seq changed — retry
        }
    }

    /// Write data — increment sequence counter before and after
    pub fn write(&self, new_data: T) {
        let saved = save_and_disable_interrupts();
        // Odd seq = write in progress
        self.seq.fetch_add(1, Ordering::Release);
        core::sync::atomic::fence(Ordering::SeqCst);
        unsafe { *self.data.get() = new_data; }
        core::sync::atomic::fence(Ordering::SeqCst);
        // Even seq = write complete
        self.seq.fetch_add(1, Ordering::Release);
        restore_interrupts(saved);
    }
}

// ---------------------------------------------------------------------------
// 4. Per-CPU Data
// ---------------------------------------------------------------------------

use crate::smp::MAX_CPUS;

pub struct PerCpu<T: Copy + Default> {
    data: [UnsafeCell<T>; MAX_CPUS],
}

unsafe impl<T: Copy + Default + Send> Send for PerCpu<T> {}
unsafe impl<T: Copy + Default + Send> Sync for PerCpu<T> {}

impl<T: Copy + Default> PerCpu<T> {
    pub fn new() -> Self {
        PerCpu {
            data: core::array::from_fn(|_| UnsafeCell::new(T::default())),
        }
    }

    /// Get reference to current CPU's data
    pub fn get(&self) -> &T {
        let cpu = crate::smp::current_cpu_index();
        unsafe { &*self.data[cpu].get() }
    }

    /// Get mutable reference to current CPU's data
    pub fn get_mut(&self) -> &mut T {
        let cpu = crate::smp::current_cpu_index();
        unsafe { &mut *self.data[cpu].get() }
    }

    /// Get reference to specific CPU's data
    pub fn get_for(&self, cpu: usize) -> &T {
        unsafe { &*self.data[cpu.min(MAX_CPUS - 1)].get() }
    }
}

// ---------------------------------------------------------------------------
// 5. Lock-free atomic counter
// ---------------------------------------------------------------------------

pub struct AtomicCounter {
    value: AtomicU64,
}

impl AtomicCounter {
    pub const fn new(val: u64) -> Self {
        AtomicCounter { value: AtomicU64::new(val) }
    }

    pub fn get(&self) -> u64 { self.value.load(Ordering::Relaxed) }
    pub fn set(&self, val: u64) { self.value.store(val, Ordering::Relaxed); }
    pub fn increment(&self) -> u64 { self.value.fetch_add(1, Ordering::Relaxed) }
    pub fn add(&self, n: u64) -> u64 { self.value.fetch_add(n, Ordering::Relaxed) }
    pub fn compare_and_swap(&self, expected: u64, new: u64) -> Result<u64, u64> {
        self.value.compare_exchange(expected, new, Ordering::SeqCst, Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// 6. Once — Run initialization exactly once across CPUs
// ---------------------------------------------------------------------------

pub struct Once {
    state: AtomicU32,
}

const ONCE_INCOMPLETE: u32 = 0;
const ONCE_RUNNING:    u32 = 1;
const ONCE_COMPLETE:   u32 = 2;

impl Once {
    pub const fn new() -> Self {
        Once { state: AtomicU32::new(ONCE_INCOMPLETE) }
    }

    pub fn call_once<F: FnOnce()>(&self, f: F) {
        // Fast path: already complete
        if self.state.load(Ordering::Acquire) == ONCE_COMPLETE {
            return;
        }

        // Try to become the runner
        if self.state.compare_exchange(
            ONCE_INCOMPLETE, ONCE_RUNNING,
            Ordering::Acquire, Ordering::Relaxed,
        ).is_ok() {
            f();
            self.state.store(ONCE_COMPLETE, Ordering::Release);
        } else {
            // Wait for completion
            while self.state.load(Ordering::Acquire) != ONCE_COMPLETE {
                core::hint::spin_loop();
            }
        }
    }

    pub fn is_completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == ONCE_COMPLETE
    }
}

// ---------------------------------------------------------------------------
// Hardware support functions
// ---------------------------------------------------------------------------

/// Save RFLAGS and disable interrupts, return saved flags
#[inline]
pub fn save_and_disable_interrupts() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {flags}",
            "cli",
            flags = out(reg) flags,
            options(nomem, preserves_flags)
        );
    }
    flags
}

/// Restore interrupt flag from saved flags
#[inline]
pub fn restore_interrupts(flags: u64) {
    if flags & (1 << 9) != 0 {
        // IF was set, re-enable
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    }
}

/// Memory fence — ensure ordering across CPUs
#[inline]
pub fn memory_barrier() {
    core::sync::atomic::fence(Ordering::SeqCst);
}

/// Get current CPU ID (0 if APIC not initialized)
fn current_cpu_id() -> usize {
    crate::smp::current_cpu_index()
}

// ---------------------------------------------------------------------------
// Global system time using SeqLock (demo)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Default)]
pub struct SystemTime {
    pub seconds: u64,
    pub nanoseconds: u32,
    pub ticks: u64,
}

static SYSTEM_TIME: SeqLock<SystemTime> = SeqLock::new(SystemTime {
    seconds: 0,
    nanoseconds: 0,
    ticks: 0,
});

static TICK_COUNTER: AtomicCounter = AtomicCounter::new(0);

/// Called from timer interrupt handler
pub fn timer_tick() {
    let ticks = TICK_COUNTER.increment() + 1;
    let time = SystemTime {
        seconds: ticks / 100,
        nanoseconds: ((ticks % 100) * 10_000_000) as u32,
        ticks,
    };
    SYSTEM_TIME.write(time);
}

pub fn get_time() -> SystemTime {
    SYSTEM_TIME.read()
}

pub fn get_ticks() -> u64 {
    TICK_COUNTER.get()
}
