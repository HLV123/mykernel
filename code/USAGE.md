# MyKernel — Hướng Dẫn Sử Dụng

> Tài liệu này hướng dẫn cách chạy và sử dụng MyKernel sau khi đã setup môi trường xong.
> Xem `MYKERNEL_SETUP_GUIDE.md` để biết cách cài đặt môi trường.

---

## Chạy kernel

### Chế độ cơ bản

```powershell
cd mykernel
cargo bootimage
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -serial stdio -no-reboot
```

Gõ lệnh trực tiếp vào PowerShell terminal. Nhấn **Ctrl+A** rồi **X** để thoát.

### Chế độ có network (virtio-net)

Trước khi chạy, tăng heap size trong `src/allocator.rs`:
```rust
pub const HEAP_SIZE: usize = 512 * 1024; // đổi từ 100 * 1024 thành 512 * 1024
```

Sau đó:
```powershell
cargo bootimage
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0 `
  -serial stdio -no-reboot
```

### Chế độ có disk (virtio-blk)

```powershell
# Tạo disk image 64MB (chỉ cần làm 1 lần)
$disk = New-Object byte[](64 * 1024 * 1024)
[System.IO.File]::WriteAllBytes("disk.img", $disk)

cargo bootimage
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -drive format=raw,file=disk.img,if=virtio `
  -serial stdio -no-reboot
```

### Chế độ đầy đủ (network + disk)

```powershell
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -drive format=raw,file=disk.img,if=virtio `
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0 `
  -serial stdio -no-reboot
```

---

## Boot banner

Khi kernel khởi động thành công sẽ thấy:

```
  __  __       _  __                    _
 |  \/  |_   _| |/ /___ _ __ _ __   ___| |
 | |\/| | | | | ' // _ \ '__| '_ \ / _ \ |
 | |  | | |_| | . \  __/ |  | | | |  __/ |
 |_|  |_|\__, |_|\_\___|_|  |_| |_|\___|_|
          |___/

  Bare-metal OS kernel  |  Rust  |  x86_64

  [ok] GDT / IDT / PIC
  [ok] Virtual memory + heap
  [ok] Filesystem  (VFS + initramfs)
  [ok] Syscalls    (Linux x86_64 ABI, 40 calls)
  [ok] virtio-net  MAC 52:54:00:12:34:56  IP 10.0.2.15
  [ok] Security    score 60/100
  [ok] APIC        BSP ID=0  1 CPU(s) detected

  Type 'help' for available commands.

kernel>
```

---

## Các lệnh shell

### `help` — Xem danh sách lệnh

```
kernel> help
```

---

### Thông tin hệ thống

| Lệnh | Mô tả |
|------|-------|
| `uname` | Thông tin kernel, kiến trúc, CPU |
| `uptime` | Số ticks kể từ khi boot |
| `cpu` | APIC ID, số CPU detect được |
| `mem` | Kích thước heap |

```
kernel> uname
Kernel:  MyKernel
Release: 1.0.0
Arch:    x86_64
CPU:     QEMU Virtual CPU version 2.5+

kernel> cpu
CPU topology:
  BSP APIC ID: 0
  Total CPUs:  1
  Online CPUs: 1
```

---

### Filesystem

Cấu trúc thư mục mặc định sau khi boot:

```
/
├── README
├── bin/
│   ├── hello
│   └── init
├── etc/
│   ├── hostname
│   ├── motd
│   ├── os-release
│   └── shells
├── tmp/          ← ghi/đọc/xóa tại đây
├── proc/
└── usr/
    └── bin/
```

| Lệnh | Cú pháp | Ví dụ |
|------|---------|-------|
| `ls` | `ls [path]` | `ls /` hoặc `ls /etc` |
| `cat` | `cat <file>` | `cat /etc/motd` |
| `write` | `write <file> <text>` | `write /tmp/note.txt Hello` |
| `mkdir` | `mkdir <path>` | `mkdir /tmp/mydir` |
| `rm` | `rm <path>` | `rm /tmp/note.txt` |
| `echo` | `echo <text>` | `echo Hello World` |

**Lưu ý:**
- Filesystem là RamFS — lưu trong RAM, mất khi tắt kernel.
- Chỉ có thể ghi vào `/tmp` và các thư mục tự tạo.
- Lệnh `write` chỉ nhận tối đa 1 argument cho text — `write /tmp/a.txt Hello World` sẽ ghi `Hello World` (splitn 3).

---

### Security

```
kernel> security
```

Hiển thị trạng thái các tính năng bảo mật:

```
  [off] SMEP   — ngăn kernel thực thi code ở user pages
  [off] SMAP   — ngăn kernel access user memory không kiểm soát
  [on]  NX/XD  — data pages không thực thi được
  [on]  Canary — phát hiện stack corruption
  [on]  KASLR  — kernel load ở địa chỉ ngẫu nhiên mỗi lần boot
  [off] RDRAND — hardware RNG
  [on]  Hardened policy

  Score: 60/100  (GOOD)
```

> SMEP/SMAP/RDRAND = off vì QEMU TCG không emulate các CPU security features này. Trên real hardware sẽ đạt 80-100/100.

```
kernel> rand
Random bytes: bbb6ba45195e64743e4106c1900565f4
```

Mỗi lần gọi `rand` cho 16 bytes ngẫu nhiên khác nhau từ xoshiro256** CSPRNG.

---

### Network

> Cần chạy với `-netdev user,id=n0 -device virtio-net-pci,netdev=n0` và heap 512KB.

```
kernel> net
Interface: virtio-net
  MAC:     52:54:00:12:34:56
  IP:      10.0.2.15/24
  Gateway: 10.0.2.2
Protocols: ARP, IPv4, ICMP, UDP, TCP
Services:  ICMP echo responder, UDP echo (port 7)

kernel> netstat
  Listening services:
    UDP port 7  - echo server
    ICMP        - echo responder

kernel> ping 10.0.2.2
PING 10.0.2.2
[virtio-net] TX: 56 bytes
  sent 56 bytes
```

> **Giới hạn trên Windows:** QEMU user-mode NAT không cho phép ping từ host vào kernel (`ping 10.0.2.15` từ PowerShell sẽ timeout). Đây là giới hạn của QEMU trên Windows, không phải lỗi kernel. Trên Linux với KVM + TAP interface thì ping 2 chiều hoạt động đầy đủ.

```
kernel> socket
Socket API demonstration:
  socket(AF_INET, SOCK_STREAM) -> fd 100
  bind(port 8080)  -> ok
  listen()         -> ok
  UDP socket fd=101 bound to port 9090 -> ok
  getsockname(tcp) -> port 8080
  close() both sockets -> ok
  POSIX socket API is fully functional.
```

---

### `clear` — Xóa màn hình

```
kernel> clear
```

---

## Chạy tests

```powershell
cargo test
```

Mỗi test chạy trong một QEMU instance riêng. Kết quả mong đợi:

```
test_breakpoint_exception...  [ok]
test_full_boot...             [ok]
simple_allocation...          [ok]
large_vec...                  [ok]
many_boxes...                 [ok]
[ok] Double fault handler triggered correctly
```

---

## Troubleshooting

| Vấn đề | Nguyên nhân | Cách xử lý |
|--------|-------------|-----------|
| `KERNEL PANIC: memory allocation failed` | Heap quá nhỏ | Tăng `HEAP_SIZE` lên `512 * 1024` trong `src/allocator.rs` |
| Gõ phím không được trong QEMU window | QEMU chưa capture input | Click vào vùng đen trong cửa sổ QEMU, hoặc dùng `-serial stdio` |
| Chuột bị mất vào QEMU | QEMU capture chuột | Nhấn **Ctrl+Alt+G** để thả chuột ra |
| Terminal bị treo sau Ctrl+C | QEMU đang chạy | Nhấn **Ctrl+A** rồi **X** để thoát đúng cách |
| `Could not open bootimage-mykernel.bin` | Chưa build | Chạy `cargo bootimage` trước |
| `ping 10.0.2.15` timeout từ host | Windows QEMU limitation | Bình thường — xem phần Giới hạn bên trên |
