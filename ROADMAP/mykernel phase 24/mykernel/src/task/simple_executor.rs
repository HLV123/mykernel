use super::{Task, TaskId};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::task::{Context, Poll, Waker};

/// Waker đơn giản — chỉ lưu TaskId, không làm gì thêm
/// (SimpleExecutor poll tất cả tasks liên tục)
struct TaskWaker(TaskId);

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {}
    fn wake_by_ref(self: &Arc<Self>) {}
}

fn dummy_waker(id: TaskId) -> Waker {
    Arc::new(TaskWaker(id)).into()
}

/// Executor đơn giản: poll tất cả tasks theo vòng cho đến khi xong hết
/// Không efficient nhưng dễ hiểu — dùng để học concept
pub struct SimpleExecutor {
    tasks: BTreeMap<TaskId, Task>,
}

impl SimpleExecutor {
    pub fn new() -> SimpleExecutor {
        SimpleExecutor {
            tasks: BTreeMap::new(),
        }
    }

    pub fn spawn(&mut self, task: Task) {
        let id = task.id;
        self.tasks.insert(id, task);
    }

    /// Chạy tất cả tasks đến khi hoàn thành
    pub fn run(&mut self) {
        while !self.tasks.is_empty() {
            let keys: alloc::vec::Vec<_> = self.tasks.keys().copied().collect();
            for id in keys {
                if let Some(task) = self.tasks.get_mut(&id) {
                    let waker = dummy_waker(id);
                    let mut ctx = Context::from_waker(&waker);
                    match task.poll(&mut ctx) {
                        Poll::Ready(()) => {
                            self.tasks.remove(&id);
                        }
                        Poll::Pending => {}
                    }
                }
            }
        }
    }
}
