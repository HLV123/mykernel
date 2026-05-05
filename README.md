# MyKernel

Một OS kernel bare-metal x86_64 viết bằng Rust từ đầu.

---

## Tại Sao Project Này Tồn Tại

Project này bắt đầu từ mong muốn hiểu sâu sáu khái niệm mà hầu hết developer dùng mỗi ngày nhưng ít khi thực sự hiểu bên trong:

1. **Boot process** — máy tính đi từ BIOS/UEFI đến kernel đang chạy như thế nào. Các chế độ CPU (real → protected → long mode), GDT, paging setup.
2. **Interrupt và exception handling** — CPU xử lý lỗi (divide by zero, page fault, GPF), hardware interrupts (timer, keyboard), IDT setup.
3. **Memory management** — physical memory allocator, paging, virtual memory, heap allocator.
4. **Concurrency ở kernel level** — cooperative tasks với async/await và preemptive scheduling.
5. **Hardware interface** — I/O ports, memory-mapped registers, driver cho VGA, serial, keyboard.
6. **Ring 0 vs Ring 3** — privilege separation, syscalls, user space.

Project lớn dần vượt quá sáu mục tiêu ban đầu. Mỗi phase trả lời một câu hỏi và đặt ra hai câu hỏi mới — đó là lý do nó đi đến phase 24. Cái kết thúc theo lý hơi điên rồ một chút: mở Linux kernel source trên repo của Linus Torvalds ra thấy quen hơn một chút mặc dù tôi chẳng còn háo hức làm vậy. Không phải hiểu hết ý đồ trong code, nhưng không còn thấy nó như một mớ chữ vô nghĩa nữa.

---

## Những Gì Đã Làm Được

MyKernel boot trên bare-metal x86_64 (hoặc QEMU) và cung cấp:

- **Interactive shell** qua serial I/O với 18 lệnh tích hợp
- **Virtual filesystem** — RamFS, DevFS, CPIO initramfs, FAT32
- **TCP/IP network stack** — ARP, IPv4, ICMP (ping responder), UDP echo, TCP state machine
- **POSIX socket API** — `socket`, `bind`, `listen`, `accept`, `send`, `recv`
- **40 syscalls tương thích Linux** (x86_64 ABI)
- **Preemptive scheduler** với Ring 3 user mode
- **SMP support** — Local APIC, I/O APIC, ACPI MADT parser, AP boot
- **virtio drivers** — block storage và ethernet NIC
- **Security subsystem** — stack canary, KASLR, pointer validation, capability system, CSPRNG

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

## Chạy Như Thế Nào

```bash
cargo bootimage
qemu-system-x86_64 \
  -drive format=raw,file=target/x86_64-mykernel/debug/bootimage-mykernel.bin \
  -serial stdio -no-reboot
```

Gõ lệnh trực tiếp vào terminal. Nhấn **Ctrl+A rồi X** để thoát QEMU.

```
kernel> ls /
kernel> cat /etc/motd
kernel> write /tmp/note.txt xin chao
kernel> security
kernel> rand
kernel> socket
```

Với networking (thêm `-netdev user,id=n0 -device virtio-net-pci,netdev=n0` vào lệnh QEMU và tăng heap lên 512KB trong `src/allocator.rs`):

```
kernel> net
kernel> ping 10.0.2.2
```

---

## Tài Liệu

Thư mục `docs/` chứa tài liệu đầy đủ từ không biết gì về OS đến có thể contribute:

**Dành cho người mới — không cần nền tảng OS:**
- `CONCEPTS.md` — kernel, virtual memory, interrupt, syscall được giải thích đơn giản
- `WHY_RUST.md` — tại sao dùng Rust thay vì C, `no_std` là gì
- `PHASES_EXPLAINED.md` — 24 phases được xây dựng là gì và tại sao cần từng phase
- `DEEP_DIVE_MEMORY.md` — paging, heap, stack canary đi sâu vào chi tiết
- `DEEP_DIVE_INTERRUPTS.md` — IDT, APIC, luồng xử lý keyboard chi tiết
- `DEEP_DIVE_NETWORK.md` — từ ethernet frame đến TCP/IP đến socket API chi tiết
- `DEEP_DIVE_FILESYSTEM.md` — VFS, RamFS, FAT32, initramfs chi tiết
- `READING_GUIDE.md` — đọc file nào trước, file nào sau và tại sao
- `EXPERIMENTS.md` — 10 thí nghiệm thực hành để tự sửa và quan sát kernel

**Dành cho người có kinh nghiệm:**
- `INTERNALS.md` — GDT layout, IDT format, syscall MSRs, virtqueue byte layout, 40 syscalls đầy đủ
- `RUST_PATTERNS.md` — các Rust pattern đặc thù trong kernel và lý do tồn tại
- `DESIGN_TRADEOFFS.md` — mọi quyết định thiết kế lớn cùng các lựa chọn thay thế đã cân nhắc
- `COMPARED_TO_LINUX.md` — so sánh song song với Linux kernel: cùng concept, implementation khác nhau
- `DEBUGGING.md` — QEMU GDB stub, serial log, danh sách panic messages thường gặp
- `KNOWN_LIMITATIONS.md` — những gì chưa implement và lý do
- `ADDING_SYSCALLS.md` — hướng dẫn từng bước thêm một syscall tương thích Linux
- `ADDING_DRIVERS.md` — cách viết PCI driver, virtio driver, hoặc filesystem driver mới
- `PORTING_GUIDE.md` — real hardware, UEFI boot, e1000 NIC, ARM64

**Setup và sử dụng:**
- `SETUP.md` — cài đặt môi trường trên Windows (Rust nightly, QEMU, bootimage)
- `USAGE.md` — cách chạy kernel và dùng shell
- `ARCHITECTURE.md` — kiến trúc hệ thống với ASCII diagrams
- `ROADMAP.md` — kết quả từng phase, những gì đã làm được
- `README.md` trong folder run code — log đầy đủ mọi lệnh đã chạy và output thực tế
