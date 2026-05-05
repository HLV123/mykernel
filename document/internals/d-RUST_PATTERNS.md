# Rust Patterns trong Kernel

> Các pattern Rust đặc thù xuất hiện trong MyKernel — giải thích lý do thiết kế và những trade-off.

---

## Pattern 1: `let x = match lock().method() {...}; x`

### Vấn đề

```rust
// Code này KHÔNG compile:
fn sys_read(fd: u64, buf: &mut [u8]) -> i64 {
    let file = get_file(fd).unwrap();
    match file.lock().read(buf) {   // ← borrow checker issue
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}
```

**Lỗi:** `file.lock()` trả về `MutexGuard<'_, dyn File>`. Guard này mượn `file` và phải sống đủ lâu để `read()` hoàn thành. Nhưng borrow checker không thể prove rằng guard sống qua toàn bộ `match` expression vì `match` có thể có nhiều arms với lifetime khác nhau.

### Fix

```rust
fn sys_read(fd: u64, buf: &mut [u8]) -> i64 {
    let file = get_file(fd).unwrap();
    let x = match file.lock().read(buf) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }; x  // ← guard drop ở đây, sau khi match kết thúc
}
```

Khi dùng `let x = match {...};`, guard được drop khi statement kết thúc (dấu `;`), sau đó `x` được trả về. Borrow checker satisfied vì lifetime rõ ràng.

**Alternative cleaners:**

```rust
// Option 1: explicit scope
let result = {
    let mut guard = file.lock();
    guard.read(buf).map(|n| n as i64).unwrap_or(-1)
};

// Option 2: helper closure
let result = file.lock().read(buf)
    .map(|n| n as i64)
    .unwrap_or(-1);
// Chỉ work nếu không có early return trong match arms
```

---

## Pattern 2: `Arc<Mutex<dyn File>>`

### Tại sao cần cả 3 layer?

```rust
type FileHandle = Arc<Mutex<dyn File>>;
```

**`dyn File`** — dynamic dispatch:
- Cho phép `RamFile`, `DevNullFile`, `Fat32File` có thể được dùng qua cùng interface
- Thay vì generics (`Box<T: File>`) → không cần biết concrete type tại compile time
- Trade-off: virtual dispatch overhead (thường không đáng kể trong kernel)

**`Mutex<...>`** — mutual exclusion:
- File state (position, buffer) là mutable
- Nhiều threads/CPUs có thể có handle đến cùng file (dup2, fork)
- Mutex đảm bảo chỉ 1 thread access tại 1 thời điểm

**`Arc<...>`** — reference counting:
- FdTable entries trong nhiều processes có thể share cùng file handle
- File chỉ bị drop khi count về 0 (tất cả FDs đã close)
- Thay vì `Rc` (không thread-safe) vì kernel là multi-core

**Nếu bỏ bớt:**

```rust
// Chỉ dùng Box<dyn File>:
//   - Chỉ 1 owner → không share được giữa processes
//   - Không thread-safe

// Chỉ dùng Arc<dyn File>:
//   - Không mutate được (immutable reference)
//   - Cần interior mutability khác (RefCell, UnsafeCell)

// Dùng Mutex<Box<dyn File>>:
//   - Không share giữa processes (không có Arc)
```

### Alternative: RwLock

Nếu reads nhiều hơn writes (như readonly files), có thể dùng:
```rust
type FileHandle = Arc<RwLock<dyn File>>;
// Nhiều readers đồng thời, exclusive writer
```

---

## Pattern 3: `#[unsafe(naked)]` cho Context Switch

### Vấn đề

Rust function bình thường có **prologue/epilogue** do compiler generate:
```asm
push rbp
mov  rbp, rsp
sub  rsp, N      ; allocate stack frame
; ... function body ...
pop  rbp
ret
```

Context switch cần lưu/restore **tất cả** registers thủ công. Nếu Rust compiler chen vào bất kỳ instruction nào, registers bị corrupt.

### Solution: Naked Function

```rust
#[unsafe(naked)]
unsafe extern "C" fn switch_context(old_rsp: *mut u64, new_rsp: u64) {
    // rdi = old_rsp (pointer để lưu rsp hiện tại)
    // rsi = new_rsp (rsp mới cần load)
    core::arch::asm!(
        // Save callee-saved registers theo System V AMD64 ABI
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rbp",
        // Save current RSP vào *old_rsp
        "mov [rdi], rsp",
        // Load new RSP
        "mov rsp, rsi",
        // Restore registers của task mới
        "pop rbp",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        // Return vào task mới (RIP đã được push khi task bị switch out)
        "ret",
        options(noreturn)
    );
}
```

`options(noreturn)` là bắt buộc cho naked functions — không có return statement Rust.

**Callee-saved registers (System V ABI):** rbx, rbp, r12-r15. Caller-saved (rax, rdi, rsi, rdx, rcx, r8, r9, r10, r11) không cần save vì caller chịu trách nhiệm.

---

## Pattern 4: `#[repr(C, packed)]` cho Hardware Structs

### Vấn đề

Rust có thể reorder fields và add padding:
```rust
struct Foo {
    a: u8,    // Rust có thể pad 3 bytes ở đây
    b: u32,
}
// sizeof(Foo) = 8 (với padding), nhưng hardware expect 5
```

Hardware structs cần layout chính xác, không có padding:

```rust
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
```

`#[repr(C)]` — C layout (field order preserved, padding như C).
`packed` — bỏ tất cả padding, fields kề nhau.

### Cạm bẫy của `packed`

```rust
#[repr(C, packed)]
struct Packed {
    a: u8,
    b: u32,  // unaligned! tại offset 1
}

let p = Packed { a: 1, b: 2 };
let r = &p.b;  // ERROR: reference to packed field may be unaligned
```

Lấy reference của field unaligned là UB trên x86 (dù x86 support unaligned access, Rust forbid reference đến unaligned location).

**Fix:** Copy ra local variable trước:
```rust
let b = p.b;  // copy (load unaligned)
let r = &b;   // reference đến aligned local
```

Hoặc dùng `ptr::read_unaligned`:
```rust
let b = unsafe { core::ptr::read_unaligned(&p.b) };
```

---

## Pattern 5: `AtomicWaker` cho Async IO

### Flow: Interrupt → Async Task

```rust
// Interrupt handler (runs in interrupt context):
fn keyboard_handler(_: InterruptStackFrame) {
    let scancode = unsafe { Port::new(0x60).read() };
    
    // Push vào lock-free queue
    if let Ok(q) = SCANCODE_QUEUE.try_get() {
        let _ = q.push(scancode);
        WAKER.wake();  // ← wake waiting async task
    }
}

// Async task (runs in executor context):
impl Stream for ScancodeStream {
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<u8>> {
        let q = SCANCODE_QUEUE.try_get().expect("not init");
        
        if let Some(sc) = q.pop() {
            return Poll::Ready(Some(sc));
        }
        
        // Register waker TRƯỚC khi check lần cuối (avoid race)
        WAKER.register(cx.waker());
        
        match q.pop() {
            Some(sc) => {
                WAKER.take();  // clean up
                Poll::Ready(Some(sc))
            }
            None => Poll::Pending,
        }
    }
}
```

**Tại sao register waker trước lần check cuối?**

Race condition nếu không:
1. Task check queue → empty
2. Interrupt fires, push scancode, call wake() → nhưng không có waker đã register!
3. Task register waker
4. → Task ngủ mãi mãi, không ai wake nó

Bằng cách register trước, nếu interrupt fire giữa bước 1 và 3, wake() sẽ thấy waker và notify executor.

---

## Pattern 6: `OnceCell` cho Lazy Initialization

```rust
use conquer_once::spin::OnceCell;

static SCANCODE_QUEUE: OnceCell<ArrayQueue<u8>> = OnceCell::uninit();

// Khởi tạo exactly once, safe với concurrent calls:
SCANCODE_QUEUE.try_init_once(|| ArrayQueue::new(100))
    .expect("already initialized");

// Sau đó get:
if let Ok(q) = SCANCODE_QUEUE.try_get() {
    // use q
}
```

**Tại sao không dùng `lazy_static!`?**

`lazy_static!` khởi tạo lần đầu khi access. `OnceCell::try_init_once` explicit hơn — biết rõ khi nào init và có thể handle lỗi nếu init 2 lần.

**`try_init_once` vs `init_once`:**
- `init_once`: panic nếu gọi 2 lần
- `try_init_once`: trả `Err` nếu đã init, có thể ignore

---

## Pattern 7: Inline Assembly Constraints

```rust
// ❌ Sai — LLVM dùng rbx internally:
core::arch::asm!(
    "cpuid",
    inout("eax") 1u32 => _,
    out("ebx") result,  // rbx không available!
);

// ✓ Đúng — save/restore rbx thủ công:
core::arch::asm!(
    "push rbx",
    "cpuid",
    "mov edi, ebx",   // copy kết quả sang edi
    "pop rbx",
    out("edi") result, // dùng edi để lấy kết quả
    inout("eax") 1u32 => _,
    out("ecx") _,
    out("edx") _,
);
```

**LLVM reserved registers:** rbx, r12-r15 (callee-saved, LLVM có thể dùng để spill). Không dùng làm output operand trực tiếp.

**Common constraints:**
```rust
core::arch::asm!(
    "in al, dx",
    in("dx")  port,          // input: port number
    out("al") value,         // output: read byte
    options(nomem, nostack)  // không access memory/stack → optimizer hint
);

core::arch::asm!(
    "out dx, al",
    in("dx") port,
    in("al") value,
    options(nomem, nostack, preserves_flags)  // không thay đổi flags
);
```

**`options` flags:**
- `nomem`: asm không đọc/ghi memory (ngoài explicit operands)
- `nostack`: asm không access stack
- `preserves_flags`: asm không thay đổi condition flags
- `pure`: asm có cùng output với cùng input (có thể CSE)
- `noreturn`: asm không return (naked functions)

---

## Pattern 8: `UnsafeCell` cho Interior Mutability

```rust
pub struct SeqLock<T: Copy> {
    sequence: AtomicU32,
    data: UnsafeCell<T>,  // ← mutable shared data
}

// SAFETY: SeqLock implements correct synchronization
unsafe impl<T: Copy + Send> Sync for SeqLock<T> {}

impl<T: Copy> SeqLock<T> {
    pub fn write(&self, val: T) {
        // &self là immutable reference, nhưng cần mutate data
        self.sequence.fetch_add(1, Ordering::Release);
        unsafe {
            *self.data.get() = val;  // get() trả về *mut T
        }
        self.sequence.fetch_add(1, Ordering::Release);
    }
}
```

`UnsafeCell<T>` là primitive duy nhất cho phép mutate qua shared reference hợp lệ trong Rust. Mọi interior mutability (`Mutex`, `RwLock`, `Cell`, `RefCell`) đều build trên `UnsafeCell`.

**Khi dùng `UnsafeCell` trực tiếp**, bạn phải tự đảm bảo safety invariants (không có aliased mutable references, proper synchronization).

---

## Pattern 9: Trait Objects vs Generics trong Kernel

### Generics (monomorphization)

```rust
fn read_file<F: FileSystem>(fs: &F, path: &str) -> Vec<u8> { ... }
// Compiler tạo 1 bản cho mỗi concrete type: read_file<RamFs>, read_file<Fat32Fs>
// → Code size tăng, nhưng không có virtual dispatch overhead
```

### Trait Objects (dynamic dispatch)

```rust
fn read_file(fs: &dyn FileSystem, path: &str) -> Vec<u8> { ... }
// 1 bản duy nhất, dùng vtable cho dispatch
// → Code size nhỏ hơn, có virtual dispatch overhead (~2-3 cycles)
```

**Kernel thường ưu tiên trait objects** vì:
1. Filesystem, File handler cần heterogeneous collections (`Vec<Arc<dyn FileSystem>>`)
2. Code size quan trọng hơn performance trong kernel context (cache pressure)
3. Virtual dispatch overhead negligible so với disk I/O hay network

**Khi nào dùng generics:**
- Hot path thực sự (không có trong MyKernel hiện tại)
- Compile-time polymorphism cần thiết (như `PhysFrame<S>` size-typed frames)

---

## Pattern 10: `Send + Sync` Bounds cho Kernel Types

```rust
// FileSystem phải Send + Sync vì:
// - Mount table là global static (Sync required)
// - Có thể access từ nhiều CPUs (Send required)
pub trait FileSystem: Send + Sync {
    fn open(&self, ...) -> ...;
}

// Arc<Mutex<dyn File>> tự động Send + Sync nếu:
// - File: Send (có thể move giữa threads)
// - Mutex<T>: Sync nếu T: Send
```

**Khi type KHÔNG phải Send/Sync:**

```rust
use core::cell::Cell;  // Cell<T> không Sync (interior mutability không thread-safe)

struct NotThreadSafe {
    counter: Cell<u32>,  // ← Cell không Sync
}

static GLOBAL: NotThreadSafe = ...;  // ERROR: NotThreadSafe không Sync
```

Fix: dùng `AtomicU32` thay vì `Cell<u32>`, hoặc wrap trong `Mutex`.

---

## Common Anti-patterns

### 1. Holding lock qua await point

```rust
// ❌ Deadlock risk:
async fn bad() {
    let guard = LOCK.lock();
    some_async_fn().await;  // task có thể suspend, guard vẫn held!
    drop(guard);
}

// ✓ Release lock trước await:
async fn good() {
    let data = {
        let guard = LOCK.lock();
        *guard  // copy data ra
    };  // guard drop ở đây
    some_async_fn().await;
}
```

### 2. Disable interrupts quá lâu

```rust
// ❌ Interrupts bị disable cả function dài:
fn bad() {
    let guard = SPINLOCK.lock();  // disable interrupts
    slow_disk_operation();        // timer interrupt bị miss!
    drop(guard);                  // re-enable
}

// ✓ Chỉ disable trong critical section nhỏ:
fn good() {
    let data = {
        let guard = SPINLOCK.lock();
        *guard  // copy nhanh
    };
    slow_disk_operation_with(data);
}
```

### 3. `unwrap()` trong kernel

```rust
// ❌ Panic trong kernel = crash toàn hệ thống:
let file = find_file(path).unwrap();

// ✓ Handle lỗi gracefully:
let file = match find_file(path) {
    Some(f) => f,
    None => return Err(FsError::NotFound),
};
```

Exception: `unwrap()` trong init code (nếu fail thì không thể tiếp tục), phải có comment giải thích tại sao safe.
