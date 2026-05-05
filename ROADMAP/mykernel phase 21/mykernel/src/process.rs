use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::{
    structures::paging::{
        FrameAllocator, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame, Size4KiB,
    },
    VirtAddr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pid(u64);

impl Pid {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Pid(NEXT.fetch_add(1, Ordering::Relaxed))
    }
    pub fn as_u64(self) -> u64 { self.0 }
}

pub struct AddressSpace {
    pub l4_frame: PhysFrame,
    pub l4_table_vaddr: VirtAddr,
}

impl AddressSpace {
    pub fn new(
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
        physical_memory_offset: VirtAddr,
        kernel_l4_table: &PageTable,
    ) -> Self {
        let l4_frame = frame_allocator
            .allocate_frame()
            .expect("failed to allocate L4 frame");

        let l4_vaddr = physical_memory_offset + l4_frame.start_address().as_u64();
        let l4_table: &mut PageTable = unsafe { &mut *l4_vaddr.as_mut_ptr() };
        l4_table.zero();

        // Copy kernel entries (256-511: upper half)
        for i in 256..512 {
            l4_table[i] = kernel_l4_table[i].clone();
        }
        // Copy lower half except entry 0 (user space) and entry 1
        for i in 2..256 {
            l4_table[i] = kernel_l4_table[i].clone();
        }

        AddressSpace { l4_frame, l4_table_vaddr: l4_vaddr }
    }

    pub fn activate(&self) {
        use x86_64::registers::control::Cr3;
        unsafe {
            Cr3::write(self.l4_frame, x86_64::registers::control::Cr3Flags::empty());
        }
    }

    pub fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        frame: PhysFrame,
        flags: PageTableFlags,
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
        physical_memory_offset: VirtAddr,
    ) -> Result<(), &'static str> {
        use x86_64::structures::paging::Mapper;
        let l4_table = unsafe { &mut *self.l4_table_vaddr.as_mut_ptr() };
        let mut mapper = unsafe {
            OffsetPageTable::new(l4_table, physical_memory_offset)
        };
        unsafe {
            mapper.map_to(page, frame, flags, frame_allocator)
                .map_err(|_| "map_to failed")?
                .flush();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessState { Ready, Running, Waiting, Zombie }

pub struct Process {
    pub pid: Pid,
    pub state: ProcessState,
    pub address_space: AddressSpace,
    pub kernel_rsp: u64,
    pub user_rsp: u64,
    pub user_rip: u64,
    _kernel_stack: Vec<u8>,
}

const PROCESS_KERNEL_STACK_SIZE: usize = 4096 * 4;

impl Process {
    pub fn new(entry_point: u64, user_stack_top: u64, address_space: AddressSpace) -> Self {
        let mut kernel_stack = Vec::with_capacity(PROCESS_KERNEL_STACK_SIZE);
        for _ in 0..PROCESS_KERNEL_STACK_SIZE { kernel_stack.push(0u8); }
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

use spin::Mutex;
pub static PROCESSES: Mutex<Vec<Process>> = Mutex::new(Vec::new());

pub fn add_process(process: Process) -> Pid {
    let pid = process.pid;
    PROCESSES.lock().push(process);
    pid
}

static PHYS_MEM_OFFSET: AtomicU64 = AtomicU64::new(0);
pub fn set_phys_mem_offset(offset: u64) { PHYS_MEM_OFFSET.store(offset, Ordering::Relaxed); }
pub fn get_phys_mem_offset() -> VirtAddr { VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::Relaxed)) }
