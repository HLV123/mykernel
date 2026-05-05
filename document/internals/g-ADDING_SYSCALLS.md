# Thêm Syscall Mới

> Hướng dẫn step-by-step để thêm một Linux-compatible syscall vào MyKernel.

---

## Tổng quan Flow

```
User process (Ring 3)
  │  mov rax, SYSCALL_NUMBER
  │  mov rdi, arg1
  │  mov rsi, arg2
  │  syscall
  ▼
syscall_handler() [src/usermode.rs]
  │  Save registers
  │  Call syscall_dispatch(rax, rdi, rsi, rdx, r10, r8, r9)
  ▼
syscall_dispatch() [src/syscall.rs]
  │  match rax {
  │    N => sys_new_call(rdi, rsi, ...)
  │  }
  ▼
sys_new_call() [src/syscall.rs]
  │  Validate arguments
  │  Implement logic
  │  Return value in rax
```

---

## Bước 1: Xác định Syscall Number

Tìm số syscall trong Linux syscall table:

```bash
# Trên Linux host:
cat /usr/include/asm/unistd_64.h | grep sys_name

# Hoặc tra online:
# https://filippo.io/linux-syscall-table/
# https://syscalls.mebeim.net/?table=x86/64/x64/latest
```

**Ví dụ:** `getpid` = 39, `gettid` = 186, `getcwd` = 79

---

## Bước 2: Thêm vào Dispatch Table

**File: `src/syscall.rs`**

Tìm function `syscall_dispatch`:

```rust
pub fn syscall_dispatch(
    num: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64
) -> i64 {
    match num {
        0  => sys_read(a1, a2, a3),
        1  => sys_write(a1, a2, a3),
        // ...
        39 => sys_getpid(),
        // Thêm syscall mới:
        NEW_NUMBER => sys_new_call(a1, a2, a3),
        _ => {
            crate::serial_println!("[syscall] unhandled: {}", num);
            -38  // ENOSYS
        }
    }
}
```

---

## Bước 3: Implement Handler Function

### Template cơ bản

```rust
/// sys_new_call(arg1: type1, arg2: type2) -> i64
///
/// Mô tả: làm gì
/// Returns: 0 on success, negative errno on error
fn sys_new_call(arg1: u64, arg2: u64) -> i64 {
    // Implement logic here
    0  // success
}
```

### Với pointer argument

```rust
fn sys_getcwd(buf_ptr: u64, size: u64) -> i64 {
    // Validate user pointer
    if !crate::security::validate_user_ptr(buf_ptr, size as usize) {
        return -14; // EFAULT
    }

    let cwd = b"/\0";  // kernel's current working directory
    if (size as usize) < cwd.len() {
        return -34;  // ERANGE
    }

    // Copy to user space safely
    match crate::security::copy_to_user(buf_ptr, cwd) {
        Ok(()) => buf_ptr as i64,  // return pointer (Linux convention)
        Err(_) => -14,  // EFAULT
    }
}
```

### Với struct argument

```rust
#[repr(C)]
struct Timeval {
    tv_sec:  u64,
    tv_usec: u64,
}

fn sys_gettimeofday(tv_ptr: u64, _tz_ptr: u64) -> i64 {
    if tv_ptr == 0 {
        return 0;  // NULL tv is valid (just ignore)
    }

    if !crate::security::validate_user_ptr(tv_ptr, core::mem::size_of::<Timeval>()) {
        return -14;  // EFAULT
    }

    let ticks = crate::sync::get_ticks();
    let tv = Timeval {
        tv_sec:  ticks / 100,
        tv_usec: (ticks % 100) * 10_000,
    };

    match crate::security::copy_to_user(tv_ptr, unsafe {
        core::slice::from_raw_parts(
            &tv as *const Timeval as *const u8,
            core::mem::size_of::<Timeval>(),
        )
    }) {
        Ok(()) => 0,
        Err(_) => -14,
    }
}
```

### Với string argument (path)

```rust
fn sys_mkdir(path_ptr: u64, _mode: u64) -> i64 {
    // Read string from user space
    let path = match read_user_string(path_ptr, 4096) {
        Some(s) => s,
        None => return -14,  // EFAULT
    };

    match crate::fs::mkdir(&path) {
        Ok(())  => 0,
        Err(crate::fs::FsError::NotFound)    => -2,   // ENOENT
        Err(crate::fs::FsError::FileExists)  => -17,  // EEXIST
        Err(_)                               => -1,   // EIO
    }
}

/// Read a null-terminated string from user space.
/// Returns None if pointer is invalid or string too long.
fn read_user_string(ptr: u64, max_len: usize) -> Option<alloc::string::String> {
    if !crate::security::validate_user_ptr(ptr, 1) {
        return None;
    }

    let mut s = alloc::vec::Vec::new();
    for i in 0..max_len {
        let byte_ptr = ptr + i as u64;
        if !crate::security::validate_user_ptr(byte_ptr, 1) {
            return None;
        }
        let byte = unsafe { *(byte_ptr as *const u8) };
        if byte == 0 { break; }
        s.push(byte);
    }

    alloc::string::String::from_utf8(s).ok()
}
```

---

## Bước 4: Thêm vào Syscall Number Table

Nếu muốn có constant cho readability:

```rust
// Ở đầu src/syscall.rs:
const SYS_READ:    u64 = 0;
const SYS_WRITE:   u64 = 1;
// ...
const SYS_NEW_CALL: u64 = NEW_NUMBER;

// Trong dispatch:
match num {
    SYS_READ  => sys_read(a1, a2, a3),
    SYS_WRITE => sys_write(a1, a2, a3),
    SYS_NEW_CALL => sys_new_call(a1, a2),
    // ...
}
```

---

## Bước 5: Test

### Test trực tiếp từ shell (nếu syscall affect filesystem)

```
kernel> mkdir /tmp/testdir
kernel> ls /tmp
```

### Test với inline test case

Thêm vào `src/main.rs`:

```rust
#[test_case]
fn test_sys_new_call() {
    // Gọi trực tiếp handler (không qua SYSCALL instruction)
    let result = mykernel::syscall::sys_new_call(0, 0);
    assert_eq!(result, 0, "sys_new_call should return 0");
    crate::serial_println!("[test] sys_new_call ok");
}
```

### Test với musl binary (advanced)

Compile một C program với musl và chạy:

```c
// test_syscall.c
#include <sys/syscall.h>
#include <unistd.h>
#include <stdio.h>

int main() {
    long result = syscall(NEW_NUMBER, arg1, arg2);
    printf("result: %ld\n", result);
    return 0;
}
```

```bash
# Compile static:
musl-gcc -static -o test_syscall test_syscall.c

# Load vào initramfs và chạy trong kernel
```

---

## Errno Reference

```rust
// Common errno values (negative):
-1  => EPERM    // Operation not permitted
-2  => ENOENT   // No such file or directory
-9  => EBADF    // Bad file descriptor
-12 => ENOMEM   // Out of memory
-13 => EACCES   // Permission denied
-14 => EFAULT   // Bad address (invalid user pointer)
-17 => EEXIST   // File exists
-19 => ENODEV   // No such device
-20 => ENOTDIR  // Not a directory
-21 => EISDIR   // Is a directory
-22 => EINVAL   // Invalid argument
-34 => ERANGE   // Result too large
-38 => ENOSYS   // Function not implemented
-98 => EADDRINUSE  // Address already in use
```

---

## Example: sys_getdents64 (list directory)

Đây là syscall phức tạp hơn — trả về array of structs vào user buffer.

```rust
#[repr(C)]
struct LinuxDirent64 {
    d_ino:    u64,    // inode number
    d_off:    i64,    // offset to next entry
    d_reclen: u16,    // length of this record
    d_type:   u8,     // file type
    d_name:   [u8; 0], // variable length name
}
// DT_UNKNOWN=0, DT_REG=8, DT_DIR=4

fn sys_getdents64(fd: u64, buf_ptr: u64, count: u64) -> i64 {
    if !crate::security::validate_user_ptr(buf_ptr, count as usize) {
        return -14;  // EFAULT
    }

    // Get path from FD (simplified — we store path in FdTable)
    let path = match get_path_for_fd(fd) {
        Some(p) => p,
        None => return -9,  // EBADF
    };

    let entries = match crate::fs::readdir(&path) {
        Ok(e) => e,
        Err(_) => return -2,  // ENOENT
    };

    let mut written = 0usize;
    let buf_end = buf_ptr + count;

    for entry in &entries {
        let name = entry.name.as_bytes();
        let name_len = name.len() + 1;  // +1 for null
        // Align to 8 bytes
        let reclen = (core::mem::size_of::<LinuxDirent64>() + name_len + 7) & !7;

        if buf_ptr + written as u64 + reclen as u64 > buf_end {
            break;  // buffer full
        }

        let dirent_ptr = (buf_ptr + written as u64) as *mut u8;

        // Write d_ino, d_off, d_reclen, d_type
        unsafe {
            let d = dirent_ptr as *mut LinuxDirent64;
            (*d).d_ino    = 1;  // fake inode
            (*d).d_off    = (written + reclen) as i64;
            (*d).d_reclen = reclen as u16;
            (*d).d_type   = match entry.file_type {
                crate::fs::FileType::Directory   => 4,   // DT_DIR
                crate::fs::FileType::RegularFile => 8,   // DT_REG
                _                                => 0,   // DT_UNKNOWN
            };
            // Write name after struct
            let name_ptr = dirent_ptr.add(core::mem::size_of::<LinuxDirent64>());
            core::ptr::copy_nonoverlapping(name.as_ptr(), name_ptr, name.len());
            *name_ptr.add(name.len()) = 0;  // null terminator
        }

        written += reclen;
    }

    written as i64
}
```

---

## Tips

**Luôn validate user pointers trước khi dereference:**
```rust
if !crate::security::validate_user_ptr(ptr, size) {
    return -14;  // EFAULT
}
```

**Sử dụng `copy_from_user` / `copy_to_user` thay vì raw pointer:**
```rust
let mut buf = vec![0u8; size];
crate::security::copy_from_user(&mut buf, user_ptr)?;
```

**Return đúng errno theo Linux convention:**
Tra cứu `errno` của syscall tương ứng trên Linux man page.

**Log syscalls không quen trong development:**
```rust
_ => {
    crate::serial_println!("[syscall] UNHANDLED: num={} a1={:#x} a2={:#x}", num, a1, a2);
    -38  // ENOSYS
}
```

**Test với strace trên Linux** để so sánh expected behavior:
```bash
strace ./your_binary 2>&1 | grep new_syscall
```
