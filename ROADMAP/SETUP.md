# MyKernel — Hướng Dẫn Setup Môi Trường

> Hướng dẫn chi tiết cách cài đặt môi trường cho **MyKernel** — một OS kernel viết bằng Rust từ đầu, chạy trên bare-metal x86_64 qua QEMU.  
> Dự án gồm **24 phases**, từ freestanding binary đến TCP/IP stack và security hardening.

---

## Mục Lục

1. [Yêu cầu hệ thống](#1-yêu-cầu-hệ-thống)
2. [Cài đặt các phần mềm cần thiết](#2-cài-đặt-các-phần-mềm-cần-thiết)
   - 2.1 [Rust (Nightly)](#21-rust-nightly)
   - 2.2 [QEMU](#22-qemu)
   - 2.3 [Công cụ hỗ trợ](#23-công-cụ-hỗ-trợ)
3. [Cấu trúc dự án](#3-cấu-trúc-dự-án)
4. [Tạo project mới từ đầu](#4-tạo-project-mới-từ-đầu)
5. [Cấu hình các file quan trọng](#5-cấu-hình-các-file-quan-trọng)
6. [Build và chạy](#6-build-và-chạy)
7. [Chạy tests](#7-chạy-tests)
8. [Chạy với networking](#8-chạy-với-networking)
9. [Chạy với virtio block device](#9-chạy-với-virtio-block-device)
10. [Troubleshooting](#10-troubleshooting)
11. [Tóm tắt các lệnh hay dùng](#11-tóm-tắt-các-lệnh-hay-dùng)

---

## 1. Yêu cầu hệ thống

| Thành phần | Yêu cầu |
|------------|---------|
| **OS** | Windows 10/11 (64-bit) |
| **RAM** | Tối thiểu 8GB (khuyến nghị 16GB) |
| **Disk** | ~5GB trống (Rust toolchain + build artifacts) |
| **CPU** | x86_64 với virtualization support |

> **Lưu ý:** Hướng dẫn này dành cho **Windows** với **PowerShell**. Trên macOS/Linux các lệnh tương tự nhưng path sẽ khác.

---

## 2. Cài đặt các phần mềm cần thiết

### 2.1 Rust (Nightly)

Dự án yêu cầu **Rust Nightly** vì dùng các unstable features như `abi_x86_interrupt`, `naked_functions`, `custom_test_frameworks`.

**Bước 1:** Tải và cài đặt `rustup` từ [https://rustup.rs](https://rustup.rs)

Chạy trình cài đặt, chọn option mặc định (1 - Proceed with standard installation).

**Bước 2:** Mở PowerShell mới, cài Nightly toolchain:

```powershell
rustup toolchain install nightly
rustup default nightly
```

**Bước 3:** Kiểm tra phiên bản:

```powershell
rustc --version
# Kết quả mong đợi: rustc 1.xx.0-nightly (...)
cargo --version
# Kết quả mong đợi: cargo 1.xx.0-nightly (...)
```

**Bước 4:** Thêm target `x86_64-unknown-none` (bare-metal):

```powershell
rustup target add x86_64-unknown-none
```

**Bước 5:** Thêm Rust source (cần để build `core` và `alloc` cho custom target):

```powershell
rustup component add rust-src
```

**Bước 6:** Thêm `llvm-tools-preview` (cần cho bootimage):

```powershell
rustup component add llvm-tools-preview
```

**Bước 7:** Cài `bootimage` — công cụ tạo bootable disk image:

```powershell
cargo install bootimage
```

> Quá trình này mất khoảng 5-10 phút lần đầu.

**Bước 8:** Kiểm tra bootimage đã cài thành công:

```powershell
bootimage --version
# Kết quả: bootimage 0.10.3
```

---

### 2.2 QEMU

QEMU là emulator dùng để chạy kernel mà không cần phần cứng thật.

**Bước 1:** Tải QEMU từ [https://www.qemu.org/download/#windows](https://www.qemu.org/download/#windows)

Chọn phiên bản **Windows 64-bit installer** mới nhất (ví dụ: `qemu-w64-setup-20240221.exe`).

**Bước 2:** Chạy installer, chọn đường dẫn cài đặt (mặc định: `C:\Program Files\qemu`).

**Bước 3:** Thêm QEMU vào PATH. Mở **System Properties → Environment Variables → System Variables → Path**, thêm:

```
C:\Program Files\qemu
```

Hoặc chạy lệnh PowerShell (cần Admin):

```powershell
$env:PATH += ";C:\Program Files\qemu"
[Environment]::SetEnvironmentVariable("PATH", $env:PATH + ";C:\Program Files\qemu", "Machine")
```

**Bước 4:** Mở PowerShell mới, kiểm tra:

```powershell
qemu-system-x86_64 --version
# Kết quả: QEMU emulator version 8.x.x
```

---

## 3. Build và chạy

### Build

```powershell
# Build debug (nhanh hơn, có debug symbols)
cargo build

# Build release (tối ưu hóa)
cargo build --release
```

### Tạo bootable image và chạy

```powershell
# Cách 1: Dùng cargo run (tự động tạo image và khởi động QEMU)
cargo run

# Cách 2: Tạo image thủ công
cargo bootimage

# Chạy QEMU thủ công với image đã tạo
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin
```

### Chạy QEMU với serial output (quan trọng cho debugging)

```powershell
cargo bootimage
qemu-system-x86_64 `
    -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
    -serial stdio `
    -no-reboot
```

> **Giải thích các flag QEMU:**
> - `-serial stdio` — redirect serial port ra terminal (thấy `serial_println!` output)
> - `-no-reboot` — không reboot khi kernel panic, giúp đọc error message
> - `-display none` — không hiển thị cửa sổ QEMU (dùng cho CI/testing)
> - `-nographic` — chạy hoàn toàn không có GUI

---

## 4. Chạy tests

Dự án dùng custom test framework chạy tests trong QEMU thật:

```powershell
# Chạy tất cả tests
cargo test

# Chạy test với output verbose (thấy từng test)
cargo test -- --nocapture

# Chạy 1 test file cụ thể
cargo test --test heap_allocation

# Chạy unit tests trong lib.rs
cargo test --lib
```

**Kết quả mong đợi:**

```
Running 6 tests
mykernel::test_breakpoint_exception...  [ok]
mykernel::test_stack_canary...          [ok]
mykernel::test_capabilities...          [ok]
...
Running 3 tests
simple_allocation...    [ok]
large_vec...    [ok]
many_boxes...   [ok]
[ok] Double fault handler triggered correctly
```

> **Lưu ý quan trọng:** Mỗi test chạy trong một QEMU instance riêng biệt. Điều này đảm bảo tests độc lập nhau nhưng cũng làm chúng chạy chậm hơn.

---

## 5. Chạy với networking

Để test các tính năng mạng (Phase 21-23):

```powershell
# Build bootimage trước
cargo bootimage

# Chạy với virtio-net (user-mode networking)
qemu-system-x86_64 `
    -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
    -netdev user,id=net0 `
    -device virtio-net-pci,netdev=net0 `
    -serial stdio `
    -no-reboot
```

**Test ping từ host:**

```powershell
# Trong một terminal khác (trong khi QEMU đang chạy)
ping 10.0.2.15

# Kết quả mong đợi: QEMU hiển thị "[net] PING from ..."
```

**Test UDP echo (port 7):**

```powershell
# Cài ncat nếu chưa có (trong Git Bash hoặc WSL)
echo "hello" | nc -u 10.0.2.15 7
```

> **Địa chỉ IP mặc định của kernel:** `10.0.2.15` (QEMU user-mode default)
> **Gateway:** `10.0.2.2`

---

## 6. Chạy với virtio block device

Để test tính năng đọc/ghi đĩa (Phase 16-17):

**Bước 1:** Tạo disk image:

```powershell
# Tạo file disk image 64MB (raw format)
$disk = New-Object byte[](64 * 1024 * 1024)
[System.IO.File]::WriteAllBytes("E:\New folder\mykernel\disk.img", $disk)
```

**Bước 2:** Chạy QEMU với disk:

```powershell
cargo bootimage
qemu-system-x86_64 `
    -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
    -drive format=raw,file=disk.img,if=virtio `
    -serial stdio `
    -no-reboot
```

---

## 7. Tóm tắt các lệnh hay dùng

```powershell
# ─────────────────────────────────────
# Build
# ─────────────────────────────────────
cargo build                    # Build debug
cargo build --release          # Build release
cargo bootimage                # Tạo bootable .bin image
cargo clean                    # Xóa build artifacts

# ─────────────────────────────────────
# Chạy
# ─────────────────────────────────────
cargo run                      # Build + chạy trong QEMU (cửa sổ VGA)

# Chạy với serial output ra terminal
cargo bootimage
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -serial stdio -no-reboot

# Chạy với networking
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -netdev user,id=net0 -device virtio-net-pci,netdev=net0 -serial stdio -no-reboot

# Chạy với disk + networking
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -drive format=raw,file=disk.img,if=virtio -netdev user,id=net0 -device virtio-net-pci,netdev=net0 -serial stdio -no-reboot

# ─────────────────────────────────────
# Test
# ─────────────────────────────────────
cargo test                     # Chạy tất cả tests
cargo test --lib               # Chỉ unit tests
cargo test --test heap_allocation  # Chỉ 1 integration test
cargo test 2>&1 | Select-String "\[ok\]|\[failed\]"  # Tóm tắt kết quả

# ─────────────────────────────────────
# Debugging
# ─────────────────────────────────────
# Xem symbols trong binary
cargo build
rust-objdump --disassemble target\x86_64-mykernel\debug\mykernel | head -100

# Chạy QEMU với GDB stub (để debug với gdb)
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -s -S
# Trong terminal khác: gdb target\x86_64-mykernel\debug\mykernel -ex "target remote :1234"
```

---

## Ghi chú về phiên bản

| Công cụ | Phiên bản đã test |
|---------|-----------------|
| Rust (Nightly) | `nightly-2024-xx-xx` |
| bootloader | `0.9.34` |
| bootimage | `0.10.3` |
| QEMU | `8.x.x` |
| x86_64 crate | `0.14.13` |
| spin crate | `0.9.8` |

> **Quan trọng:** Do dùng Nightly Rust, một số unstable features có thể thay đổi API. Nếu gặp lỗi compile lạ, thử pin xuống một ngày cụ thể trong `rust-toolchain.toml`:
> ```toml
> [toolchain]
> channel = "nightly-2024-12-01"
> ```

---

## Nguồn tham khảo

- [Writing an OS in Rust](https://os.phil-opp.com/) — Blog series của Philipp Oppermann, nền tảng của dự án này
- [OSDev Wiki](https://wiki.osdev.org/) — Reference cho x86 hardware programming
- [Intel Software Developer Manual](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) — Tài liệu chính thức về x86_64
- [Virtio Specification](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html) — Spec cho virtio devices
- [Linux Syscall Table](https://filippo.io/linux-syscall-table/) — Tham khảo syscall numbers

---

*Tài liệu này được tạo cho MyKernel — dự án OS kernel 24 phases viết bằng Rust.*  
*~3500 dòng code | bare-metal x86_64 | QEMU*
