# Deep Dive: Interrupts & Exceptions

> Giải thích cách CPU xử lý sự kiện bất ngờ — từ lỗi chia cho 0 đến phím bàn phím được nhấn.

---

## Vấn đề cần giải quyết

CPU chạy code tuần tự — instruction này rồi instruction tiếp theo. Nhưng thế giới không tuần tự:
- Người dùng nhấn phím bất kỳ lúc nào
- Network packet đến không báo trước
- Timer tick mỗi 10ms để scheduler chạy
- Code có thể chia cho 0 hoặc truy cập địa chỉ sai

Làm sao CPU biết xử lý những sự kiện này?

---

## Hai loại "ngắt"

### Exceptions — CPU tự tạo ra

Khi CPU gặp điều kiện bất thường trong quá trình thực thi code:

| Vector | Tên | Nguyên nhân |
|--------|-----|-------------|
| 0 | Divide Error | Chia cho 0 (`div` instruction) |
| 3 | Breakpoint | `INT3` instruction (dùng cho debugger) |
| 6 | Invalid Opcode | Lệnh CPU không hiểu |
| 8 | Double Fault | Exception khi đang xử lý exception |
| 13 | General Protection Fault | Vi phạm quyền (Ring 3 dùng lệnh Ring 0) |
| 14 | Page Fault | Truy cập địa chỉ ảo không mapped |

### Hardware IRQs — Thiết bị gửi tín hiệu

| IRQ | Vector | Thiết bị |
|-----|--------|---------|
| 0 | 32 | Timer (PIT/APIC) |
| 1 | 33 | PS/2 Keyboard |
| 4 | 36 | Serial port COM1 |
| 9 | 41 | Network card (thường) |

---

## IDT — Bảng tra cứu handler

**IDT (Interrupt Descriptor Table)** là mảng 256 entries. Mỗi entry (16 bytes) chứa:
- Địa chỉ của handler function
- Segment selector (kernel code segment)
- Flags: DPL (privilege), type (interrupt gate vs trap gate)

Khi interrupt/exception xảy ra, CPU dùng vector number làm index vào IDT, lấy handler address và jump đến đó.

```
IDTR register trỏ vào IDT:

IDT:
Entry  0: → divide_error_handler()
Entry  3: → breakpoint_handler()
Entry  8: → double_fault_handler()
Entry 14: → page_fault_handler()
Entry 32: → timer_handler()
Entry 33: → keyboard_handler()
...
Entry 255: (unused)
```

MyKernel load IDT bằng `LIDT` instruction.

---

## Điều gì xảy ra khi interrupt đến

### 1. CPU đang thực thi code bình thường

```
... instruction N-2 ...
... instruction N-1 ...      ← CPU đang ở đây
... instruction N   ...
... instruction N+1 ...
```

### 2. Interrupt signal đến (hardware) hoặc exception xảy ra

CPU check sau mỗi instruction xem có interrupt pending không.

### 3. CPU lưu trạng thái và switch stack

CPU tự động push vào stack (kernel stack):
```
Stack sau khi interrupt:
┌────────────────┐
│  SS            │ ← stack segment của code bị interrupt
│  RSP           │ ← stack pointer của code bị interrupt
│  RFLAGS        │ ← CPU flags
│  CS            │ ← code segment
│  RIP           │ ← instruction pointer (địa chỉ sẽ resume)
│  Error Code    │ ← (chỉ một số exceptions)
└────────────────┘ ← RSP trỏ đến đây sau khi push
```

Nếu đang chạy ở Ring 3 khi interrupt đến, CPU còn switch sang kernel stack (lấy từ TSS).

### 4. CPU jump đến handler

Handler nhìn thấy `InterruptStackFrame` — struct chứa thông tin về code bị interrupt.

### 5. Handler xử lý

```rust
extern "x86-interrupt" fn timer_handler(stack_frame: InterruptStackFrame) {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    // Gửi EOI để APIC biết handler đã xong
    unsafe { LAPIC.end_of_interrupt(); }
}
```

### 6. IRETQ — trở về code bị interrupt

Handler kết thúc bằng `IRETQ` instruction. CPU pop lại `RIP`, `CS`, `RFLAGS`, `RSP`, `SS` → tiếp tục code như không có gì xảy ra.

---

## Double Fault — Exception của Exception

Nếu exception xảy ra trong khi đang xử lý exception khác → **Double Fault** (vector 8).

Trường hợp nguy hiểm nhất: **stack overflow**

```
1. Code đệ quy quá sâu → stack hết chỗ
2. Stack pointer đang trỏ vào guard page
3. CPU cố push interrupt frame → Page Fault
4. Page Fault handler cần stack → stack vẫn overflow → Stack Fault  
5. → Double Fault
```

### IST — Independent Stack Table

Double fault handler cần stack riêng, không phụ thuộc vào stack đang overflow. TSS (Task State Segment) cung cấp **IST (Interrupt Stack Table)** — 7 slots cho stack address riêng biệt.

```
TSS:
  RSP0: kernel stack (dùng khi Ring 3 → Ring 0)
  IST1: → double fault stack  ← stack riêng, luôn valid
  IST2: (unused)
  ...
```

IDT entry của double fault chỉ định dùng IST1:
```rust
idt.double_fault.set_handler_fn(double_fault_handler)
    .set_stack_index(0);  // IST slot 0 = IST1
```

Nhờ đó, dù stack chính có overflow, double fault handler vẫn chạy được và in lỗi thay vì triple fault.

---

## PIC 8259 vs APIC

### PIC 8259 (cũ, từ IBM PC 1981)

```
Cascade mode:
CPU ← Master PIC (IRQ 0-7) ← Slave PIC (IRQ 8-15)
```

Vấn đề:
- Chỉ 15 IRQ lines (IRQ 2 dùng để cascade)
- Chỉ 1 CPU — không route được đến CPU cụ thể
- Vector conflict với CPU exceptions (vector 0-7 trùng với exceptions 0-7)

MyKernel init PIC với offset 32 (để tránh conflict), sau đó mask toàn bộ khi APIC sẵn sàng.

### Local APIC (hiện đại)

Mỗi CPU core có 1 Local APIC riêng. Memory-mapped tại `0xFEE00000`:

```
Local APIC registers (chọn lọc):
Offset 0x020: APIC ID
Offset 0x030: APIC Version
Offset 0x0B0: EOI register ← ghi 0 vào đây để signal "handler done"
Offset 0x0F0: Spurious Interrupt Vector Register
Offset 0x320: LVT Timer
Offset 0x380: Timer Initial Count
```

APIC Timer thay thế PIT:
```rust
// Configure APIC timer để fire interrupt mỗi ~10ms
lapic.write(LAPIC_TIMER_DCR, 3);   // divide by 16
lapic.write(LAPIC_TIMER_ICR, initial_count);
lapic.write(LAPIC_LVT_TIMER, 32 | (1 << 17));  // vector 32, periodic mode
```

### I/O APIC

Nhận IRQ từ devices, route đến Local APIC của CPU được chọn:

```
Keyboard → I/O APIC → (route table) → CPU 0 Local APIC → IDT[33] → handler
```

Redirection table: 24 entries, mỗi entry 64 bits chứa destination CPU và vector number.

---

## Keyboard — Từ Phím Bấm đến Ký Tự

### Luồng hoàn chỉnh

```
1. Phím được nhấn
2. PS/2 controller gửi scancode qua port 0x60
3. IRQ1 signal đến I/O APIC
4. I/O APIC route đến Local APIC của BSP
5. BSP nhận interrupt vector 33
6. CPU jump đến keyboard_handler()

keyboard_handler():
  │
  ├── Đọc scancode từ port 0x60
  ├── Push vào ArrayQueue<u8> (lock-free)
  ├── Wake async task (AtomicWaker::wake())
  └── Gửi EOI

ScancodeStream (async):
  │
  ├── Poll queue → có scancode → trả về
  └── Không có → đăng ký waker → Pending (CPU sleep)

Shell (async):
  │
  ├── Await ScancodeStream::next()
  ├── Decode scancode → DecodedKey (pc-keyboard crate)
  └── Xử lý ký tự
```

### Scancode vs Character

Keyboard gửi **scancode** — số nhị phân đại diện cho key vật lý, không phải ký tự:
- Key A nhấn xuống: `0x1E`
- Key A nhả ra: `0x9E` (high bit set = release)
- Shift + A: `0x2A` (Shift press) + `0x1E` (A press) → cần decode thành 'A'

`pc-keyboard` crate xử lý việc decode này, tính đến Shift, Ctrl, layout bàn phím.

---

## Timer — Trái Tim của Scheduler

Timer interrupt là cơ chế scheduler dùng để preempt tasks.

### PIT (legacy)
```
PIT Channel 0 → IRQ0 → timer_handler()
Frequency: 1.193182 MHz / divisor
Divisor 11932 → ~100Hz (10ms per tick)
```

### APIC Timer (modern)
```
APIC Timer → Local APIC → IDT[32] → timer_handler()
Cần calibrate: đo bao nhiêu APIC ticks trong 1 PIT tick
```

### Timer Handler trong MyKernel

```rust
extern "x86-interrupt" fn timer_handler(_: InterruptStackFrame) {
    // Increment tick counter (dùng SeqLock để safe với SMP)
    crate::sync::increment_ticks();

    // Gửi EOI
    unsafe { crate::apic::end_of_interrupt(); }
    
    // (Scheduler sẽ check ở đây nếu implement preemption)
}
```

Tick count được dùng cho `uptime` command: `ticks / 100` = seconds.
