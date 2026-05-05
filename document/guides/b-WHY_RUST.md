# Tại Sao Dùng Rust cho Kernel?

> Giải thích lý do MyKernel chọn Rust thay vì C/C++, và những khái niệm Rust đặc thù quan trọng khi viết kernel.

---

## Vấn đề với C kernel

Phần lớn OS kernels trong lịch sử viết bằng C: Linux, Windows NT, macOS (XNU), FreeBSD. C cho phép kiểm soát hardware trực tiếp, nhanh, và không có overhead. Nhưng C có một vấn đề lớn: **không có memory safety**.

### Các lỗi phổ biến trong C kernel

**Buffer overflow:**
```c
char buf[64];
memcpy(buf, user_input, user_len);  // user_len có thể > 64!
```
Ghi quá buffer → ghi đè dữ liệu khác → crash hoặc security exploit.

**Use-after-free:**
```c
struct Task *t = alloc_task();
free(t);
// ... sau đó ...
t->pid = 1234;  // t đã bị free! undefined behavior
```

**Null pointer dereference:**
```c
struct File *f = find_file(path);
// quên check null
f->size = 0;  // crash nếu f == NULL
```

**Data race:**
```c
// CPU 0:              // CPU 1:
counter++;             counter++;
// Không có lock → kết quả không xác định trên SMP
```

Những lỗi này trong **user space** thì crash app. Trong **kernel** thì crash toàn bộ hệ thống, hoặc tệ hơn — tạo ra security vulnerability cho hacker khai thác.

Linux kernel có hàng nghìn CVE (lỗ hổng bảo mật) theo năm, phần lớn là memory safety issues.

---

## Rust giải quyết vấn đề thế nào

Rust có **ownership system** và **borrow checker** — compiler kiểm tra memory safety tại compile time, không cần runtime overhead.

### Ownership — mỗi giá trị có 1 owner

```rust
let v = Vec::new();     // v owns the Vec
let w = v;              // ownership move sang w
println!("{:?}", v);    // COMPILE ERROR: v đã bị move
```

Compiler từ chối code này — không có use-after-move.

### Borrow checker — kiểm tra tham chiếu

```rust
let mut data = vec![1, 2, 3];
let r = &data[0];        // immutable borrow
data.push(4);            // COMPILE ERROR: không thể mutate khi có borrow
println!("{}", r);
```

Không có dangling pointer — compiler đảm bảo reference luôn hợp lệ.

### Không có null — dùng Option

```rust
fn find_file(path: &str) -> Option<File> {
    // trả về Some(file) hoặc None
}

let file = find_file("/etc/hosts");
file.size  // COMPILE ERROR: phải unwrap Option trước
```

Buộc lập trình viên xử lý trường hợp "không tìm thấy" — không có null pointer dereference.

### Thread safety — Send + Sync traits

```rust
// Compiler từ chối share non-thread-safe data giữa threads
let counter = Rc::new(0);  // Rc không phải Send
thread::spawn(move || {
    *counter += 1;  // COMPILE ERROR
});
```

Data race được phát hiện tại compile time.

---

## `#![no_std]` — Rust không có standard library

Rust standard library (`std`) phụ thuộc vào OS: file I/O, threads, heap allocator, network... đều cần OS support.

Khi viết kernel (chính là OS), không có OS nào bên dưới để gọi. Do đó kernel dùng `#![no_std]` — chỉ dùng `core` library (không cần OS).

```rust
#![no_std]       // không có std
#![no_main]      // không có fn main() — kernel có entry point riêng

extern crate alloc;  // dùng alloc crate cho Box, Vec, String
                     // nhưng phải tự cung cấp allocator
```

Điều này có nghĩa:
- Không có `println!` → phải viết VGA/serial driver
- Không có `Vec`, `String` mặc định → phải init heap allocator trước
- Không có `thread::spawn` → phải viết scheduler
- Không có `File::open` → phải viết filesystem
- Không có network stack → phải viết TCP/IP

MyKernel tự implement tất cả những thứ trên từ đầu.

---

## So sánh C vs Rust trong kernel code

### Ví dụ 1: Đọc từ file descriptor

**C (Linux kernel style):**
```c
ssize_t sys_read(int fd, char __user *buf, size_t count) {
    struct file *f;
    ssize_t ret;

    f = fget(fd);
    if (!f)
        return -EBADF;

    // validate user pointer — dễ quên
    if (!access_ok(buf, count))
        return -EFAULT;

    ret = f->f_op->read(f, buf, count, &f->f_pos);
    fput(f);
    return ret;
}
```

**Rust (MyKernel style):**
```rust
fn sys_read(fd: u64, buf_ptr: u64, count: u64) -> i64 {
    // validate user pointer — compiler nhắc nếu quên
    if !validate_user_ptr(buf_ptr, count as usize) {
        return -EFAULT;
    }

    let file = match get_file(fd) {
        Some(f) => f,
        None => return -EBADF,   // buộc xử lý None case
    };

    let buf = unsafe { /* copy from user */ };
    let x = match file.lock().read(buf) {
        Ok(n) => n as i64,
        Err(_) => -EIO,
    };
    x
}
```

Rust buộc xử lý mọi error case — không thể bỏ qua như C.

### Ví dụ 2: Spinlock

**C:**
```c
spinlock_t lock = SPIN_LOCK_UNLOCKED;

spin_lock(&lock);
// critical section
spin_unlock(&lock);  // dễ quên, hoặc early return mà quên unlock
```

**Rust:**
```rust
let lock = SpinLock::new(data);

let guard = lock.lock();
// critical section — guard tự động unlock khi ra khỏi scope
// không thể quên unlock
```

`Drop` trait đảm bảo lock luôn được release khi guard ra khỏi scope — kể cả khi có early return hay panic.

---

## Những chỗ vẫn cần `unsafe`

Rust không thể verify mọi thứ — một số thao tác hardware về bản chất là unsafe và cần được đánh dấu rõ ràng:

```rust
// Đọc hardware register
unsafe {
    core::arch::asm!("in al, dx", in("dx") port, out("al") value);
}

// Dereference raw pointer (từ bootloader)
unsafe {
    let vga_buffer = 0xb8000 as *mut u8;
    *vga_buffer = b'H';
}

// Context switch — phải lưu/restore registers thủ công
unsafe {
    core::arch::asm!(
        "mov [rdi], rsp",
        "mov rsp, rsi",
        ...
    );
}
```

`unsafe` block không tắt borrow checker — nó chỉ cho phép thêm một số thao tác mà borrow checker không thể verify. Lập trình viên phải đảm bảo tự tay rằng code trong `unsafe` là đúng.

Trong MyKernel, `unsafe` chỉ xuất hiện ở những chỗ thực sự cần thiết: hardware access, inline assembly, raw pointer từ bootloader.

---

## Kết luận

| | C | Rust |
|--|---|------|
| Memory safety | Không — lập trình viên tự quản lý | Có — compiler kiểm tra |
| Null pointer | Có thể crash | Không có — dùng Option |
| Data race | Có thể xảy ra | Phát hiện tại compile time |
| Buffer overflow | Có thể | Không — bounds check |
| Tốc độ | Nhanh | Nhanh như C |
| Learning curve | Thấp hơn | Cao hơn |
| Code kernel hiện có | Nhiều (Linux, BSD) | Ít hơn (Redox, MyKernel) |

Rust không phải silver bullet — vẫn có thể viết code sai trong `unsafe` block. Nhưng đối với phần lớn kernel code, Rust loại bỏ toàn bộ một category lỗi mà C developers phải kiểm tra thủ công.
