/// UDP — User Datagram Protocol

use alloc::vec::Vec;
use super::ip;

pub const UDP_HDR_LEN: usize = 8;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UdpHdr {
    pub src_port: [u8; 2],
    pub dst_port: [u8; 2],
    pub length:   [u8; 2],
    pub checksum: [u8; 2],
}

impl UdpHdr {
    pub fn src_port(&self) -> u16 { u16::from_be_bytes(self.src_port) }
    pub fn dst_port(&self) -> u16 { u16::from_be_bytes(self.dst_port) }
    pub fn length(&self)   -> u16 { u16::from_be_bytes(self.length) }
}

pub fn parse_udp(data: &[u8]) -> Option<(&UdpHdr, &[u8])> {
    if data.len() < UDP_HDR_LEN { return None; }
    let hdr = unsafe { &*(data.as_ptr() as *const UdpHdr) };
    let payload_len = hdr.length() as usize - UDP_HDR_LEN;
    let payload = &data[UDP_HDR_LEN..][..payload_len.min(data.len() - UDP_HDR_LEN)];
    Some((hdr, payload))
}

pub fn build_udp(
    src_ip: [u8; 4], dst_ip: [u8; 4],
    src_port: u16, dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let length = (UDP_HDR_LEN + payload.len()) as u16;
    let mut udp = Vec::with_capacity(UDP_HDR_LEN + payload.len());
    udp.push((src_port >> 8) as u8); udp.push(src_port as u8);
    udp.push((dst_port >> 8) as u8); udp.push(dst_port as u8);
    udp.push((length >> 8) as u8);   udp.push(length as u8);
    udp.push(0u8); udp.push(0u8);   // checksum=0 (optional for IPv4)
    udp.extend_from_slice(payload);

    // UDP checksum via pseudo-header
    let cksum = udp_checksum(&src_ip, &dst_ip, &udp);
    udp[6] = (cksum >> 8) as u8;
    udp[7] = cksum as u8;

    ip::build_ipv4(src_ip, dst_ip, ip::PROTO_UDP, &udp)
}

fn udp_checksum(src_ip: &[u8; 4], dst_ip: &[u8; 4], udp: &[u8]) -> u16 {
    // Pseudo-header: src_ip + dst_ip + zero + proto + udp_len
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(src_ip);
    pseudo.extend_from_slice(dst_ip);
    pseudo.push(0u8);
    pseudo.push(ip::PROTO_UDP);
    let len = udp.len() as u16;
    pseudo.push((len >> 8) as u8);
    pseudo.push(len as u8);
    pseudo.extend_from_slice(udp);
    crate::drivers::virtio_net::internet_checksum(&pseudo)
}
