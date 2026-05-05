# Deep Dive: Memory Management

> Giải thích chi tiết cách MyKernel quản lý bộ nhớ từ đầu — từ physical RAM đến virtual addresses, heap, và stack protection.

---

## Physical Memory — RAM thật sự

Khi máy bật, RAM là một dải byte liên tiếp từ địa chỉ 0 đến N. Nhưng không phải toàn bộ dải này là RAM trống:

```
Physical Address Space (ví dụ máy 4GB RAM):
0x00000000 - 0x0009FFFF  : Conventional RAM (640KB)
0x000A0000 - 0x000BFFFF  : VGA memory (video RAM)
0x000C0000 - 0x000FFFFF  : BIOS, ROM, hardware reserved
0x00100000 - 0xBFFFFFFF  : Extended RAM (hầu hết RAM ở đây)
0xC0000000 - 0xFFFFFFFF  : MMIO cho hardware (APIC, PCI...)
```

Bootloader cung cấp **memory map** — danh sách các vùng RAM nào available, vùng nào reserved. MyKernel đọc map này để biết frame nào có thể cấp phát.

---

## Frame Allocator — Quản lý Physical Pages

RAM được chia thành **frames** 4KB. Frame allocator theo dõi frame nào đang dùng, frame nào trống.

MyKernel dùng `BootInfoFrameAllocator` — đọc memory map từ bootloader, iterate qua các vùng `Usable` để trả về frames.

```
Memory Map từ bootloader:
Region 0: 0x0000 - 0x9FFF  Usable
Region 1: 0xA000 - 0xFFFF  Reserved (VGA)
Region 2: 0x100000 - 0x..  Usable  ← hầu hết RAM ở đây

Frame Allocator:
- Lấy từng Region Usable
- Chia thành 4KB frames
- Trả từng frame khi được yêu cầu
```

**Giới hạn của MyKernel**: `BootInfoFrameAllocator` chỉ cấp phát, không giải phóng. Production kernel cần buddy allocator hoặc bitmap allocator có thể free frames.

---

## Paging — Từ Virtual đến Physical

### 4-level Page Table

x86_64 dùng 4-level page table để translate 48-bit virtual address → physical address:

```
Virtual Address (48 bits):
 47       39 38      30 29      21 20      12 11        0
 ┌──────────┬──────────┬──────────┬──────────┬──────────┐
 │  PML4 idx│  PDPT idx│   PD idx │   PT idx │  Offset  │
 │  9 bits  │  9 bits  │  9 bits  │  9 bits  │  12 bits │
 └──────────┴──────────┴──────────┴──────────┴──────────┘

Lookup:
CR3 → PML4[idx] → PDPT[idx] → PD[idx] → PT[idx] → Physical Frame
                                                     + Offset = Physical Address
```

Mỗi level là 1 page (4KB) chứa 512 entries × 8 bytes = 4096 bytes.

### Page Table Entry Flags

Mỗi entry có flags quan trọng:
- **Present** (bit 0): entry hợp lệ
- **Writable** (bit 1): cho phép ghi
- **User** (bit 2): user mode có thể access
- **NX** (bit 63): no-execute — không thể chạy code tại page này

Ví dụ:
- Code pages: Present + User (không Writable, không NX)
- Data pages: Present + Writable + User + NX
- Stack pages: Present + Writable + User + NX

### Higher Half Kernel

Kernel được load ở địa chỉ cao (higher half), user space ở địa chỉ thấp:

```
0x0000_0000_0000_0000  ┐
                       │  User Space (0 - 128 TB)
0x0000_7FFF_FFFF_FFFF  ┘
         (gap)
0xFFFF_8000_0000_0000  ┐
                       │  Kernel Space (physical memory map)
0xFFFF_FFFF_FFFF_FFFF  ┘
```

Bootloader map toàn bộ physical RAM vào `0xFFFF_8000_0000_0000 + physical_addr`. Kernel dùng offset này để convert physical → virtual khi cần đọc page table entries.

---

## Heap — Cấp Phát Bộ Nhớ Động

### Vấn đề

Kernel cần tạo structs mới lúc runtime — số lượng không biết trước tại compile time. Ví dụ: danh sách files đang mở, danh sách processes, network packet buffers.

### Linked-List Allocator

MyKernel dùng `linked_list_allocator` crate. Cách hoạt động:

```
Heap ban đầu (1 hole = toàn bộ heap):
┌──────────────────────────────────────────┐
│ Hole: size=512KB, next=None              │
└──────────────────────────────────────────┘

Sau Box::new(100u8):
┌────────────┬────────────────────────────┐
│ Used: 100B │ Hole: size=511KB+, next=.. │
└────────────┴────────────────────────────┘

Sau Box::new([0u8; 1024]):
┌────────────┬──────────┬─────────────────┐
│ Used: 100B │ Used:1KB │ Hole: size=...  │
└────────────┴──────────┴─────────────────┘

Sau drop đầu tiên:
┌────────────┬──────────┬─────────────────┐
│ Hole: 100B │ Used:1KB │ Hole: size=...  │
└────────────┴──────────┴─────────────────┘
(merge holes nếu liền kề)
```

### Heap Size

Heap được map tại địa chỉ ảo cố định. Kích thước trong `src/allocator.rs`:

```rust
pub const HEAP_SIZE: usize = 512 * 1024;  // 512KB
```

Cần đủ lớn cho:
- VFS structures (RamFS inodes, file handles)
- Network buffers (virtio-net cần ~100KB cho RX buffers)
- Kernel data structures (process table, socket table)

Nếu heap quá nhỏ → `KERNEL PANIC: memory allocation failed`.

---

## Stack — Bộ Nhớ Tự Động

Stack là vùng RAM cho local variables, function call frames, return addresses.

```
Stack (grows downward):
High address ──────────────────────┐
                                   │ caller's frame
             ──────────────────────┤
             return address        │
             saved rbp             │ callee's frame
             local variable 1      │
             local variable 2      │
Low address  ──────────────────────┘ ← rsp (stack pointer)
```

### Stack Overflow

Khi function đệ quy quá sâu hoặc local array quá lớn, stack vượt quá giới hạn. Nếu không có guard:

```
Stack page      ← rsp, đang ghi
Guard page      ← CPU gây Page Fault
...
```

Page Fault → kernel handler. Nhưng handler cũng cần stack! Nếu dùng cùng stack → **triple fault** → machine reset.

### Double Fault với IST

MyKernel dùng **IST (Interrupt Stack Table)** — double fault handler có stack riêng trong TSS:

```
TSS (Task State Segment):
  IST1 → dedicated stack cho double fault handler
```

Khi stack overflow xảy ra:
1. Stack pointer đang ở guard page → Page Fault
2. Page Fault handler cũng dùng stack → Stack Fault
3. → Double Fault được triggered
4. CPU switch sang IST1 stack (không phải stack bị overflow)
5. Double fault handler chạy → in lỗi → halt

Nhờ IST, kernel có thể gracefully handle stack overflow thay vì silent reset.

---

## Stack Canary — Phát Hiện Buffer Overflow

Buffer overflow xảy ra khi ghi quá kích thước buffer, ghi đè vào stack frame của caller:

```
Stack frame:
┌───────────────┐
│ return address│ ← attacker muốn ghi đè cái này
├───────────────┤
│ canary value  │ ← 0xDEADBEEF... (random)
├───────────────┤
│ buf[64]       │ ← ghi 100 bytes vào đây
│               │    → ghi đè canary → ghi đè return addr
└───────────────┘
```

Trước khi function return, kernel check canary:
- Canary còn nguyên → OK, return bình thường
- Canary bị thay đổi → **stack corruption detected** → panic ngay

MyKernel dùng canary từ `RDTSC XOR 0xDEAD_BEEF_CAFE_BABE` — giá trị ngẫu nhiên mỗi lần boot, attacker không đoán được.

---

## Virtual Address Space của Kernel

```
Toàn bộ virtual address space (48-bit, 256TB):

0x0000_0000_0000_0000 ─────────────────────
│  User Space                               │
│  (code, stack, heap của user processes)   │
0x0000_7FFF_FFFF_FFFF ─────────────────────
│  (non-canonical hole — hardware enforced) │
0xFFFF_8000_0000_0000 ─────────────────────
│  Physical Memory Map                      │
│  (tất cả RAM mapped ở đây)                │
0xFFFF_BFFF_FFFF_FFFF ─────────────────────
│  Kernel code, data (từ bootloader)        │
│  Kernel heap (0x4444_4440_0000)           │
│  Local APIC MMIO (0xFEE0_0000)            │
│  I/O APIC MMIO  (0xFEC0_0000)             │
0xFFFF_FFFF_FFFF_FFFF ─────────────────────
```

Kernel entries trong PML4 được copy vào page table của mọi process → kernel code accessible từ mọi virtual address space, nhưng chỉ Ring 0 mới thực thi được (User bit = 0 cho kernel pages).
