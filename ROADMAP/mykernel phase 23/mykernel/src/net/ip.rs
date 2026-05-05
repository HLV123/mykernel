/// IPv4 Layer
///
/// Xử lý IPv4 packet parsing và building.
/// Hỗ trợ: ICMP, UDP, TCP (basic)

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};

// Protocol numbers
pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP:  u8 = 6;
pub const PROTO_UDP:  u8 = 17;

// IP header size (no options)
pub const IP_HDR_LEN: usize = 20;

static NEXT_IP_ID: AtomicU16 = AtomicU16::new(1);

/// IPv4 header (20 bytes, no options)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Ipv4Hdr {
    pub ver_ihl:   u8,   // version=4, IHL=5 (4-byte words)
    pub dscp_ecn:  u8,
    pub total_len: [u8; 2], // big-endian
    pub id:        [u8; 2],
    pub flags_frag:[u8; 2],
    pub ttl:       u8,
    pub protocol:  u8,
    pub checksum:  [u8; 2],
    pub src_ip:    [u8; 4],
    pub dst_ip:    [u8; 4],
}

impl Ipv4Hdr {
    pub fn total_len(&self) -> u16 {
        u16::from_be_bytes(self.total_len)
    }
    pub fn payload_len(&self) -> usize {
        self.total_len() as usize - IP_HDR_LEN
    }
    pub fn id(&self) -> u16 { u16::from_be_bytes(self.id) }
    pub fn is_valid(&self) -> bool {
        self.ver_ihl >> 4 == 4
    }
}

/// Parse IPv4 header from bytes
pub fn parse_ipv4(data: &[u8]) -> Option<(&Ipv4Hdr, &[u8])> {
    if data.len() < IP_HDR_LEN { return None; }
    let hdr = unsafe { &*(data.as_ptr() as *const Ipv4Hdr) };
    if !hdr.is_valid() { return None; }
    let payload = &data[IP_HDR_LEN..];
    Some((hdr, payload))
}

/// Build IPv4 packet
pub fn build_ipv4(
    src: [u8; 4],
    dst: [u8; 4],
    protocol: u8,
    payload: &[u8],
) -> Vec<u8> {
    let id = NEXT_IP_ID.fetch_add(1, Ordering::Relaxed);
    let total = (IP_HDR_LEN + payload.len()) as u16;

    let mut pkt = Vec::with_capacity(IP_HDR_LEN + payload.len());

    // Build header with checksum=0
    let hdr_bytes: [u8; IP_HDR_LEN] = [
        0x45,                          // ver=4 ihl=5
        0x00,                          // dscp/ecn
        (total >> 8) as u8, total as u8,
        (id >> 8) as u8, id as u8,
        0x40, 0x00,                    // flags=DF, frag=0
        64,                            // TTL
        protocol,
        0x00, 0x00,                    // checksum placeholder
        src[0], src[1], src[2], src[3],
        dst[0], dst[1], dst[2], dst[3],
    ];

    let cksum = crate::drivers::virtio_net::internet_checksum(&hdr_bytes);
    pkt.extend_from_slice(&hdr_bytes);
    pkt[10] = (cksum >> 8) as u8;
    pkt[11] = cksum as u8;
    pkt.extend_from_slice(payload);
    pkt
}
