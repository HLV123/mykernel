/// Preemptive Round-Robin Scheduler
///
/// Mỗi Task có:
/// - Stack riêng (8KiB)
/// - Saved register state (TaskContext)
/// - State: Ready / Running / Finished
///
/// Context switch xảy ra trong timer interrupt handler.
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
// CPU context — registers được save/restore khi context switch
// ---------------------------------------------------------------------------

/// Chỉ cần save callee-saved registers theo System V AMD64 ABI:
/// rbx, r12, r13, r14, r15, rbp, rsp, rip
/// (rax, rcx, rdx, rsi, rdi, r8-r11 là caller-saved — caller tự lo)
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
    // Stack được allocate trên heap, giữ ownership ở đây
    _stack: Vec<u8>,
}

impl Task {
    /// Tạo task mới từ một function pointer
    pub fn new(entry: fn() -> !) -> Self {
        let mut stack = Vec::with_capacity(STACK_SIZE);
        // Init stack với zeros
        for _ in 0..STACK_SIZE {
            stack.push(0u8);
        }

        // Stack grows downward — rsp bắt đầu ở cuối stack
        // Push entry address lên stack để `ret` trong switch_context jump vào đó
        let stack_top = stack.as_ptr() as usize + STACK_SIZE;
        // Align xuống 16 bytes rồi để chỗ cho return address
        let rsp = (stack_top - 8) & !0xf;

        // Ghi entry point vào đầu stack (như return address)
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
    current: usize, // index của task đang chạy
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

    /// Chọn task tiếp theo (round-robin, bỏ qua Finished)
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

    /// Thực hiện context switch sang task tiếp theo
    /// Gọi từ timer interrupt handler
    pub fn schedule(&mut self) {
        if self.tasks.is_empty() { return; }

        let next = match self.next_task_index() {
            Some(i) => i,
            None => return,
        };

        if next == self.current { return; }

        // Mark current task là Ready (không còn Running)
        if self.tasks[self.current].state == TaskState::Running {
            self.tasks[self.current].state = TaskState::Ready;
        }
        self.tasks[next].state = TaskState::Running;

        // Lấy pointer tới 2 contexts
        let current_ctx = &mut self.tasks[self.current].context as *mut TaskContext;
        let next_ctx = &self.tasks[next].context as *const TaskContext;

        self.current = next;

        // Thực hiện context switch
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

/// Gọi từ timer interrupt — trigger context switch
pub fn timer_tick() {
    // Dùng try_lock để tránh deadlock nếu scheduler đang bị lock
    if let Some(scheduler) = SCHEDULER.try_lock().as_mut() {
        if let Some(s) = scheduler.as_mut() {
            s.schedule();
        }
    }
}

// ---------------------------------------------------------------------------
// Context switch — assembly
// ---------------------------------------------------------------------------

/// Save context của `current`, restore context của `next`, jump vào next.
///
/// Convention:
///   rdi = *mut TaskContext (current — để save vào)
///   rsi = *const TaskContext (next — để load từ)
///
/// Sau khi `ret`, CPU đang chạy task `next` tại rip đã save.
#[unsafe(naked)]
unsafe extern "C" fn switch_context(
    current: *mut TaskContext,
    next: *const TaskContext,
) {
    core::arch::naked_asm!(
        // Save callee-saved registers của task hiện tại vào *current (rdi)
        "mov [rdi + 0x00], r15",
        "mov [rdi + 0x08], r14",
        "mov [rdi + 0x10], r13",
        "mov [rdi + 0x18], r12",
        "mov [rdi + 0x20], rbp",
        "mov [rdi + 0x28], rbx",
        "mov [rdi + 0x30], rsp",
        // Save return address (rip) — lấy từ [rsp] vì chúng ta được gọi bằng `call`
        "mov rax, [rsp]",
        "mov [rdi + 0x38], rax",

        // Restore registers của task tiếp theo từ *next (rsi)
        "mov r15, [rsi + 0x00]",
        "mov r14, [rsi + 0x08]",
        "mov r13, [rsi + 0x10]",
        "mov r12, [rsi + 0x18]",
        "mov rbp, [rsi + 0x20]",
        "mov rbx, [rsi + 0x28]",
        "mov rsp, [rsi + 0x30]",
        // Jump tới rip của next task
        "mov rax, [rsi + 0x38]",
        "jmp rax",
    );
}
