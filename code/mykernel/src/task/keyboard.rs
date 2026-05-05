// Async keyboard driver
// Keyboard interrupt handler calls add_scancode() to push raw scancodes into
// a lock-free queue.  The shell calls read_key().await to get decoded chars.

use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use core::pin::Pin;
use core::task::{Context, Poll};
use futures_util::stream::{Stream, StreamExt};
use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};

static SCANCODE_QUEUE: OnceCell<ArrayQueue<u8>> = OnceCell::uninit();
static WAKER: futures_util::task::AtomicWaker = futures_util::task::AtomicWaker::new();

/// Called by the PS/2 keyboard interrupt handler (interrupts.rs).
/// Pushes the raw scancode into the queue and wakes any waiting reader.
pub fn add_scancode(scancode: u8) {
    if let Ok(queue) = SCANCODE_QUEUE.try_get() {
        if queue.push(scancode).is_ok() {
            WAKER.wake();
        }
    }
}

/// Await the next printable character from the keyboard.
/// Used as fallback when serial input is not available.
pub async fn read_key() -> char {
    let mut stream = ScancodeStream::new();
    let mut keyboard = Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::Ignore,
    );
    loop {
        if let Some(sc) = stream.next().await {
            if let Ok(Some(key_event)) = keyboard.add_byte(sc) {
                if let Some(DecodedKey::Unicode(c)) = keyboard.process_keyevent(key_event) {
                    return c;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Async scancode stream (implements futures Stream)
// ---------------------------------------------------------------------------

struct ScancodeStream { _private: () }

impl ScancodeStream {
    fn new() -> Self {
        SCANCODE_QUEUE.try_init_once(|| ArrayQueue::new(100))
            .expect("ScancodeStream::new should only be called once");
        ScancodeStream { _private: () }
    }
}

impl Stream for ScancodeStream {
    type Item = u8;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<u8>> {
        let queue = SCANCODE_QUEUE.try_get().expect("queue not initialized");
        if let Some(sc) = queue.pop() {
            return Poll::Ready(Some(sc));
        }
        WAKER.register(cx.waker());
        match queue.pop() {
            Some(sc) => { WAKER.take(); Poll::Ready(Some(sc)) }
            None     => Poll::Pending,
        }
    }
}
