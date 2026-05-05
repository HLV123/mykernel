/// ICMP — Internet Control Message Protocol
/// Hỗ trợ Echo Request/Reply (ping)

use alloc::vec::Vec;
use crate::drivers::virtio_net::internet_checksum;
use super::ip;

pub const ICMP_ECHO_REPLY:   u8 = 0;
pub const ICMP_ECHO_REQUEST: u8 = 8;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IcmpHdr {
    pub icmp_type: u8,
    pub code:      u8,
    pub checksum:  [u8; 2],
    pub id:        [u8; 2],
    pub seq:       [u8; 2],
}

impl IcmpHdr {
    pub fn id(&self)  -> u16 { u16::from_be_bytes(self.id) }
    pub fn seq(&self) -> u16 { u16::from_be_bytes(self.seq) }
}

pub fn parse_icmp(data: &[u8]) -> Option<(&IcmpHdr, &[u8])> {
    if data.len() < 8 { return None; }
    let hdr = unsafe { &*(data.as_ptr() as *const IcmpHdr) };
    Some((hdr, &data[8..]))
}

pub fn build_icmp_reply(
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    id: u16, seq: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut icmp = Vec::new();
    icmp.push(ICMP_ECHO_REPLY);
    icmp.push(0u8);
    icmp.push(0u8); icmp.push(0u8); // checksum placeholder
    icmp.push((id >> 8) as u8); icmp.push(id as u8);
    icmp.push((seq >> 8) as u8); icmp.push(seq as u8);
    icmp.extend_from_slice(payload);

    let cksum = internet_checksum(&icmp);
    icmp[2] = (cksum >> 8) as u8;
    icmp[3] = cksum as u8;

    ip::build_ipv4(src_ip, dst_ip, ip::PROTO_ICMP, &icmp)
}
