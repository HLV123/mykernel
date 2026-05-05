# Các Khái Niệm Cơ Bản về OS

> Tài liệu này giải thích các khái niệm cốt lõi của một hệ điều hành bằng ngôn ngữ đơn giản, không yêu cầu kiến thức nền về CS. Đọc file này trước khi xem bất kỳ file nào khác.

---

## Kernel là gì?

Hãy tưởng tượng máy tính như một nhà hàng:

- **Hardware** (CPU, RAM, ổ cứng) = nhà bếp, nguyên liệu, bếp nấu
- **Kernel** = bếp trưởng — người duy nhất được vào bếp, quyết định ai được dùng gì
- **Applications** (Chrome, Word, game) = khách hàng — họ gọi món nhưng không tự vào bếp

Kernel là phần mềm chạy đầu tiên khi máy bật, và **không bao giờ tắt** trong suốt thời gian máy hoạt động. Nó là người trung gian duy nhất giữa phần mềm và phần cứng.

Nhiệm vụ chính của kernel:
- Quản lý bộ nhớ — ai được dùng vùng RAM nào
- Quản lý CPU — process nào được chạy, bao lâu
- Quản lý thiết bị — ai được đọc/ghi ổ cứng, network
- Bảo mật — ngăn process này xâm phạm process kia

---

## Ring 0 vs Ring 3 — Kernel Mode vs User Mode

CPU x86_64 có 4 "vòng bảo vệ" (privilege rings), nhưng thực tế chỉ dùng 2:

```
┌─────────────────────────────────────────┐
│           Ring 3 (User Mode)            │
│   Chrome, Word, game, terminal...       │
│   - Không đọc/ghi hardware trực tiếp    │
│   - Không truy cập RAM của process khác │
│   - Vi phạm → CPU crash ngay            │
├─────────────────────────────────────────┤
│           Ring 0 (Kernel Mode)          │
│           MyKernel chạy ở đây           │
│   - Toàn quyền với mọi hardware         │
│   - Đọc/ghi bất kỳ địa chỉ RAM nào      │
│   - Một lỗi nhỏ = crash toàn máy        │
└─────────────────────────────────────────┘
```

**Tại sao cần phân biệt?**

Nếu mọi thứ đều chạy ở Ring 0, một đoạn code lỗi trong Chrome có thể xóa toàn bộ file hệ thống, đọc mật khẩu của app khác, hay tắt máy. Phân tách Ring 0/Ring 3 là lớp bảo vệ cơ bản nhất.

**Trong MyKernel:**
- Kernel chạy ở Ring 0 — có thể làm mọi thứ
- User processes (nếu có) chạy ở Ring 3 — bị giới hạn
- Khi user process muốn làm gì đó cần quyền cao hơn, nó gọi **syscall**

---

## Bộ Nhớ — RAM hoạt động thế nào

### Địa chỉ vật lý vs địa chỉ ảo

RAM vật lý trong máy có địa chỉ từ 0 đến N (N = dung lượng RAM). Đây là **địa chỉ vật lý**.

Nhưng mỗi process không thấy địa chỉ vật lý thật — nó thấy **địa chỉ ảo**, một không gian địa chỉ riêng biệt do CPU tạo ra.

```
Process A thấy:          Process B thấy:
0x1000 → code A          0x1000 → code B
0x2000 → data A          0x2000 → data B

Thực tế trong RAM:
0x10000 → code A         0x50000 → code B
0x20000 → data A         0x60000 → data B
```

Cả A và B đều nghĩ mình có địa chỉ `0x1000`, nhưng thực tế trỏ vào 2 chỗ khác nhau trong RAM. Process A không thể đọc dữ liệu của B vì địa chỉ ảo của chúng độc lập.

### Page Table — bản đồ dịch địa chỉ

CPU dùng **page table** để dịch địa chỉ ảo → địa chỉ vật lý. Kernel tạo và quản lý page table. Mỗi process có page table riêng.

RAM được chia thành các **page** 4KB. Mỗi entry trong page table ánh xạ 1 virtual page → 1 physical page.

### Tại sao cần virtual memory?

1. **Isolation**: Process A không đọc được RAM của process B
2. **Abstraction**: Mỗi process nghĩ mình có toàn bộ không gian địa chỉ
3. **Protection**: Kernel đánh dấu page nào là read-only, no-execute

---

## Interrupt — CPU bị "ngắt" để xử lý sự kiện

Khi bạn nhấn phím, CPU không liên tục kiểm tra xem có phím được nhấn không — như vậy sẽ lãng phí. Thay vào đó, bàn phím **gửi tín hiệu interrupt** đến CPU.

```
CPU đang chạy code bình thường...
        │
        │  ← Interrupt đến! (phím được nhấn)
        ↓
CPU dừng code hiện tại (lưu trạng thái)
        │
        ↓
CPU chạy Interrupt Handler (code xử lý phím)
        │
        ↓
CPU khôi phục trạng thái và tiếp tục code cũ
```

### IDT — Bảng tra cứu interrupt handler

Kernel tạo một bảng **IDT (Interrupt Descriptor Table)** với 256 entries. Mỗi entry chứa địa chỉ của hàm xử lý (handler) cho mỗi loại interrupt.

Ví dụ:
- Entry 0: Division by zero handler
- Entry 3: Breakpoint handler
- Entry 14: Page Fault handler
- Entry 33: Keyboard IRQ handler
- Entry 36: Serial port handler

### Exceptions vs Hardware IRQs

- **Exception**: CPU tự tạo ra khi có lỗi (chia cho 0, truy cập địa chỉ không hợp lệ)
- **Hardware IRQ**: Thiết bị ngoài gửi tín hiệu (bàn phím, timer, network card)

---

## Process — Chương trình đang chạy

Một **program** là file ELF nằm trên ổ cứng — code tĩnh, chưa chạy.

Khi bạn chạy program, OS tạo một **process**:
- Load code vào RAM
- Tạo virtual address space riêng (page table mới)
- Cấp phát stack
- Ghi vào CPU instruction pointer → bắt đầu thực thi

Một máy có thể có hàng trăm processes chạy "đồng thời". Thực ra CPU chỉ chạy 1 process tại 1 thời điểm, nhưng **scheduler** luân phiên rất nhanh (~100 lần/giây) tạo cảm giác song song.

### Context Switch

Khi scheduler muốn chuyển từ process A sang B:
1. Lưu toàn bộ register state của A (rax, rbx, rsp, rip, ...)
2. Load register state của B
3. Đổi page table sang page table của B
4. CPU tiếp tục chạy B từ chỗ B dừng lại

---

## Syscall — User program xin kernel làm việc

User process chạy ở Ring 3 — không được đọc file, không được gửi network packet, không được cấp phát bộ nhớ trực tiếp. Mọi thứ phải xin kernel qua **syscall**.

```
User process (Ring 3)          Kernel (Ring 0)
        │                             │
        │  write(1, "hello", 5)       │
        │ ──────────────────────────► │
        │  SYSCALL instruction        │  kiểm tra quyền
        │                             │  ghi "hello" ra stdout
        │ ◄────────────────────────── │
        │  return 5 (bytes written)   │
```

**SYSCALL instruction** là cơ chế CPU cung cấp để chuyển từ Ring 3 → Ring 0 một cách an toàn và có kiểm soát.

Linux có ~400 syscalls. MyKernel implement 40 cái phổ biến nhất:
`read`, `write`, `open`, `close`, `stat`, `mmap`, `getpid`, `exit`, `uname`, ...

---

## Filesystem — Tổ chức dữ liệu trên bộ nhớ

Filesystem là cách tổ chức dữ liệu thành files và directories. Kernel cung cấp abstraction thống nhất qua **VFS (Virtual Filesystem)**.

```
Application: open("/etc/hosts", O_RDONLY)
                    │
                    ▼
               VFS Layer
                    │ tìm mount point
          ┌─────────┴──────────┐
          ▼                    ▼
        RamFS               FAT32
    (files in RAM)      (files on disk)
```

Nhờ VFS, application không cần biết file nằm trong RAM hay trên disk — interface đọc/ghi giống nhau.

### Inode

Mỗi file được đại diện bởi một **inode** — struct chứa metadata:
- Kích thước file
- Loại (regular file, directory, symlink)
- Permissions
- Con trỏ đến data blocks

---

## Heap — Bộ nhớ động trong kernel

Khi kernel cần cấp phát bộ nhớ động (tạo struct mới, Vec, String...), nó dùng **heap allocator**.

Heap là một vùng RAM được map sẵn. Allocator quản lý vùng này, chia nhỏ và cấp phát theo yêu cầu.

MyKernel dùng **linked-list allocator**: danh sách các "hole" (vùng trống) liên kết nhau. Khi cần N bytes, tìm hole đủ lớn, chia ra, trả về pointer.

Không có garbage collector — kernel phải `free` đúng lúc (Rust's ownership system làm việc này tự động).

---

## Driver — Kernel nói chuyện với hardware

Hardware (bàn phím, ổ cứng, card mạng) không tự nhiên "hiểu" Rust code. Kernel cần **driver** — code biết cách giao tiếp với từng thiết bị cụ thể.

Giao tiếp thường qua:
- **Port I/O**: CPU dùng lệnh `IN`/`OUT` để đọc/ghi register của thiết bị
- **Memory-mapped I/O (MMIO)**: Thiết bị map register vào địa chỉ RAM, kernel đọc/ghi như RAM bình thường
- **DMA**: Thiết bị đọc/ghi RAM trực tiếp mà không qua CPU

MyKernel có driver cho: UART (serial), VGA, PS/2 keyboard, PCI bus, virtio-blk, virtio-net.

---

## Tóm tắt các khái niệm

| Khái niệm | Định nghĩa ngắn |
|-----------|----------------|
| Kernel | Phần mềm trung gian giữa hardware và applications |
| Ring 0 / Ring 3 | Mức độ quyền của CPU — kernel vs user |
| Virtual memory | Mỗi process có không gian địa chỉ riêng ảo |
| Page table | Bản đồ dịch địa chỉ ảo → vật lý |
| Interrupt | CPU dừng để xử lý sự kiện (phím, timer, packet) |
| IDT | Bảng tra cứu interrupt handler |
| Process | Program đang chạy với RAM và CPU riêng |
| Context switch | Chuyển CPU từ process này sang process khác |
| Syscall | User process xin kernel làm việc thay |
| VFS | Abstraction layer thống nhất cho mọi filesystem |
| Heap | Vùng RAM cho cấp phát bộ nhớ động |
| Driver | Code giao tiếp với từng thiết bị hardware |
