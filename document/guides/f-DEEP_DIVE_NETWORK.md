# Deep Dive: Network Stack

> Giải thích từng layer của network stack trong MyKernel — từ ethernet frame đến POSIX socket API.

---

## Network Stack là gì?

Khi bạn `ping google.com`, hàng chục thứ xảy ra:
1. Tên miền được resolve thành IP (DNS)
2. ICMP Echo Request packet được tạo
3. Packet được đóng gói vào IP header
4. IP packet được đóng gói vào Ethernet frame
5. Frame được gửi qua network card (NIC)
6. Frame đi qua cable/wifi đến router
7. Router forward đến destination
8. Response đi ngược lại

MyKernel implement các layer từ Ethernet đến TCP/IP — không có DNS, không có HTTP, nhưng đủ để ping và tạo socket connections.

---

## Mô hình OSI vs Implementation thực tế

```
OSI Model           MyKernel
─────────           ─────────────────────────
Application    ←→   Socket API (sys_send/recv)
Transport      ←→   TCP / UDP (src/net/tcp.rs, udp.rs)
Network        ←→   IPv4 + ICMP (src/net/ip.rs, icmp.rs)
Data Link      ←→   Ethernet + ARP (src/net/arp.rs)
Physical       ←→   virtio-net driver (src/drivers/virtio_net.rs)
```

Dữ liệu đi xuống (gửi): Application → Transport → Network → Data Link → Physical

Dữ liệu đi lên (nhận): Physical → Data Link → Network → Transport → Application

---

## Layer 1: virtio-net Driver — Giao tiếp với NIC

### Virtio là gì?

Virtio là chuẩn cho virtual devices trong QEMU. Thay vì emulate real hardware (như Intel e1000), QEMU cung cấp interface đơn giản hơn để driver giao tiếp hiệu quả.

### Virtqueue — Shared Memory giữa Kernel và QEMU

```
Kernel Memory:
┌─────────────────────────────────────────┐
│ Descriptor Ring (256 entries)           │
│   entry 0: { addr, len, flags, next }   │
│   entry 1: { addr, len, flags, next }   │
│   ...                                   │
├─────────────────────────────────────────┤
│ Available Ring                          │
│   { flags, idx, ring[256] }             │
│   ring[i] = descriptor index to process │
├─────────────────────────────────────────┤
│ Used Ring                               │
│   { flags, idx, ring[256] }             │
│   QEMU ghi vào đây khi xử lý xong       │
└─────────────────────────────────────────┘
```

**TX (gửi packet):**
1. Kernel ghi packet data vào memory
2. Tạo descriptor trỏ vào packet
3. Add descriptor index vào Available Ring
4. Notify QEMU (ghi vào Queue Notify register)
5. QEMU đọc và "gửi" packet
6. QEMU add entry vào Used Ring
7. Kernel poll Used Ring để biết khi nào xong

**RX (nhận packet):**
1. Kernel pre-allocate buffer (1526 bytes) cho mỗi slot
2. Add descriptor vào Available Ring
3. QEMU điền packet vào buffer khi có packet đến
4. QEMU add vào Used Ring
5. Kernel poll Used Ring → lấy packet

### MAC Address

MAC address (6 bytes) của NIC được đọc từ virtio device config space:
```
virtio-net config space (offset 0x14):
Byte 0-5: MAC address
```

QEMU mặc định: `52:54:00:12:34:56`

---

## Layer 2: Ethernet Frame

Ethernet frame là đơn vị cơ bản nhất được gửi qua mạng:

```
Ethernet Frame:
┌─────────────┬─────────────┬──────────┬─────────────────┬─────────┐
│  Dst MAC    │  Src MAC    │EtherType │    Payload      │  FCS    │
│   6 bytes   │   6 bytes   │ 2 bytes  │  46-1500 bytes  │4 bytes  │
└─────────────┴─────────────┴──────────┴─────────────────┴─────────┘

EtherType:
  0x0800 = IPv4
  0x0806 = ARP
  0x86DD = IPv6
```

Kernel nhận raw bytes từ virtio driver, đọc EtherType để biết payload là gì.

**Broadcast MAC**: `FF:FF:FF:FF:FF:FF` — gửi đến tất cả thiết bị trong mạng.

---

## Layer 2.5: ARP — Tìm MAC Address

### Vấn đề

IP address là địa chỉ logic (ví dụ `10.0.2.2`). Để gửi Ethernet frame cần MAC address vật lý. Làm sao biết MAC của `10.0.2.2`?

### ARP (Address Resolution Protocol)

```
Kernel muốn gửi đến 10.0.2.2:
1. Broadcast: "Ai có IP 10.0.2.2? Cho tôi biết MAC!"
   ARP Request: src_ip=10.0.2.15, src_mac=52:54:00:12:34:56
                dst_ip=10.0.2.2,  dst_mac=FF:FF:FF:FF:FF:FF

2. Host 10.0.2.2 trả lời unicast:
   ARP Reply: src_ip=10.0.2.2, src_mac=52:54:00:XX:XX:XX
              dst_ip=10.0.2.15, dst_mac=52:54:00:12:34:56

3. Kernel lưu vào ARP cache:
   10.0.2.2 → 52:54:00:XX:XX:XX
```

### ARP Packet Format

```
ARP Packet (28 bytes):
┌──────────┬──────────┬──────┬──────┬──────────────────────────────┐
│ HW Type  │ Protocol │ HLen │ PLen │ Operation (Request=1/Reply=2)│
│ (Ether=1)│(IPv4=0800│  6   │  4   │              2 bytes         │
│ 2 bytes  │) 2 bytes │1 byte│1 byte│                              │
├──────────┴──────────┴──────┴──────┴──────────────────────────────┤
│ Sender MAC (6) │ Sender IP (4) │ Target MAC (6) │ Target IP (4)  │
└────────────────┴───────────────┴────────────────┴────────────────┘
```

### ARP Cache trong MyKernel

```rust
struct ArpCache {
    entries: [(Ipv4Addr, MacAddr); 16],  // tối đa 16 entries
    count: usize,
}
```

Khi nhận ARP packet:
- Request: update cache + gửi Reply
- Reply: update cache

Khi cần gửi packet đến IP X:
- Tìm trong cache → có → dùng MAC đó
- Không có → dùng broadcast MAC (FF:FF:FF:FF:FF:FF)

---

## Layer 3: IPv4

### IP Header

```
IPv4 Header (20 bytes tối thiểu):
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
├─────────────────────┬─────────────────────────────────────────────┤
│ Ver │ IHL │  DSCP/ECN│              Total Length                  │
├─────────────────────┬───────────────────┬──┬──────────────────────┤
│      Identification  │  Flags │ Fragment Offset                   │
├─────────────────────┴───────────────────┴──┴──────────────────────┤
│      TTL      │   Protocol   │          Header Checksum           │
├───────────────────────────────────────────────────────────────────┤
│                    Source IP Address                              │
├───────────────────────────────────────────────────────────────────┤
│                  Destination IP Address                           │
└───────────────────────────────────────────────────────────────────┘
```

- **TTL** (Time To Live): bắt đầu = 64, mỗi router giảm 1, đến 0 thì drop packet
- **Protocol**: 1=ICMP, 6=TCP, 17=UDP
- **Checksum**: one's complement sum của header

### IP Checksum (RFC 1071)

```rust
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | (data[i+1] as u32);
        i += 2;
    }
    if i < data.len() { sum += (data[i] as u32) << 8; }
    while sum >> 16 != 0 { sum = (sum & 0xFFFF) + (sum >> 16); }
    !(sum as u16)
}
```

Checksum phải = 0 khi tính lại (bao gồm cả field checksum).

---

## Layer 3.5: ICMP — Ping

**ICMP (Internet Control Message Protocol)** dùng để test connectivity và báo lỗi.

### ICMP Echo Request/Reply (Ping)

```
Echo Request (ping từ host):
  Type=8, Code=0, Identifier=X, Sequence=Y, Data=...

Echo Reply (kernel trả lời):
  Type=0, Code=0, Identifier=X, Sequence=Y, Data=...
  (copy Identifier và Sequence từ Request)
```

MyKernel tự động reply mọi ICMP Echo Request — không cần app nào chạy.

Khi host `ping 10.0.2.15`:
1. Host gửi Echo Request
2. Kernel nhận, gọi `process_icmp()`
3. Kernel tạo Echo Reply với cùng Identifier/Sequence
4. Gửi lại qua virtio-net

---

## Layer 4: UDP — Gửi Nhận Đơn Giản

UDP (User Datagram Protocol) là protocol không kết nối, không đảm bảo delivery.

```
UDP Header (8 bytes):
┌─────────────────┬─────────────────┐
│   Source Port   │ Destination Port│
│    2 bytes      │     2 bytes     │
├─────────────────┼─────────────────┤
│     Length      │    Checksum     │
│    2 bytes      │    2 bytes      │
└─────────────────┴─────────────────┘
```

### UDP Echo Server (Port 7)

MyKernel có built-in UDP echo server: nhận bất kỳ UDP packet nào đến port 7, gửi ngược lại.

Dùng để test: `echo "test" | nc -u 10.0.2.15 7` → nhận lại "test"

### Tại sao UDP thay vì chỉ TCP?

- **DNS** dùng UDP (query nhỏ, latency quan trọng)
- **Game** dùng UDP (mất packet thì drop, không retransmit)
- **Streaming** dùng UDP (1 frame trễ không đáng retransmit cả stream)
- **DHCP, TFTP, NTP** đều dùng UDP

---

## Layer 4: TCP — Kết Nối Đáng Tin

TCP đảm bảo data đến đúng thứ tự, không mất, không duplicate.

### TCP State Machine

```
                    ┌─────────┐
                    │ CLOSED  │
                    └────┬────┘
                         │ listen()
                         ▼
                    ┌─────────┐
                    │ LISTEN  │ ← server đang chờ
                    └────┬────┘
                         │ nhận SYN
                         │ gửi SYN-ACK
                         ▼
               ┌──────────────────┐
               │  SYN_RECEIVED    │
               └────────┬─────────┘
                         │ nhận ACK
                         ▼
               ┌──────────────────┐
               │   ESTABLISHED    │ ← kết nối thành công
               └────────┬─────────┘
                         │ nhận FIN (peer muốn đóng)
                         ▼
               ┌──────────────────┐
               │   CLOSE_WAIT     │
               └────────┬─────────┘
                         │ gửi FIN
                         ▼
               ┌──────────────────┐
               │    LAST_ACK      │
               └────────┬─────────┘
                         │ nhận ACK
                         ▼
                    ┌─────────┐
                    │ CLOSED  │
                    └─────────┘
```

### Three-Way Handshake

```
Client              Server
  │                   │
  │──── SYN ─────────►│  "Tôi muốn kết nối, seq=X"
  │                   │
  │◄─── SYN-ACK ──────│  "OK, seq=Y, ack=X+1"
  │                   │
  │──── ACK ─────────►│  "Nhận rồi, ack=Y+1"
  │                   │
  │    [ESTABLISHED]  │
```

### Sequence Number

TCP track từng byte:
- `seq`: byte đầu tiên trong segment này
- `ack`: byte tiếp theo tôi expect nhận

Đảm bảo: out-of-order packets được reorder, missing packets được detect và retransmit.

**MyKernel TCP giới hạn**: không có retransmission, không có congestion control, không có sliding window. Đủ để demo handshake và basic data transfer.

---

## Layer 5: POSIX Socket API

Socket API là interface quen thuộc mà developer dùng để viết network apps.

### Lifecycle của TCP server socket

```rust
// 1. Tạo socket
let fd = socket(AF_INET, SOCK_STREAM, 0);  // fd = 100

// 2. Gán địa chỉ
bind(fd, &addr { port: 8080 });

// 3. Chờ kết nối
listen(fd, 5);  // backlog = 5

// 4. Chấp nhận kết nối (block cho đến khi có client)
let client_fd = accept(fd, &client_addr);  // fd = 101

// 5. Giao tiếp
send(client_fd, b"Hello", 5, 0);
let n = recv(client_fd, buf, 1024, 0);

// 6. Đóng
close(client_fd);
close(fd);
```

### Socket trong MyKernel

```
Socket table (global):
Entry 0: fd=100, type=TCP, state=LISTEN, port=8080
Entry 1: fd=101, type=TCP, state=ESTABLISHED, port=8080
Entry 2: fd=102, type=UDP, state=BOUND, port=9090
...

FD 100+ = sockets (tránh conflict với file FDs 0-99)
Port registry: { 8080: taken, 9090: taken }
```

### rx_dispatch — Phân Phối Packets

Khi packet đến:

```
rx_dispatch(ethernet_frame):
    │
    ├── EtherType = ARP?  → process_arp()
    └── EtherType = IPv4? → process_ipv4()
                                │
                                ├── Protocol = ICMP? → process_icmp()
                                ├── Protocol = UDP?  → process_udp()
                                │                         │
                                │                    port 7? → echo
                                │                    có socket? → deliver
                                └── Protocol = TCP?  → process_tcp()
                                                          │
                                                     tìm socket bằng dst port
                                                     update state machine
                                                     deliver data
```

---

## Tại Sao Không Ping 2 Chiều Trên Windows?

Nhiều người thắc mắc tại sao `ping 10.0.2.15` từ Windows timeout.

QEMU user-mode networking (`-netdev user`) dùng **SLiRP** — user-space TCP/IP stack hoạt động như NAT:

```
Windows host:
  Không có route đến 10.0.2.x (đây là địa chỉ nội bộ của SLiRP)
  
QEMU/SLiRP:
  Guest có thể initiate connections ra ngoài (qua NAT)
  Host KHÔNG thể initiate connections vào guest
  ICMP (ping) không được forward bởi SLiRP trên Windows

Kết quả:
  kernel> ping 10.0.2.2 → gửi được, SLiRP không forward ICMP reply
  Windows> ping 10.0.2.15 → không có route, timeout ngay
```

Để test ping 2 chiều cần TAP interface (chỉ trên Linux/macOS) hoặc host-only networking.
