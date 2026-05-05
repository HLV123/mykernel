/// Preemptive Round-Robin Scheduler
///
/// Má»—i Task cÃ³:
/// - Stack riÃªng (8KiB)
/// - Saved register state (TaskContext)
/// - State: Ready / Running / Finished
///
/// Context switch xáº£y ra trong timer interrupt handler.
/// Assembly routine `switch_context` save/restore registers.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

// ---------------------------------------------------------------------------
// Task ID
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskId(u64);

impl TaskId {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        TaskId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
    pub fn as_u64(self) -> u64 { self.0 }
}

// ---------------------------------------------------------------------------
// Task state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Finished,
}

// ---------------------------------------------------------------------------
// CPU context â€” registers Ä‘Æ°á»£c save/restore khi context switch
// ---------------------------------------------------------------------------

/// Chá»‰ cáº§n save callee-saved registers theo System V AMD64 ABI:
/// rbx, r12, r13, r14, r15, rbp, rsp, rip
/// (rax, rcx, rdx, rsi, rdi, r8-r11 lÃ  caller-saved â€” caller tá»± lo)
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct TaskContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rsp: u64, // stack pointer
    pub rip: u64, // instruction pointer (return address)
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

const STACK_SIZE: usize = 8192; // 8 KiB per task

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub context: TaskContext,
    // Stack Ä‘Æ°á»£c allocate trÃªn heap, giá»¯ ownership á»Ÿ Ä‘Ã¢y
    _stack: Vec<u8>,
}

impl Task {
    /// Táº¡o task má»›i tá»« má»™t function pointer
    pub fn new(entry: fn() -> !) -> Self {
        let mut stack = Vec::with_capacity(STACK_SIZE);
        // Init stack vá»›i zeros
        for _ in 0..STACK_SIZE {
            stack.push(0u8);
        }

        // Stack grows downward â€” rsp báº¯t Ä‘áº§u á»Ÿ cuá»‘i stack
        // Push entry address lÃªn stack Ä‘á»ƒ `ret` trong switch_context jump vÃ o Ä‘Ã³
        let stack_top = stack.as_ptr() as usize + STACK_SIZE;
        // Align xuá»‘ng 16 bytes rá»“i Ä‘á»ƒ chá»— cho return address
        let rsp = (stack_top - 8) & !0xf;

        // Ghi entry point vÃ o Ä‘áº§u stack (nhÆ° return address)
        unsafe {
            let ret_addr_ptr = rsp as *mut u64;
            *ret_addr_ptr = entry as u64;
        }

        let context = TaskContext {
            rsp: rsp as u64,
            rip: entry as u64,
            ..Default::default()
        };

        Task {
            id: TaskId::new(),
            state: TaskState::Ready,
            context,
            _stack: stack,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

pub struct Scheduler {
    tasks: Vec<Task>,
    current: usize, // index cá»§a task Ä‘ang cháº¡y
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            tasks: Vec::new(),
            current: 0,
        }
    }

    pub fn add_task(&mut self, task: Task) {
        self.tasks.push(task);
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Chá»n task tiáº¿p theo (round-robin, bá» qua Finished)
    pub fn next_task_index(&self) -> Option<usize> {
        let n = self.tasks.len();
        if n == 0 { return None; }

        let start = (self.current + 1) % n;
        for i in 0..n {
            let idx = (start + i) % n;
            if self.tasks[idx].state == TaskState::Ready
                || self.tasks[idx].state == TaskState::Running
            {
                return Some(idx);
            }
        }
        None
    }

    /// Thá»±c hiá»‡n context switch sang task tiáº¿p theo
    /// Gá»i tá»« timer interrupt handler
    pub fn schedule(&mut self) {
        if self.tasks.is_empty() { return; }

        let next = match self.next_task_index() {
            Some(i) => i,
            None => return,
        };

        if next == self.current { return; }

        // Mark current task lÃ  Ready (khÃ´ng cÃ²n Running)
        if self.tasks[self.current].state == TaskState::Running {
            self.tasks[self.current].state = TaskState::Ready;
        }
        self.tasks[next].state = TaskState::Running;

        // Láº¥y pointer tá»›i 2 contexts
        let current_ctx = &mut self.tasks[self.current].context as *mut TaskContext;
        let next_ctx = &self.tasks[next].context as *const TaskContext;

        self.current = next;

        // Thá»±c hiá»‡n context switch
        unsafe { switch_context(current_ctx, next_ctx); }
    }

    pub fn current_id(&self) -> TaskId {
        self.tasks[self.current].id
    }

    pub fn mark_current_finished(&mut self) {
        self.tasks[self.current].state = TaskState::Finished;
    }
}

// ---------------------------------------------------------------------------
// Global scheduler
// ---------------------------------------------------------------------------

pub static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

pub fn init() {
    *SCHEDULER.lock() = Some(Scheduler::new());
}

pub fn spawn(task: Task) {
    SCHEDULER.lock().as_mut().unwrap().add_task(task);
}

/// Gá»i tá»« timer interrupt â€” trigger context switch
pub fn timer_tick() {
    // DÃ¹ng try_lock Ä‘á»ƒ trÃ¡nh deadlock náº¿u scheduler Ä‘ang bá»‹ lock
    if let Some(scheduler) = SCHEDULER.try_lock().as_mut() {
        if let Some(s) = scheduler.as_mut() {
            s.schedule();
        }
    }
}

// ---------------------------------------------------------------------------
// Context switch â€” assembly
// ---------------------------------------------------------------------------

/// Save context cá»§a `current`, restore context cá»§a `next`, jump vÃ o next.
///
/// Convention:
///   rdi = *mut TaskContext (current â€” Ä‘á»ƒ save vÃ o)
///   rsi = *const TaskContext (next â€” Ä‘á»ƒ load tá»«)
///
/// Sau khi `ret`, CPU Ä‘ang cháº¡y task `next` táº¡i rip Ä‘Ã£ save.
#[unsafe(naked)]
unsafe extern "C" fn switch_context(
    current: *mut TaskContext,
    next: *const TaskContext,
) {
    core::arch::naked_asm!(
        // Save callee-saved registers cá»§a task hiá»‡n táº¡i vÃ o *current (rdi)
        "mov [rdi + 0x00], r15",
        "mov [rdi + 0x08], r14",
        "mov [rdi + 0x10], r13",
        "mov [rdi + 0x18], r12",
        "mov [rdi + 0x20], rbp",
        "mov [rdi + 0x28], rbx",
        "mov [rdi + 0x30], rsp",
        // Save return address (rip) â€” láº¥y tá»« [rsp] vÃ¬ chÃºng ta Ä‘Æ°á»£c gá»i báº±ng `call`
        "mov rax, [rsp]",
        "mov [rdi + 0x38], rax",

        // Restore registers cá»§a task tiáº¿p theo tá»« *next (rsi)
        "mov r15, [rsi + 0x00]",
        "mov r14, [rsi + 0x08]",
        "mov r13, [rsi + 0x10]",
        "mov r12, [rsi + 0x18]",
        "mov rbp, [rsi + 0x20]",
        "mov rbx, [rsi + 0x28]",
        "mov rsp, [rsi + 0x30]",
        // Jump tá»›i rip cá»§a next task
        "mov rax, [rsi + 0x38]",
        "jmp rax",
    );
}
