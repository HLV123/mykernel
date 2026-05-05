/// Socket API — POSIX-compatible socket layer
///
/// Bridges Linux syscalls (socket/bind/connect/listen/accept/send/recv)
/// với TCP/IP stack bên dưới.
///
/// Socket types hỗ trợ:
///   SOCK_STREAM (TCP) — connection-oriented, reliable
///   SOCK_DGRAM  (UDP) — connectionless, unreliable
///
/// Socket lifecycle:
///
/// Server TCP:
///   socket() → bind() → listen() → accept() → recv()/send() → close()
///
/// Client TCP:
///   socket() → connect() → send()/recv() → close()
///
/// UDP:
///   socket() → bind() → recvfrom()/sendto() → close()

use alloc::vec::Vec;
use alloc::collections::VecDeque;
use spin::Mutex;
use core::sync::atomic::{AtomicI32, Ordering};

use crate::net::{tcp, udp, arp};
use crate::drivers::virtio_net::{build_ethernet_frame, ETH_P_IP, send_packet};

// ---------------------------------------------------------------------------
// Socket constants (Linux-compatible)
// ---------------------------------------------------------------------------

pub const AF_INET:     i32 = 2;
pub const AF_INET6:    i32 = 10;

pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM:  i32 = 2;
pub const SOCK_RAW:    i32 = 3;

pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;

pub const SOL_SOCKET:  i32 = 1;
pub const SO_REUSEADDR:i32 = 2;
pub const SO_RCVBUF:   i32 = 8;
pub const SO_SNDBUF:   i32 = 7;

pub const SHUT_RD:  i32 = 0;
pub const SHUT_WR:  i32 = 1;
pub const SHUT_RDWR:i32 = 2;

// Error codes
pub const EBADF:     i64 = -9;
pub const EINVAL:    i64 = -22;
pub const ENOTSUP:   i64 = -95;
pub const ENOTCONN:  i64 = -107;
pub const EADDRINUSE:i64 = -98;
pub const EAGAIN:    i64 = -11;
pub const ECONNREFUSED: i64 = -111;

// ---------------------------------------------------------------------------
// Socket address structure (Linux sockaddr_in)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SockaddrIn {
    pub sin_family: u16,
    pub sin_port:   [u8; 2], // big-endian
    pub sin_addr:   [u8; 4],
    pub _pad:       [u8; 8],
}

impl SockaddrIn {
    pub fn port(&self) -> u16 {
        u16::from_be_bytes(self.sin_port)
    }
    pub fn ip(&self) -> [u8; 4] {
        self.sin_addr
    }
}

// ---------------------------------------------------------------------------
// Socket states
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SocketState {
    Closed,
    Bound,
    Listening,
    Connecting,
    Connected,
    CloseWait,
}

// ---------------------------------------------------------------------------
// Socket descriptor
// ---------------------------------------------------------------------------

pub struct Socket {
    pub sock_type:  i32,    // SOCK_STREAM or SOCK_DGRAM
    pub state:      SocketState,
    pub local_ip:   [u8; 4],
    pub local_port: u16,
    pub remote_ip:  [u8; 4],
    pub remote_port:u16,
    pub rx_buf:     VecDeque<u8>,
    pub tx_buf:     VecDeque<u8>,
    pub backlog:    VecDeque<Socket>, // for listening TCP sockets
    // TCP state
    pub tcp_conn:   Option<tcp::TcpConn>,
    pub nonblocking: bool,
    pub reuse_addr:  bool,
}

impl Socket {
    pub fn new(sock_type: i32) -> Self {
        Socket {
            sock_type,
            state: SocketState::Closed,
            local_ip:    [0; 4],
            local_port:  0,
            remote_ip:   [0; 4],
            remote_port: 0,
            rx_buf:      VecDeque::new(),
            tx_buf:      VecDeque::new(),
            backlog:     VecDeque::new(),
            tcp_conn:    None,
            nonblocking: false,
            reuse_addr:  false,
        }
    }
}

// ---------------------------------------------------------------------------
// Global socket table
// ---------------------------------------------------------------------------

const MAX_SOCKETS: usize = 64;
const SOCKET_FD_BASE: usize = 100; // FDs 100..163 = sockets

struct SocketTable {
    sockets: [Option<Socket>; MAX_SOCKETS],
}

impl SocketTable {
    const fn new() -> Self {
        SocketTable { sockets: [const { None }; MAX_SOCKETS] }
    }

    fn alloc(&mut self, sock: Socket) -> Option<usize> {
        for (i, slot) in self.sockets.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(sock);
                return Some(i + SOCKET_FD_BASE);
            }
        }
        None
    }

    fn get(&self, fd: usize) -> Option<&Socket> {
        if fd < SOCKET_FD_BASE || fd >= SOCKET_FD_BASE + MAX_SOCKETS {
            return None;
        }
        self.sockets[fd - SOCKET_FD_BASE].as_ref()
    }

    fn get_mut(&mut self, fd: usize) -> Option<&mut Socket> {
        if fd < SOCKET_FD_BASE || fd >= SOCKET_FD_BASE + MAX_SOCKETS {
            return None;
        }
        self.sockets[fd - SOCKET_FD_BASE].as_mut()
    }

    fn free(&mut self, fd: usize) -> bool {
        if fd < SOCKET_FD_BASE || fd >= SOCKET_FD_BASE + MAX_SOCKETS {
            return false;
        }
        self.sockets[fd - SOCKET_FD_BASE].take().is_some()
    }
}

static SOCKET_TABLE: Mutex<SocketTable> = Mutex::new(SocketTable::new());

// ---------------------------------------------------------------------------
// Port registry (prevent duplicate binding)
// ---------------------------------------------------------------------------

static mut USED_PORTS: [bool; 65536] = [false; 65536];

fn port_in_use(port: u16) -> bool {
    unsafe { USED_PORTS[port as usize] }
}

fn reserve_port(port: u16) {
    unsafe { USED_PORTS[port as usize] = true; }
}

fn release_port(port: u16) {
    unsafe { USED_PORTS[port as usize] = false; }
}

// ---------------------------------------------------------------------------
// Socket syscall implementations
// ---------------------------------------------------------------------------

/// sys_socket(domain, type, protocol) → fd
pub fn sys_socket(domain: i32, sock_type: i32, _protocol: i32) -> i64 {
    if domain != AF_INET {
        crate::serial_println!("[socket] Only AF_INET supported");
        return EINVAL;
    }
    if sock_type != SOCK_STREAM && sock_type != SOCK_DGRAM {
        return ENOTSUP;
    }

    let sock = Socket::new(sock_type);
    match SOCKET_TABLE.lock().alloc(sock) {
        Some(fd) => {
            crate::serial_println!("[socket] Created fd={} type={}", fd,
                if sock_type == SOCK_STREAM { "TCP" } else { "UDP" });
            fd as i64
        }
        None => -24, // EMFILE
    }
}

/// sys_bind(fd, addr_ptr, addrlen) → 0 or error
pub fn sys_bind(fd: usize, addr_ptr: u64, _addrlen: u32) -> i64 {
    let addr = unsafe { &*(addr_ptr as *const SockaddrIn) };
    let port = addr.port();
    let ip   = addr.ip();

    if port == 0 { return EINVAL; }

    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.state != SocketState::Closed { return EINVAL; }
    if port_in_use(port) && !sock.reuse_addr { return EADDRINUSE; }

    sock.local_port = port;
    sock.local_ip   = if ip == [0,0,0,0] { crate::net::our_ip() } else { ip };
    sock.state = SocketState::Bound;
    reserve_port(port);

    crate::serial_println!("[socket] Bound fd={} to port {}", fd, port);
    0
}

/// sys_listen(fd, backlog) → 0 or error
pub fn sys_listen(fd: usize, _backlog: i32) -> i64 {
    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.sock_type != SOCK_STREAM { return EINVAL; }
    if sock.state != SocketState::Bound { return EINVAL; }

    // Initialize TCP connection in LISTEN state
    let local_ip = sock.local_ip;
    let local_port = sock.local_port;
    let mut conn = tcp::TcpConn::new(local_ip, local_port);
    conn.state = tcp::TcpState::Listen;
    sock.tcp_conn = Some(conn);
    sock.state = SocketState::Listening;

    crate::serial_println!("[socket] Listening fd={} port={}", fd, local_port);
    0
}

/// sys_connect(fd, addr_ptr, addrlen) → 0 or error
pub fn sys_connect(fd: usize, addr_ptr: u64, _addrlen: u32) -> i64 {
    let addr = unsafe { &*(addr_ptr as *const SockaddrIn) };
    let dst_port = addr.port();
    let dst_ip   = addr.ip();

    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.sock_type != SOCK_STREAM { return EINVAL; }

    let our_ip = crate::net::our_ip();
    let src_port = tcp::alloc_ephemeral_port();

    // Create TCP connection and send SYN
    let mut conn = tcp::TcpConn::new(our_ip, src_port);
    conn.remote_ip   = dst_ip;
    conn.remote_port = dst_port;
    conn.state = tcp::TcpState::SynSent;

    // Build SYN packet
    let syn = conn.build_tcp_pkt(tcp::TCP_SYN, &[]);
    conn.snd_nxt = conn.snd_nxt.wrapping_add(1);

    sock.tcp_conn   = Some(conn);
    sock.local_port  = src_port;
    sock.remote_ip   = dst_ip;
    sock.remote_port = dst_port;
    sock.state = SocketState::Connecting;

    reserve_port(src_port);

    // Send SYN via ethernet
    let dst_mac = arp::cache_lookup(&dst_ip).unwrap_or([0xFF; 6]);
    let our_mac = crate::net::our_mac();
    let eth_frame = build_ethernet_frame(dst_mac, our_mac, ETH_P_IP, &syn);
    let _ = send_packet(&eth_frame);

    crate::serial_println!("[socket] SYN sent to {}.{}.{}.{}:{}", 
        dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], dst_port);

    // For simplicity in QEMU environment, mark as connected
    // (real implementation would wait for SYN-ACK)
    if let Some(conn) = sock.tcp_conn.as_mut() {
        conn.state = tcp::TcpState::Established;
    }
    sock.state = SocketState::Connected;
    0
}

/// sys_accept(fd, addr_ptr, addrlen_ptr) → new_fd or error
pub fn sys_accept(fd: usize, addr_ptr: u64, _addrlen_ptr: u64) -> i64 {
    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.state != SocketState::Listening { return EINVAL; }

    // Check if there's a pending connection
    if sock.backlog.is_empty() {
        if sock.nonblocking { return EAGAIN; }
        // Block (simplified: just return EAGAIN)
        return EAGAIN;
    }

    let conn_sock = sock.backlog.pop_front().unwrap();
    let remote_ip   = conn_sock.remote_ip;
    let remote_port = conn_sock.remote_port;

    // Fill in remote address if provided
    if addr_ptr != 0 {
        let addr = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
        addr.sin_family = AF_INET as u16;
        addr.sin_port   = remote_port.to_be_bytes();
        addr.sin_addr   = remote_ip;
    }

    match table.alloc(conn_sock) {
        Some(new_fd) => {
            crate::serial_println!("[socket] Accept fd={} from {}:{}", 
                new_fd, remote_ip[0], remote_port);
            new_fd as i64
        }
        None => -24, // EMFILE
    }
}

/// sys_send(fd, buf_ptr, len, flags) → bytes_sent or error
pub fn sys_send(fd: usize, buf_ptr: u64, len: usize, _flags: i32) -> i64 {
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };

    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.state != SocketState::Connected { return ENOTCONN; }

    match sock.sock_type {
        SOCK_STREAM => {
            if let Some(conn) = sock.tcp_conn.as_mut() {
                if let Some(pkt) = conn.send(data) {
                    let dst_mac = arp::cache_lookup(&conn.remote_ip).unwrap_or([0xFF; 6]);
                    let our_mac = crate::net::our_mac();
                    let eth = build_ethernet_frame(dst_mac, our_mac, ETH_P_IP, &pkt);
                    let _ = send_packet(&eth);
                    crate::serial_println!("[socket] TCP send {} bytes", len);
                    return len as i64;
                }
            }
            ENOTCONN
        }
        SOCK_DGRAM => {
            let pkt = udp::build_udp(
                sock.local_ip, sock.remote_ip,
                sock.local_port, sock.remote_port,
                data,
            );
            let dst_mac = arp::cache_lookup(&sock.remote_ip).unwrap_or([0xFF; 6]);
            let our_mac = crate::net::our_mac();
            let eth = build_ethernet_frame(dst_mac, our_mac, ETH_P_IP, &pkt);
            let _ = send_packet(&eth);
            len as i64
        }
        _ => EINVAL,
    }
}

/// sys_recv(fd, buf_ptr, len, flags) → bytes_received or error
pub fn sys_recv(fd: usize, buf_ptr: u64, len: usize, _flags: i32) -> i64 {
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };

    // Poll network first
    crate::net::poll();

    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.state != SocketState::Connected && sock.state != SocketState::CloseWait {
        return ENOTCONN;
    }

    // Read from rx buffer
    let n = len.min(sock.rx_buf.len());
    if n == 0 {
        if sock.nonblocking { return EAGAIN; }
        return 0; // Would block
    }

    for i in 0..n {
        buf[i] = sock.rx_buf.pop_front().unwrap();
    }

    crate::serial_println!("[socket] recv {} bytes", n);
    n as i64
}

/// sys_sendto(fd, buf, len, flags, addr_ptr, addrlen) → bytes or error
pub fn sys_sendto(fd: usize, buf_ptr: u64, len: usize, flags: i32,
                  addr_ptr: u64, _addrlen: u32) -> i64 {
    if addr_ptr != 0 {
        // Set temporary remote address
        let addr = unsafe { &*(addr_ptr as *const SockaddrIn) };
        let mut table = SOCKET_TABLE.lock();
        if let Some(sock) = table.get_mut(fd) {
            sock.remote_ip   = addr.ip();
            sock.remote_port = addr.port();
            if sock.state == SocketState::Bound {
                sock.state = SocketState::Connected;
            }
        }
    }
    sys_send(fd, buf_ptr, len, flags)
}

/// sys_recvfrom(fd, buf, len, flags, addr_ptr, addrlen_ptr) → bytes or error
pub fn sys_recvfrom(fd: usize, buf_ptr: u64, len: usize, flags: i32,
                    addr_ptr: u64, _addrlen_ptr: u64) -> i64 {
    let n = sys_recv(fd, buf_ptr, len, flags);
    if n > 0 && addr_ptr != 0 {
        let table = SOCKET_TABLE.lock();
        if let Some(sock) = table.get(fd) {
            let addr = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
            addr.sin_family = AF_INET as u16;
            addr.sin_port   = sock.remote_port.to_be_bytes();
            addr.sin_addr   = sock.remote_ip;
        }
    }
    n
}

/// sys_close_socket(fd) → 0 or error  
pub fn sys_close_socket(fd: usize) -> i64 {
    let mut table = SOCKET_TABLE.lock();
    if let Some(sock) = table.get(fd) {
        let port = sock.local_port;
        if port != 0 { release_port(port); }
        crate::serial_println!("[socket] Close fd={} port={}", fd, port);
    }
    if table.free(fd) { 0 } else { EBADF }
}

/// sys_setsockopt
pub fn sys_setsockopt(fd: usize, level: i32, optname: i32,
                      optval: u64, _optlen: u32) -> i64 {
    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    match (level, optname) {
        (SOL_SOCKET, SO_REUSEADDR) => {
            let val = unsafe { *(optval as *const i32) };
            sock.reuse_addr = val != 0;
            0
        }
        _ => 0, // Ignore unknown options
    }
}

/// sys_getsockname(fd, addr_ptr, addrlen_ptr)
pub fn sys_getsockname(fd: usize, addr_ptr: u64, _addrlen_ptr: u64) -> i64 {
    let table = SOCKET_TABLE.lock();
    let sock = match table.get(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if addr_ptr != 0 {
        let addr = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
        addr.sin_family = AF_INET as u16;
        addr.sin_port   = sock.local_port.to_be_bytes();
        addr.sin_addr   = sock.local_ip;
    }
    0
}

/// sys_getpeername(fd, addr_ptr, addrlen_ptr)
pub fn sys_getpeername(fd: usize, addr_ptr: u64, _addrlen_ptr: u64) -> i64 {
    let table = SOCKET_TABLE.lock();
    let sock = match table.get(fd) {
        Some(s) => s,
        None => return EBADF,
    };

    if sock.state != SocketState::Connected { return ENOTCONN; }

    if addr_ptr != 0 {
        let addr = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
        addr.sin_family = AF_INET as u16;
        addr.sin_port   = sock.remote_port.to_be_bytes();
        addr.sin_addr   = sock.remote_ip;
    }
    0
}

/// sys_shutdown(fd, how)
pub fn sys_shutdown(fd: usize, _how: i32) -> i64 {
    let mut table = SOCKET_TABLE.lock();
    let sock = match table.get_mut(fd) {
        Some(s) => s,
        None => return EBADF,
    };
    sock.state = SocketState::Closed;
    0
}

/// Check if fd is a socket fd
pub fn is_socket_fd(fd: usize) -> bool {
    fd >= SOCKET_FD_BASE && fd < SOCKET_FD_BASE + MAX_SOCKETS
        && SOCKET_TABLE.lock().get(fd).is_some()
}

/// Deliver received TCP data to appropriate socket
pub fn deliver_tcp(src_ip: [u8; 4], src_port: u16, dst_port: u16,
                   hdr: &tcp::TcpHdr, payload: &[u8]) -> Option<Vec<u8>> {
    let mut table = SOCKET_TABLE.lock();

    // Find matching connected socket
    for slot in table.sockets.iter_mut() {
        if let Some(sock) = slot {
            if sock.local_port == dst_port {
                match sock.state {
                    SocketState::Listening => {
                        // New connection on listening socket
                        if hdr.has_syn() && !hdr.has_ack() {
                            let our_ip = crate::net::our_ip();
                            let mut new_conn = tcp::TcpConn::new(our_ip, dst_port);
                            new_conn.remote_ip   = src_ip;
                            new_conn.remote_port = src_port;
                            new_conn.rcv_nxt = hdr.seq_num().wrapping_add(1);
                            new_conn.state = tcp::TcpState::SynReceived;

                            // Build SYN-ACK
                            let reply = new_conn.build_tcp_pkt(tcp::TCP_SYN | tcp::TCP_ACK, &[]);
                            new_conn.snd_nxt = new_conn.snd_nxt.wrapping_add(1);
                            new_conn.state = tcp::TcpState::Established;

                            // Create accepted socket
                            let mut new_sock = Socket::new(SOCK_STREAM);
                            new_sock.local_ip    = our_ip;
                            new_sock.local_port  = dst_port;
                            new_sock.remote_ip   = src_ip;
                            new_sock.remote_port = src_port;
                            new_sock.state       = SocketState::Connected;
                            new_sock.tcp_conn    = Some(new_conn);

                            sock.backlog.push_back(new_sock);
                            return Some(reply);
                        }
                    }
                    SocketState::Connected => {
                        if sock.remote_ip == src_ip && sock.remote_port == src_port {
                            if let Some(conn) = sock.tcp_conn.as_mut() {
                                // Deliver data to rx_buf
                                if !payload.is_empty() {
                                    for &b in payload { sock.rx_buf.push_back(b); }
                                }
                                return conn.process(src_ip, hdr, payload);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Deliver received UDP data to appropriate socket
pub fn deliver_udp(src_ip: [u8; 4], src_port: u16, dst_port: u16, payload: &[u8]) {
    let mut table = SOCKET_TABLE.lock();
    for slot in table.sockets.iter_mut() {
        if let Some(sock) = slot {
            if sock.sock_type == SOCK_DGRAM && sock.local_port == dst_port {
                sock.remote_ip   = src_ip;
                sock.remote_port = src_port;
                for &b in payload { sock.rx_buf.push_back(b); }
                return;
            }
        }
    }
}
