/// APIC (Advanced Programmable Interrupt Controller)
///
/// Modern x86 systems dùng APIC thay vì PIC 8259:
/// - Local APIC: mỗi CPU core có một, nhận interrupts từ I/O APIC và local sources
/// - I/O APIC: nhận interrupts từ devices, route đến Local APICs
///
/// Memory-mapped registers của Local APIC ở physical 0xFEE00000
/// (mapped vào virtual space qua phys_mem_offset)
///
/// Key registers:
///   0x020: ID Register
///   0x030: Version Register
///   0x080: Task Priority Register (TPR)
///   0x0B0: EOI Register — write 0 to acknowledge interrupt
///   0x0D0: Logical Destination Register
///   0x0E0: Destination Format Register
///   0x0F0: Spurious Interrupt Vector Register (SVR)
///   0x100-0x170: In-Service Registers (ISR)
///   0x180-0x1F0: Trigger Mode Registers (TMR)
///   0x200-0x270: Interrupt Request Registers (IRR)
///   0x280: Error Status Register
///   0x300: Interrupt Command Register Low (ICR_LO) — for IPIs
///   0x310: Interrupt Command Register High (ICR_HI)
///   0x320: LVT Timer Register
///   0x370: LVT Error Register
///   0x380: Initial Count Register (timer)
///   0x390: Current Count Register (timer)
///   0x3E0: Divide Configuration Register (timer)
///
/// SMP Boot sequence (AP startup):
///   1. BSP sends INIT IPI to target AP
///   2. BSP waits 10ms
///   3. BSP sends SIPI (Startup IPI) with startup page
///   4. AP starts executing from startup page (real mode!)
///   5. AP transitions to protected/long mode and signals ready

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// Local APIC register offsets
// ---------------------------------------------------------------------------

const LAPIC_ID:      u32 = 0x020;
const LAPIC_VER:     u32 = 0x030;
const LAPIC_TPR:     u32 = 0x080;
const LAPIC_EOI:     u32 = 0x0B0;
const LAPIC_LDR:     u32 = 0x0D0;
const LAPIC_DFR:     u32 = 0x0E0;
const LAPIC_SVR:     u32 = 0x0F0;
const LAPIC_ESR:     u32 = 0x280;
const LAPIC_ICR_LO:  u32 = 0x300;
const LAPIC_ICR_HI:  u32 = 0x310;
const LAPIC_LVT_TIMER: u32 = 0x320;
const LAPIC_LVT_LINT0:  u32 = 0x350;
const LAPIC_LVT_LINT1:  u32 = 0x360;
const LAPIC_LVT_ERROR:  u32 = 0x370;
const LAPIC_TIMER_ICR: u32 = 0x380;
const LAPIC_TIMER_CCR: u32 = 0x390;
const LAPIC_TIMER_DCR: u32 = 0x3E0;

// SVR bits
const SVR_ENABLE: u32 = 0x100;
const SVR_SPURIOUS_VECTOR: u32 = 0xFF;

// LVT timer bits
const LVT_PERIODIC: u32 = 0x20000;
const LVT_MASKED:   u32 = 0x10000;

// ICR delivery modes
pub const ICR_INIT:  u32 = 0x00500;
pub const ICR_SIPI:  u32 = 0x00600;
pub const ICR_FIXED: u32 = 0x00000;

// ICR destination modes
pub const ICR_PHYSICAL:   u32 = 0x00000;
pub const ICR_ASSERT:     u32 = 0x04000;
pub const ICR_EDGE:       u32 = 0x00000;
pub const ICR_NO_SHORTHAND:u32 = 0x00000;
pub const ICR_ALL_EXCL_SELF:u32 = 0xC0000;

// ---------------------------------------------------------------------------
// Local APIC driver
// ---------------------------------------------------------------------------

static LAPIC_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn init_lapic(phys_mem_offset: u64) {
    use x86_64::registers::model_specific::Msr;

    // IA32_APIC_BASE MSR = 0x1B
    let apic_base_msr = unsafe { Msr::new(0x1B).read() };
    let lapic_phys = apic_base_msr & 0xFFFF_F000;
    let lapic_virt = phys_mem_offset + lapic_phys;

    LAPIC_BASE.store(lapic_virt, Ordering::Relaxed);

    crate::serial_println!(
        "[apic] Local APIC phys={:#x} virt={:#x}",
        lapic_phys, lapic_virt
    );

    // Enable Local APIC via SVR
    let svr = lapic_read(LAPIC_SVR);
    lapic_write(LAPIC_SVR, svr | SVR_ENABLE | SVR_SPURIOUS_VECTOR);

    // Set Task Priority to 0 (accept all interrupts)
    lapic_write(LAPIC_TPR, 0);

    // Disable LINT0/LINT1
    lapic_write(LAPIC_LVT_LINT0, LVT_MASKED);
    lapic_write(LAPIC_LVT_LINT1, LVT_MASKED);

    // Mask error
    lapic_write(LAPIC_LVT_ERROR, LVT_MASKED);

    // Clear ESR
    lapic_write(LAPIC_ESR, 0);
    lapic_write(LAPIC_ESR, 0);

    // Send EOI
    lapic_write(LAPIC_EOI, 0);

    let id = lapic_read(LAPIC_ID) >> 24;
    let ver = lapic_read(LAPIC_VER);
    crate::serial_println!(
        "[apic] Local APIC ID={} Version={:#x} MaxLVT={}",
        id, ver & 0xFF, (ver >> 16) & 0xFF
    );
}

/// Read from Local APIC register
pub fn lapic_read(reg: u32) -> u32 {
    let base = LAPIC_BASE.load(Ordering::Relaxed);
    if base == 0 { return 0; }
    unsafe {
        core::ptr::read_volatile((base + reg as u64) as *const u32)
    }
}

/// Write to Local APIC register
pub fn lapic_write(reg: u32, val: u32) {
    let base = LAPIC_BASE.load(Ordering::Relaxed);
    if base == 0 { return; }
    unsafe {
        core::ptr::write_volatile((base + reg as u64) as *mut u32, val);
    }
}

/// Send EOI to Local APIC (must call after handling each interrupt)
pub fn lapic_eoi() {
    lapic_write(LAPIC_EOI, 0);
}

/// Get current CPU's Local APIC ID
pub fn lapic_id() -> u8 {
    (lapic_read(LAPIC_ID) >> 24) as u8
}

/// Setup APIC timer for periodic interrupts
/// vector: interrupt vector number
/// hz: frequency in Hz
pub fn lapic_timer_init(vector: u8, hz: u32) {
    // Divide by 16
    lapic_write(LAPIC_TIMER_DCR, 0x3);

    // Initial count — calibrate using PIT or just use a fixed value
    // QEMU APIC runs at ~100MHz, divide by 16 = ~6.25MHz
    // For 100Hz timer: 6_250_000 / 100 = 62_500
    let initial_count = 1_000_000u32 / hz;

    // Setup LVT timer: periodic mode, vector
    lapic_write(LAPIC_LVT_TIMER, LVT_PERIODIC | vector as u32);
    lapic_write(LAPIC_TIMER_ICR, initial_count);

    crate::serial_println!(
        "[apic] Timer: vector={} hz={} count={}",
        vector, hz, initial_count
    );
}

/// Send Inter-Processor Interrupt (IPI)
pub fn lapic_send_ipi(dest_apic_id: u8, vector: u8, delivery_mode: u32) {
    // Write high word first (destination)
    lapic_write(LAPIC_ICR_HI, (dest_apic_id as u32) << 24);
    // Write low word (trigger IPI)
    lapic_write(LAPIC_ICR_LO, delivery_mode | ICR_PHYSICAL | ICR_ASSERT | ICR_EDGE | vector as u32);

    // Wait for delivery
    let mut timeout = 100_000u32;
    loop {
        let icr = lapic_read(LAPIC_ICR_LO);
        if icr & (1 << 12) == 0 { break; } // Delivery status = idle
        timeout -= 1;
        if timeout == 0 {
            crate::serial_println!("[apic] IPI delivery timeout");
            break;
        }
        core::hint::spin_loop();
    }
}

/// Send INIT IPI to AP
pub fn lapic_send_init(dest_apic_id: u8) {
    lapic_send_ipi(dest_apic_id, 0, ICR_INIT);
}

/// Send Startup IPI to AP with startup page
pub fn lapic_send_sipi(dest_apic_id: u8, startup_page: u8) {
    lapic_send_ipi(dest_apic_id, startup_page, ICR_SIPI);
}

// ---------------------------------------------------------------------------
// I/O APIC
// ---------------------------------------------------------------------------

const IOAPIC_PHYS: u64 = 0xFEC0_0000;
const IOAPIC_IOREGSEL: u64 = 0x00;
const IOAPIC_IOWIN:    u64 = 0x10;

const IOAPIC_ID:    u32 = 0x00;
const IOAPIC_VER:   u32 = 0x01;
const IOAPIC_ARB:   u32 = 0x02;
const IOAPIC_REDTBL_BASE: u32 = 0x10; // Entry 0 starts at 0x10

static IOAPIC_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn init_ioapic(phys_mem_offset: u64) {
    let ioapic_virt = phys_mem_offset + IOAPIC_PHYS;
    IOAPIC_BASE.store(ioapic_virt, Ordering::Relaxed);

    let id = ioapic_read(IOAPIC_ID) >> 24;
    let ver = ioapic_read(IOAPIC_VER);
    let max_redir = (ver >> 16) & 0xFF;

    crate::serial_println!(
        "[apic] I/O APIC: ID={} Version={:#x} MaxRedirEntries={}",
        id, ver & 0xFF, max_redir + 1
    );

    // Mask all IRQs initially
    for i in 0..=(max_redir as u32) {
        ioapic_write_redir(i, 0x00010000, 0); // Masked
    }
}

/// Route an IRQ to a CPU with a given vector
pub fn ioapic_route_irq(irq: u8, vector: u8, dest_apic_id: u8, level_triggered: bool) {
    let flags: u32 = if level_triggered { 0x0000_8000 } else { 0 };
    let lo = vector as u32 | flags; // Fixed delivery, physical destination
    let hi = (dest_apic_id as u32) << 24;
    ioapic_write_redir(irq as u32, lo, hi);
    crate::serial_println!(
        "[apic] IRQ {} → vector {} CPU {}",
        irq, vector, dest_apic_id
    );
}

fn ioapic_read(reg: u32) -> u32 {
    let base = IOAPIC_BASE.load(Ordering::Relaxed);
    if base == 0 { return 0; }
    unsafe {
        core::ptr::write_volatile((base + IOAPIC_IOREGSEL) as *mut u32, reg);
        core::ptr::read_volatile((base + IOAPIC_IOWIN) as *const u32)
    }
}

fn ioapic_write(reg: u32, val: u32) {
    let base = IOAPIC_BASE.load(Ordering::Relaxed);
    if base == 0 { return; }
    unsafe {
        core::ptr::write_volatile((base + IOAPIC_IOREGSEL) as *mut u32, reg);
        core::ptr::write_volatile((base + IOAPIC_IOWIN) as *mut u32, val);
    }
}

fn ioapic_write_redir(entry: u32, lo: u32, hi: u32) {
    ioapic_write(IOAPIC_REDTBL_BASE + entry * 2, lo);
    ioapic_write(IOAPIC_REDTBL_BASE + entry * 2 + 1, hi);
}

// ---------------------------------------------------------------------------
// Disable legacy PIC (8259)
// ---------------------------------------------------------------------------

/// Disable 8259 PIC — required when using APIC
pub fn disable_pic() {
    use x86_64::instructions::port::Port;
    unsafe {
        // Mask all interrupts on both PICs
        let mut p1: Port<u8> = Port::new(0xA1);
        let mut p2: Port<u8> = Port::new(0x21);
        p1.write(0xFF);
        p2.write(0xFF);
    }
    crate::serial_println!("[apic] Legacy PIC disabled");
}
