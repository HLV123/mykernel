/// TCP — Transmission Control Protocol
///
/// State machine:
///   CLOSED → LISTEN → SYN_RECEIVED → ESTABLISHED → CLOSE_WAIT → LAST_ACK → CLOSED
///   CLOSED → SYN_SENT → ESTABLISHED → FIN_WAIT_1 → FIN_WAIT_2 → TIME_WAIT → CLOSED
///
/// Flags: SYN=0x02, ACK=0x10, FIN=0x01, RST=0x04, PSH=0x08

use alloc::vec::Vec;
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU32, Ordering};
use super::ip;
use crate::drivers::virtio_net::internet_checksum;

// TCP flags
pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;
pub const TCP_URG: u8 = 0x20;

pub const TCP_HDR_LEN: usize = 20;

static NEXT_PORT: AtomicU32 = AtomicU32::new(49152); // ephemeral ports
static NEXT_ISN:  AtomicU32 = AtomicU32::new(0x1234_5678);

/// TCP header (20 bytes, no options)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TcpHdr {
    pub src_port:  [u8; 2],
    pub dst_port:  [u8; 2],
    pub seq_num:   [u8; 4],
    pub ack_num:   [u8; 4],
    pub data_off:  u8,   // upper 4 bits = header len in 32-bit words
    pub flags:     u8,
    pub window:    [u8; 2],
    pub checksum:  [u8; 2],
    pub urgent:    [u8; 2],
}

impl TcpHdr {
    pub fn src_port(&self) -> u16 { u16::from_be_bytes(self.src_port) }
    pub fn dst_port(&self) -> u16 { u16::from_be_bytes(self.dst_port) }
    pub fn seq_num(&self)  -> u32 { u32::from_be_bytes(self.seq_num) }
    pub fn ack_num(&self)  -> u32 { u32::from_be_bytes(self.ack_num) }
    pub fn hdr_len(&self)  -> usize { ((self.data_off >> 4) as usize) * 4 }
    pub fn has_syn(&self)  -> bool { self.flags & TCP_SYN != 0 }
    pub fn has_ack(&self)  -> bool { self.flags & TCP_ACK != 0 }
    pub fn has_fin(&self)  -> bool { self.flags & TCP_FIN != 0 }
    pub fn has_rst(&self)  -> bool { self.flags & TCP_RST != 0 }
    pub fn has_psh(&self)  -> bool { self.flags & TCP_PSH != 0 }
    pub fn window(&self)   -> u16  { u16::from_be_bytes(self.window) }
}

pub fn parse_tcp(data: &[u8]) -> Option<(&TcpHdr, &[u8])> {
    if data.len() < TCP_HDR_LEN { return None; }
    let hdr = unsafe { &*(data.as_ptr() as *const TcpHdr) };
    let hdr_len = hdr.hdr_len();
    if hdr_len < TCP_HDR_LEN || hdr_len > data.len() { return None; }
    Some((hdr, &data[hdr_len..]))
}

// ---------------------------------------------------------------------------
// TCP Connection State Machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TcpState {
    Closed,
    Listen,
    SynReceived,
    SynSent,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    LastAck,
    TimeWait,
}

#[derive(Debug, Clone)]
pub struct TcpConn {
    pub state:     TcpState,
    pub local_ip:  [u8; 4],
    pub remote_ip: [u8; 4],
    pub local_port:  u16,
    pub remote_port: u16,
    pub snd_una:   u32,  // send unacknowledged
    pub snd_nxt:   u32,  // send next
    pub rcv_nxt:   u32,  // receive next
    pub snd_wnd:   u16,  // send window
    pub rcv_wnd:   u16,  // receive window
    pub rx_buf:    VecDeque<u8>,
    pub tx_buf:    VecDeque<u8>,
}

impl TcpConn {
    pub fn new(local_ip: [u8; 4], local_port: u16) -> Self {
        let isn = NEXT_ISN.fetch_add(64513, Ordering::Relaxed);
        TcpConn {
            state: TcpState::Closed,
            local_ip,
            remote_ip: [0; 4],
            local_port,
            remote_port: 0,
            snd_una: isn,
            snd_nxt: isn,
            rcv_nxt: 0,
            snd_wnd: 65535,
            rcv_wnd: 65535,
            rx_buf: VecDeque::new(),
            tx_buf: VecDeque::new(),
        }
    }

    /// Process incoming TCP segment, return optional reply packet
    pub fn process(&mut self, src_ip: [u8; 4], hdr: &TcpHdr, payload: &[u8]) -> Option<Vec<u8>> {
        match self.state {
            TcpState::Listen => self.process_listen(src_ip, hdr, payload),
            TcpState::SynReceived => self.process_syn_rcvd(hdr, payload),
            TcpState::Established => self.process_established(hdr, payload),
            TcpState::CloseWait => self.process_close_wait(hdr),
            TcpState::LastAck => self.process_last_ack(hdr),
            _ => None,
        }
    }

    fn process_listen(&mut self, src_ip: [u8; 4], hdr: &TcpHdr, _payload: &[u8]) -> Option<Vec<u8>> {
        if !hdr.has_syn() || hdr.has_ack() { return None; }

        self.remote_ip = src_ip;
        self.remote_port = hdr.src_port();
        self.rcv_nxt = hdr.seq_num().wrapping_add(1);
        self.state = TcpState::SynReceived;

        crate::serial_println!("[tcp] SYN from {}:{}", 
            self.remote_port, hdr.seq_num());

        // Send SYN-ACK
        let pkt = self.build_tcp_pkt(TCP_SYN | TCP_ACK, &[]);
        self.snd_nxt = self.snd_nxt.wrapping_add(1); // SYN counts as 1
        Some(pkt)
    }

    fn process_syn_rcvd(&mut self, hdr: &TcpHdr, _payload: &[u8]) -> Option<Vec<u8>> {
        if hdr.has_ack() && hdr.ack_num() == self.snd_nxt {
            self.snd_una = hdr.ack_num();
            self.state = TcpState::Established;
            crate::serial_println!("[tcp] ESTABLISHED port {}", self.local_port);
        }
        None
    }

    fn process_established(&mut self, hdr: &TcpHdr, payload: &[u8]) -> Option<Vec<u8>> {
        // Update ACK
        if hdr.has_ack() {
            self.snd_una = hdr.ack_num();
        }

        // Receive data
        if !payload.is_empty() && hdr.seq_num() == self.rcv_nxt {
            for &b in payload {
                self.rx_buf.push_back(b);
            }
            self.rcv_nxt = self.rcv_nxt.wrapping_add(payload.len() as u32);

            crate::serial_println!("[tcp] RX {} bytes, rcv_nxt={}", 
                payload.len(), self.rcv_nxt);

            // ACK the data
            return Some(self.build_tcp_pkt(TCP_ACK, &[]));
        }

        // FIN from remote
        if hdr.has_fin() {
            self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
            self.state = TcpState::CloseWait;
            crate::serial_println!("[tcp] FIN received, entering CLOSE_WAIT");
            return Some(self.build_tcp_pkt(TCP_ACK, &[]));
        }

        None
    }

    fn process_close_wait(&mut self, _hdr: &TcpHdr) -> Option<Vec<u8>> {
        // Send our FIN
        self.state = TcpState::LastAck;
        let pkt = self.build_tcp_pkt(TCP_FIN | TCP_ACK, &[]);
        self.snd_nxt = self.snd_nxt.wrapping_add(1);
        Some(pkt)
    }

    fn process_last_ack(&mut self, hdr: &TcpHdr) -> Option<Vec<u8>> {
        if hdr.has_ack() {
            self.state = TcpState::Closed;
            crate::serial_println!("[tcp] Connection closed");
        }
        None
    }

    /// Build a TCP segment with optional payload
    pub fn build_tcp_pkt(&self, flags: u8, payload: &[u8]) -> Vec<u8> {
        build_tcp(
            self.local_ip, self.remote_ip,
            self.local_port, self.remote_port,
            self.snd_nxt, self.rcv_nxt,
            self.rcv_wnd, flags, payload,
        )
    }

    /// Queue data to send
    pub fn send(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        if self.state != TcpState::Established { return None; }
        let pkt = self.build_tcp_pkt(TCP_PSH | TCP_ACK, data);
        self.snd_nxt = self.snd_nxt.wrapping_add(data.len() as u32);
        Some(pkt)
    }

    /// Read received data
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.rx_buf.len());
        for i in 0..n {
            buf[i] = self.rx_buf.pop_front().unwrap();
        }
        n
    }

    pub fn is_established(&self) -> bool {
        self.state == TcpState::Established
    }
}

// ---------------------------------------------------------------------------
// TCP packet builder (low-level)
// ---------------------------------------------------------------------------

pub fn build_tcp(
    src_ip: [u8; 4], dst_ip: [u8; 4],
    src_port: u16, dst_port: u16,
    seq: u32, ack: u32,
    window: u16, flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut tcp = Vec::with_capacity(TCP_HDR_LEN + payload.len());

    tcp.push((src_port >> 8) as u8); tcp.push(src_port as u8);
    tcp.push((dst_port >> 8) as u8); tcp.push(dst_port as u8);
    // seq
    tcp.push((seq >> 24) as u8); tcp.push((seq >> 16) as u8);
    tcp.push((seq >> 8) as u8);  tcp.push(seq as u8);
    // ack
    tcp.push((ack >> 24) as u8); tcp.push((ack >> 16) as u8);
    tcp.push((ack >> 8) as u8);  tcp.push(ack as u8);
    tcp.push(0x50u8); // data offset = 5 (20 bytes), reserved = 0
    tcp.push(flags);
    tcp.push((window >> 8) as u8); tcp.push(window as u8);
    tcp.push(0u8); tcp.push(0u8); // checksum placeholder
    tcp.push(0u8); tcp.push(0u8); // urgent pointer
    tcp.extend_from_slice(payload);

    // TCP checksum via pseudo-header
    let cksum = tcp_checksum(&src_ip, &dst_ip, &tcp);
    tcp[16] = (cksum >> 8) as u8;
    tcp[17] = cksum as u8;

    ip::build_ipv4(src_ip, dst_ip, ip::PROTO_TCP, &tcp)
}

fn tcp_checksum(src_ip: &[u8; 4], dst_ip: &[u8; 4], tcp: &[u8]) -> u16 {
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(src_ip);
    pseudo.extend_from_slice(dst_ip);
    pseudo.push(0u8);
    pseudo.push(ip::PROTO_TCP);
    let len = tcp.len() as u16;
    pseudo.push((len >> 8) as u8);
    pseudo.push(len as u8);
    pseudo.extend_from_slice(tcp);
    internet_checksum(&pseudo)
}

pub fn alloc_ephemeral_port() -> u16 {
    (NEXT_PORT.fetch_add(1, Ordering::Relaxed) as u16).max(49152)
}
