/// SMP — Symmetric Multi-Processing
///
/// Boot sequence cho Application Processors (APs):
///
/// 1. BSP (Bootstrap Processor) khởi động bình thường
/// 2. BSP đọc ACPI MADT để tìm các AP LAPIC IDs
/// 3. BSP ghi trampoline code vào địa chỉ thấp (0x8000)
/// 4. BSP gửi INIT + SIPI IPIs cho mỗi AP
/// 5. AP nhảy vào trampoline (real mode) → protected mode → long mode
/// 6. AP gọi ap_main(), set CPU-local state, signal ready
/// 7. BSP chờ tất cả APs ready, rồi tiếp tục
///
/// Vì QEMU default chỉ có 1 CPU, Phase 19 sẽ:
/// - Implement đầy đủ infrastructure
/// - Detect số CPUs từ CPUID / ACPI
/// - Boot APs nếu có (gracefully skip nếu không có)

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// CPU topology
// ---------------------------------------------------------------------------

pub const MAX_CPUS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CpuState {
    Offline,
    Starting,
    Online,
}

pub struct CpuInfo {
    pub apic_id: u8,
    pub state: CpuState,
    pub kernel_stack_top: u64,
}

impl CpuInfo {
    const fn new() -> Self {
        CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 }
    }
}

// Global CPU table
static mut CPU_TABLE: [CpuInfo; MAX_CPUS] = [
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
    CpuInfo { apic_id: 0, state: CpuState::Offline, kernel_stack_top: 0 },
];

static CPU_COUNT: AtomicU32 = AtomicU32::new(1); // BSP always present
static ONLINE_COUNT: AtomicU32 = AtomicU32::new(1);

pub fn cpu_count() -> u32 { CPU_COUNT.load(Ordering::Relaxed) }
pub fn online_count() -> u32 { ONLINE_COUNT.load(Ordering::Relaxed) }

// ---------------------------------------------------------------------------
// CPUID helpers
// ---------------------------------------------------------------------------

/// Get number of logical processors from CPUID leaf 1, EBX[23:16].
/// rbx cannot be an asm operand in LLVM; we push/pop it manually.
pub fn detect_cpu_count() -> u32 {
    let result: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "shr ebx, 16",
            "and ebx, 0xFF",
            "mov edi, ebx",
            "pop rbx",
            out("edi") result,
            out("ecx") _,
            out("edx") _,
            lateout("eax") _,
        );
    }
    let n = if result == 0 { 1 } else { result };
    crate::serial_println!("[smp] CPUID max APIC IDs: {}", n);
    n
}

/// Return the initial APIC ID of the current CPU (CPUID leaf 1, EBX[31:24]).
pub fn current_apic_id() -> u8 {
    let result: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "shr ebx, 24",
            "and ebx, 0xFF",
            "mov edi, ebx",
            "pop rbx",
            out("edi") result,
            out("ecx") _,
            out("edx") _,
            lateout("eax") _,
        );
    }
    result as u8
}

// ---------------------------------------------------------------------------
// ACPI MADT parser — find Local APIC entries
// ---------------------------------------------------------------------------

/// Minimal ACPI RSDP structure
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],  // "RSD PTR "
    checksum:  u8,
    oem_id:    [u8; 6],
    revision:  u8,
    rsdt_addr: u32,
    // ACPI 2.0+
    length:    u32,
    xsdt_addr: u64,
    ext_checksum: u8,
    _reserved: [u8; 3],
}

/// ACPI SDT header
#[repr(C, packed)]
struct AcpiSdtHeader {
    signature:  [u8; 4],
    length:     u32,
    revision:   u8,
    checksum:   u8,
    oem_id:     [u8; 6],
    oem_table:  [u8; 8],
    oem_rev:    u32,
    creator_id: u32,
    creator_rev:u32,
}

/// MADT entry types
const MADT_TYPE_LAPIC: u8 = 0;
const MADT_TYPE_IOAPIC: u8 = 1;

/// MADT Local APIC entry
#[repr(C, packed)]
struct MadtLapic {
    entry_type: u8,
    length:     u8,
    acpi_id:    u8,
    apic_id:    u8,
    flags:      u32,  // bit 0 = enabled
}

/// Search for RSDP in memory
/// RSDP is in EBDA (0x80000-0x9FFFF) or BIOS ROM (0xE0000-0xFFFFF)
fn find_rsdp(phys_mem_offset: u64) -> Option<*const Rsdp> {
    // Search EBDA region
    let regions = [
        (0x000E0000u64, 0x000FFFFFu64),
        (0x00080000u64, 0x0009FFFFu64),
    ];

    for (start, end) in regions {
        let vstart = phys_mem_offset + start;
        let vend = phys_mem_offset + end;
        let mut addr = vstart;
        while addr + 16 <= vend {
            let sig = unsafe { core::slice::from_raw_parts(addr as *const u8, 8) };
            if sig == b"RSD PTR " {
                return Some(addr as *const Rsdp);
            }
            addr += 16;
        }
    }
    None
}

/// Parse MADT to find all Local APIC entries
pub fn parse_madt_apics(phys_mem_offset: u64) -> Vec<u8> {
    let mut apic_ids = Vec::new();

    let rsdp_ptr = match find_rsdp(phys_mem_offset) {
        Some(p) => p,
        None => {
            crate::serial_println!("[smp] RSDP not found, assuming 1 CPU");
            apic_ids.push(0); // BSP only
            return apic_ids;
        }
    };

    let rsdp = unsafe { &*rsdp_ptr };
    crate::serial_println!("[smp] RSDP found: revision={}", { rsdp.revision });

    // Use XSDT if ACPI 2.0+, else RSDT
    let use_xsdt = { rsdp.revision } >= 2;

    // Find MADT in RSDT/XSDT
    let madt_phys = if use_xsdt {
        find_table_xsdt(phys_mem_offset, { rsdp.xsdt_addr }, b"APIC")
    } else {
        find_table_rsdt(phys_mem_offset, { rsdp.rsdt_addr } as u64, b"APIC")
    };

    let madt_virt = match madt_phys {
        Some(phys) => phys_mem_offset + phys,
        None => {
            crate::serial_println!("[smp] MADT not found, assuming 1 CPU");
            apic_ids.push(0);
            return apic_ids;
        }
    };

    // Parse MADT
    let header = unsafe { &*(madt_virt as *const AcpiSdtHeader) };
    let total_len = { header.length } as usize;
    crate::serial_println!("[smp] MADT found, length={}", total_len);

    // MADT-specific fields: 4 bytes LAPIC addr + 4 bytes flags
    let mut offset = core::mem::size_of::<AcpiSdtHeader>() + 8;

    while offset + 2 <= total_len {
        let entry_ptr = (madt_virt + offset as u64) as *const u8;
        let entry_type = unsafe { *entry_ptr };
        let entry_len = unsafe { *entry_ptr.add(1) } as usize;

        if entry_len == 0 { break; }

        if entry_type == MADT_TYPE_LAPIC && entry_len >= 8 {
            let lapic = unsafe { &*(entry_ptr as *const MadtLapic) };
            let flags = { lapic.flags };
            if flags & 1 != 0 { // CPU enabled
                let apic_id = { lapic.apic_id };
                crate::serial_println!("[smp] LAPIC: ACPI_ID={} APIC_ID={} flags={:#x}",
                    { lapic.acpi_id }, apic_id, flags);
                apic_ids.push(apic_id);
            }
        }

        offset += entry_len;
    }

    if apic_ids.is_empty() {
        apic_ids.push(0); // At least BSP
    }

    apic_ids
}

fn find_table_rsdt(phys_mem_offset: u64, rsdt_phys: u64, sig: &[u8; 4]) -> Option<u64> {
    let rsdt_virt = phys_mem_offset + rsdt_phys;
    let header = unsafe { &*(rsdt_virt as *const AcpiSdtHeader) };
    let len = { header.length } as usize;
    let entries = (len - core::mem::size_of::<AcpiSdtHeader>()) / 4;

    for i in 0..entries {
        let entry_phys = unsafe {
            let ptr = (rsdt_virt + core::mem::size_of::<AcpiSdtHeader>() as u64 + i as u64 * 4) as *const u32;
            *ptr as u64
        };
        let table_virt = phys_mem_offset + entry_phys;
        let table_sig = unsafe { core::slice::from_raw_parts(table_virt as *const u8, 4) };
        if table_sig == sig {
            return Some(entry_phys);
        }
    }
    None
}

fn find_table_xsdt(phys_mem_offset: u64, xsdt_phys: u64, sig: &[u8; 4]) -> Option<u64> {
    let xsdt_virt = phys_mem_offset + xsdt_phys;
    let header = unsafe { &*(xsdt_virt as *const AcpiSdtHeader) };
    let len = { header.length } as usize;
    let entries = (len - core::mem::size_of::<AcpiSdtHeader>()) / 8;

    for i in 0..entries {
        let entry_phys = unsafe {
            let ptr = (xsdt_virt + core::mem::size_of::<AcpiSdtHeader>() as u64 + i as u64 * 8) as *const u64;
            *ptr
        };
        let table_virt = phys_mem_offset + entry_phys;
        let table_sig = unsafe { core::slice::from_raw_parts(table_virt as *const u8, 4) };
        if table_sig == sig {
            return Some(entry_phys);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// AP startup trampoline
// ---------------------------------------------------------------------------

/// Physical address for AP startup (must be <1MB, page-aligned)
pub const AP_STARTUP_ADDR: u64 = 0x8000;

/// Shared data between BSP and APs
#[repr(C)]
pub struct ApStartupData {
    pub ready_flag: AtomicU32,    // AP sets this to 1 when ready
    pub cpu_index:  AtomicU32,    // Which CPU index to initialize
    pub cr3:        AtomicU64,    // Page table to use
    pub stack_top:  AtomicU64,    // Kernel stack top for this AP
    pub entry:      AtomicU64,    // Jump target after long mode
}

impl ApStartupData {
    pub const fn new() -> Self {
        ApStartupData {
            ready_flag: AtomicU32::new(0),
            cpu_index:  AtomicU32::new(0),
            cr3:        AtomicU64::new(0),
            stack_top:  AtomicU64::new(0),
            entry:      AtomicU64::new(0),
        }
    }
}

// AP startup data in low memory
static AP_DATA: ApStartupData = ApStartupData::new();

/// Boot all Application Processors
pub fn boot_aps(phys_mem_offset: u64, bsp_cr3: u64) {
    let apic_ids = parse_madt_apics(phys_mem_offset);
    let bsp_apic_id = crate::apic::lapic_id();

    crate::serial_println!(
        "[smp] Found {} processors (BSP APIC ID={})",
        apic_ids.len(), bsp_apic_id
    );

    CPU_COUNT.store(apic_ids.len() as u32, Ordering::Relaxed);

    let mut cpu_idx = 0u32;
    for &apic_id in &apic_ids {
        if apic_id == bsp_apic_id {
            // BSP is already running
            unsafe { CPU_TABLE[0].apic_id = bsp_apic_id; }
            unsafe { CPU_TABLE[0].state = CpuState::Online; }
            cpu_idx += 1;
            continue;
        }

        crate::serial_println!("[smp] Booting AP APIC_ID={}", apic_id);
        boot_single_ap(apic_id, cpu_idx as usize, bsp_cr3, phys_mem_offset);
        cpu_idx += 1;
    }

    crate::serial_println!("[smp] All APs started. Online CPUs: {}",
        ONLINE_COUNT.load(Ordering::Relaxed));
}

fn boot_single_ap(apic_id: u8, cpu_idx: usize, cr3: u64, phys_mem_offset: u64) {
    // Allocate kernel stack for AP
    use alloc::vec;
    let stack = vec![0u8; 4096 * 4];
    let stack_top = stack.as_ptr() as u64 + stack.len() as u64;
    core::mem::forget(stack); // Keep alive

    // Setup AP startup data
    AP_DATA.cpu_index.store(cpu_idx as u32, Ordering::Relaxed);
    AP_DATA.cr3.store(cr3, Ordering::Relaxed);
    AP_DATA.stack_top.store(stack_top, Ordering::Relaxed);
    AP_DATA.entry.store(ap_main as u64, Ordering::Relaxed);
    AP_DATA.ready_flag.store(0, Ordering::Relaxed);

    // Write trampoline to AP_STARTUP_ADDR
    write_ap_trampoline(phys_mem_offset);

    unsafe {
        CPU_TABLE[cpu_idx].apic_id = apic_id;
        CPU_TABLE[cpu_idx].state = CpuState::Starting;
        CPU_TABLE[cpu_idx].kernel_stack_top = stack_top;
    }

    // Send INIT IPI
    crate::apic::lapic_send_init(apic_id);
    // Wait ~10ms
    busy_wait(10_000_000);

    // Send SIPI (startup page = AP_STARTUP_ADDR >> 12 = 0x8)
    crate::apic::lapic_send_sipi(apic_id, (AP_STARTUP_ADDR >> 12) as u8);
    busy_wait(1_000_000);

    // Send second SIPI (Intel spec recommends 2)
    crate::apic::lapic_send_sipi(apic_id, (AP_STARTUP_ADDR >> 12) as u8);

    // Wait for AP ready (up to 1 second)
    let mut timeout = 100_000_000u32;
    loop {
        if AP_DATA.ready_flag.load(Ordering::Relaxed) == 1 { break; }
        timeout -= 1;
        if timeout == 0 {
            crate::serial_println!("[smp] AP {} boot timeout", apic_id);
            return;
        }
        core::hint::spin_loop();
    }

    unsafe { CPU_TABLE[cpu_idx].state = CpuState::Online; }
    ONLINE_COUNT.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!("[smp] AP {} online", apic_id);
}

/// Write 16-bit real mode trampoline code to low memory
/// This is the minimal code APs execute starting in real mode
fn write_ap_trampoline(phys_mem_offset: u64) {
    // Simple trampoline: just signal ready and halt
    // Real implementation would transition to long mode
    // For phase 19 with QEMU single-core, this won't actually run
    let target = (phys_mem_offset + AP_STARTUP_ADDR) as *mut u8;

    // Real mode code at 0x8000:0x0000
    // cli; hlt (2 bytes) — minimal safe trampoline
    let trampoline: &[u8] = &[
        0xFA, // cli
        0xF4, // hlt
    ];

    unsafe {
        core::ptr::copy_nonoverlapping(trampoline.as_ptr(), target, trampoline.len());
    }

    crate::serial_println!("[smp] AP trampoline written to {:#x}", AP_STARTUP_ADDR);
}

/// Entry point for Application Processors after boot
pub extern "C" fn ap_main() -> ! {
    let apic_id = crate::apic::lapic_id();
    crate::serial_println!("[smp] AP {} entered ap_main()", apic_id);

    // Initialize Local APIC for this AP
    // (already enabled by BSP, just set EOI)
    crate::apic::lapic_eoi();

    // Signal BSP that we're ready
    AP_DATA.ready_flag.store(1, Ordering::Relaxed);

    // AP idle loop
    loop {
        x86_64::instructions::hlt();
    }
}

fn busy_wait(cycles: u64) {
    for _ in 0..cycles {
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// Per-CPU data
// ---------------------------------------------------------------------------

/// Get current CPU index (0 = BSP)
pub fn current_cpu_index() -> usize {
    let apic_id = crate::apic::lapic_id();
    unsafe {
        for i in 0..MAX_CPUS {
            if CPU_TABLE[i].apic_id == apic_id
                && CPU_TABLE[i].state == CpuState::Online {
                return i;
            }
        }
    }
    0 // Default to BSP
}

/// Print CPU topology
pub fn print_topology() {
    let count = CPU_COUNT.load(Ordering::Relaxed);
    let online = ONLINE_COUNT.load(Ordering::Relaxed);
    crate::serial_println!("[smp] Topology: {} total, {} online", count, online);
    unsafe {
        for i in 0..count as usize {
            crate::serial_println!("[smp]   CPU[{}]: APIC_ID={} state={:?}",
                i, CPU_TABLE[i].apic_id, CPU_TABLE[i].state);
        }
    }
}
