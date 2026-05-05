/// Process management â€” má»—i process cÃ³ address space riÃªng
///
/// Phase 12 concept:
/// - Má»—i process cÃ³ Level-4 page table riÃªng (CR3 riÃªng)
/// - Kernel space (upper half: 0xffff_8000_0000_0000+) Ä‘Æ°á»£c map vÃ o Má»ŒI process
/// - User space (lower half: 0x0 - 0x7fff_ffff_ffff) riÃªng cho tá»«ng process
/// - Context switch = thay CR3 â†’ CPU dÃ¹ng page table má»›i â†’ memory isolation

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::{
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

// ---------------------------------------------------------------------------
// Process ID
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pid(u64);

impl Pid {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Pid(NEXT.fetch_add(1, Ordering::Relaxed))
    }
    pub fn as_u64(self) -> u64 { self.0 }
}

// ---------------------------------------------------------------------------
// AddressSpace â€” má»™t page table riÃªng cho má»™t process
// ---------------------------------------------------------------------------

pub struct AddressSpace {
    /// Physical address cá»§a Level-4 page table
    pub l4_frame: PhysFrame,
    /// Virtual address cá»§a L4 table (kernel cÃ³ thá»ƒ access qua phys_mem_offset)
    pub l4_table_vaddr: VirtAddr,
}

impl AddressSpace {
    /// Táº¡o address space má»›i:
    /// 1. Allocate má»™t physical frame cho L4 page table
    /// 2. Copy kernel mappings tá»« current L4 table vÃ o
    /// 3. Clear user space entries
    pub fn new(
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
        physical_memory_offset: VirtAddr,
        kernel_l4_table: &PageTable,
    ) -> Self {
        // Allocate frame cho L4 table má»›i
        let l4_frame = frame_allocator
            .allocate_frame()
            .expect("failed to allocate L4 frame");

        let l4_vaddr = physical_memory_offset + l4_frame.start_address().as_u64();
        crate::serial_println!("[AddressSpace::new] l4_frame={:#x} l4_vaddr={:#x}", l4_frame.start_address().as_u64(), l4_vaddr.as_u64());
        let l4_table: &mut PageTable = unsafe { &mut *l4_vaddr.as_mut_ptr() };

        // Clear toÃ n bá»™ L4 table
        l4_table.zero();

        // Copy kernel mappings (upper half: entries 256-511)
        // Kernel space báº¯t Ä‘áº§u tá»« entry 256 trong L4 table
        for i in 0..512 {
            l4_table[i] = kernel_l4_table[i].clone();
        }

        AddressSpace {
            l4_frame,
            l4_table_vaddr: l4_vaddr,
        }
    }

    /// Load address space nÃ y vÃ o CPU (write CR3)
    pub fn activate(&self) {
        use x86_64::registers::control::Cr3;
        unsafe {
            Cr3::write(self.l4_frame, x86_64::registers::control::Cr3Flags::empty());
        }
    }

    /// Láº¥y mutable reference tá»›i L4 page table
    pub fn l4_table_mut(&mut self) -> &mut PageTable {
        unsafe { &mut *self.l4_table_vaddr.as_mut_ptr() }
    }

    /// Map má»™t page vÃ o address space nÃ y
    pub fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        frame: PhysFrame,
        flags: PageTableFlags,
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
        physical_memory_offset: VirtAddr,
    ) -> Result<(), &'static str> {
        let l4_table = unsafe { &mut *self.l4_table_vaddr.as_mut_ptr() };
        let mut mapper = unsafe {
            OffsetPageTable::new(l4_table, physical_memory_offset)
        };
        unsafe {
            mapper.map_to(page, frame, flags, frame_allocator)
                .map_err(|e| { crate::serial_println!("[map_page] map_to error: {:?}", e); "map_to failed" })?
                .flush();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Process
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessState {
    Ready,
    Running,
    Waiting,
    Zombie,
}

pub struct Process {
    pub pid: Pid,
    pub state: ProcessState,
    pub address_space: AddressSpace,
    /// Saved stack pointer (kernel stack khi bá»‹ preempt)
    pub kernel_rsp: u64,
    /// User stack pointer
    pub user_rsp: u64,
    /// User instruction pointer
    pub user_rip: u64,
    /// Kernel stack cho process nÃ y
    _kernel_stack: Vec<u8>,
}

const PROCESS_KERNEL_STACK_SIZE: usize = 4096 * 4; // 16 KiB

impl Process {
    pub fn new(
        entry_point: u64,
        user_stack_top: u64,
        address_space: AddressSpace,
    ) -> Self {
        let mut kernel_stack = Vec::with_capacity(PROCESS_KERNEL_STACK_SIZE);
        for _ in 0..PROCESS_KERNEL_STACK_SIZE {
            kernel_stack.push(0u8);
        }
        let kernel_rsp = kernel_stack.as_ptr() as u64 + PROCESS_KERNEL_STACK_SIZE as u64;

        Process {
            pid: Pid::new(),
            state: ProcessState::Ready,
            address_space,
            kernel_rsp,
            user_rsp: user_stack_top,
            user_rip: entry_point,
            _kernel_stack: kernel_stack,
        }
    }
}

// ---------------------------------------------------------------------------
// Process table
// ---------------------------------------------------------------------------

use spin::Mutex;

pub static PROCESSES: Mutex<Vec<Process>> = Mutex::new(Vec::new());

pub fn add_process(process: Process) -> Pid {
    let pid = process.pid;
    PROCESSES.lock().push(process);
    pid
}

/// Láº¥y physical memory offset tá»« bootloader (Ä‘Æ°á»£c set khi init)
static PHYS_MEM_OFFSET: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn set_phys_mem_offset(offset: u64) {
    PHYS_MEM_OFFSET.store(offset, Ordering::Relaxed);
}

pub fn get_phys_mem_offset() -> VirtAddr {
    VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::Relaxed))
}





