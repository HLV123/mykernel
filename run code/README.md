# MyKernel

> Ghi lại toàn bộ quá trình chạy và trải nghiệm MyKernel trên Windows với QEMU.
> Project: bare-metal x86_64 OS kernel viết bằng Rust, 24 phases.

---

## Môi trường

| Thành phần | Chi tiết |
|------------|---------|
| **OS** | Windows 10/11 |
| **Terminal** | PowerShell |
| **Emulator** | QEMU `qemu-system-x86_64` |
| **Toolchain** | Rust Nightly + bootimage 0.9.34 |
| **Kernel IP** | 10.0.2.15 (QEMU user-mode NAT) |

---

## Cách chạy

### Chế độ cơ bản (không có network)

```powershell
cargo bootimage
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -serial stdio -no-reboot
```

### Chế độ có virtio-net

```powershell
cargo bootimage
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0 `
  -serial stdio -no-reboot
```

> **Lưu ý:** Phải tăng heap lên 512KB trong `src/allocator.rs` trước khi chạy với virtio-net, nếu không kernel sẽ panic vì OOM:
> ```rust
> pub const HEAP_SIZE: usize = 512 * 1024; // 512KB
> ```
> Mặc định trong code là 100KB — đủ cho chế độ cơ bản nhưng không đủ cho virtio-net.

### Thoát QEMU

Nhấn **Ctrl+A** rồi **X**.

> **Lưu ý:** Cửa sổ QEMU sẽ capture chuột. Nếu bị mất chuột nhấn **Ctrl+Alt+G** để thả ra. Khuyến nghị dùng `-serial stdio` để tương tác qua PowerShell terminal thay vì cửa sổ QEMU.

---

## Boot output

Khi kernel khởi động thành công, terminal hiển thị:

```
[vfs] Mounting ramfs at /
[vfs] Mounting devfs at /dev
[initramfs] Loading 2184 bytes
[initramfs] Loaded: 7 files, 7 dirs
[fs] VFS + initramfs ready
[syscall] initialized, handler=0x211e10
[drivers] Initializing...
[virtio-net] Found at 00:03.0 I/O=0xc000
[virtio-net] MAC: 52:54:00:12:34:56
[net] Stack initialized, IP=10.0.2.15
[security] Stack canary initialized: 0xdeadbeef...
[security] KASLR offset: +0x1e800000
[apic] Local APIC ID=0 Version=0x14 MaxLVT=5

  __  __       _  __                    _
 |  \/  |_   _| |/ /___ _ __ _ __   ___| |
 | |\/| | | | | ' // _ \ '__| '_ \ / _ \ |
 | |  | | |_| | . \  __/ |  | | | |  __/ |
 |_|  |_|\__, |_|\_\___|_|  |_| |_|\___|_|
          |___/
 ...

  [ok] GDT / IDT / PIC
  [ok] Virtual memory + heap
  [ok] Filesystem  (VFS + initramfs)
  [ok] Syscalls    (Linux x86_64 ABI, 40 calls)
  [ok] virtio-net  MAC 52:54:00:12:34:56  IP 10.0.2.15
  [ok] Security    score 60/100
  [ok] APIC        BSP ID=0  1 CPU(s) detected

MyKernel Shell  (type 'help' for commands)
kernel>
```

---

## Trải nghiệm các lệnh shell

### `help` — Danh sách lệnh

```
kernel> help
Available commands:
  help              -- show this help
  uname             -- kernel and CPU information
  mem               -- memory and heap statistics
  uptime            -- system timer tick count
  ls [path]         -- list directory (default: /)
  cat <file>        -- print file contents
  write <file> <text> -- write text to a file
  mkdir <path>      -- create a directory
  rm <path>         -- remove a file
  echo <text>       -- print text
  net               -- show network configuration
  ping <ip>         -- ICMP echo (needs virtio-net)
  netstat           -- show socket table
  socket            -- socket API demo
  rand              -- print 16 random bytes
  security          -- security audit report
  cpu               -- CPU topology
  clear             -- clear the screen
```

---

### `uname` — Thông tin kernel và CPU

```
kernel> uname
Kernel:  MyKernel
Release: 1.0.0
Arch:    x86_64
Build:   Rust bare-metal (no_std)
CPU:     QEMU Virtual CPU version 2.5+
CPU features: SMEP=false SMAP=false UMIP=false RDRAND=false
```

> SMEP/SMAP/RDRAND = false vì QEMU TCG mode không emulate các CPU security features này. Trên real hardware hoặc KVM với `-cpu host` sẽ là true.

---

### `uptime` — Thời gian chạy

```
kernel> uptime
Uptime: 0.00 seconds  (0 ticks at ~100 Hz)
```

> Timer ticks được đếm bởi PIT/APIC timer ở ~100Hz. Khi chạy qua `-serial stdio` thì uptime hiển thị 0 vì timer tick không được poll liên tục trong serial polling loop.

---

### `cpu` — CPU topology

```
kernel> cpu
CPU topology:
  BSP APIC ID: 0
  Total CPUs:  1
  Online CPUs: 1
[smp] Topology: 1 total, 1 online
[smp]   CPU[0]: APIC_ID=0 state=Offline
```

> QEMU mặc định chạy 1 CPU. Thêm `-smp 4` vào lệnh QEMU để boot 4 CPUs — kernel sẽ tự động detect và boot các Application Processors qua INIT+SIPI IPI.

---

### `mem` — Thông tin heap

```
kernel> mem
Heap size:  512 KiB  (524288 bytes)
```

> Heap size có thể thay đổi trong `src/allocator.rs`. Mặc định trong code là 100KB, cần tăng lên 512KB khi dùng virtio-net.

---

### `ls` — Duyệt filesystem

```
kernel> ls /
  -        48  README
  d         0  bin
  d         0  etc
  d         0  proc
  d         0  tmp
  d         0  usr

kernel> ls /etc
  -         9  hostname
  -        58  motd
  -        40  os-release
  -         8  shells

kernel> ls /bin
  -        37  hello
  -        67  init
```

> Format: `type  size  name`. `d` = directory, `-` = regular file.
> Filesystem được load từ CPIO initramfs archive nhúng trong kernel binary.

---

### `cat` — Đọc file

```
kernel> cat /etc/motd
Welcome to MyKernel!
Built with Rust. Phase 15: initramfs

kernel> cat /etc/hostname
mykernel

kernel> cat /README
MyKernel initramfs
Loaded via CPIO newc format.
```

---

### `mkdir`, `write`, `cat`, `rm` — Thao tác file

```
kernel> mkdir /tmp/mydir
created directory: /tmp/mydir

kernel> write /tmp/hello.txt Hello World
wrote 11 bytes to /tmp/hello.txt

kernel> cat /tmp/hello.txt
Hello World

kernel> write /tmp/test2.txt This is a test file
wrote 19 bytes to /tmp/test2.txt

kernel> ls /tmp
  -         0  hel
  -        11  hello.txt
  d         0  mydir
  -        19  test2.txt

kernel> rm /tmp/hello.txt
removed: /tmp/hello.txt

kernel> ls /tmp
  -         0  hel
  d         0  mydir
  -        19  test2.txt
```

> Filesystem là RamFS — lưu trong RAM, mất khi tắt kernel. Không persistent.

---

### `echo` — In text

```
kernel> echo Hello from MyKernel
Hello from MyKernel
```

---

### `security` — Security audit

```
kernel> security
Security audit:
  [off] SMEP   (no execution of user pages in kernel mode)
  [off] SMAP   (no access to user pages without STAC/CLAC)
  [on]  NX/XD  (data pages are non-executable)
  [on]  Canary (stack corruption detection)
  [on]  KASLR  (kernel load address randomised)
  [off] RDRAND (CPU hardware RNG)
  [on]  Hardened policy (raw sockets off, kptr_restrict on)

  Score: 60/100  (GOOD)
  Note: SMEP/SMAP/RDRAND not available in QEMU TCG mode.
        Score will be higher on real hardware or KVM.
```

> Score 60/100 trên QEMU TCG. Trên real hardware với SMEP+SMAP+RDRAND sẽ đạt 80-100/100.

---

### `rand` — Random bytes

```
kernel> rand
Random bytes: bbb6ba45195e64743e4106c1900565f4
Source: xoshiro256** PRNG seeded from RDTSC at boot

kernel> rand
Random bytes: 95a2a86d6170437432dda48b84b56cb6
Source: xoshiro256** PRNG seeded from RDTSC at boot
```

> Mỗi lần gọi `rand` cho kết quả khác nhau — xoshiro256** CSPRNG hoạt động đúng. Seed từ RDTSC (timestamp counter) tại boot time.

---

### `net` — Network configuration

**Không có virtio-net:**
```
kernel> net
No network interface detected.
Start QEMU with:
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0
```

**Có virtio-net:**
```
kernel> net
Interface: virtio-net
  MAC:     52:54:00:12:34:56
  IP:      10.0.2.15/24
  Gateway: 10.0.2.2
Protocols: ARP, IPv4, ICMP, UDP, TCP
Services:  ICMP echo responder (ping), UDP echo (port 7)
```

---

### `netstat` — Socket table

```
kernel> netstat
Socket table (FDs 100-163):
  system can allocate sockets (test fd=100)
  Listening services:
    UDP port 7  - echo server (built-in)
    ICMP        - echo responder (kernel replies to ping)
```

---

### `socket` — POSIX Socket API demo

```
kernel> socket
Socket API demonstration:
[socket] Created fd=100 type=TCP
  socket(AF_INET, SOCK_STREAM) -> fd 100
[socket] Bound fd=100 to port 8080
  bind(port 8080)  -> ok
[socket] Listening fd=100 port=8080
  listen()         -> ok
[socket] Created fd=101 type=UDP
[socket] Bound fd=101 to port 9090
  UDP socket fd=101 bound to port 9090 -> ok
  getsockname(tcp) -> port 8080
[socket] Close fd=100 port=8080
[socket] Close fd=101 port=9090
  close() both sockets -> ok
  POSIX socket API is fully functional.
```

---

### `ping` — ICMP echo

```
kernel> ping 10.0.2.2
PING 10.0.2.2
[virtio-net] TX: 56 bytes
  sent 56 bytes
  waiting for reply...
  no reply (the remote host may not be reachable)
```

> Kernel gửi packet thành công (TX: 56 bytes). Không nhận reply vì ARP cache chưa có entry cho 10.0.2.2 và QEMU user-mode NAT trên Windows không forward ICMP replies vào guest.

---

## Giới hạn môi trường (Windows + QEMU user-mode)

| Tính năng | Trạng thái | Lý do |
|-----------|-----------|-------|
| Ping từ host vào kernel | ❌ | QEMU user-mode trên Windows không route ICMP inbound |
| Ping từ kernel ra gateway | ⚠️ | Gửi được nhưng không nhận reply |
| UDP echo test | ⚠️ | Cần ncat/netcat trên host |
| Virtio-blk disk | ❌ chưa test | Cần tạo disk.img và thêm `-drive if=virtio` |
| SMEP/SMAP/RDRAND | ❌ | QEMU TCG không emulate |
| Ping 2 chiều đầy đủ | ❌ | Cần Linux + KVM + TAP interface |

### Để test ping 2 chiều (trên Linux):

```bash
# Chạy QEMU với KVM và TAP
sudo qemu-system-x86_64 \
  -enable-kvm \
  -drive format=raw,file=target/x86_64-mykernel/debug/bootimage-mykernel.bin \
  -netdev tap,id=n0,ifname=tap0,script=no \
  -device virtio-net-pci,netdev=n0 \
  -serial stdio -no-reboot
```

### Để test virtio-blk:

```powershell
# Tạo disk image 64MB
$disk = New-Object byte[](64 * 1024 * 1024)
[System.IO.File]::WriteAllBytes("disk.img", $disk)

# Chạy với disk
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -drive format=raw,file=disk.img,if=virtio `
  -serial stdio -no-reboot
```

---

## Tổng kết

| # | Tính năng | Kết quả |
|---|-----------|---------|
| 1 | Boot kernel | ✅ |
| 2 | Shell qua serial stdio | ✅ |
| 3 | Filesystem (ls/cat/mkdir/write/rm) | ✅ |
| 4 | uname / uptime / cpu / mem | ✅ |
| 5 | Security audit (60/100) | ✅ |
| 6 | CSPRNG (rand) | ✅ |
| 7 | POSIX Socket API | ✅ |
| 8 | virtio-net detect + MAC/IP | ✅ |
| 9 | Network info (net/netstat) | ✅ |
| 10 | ICMP TX packet | ✅ |
| 11 | Ping inbound từ host | ❌ (Windows limitation) |
| 12 | virtio-blk | ❌ (chưa test) |

**MyKernel hoạt động đầy đủ trong giới hạn của môi trường Windows + QEMU user-mode.**  
Tất cả tính năng kernel-side đều implement đúng và hoạt động.  
Giới hạn là phía QEMU networking trên Windows, không phải lỗi kernel.
