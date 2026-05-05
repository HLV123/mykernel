pub mod arp;
pub mod icmp;
pub mod ip;
pub mod socket;
pub mod tcp;
pub mod udp;

use alloc::vec::Vec;
use spin::Mutex;
use crate::drivers::virtio_net::{ETH_P_ARP, ETH_P_IP, build_ethernet_frame};

pub struct NetConfig {
    pub mac:     [u8; 6],
    pub ip:      [u8; 4],
    pub gateway: [u8; 4],
    pub netmask: [u8; 4],
}

static NET_CONFIG: Mutex<NetConfig> = Mutex::new(NetConfig {
    mac:     [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
    ip:      [10, 0, 2, 15],
    gateway: [10, 0, 2, 2],
    netmask: [255, 255, 255, 0],
});

pub fn init() {
    if let Some(mac) = crate::drivers::virtio_net::get_mac() {
        NET_CONFIG.lock().mac = mac;
        crate::serial_println!("[net] Stack initialized, IP=10.0.2.15");
    } else {
        crate::serial_println!("[net] No NIC found, stack in loopback mode");
    }
}

pub fn our_ip()  -> [u8; 4] { NET_CONFIG.lock().ip }
pub fn our_mac() -> [u8; 6] { NET_CONFIG.lock().mac }

pub fn rx_dispatch(eth_frame: &[u8]) -> Option<Vec<u8>> {
    use crate::drivers::virtio_net::EthernetHdr;

    if eth_frame.len() < 14 { return None; }
    let hdr = unsafe { &*(eth_frame.as_ptr() as *const EthernetHdr) };
    let ethertype = hdr.ethertype();
    let payload = &eth_frame[14..];
    let mac = our_mac();
    let ip  = our_ip();

    match ethertype {
        ETH_P_ARP => {
            arp::process_arp(payload, &ip, &mac)
        }
        ETH_P_IP => {
            let (ip_hdr, ip_payload) = ip::parse_ipv4(payload)?;
            let src_ip = ip_hdr.src_ip;
            if ip_hdr.dst_ip != ip { return None; }

            match ip_hdr.protocol {
                ip::PROTO_ICMP => {
                    let (icmp_hdr, icmp_payload) = icmp::parse_icmp(ip_payload)?;
                    if icmp_hdr.icmp_type == icmp::ICMP_ECHO_REQUEST {
                        crate::serial_println!("[net] PING from {}.{}.{}.{}",
                            src_ip[0], src_ip[1], src_ip[2], src_ip[3]);
                        let reply_ip = icmp::build_icmp_reply(
                            ip, src_ip,
                            icmp_hdr.id(), icmp_hdr.seq(),
                            icmp_payload,
                        );
                        let dst_mac = arp::cache_lookup(&src_ip).unwrap_or([0xFF; 6]);
                        Some(build_ethernet_frame(dst_mac, mac, ETH_P_IP, &reply_ip))
                    } else { None }
                }
                ip::PROTO_UDP => {
                    let (udp_hdr, udp_payload) = udp::parse_udp(ip_payload)?;
                    let src_port = udp_hdr.src_port();
                    let dst_port = udp_hdr.dst_port();

                    // Deliver to socket layer first
                    socket::deliver_udp(src_ip, src_port, dst_port, udp_payload);

                    // Built-in UDP echo on port 7 (if no socket is listening)
                    if dst_port == 7 {
                        let reply = udp::build_udp(ip, src_ip, 7, src_port, udp_payload);
                        let dst_mac = arp::cache_lookup(&src_ip).unwrap_or([0xFF; 6]);
                        Some(build_ethernet_frame(dst_mac, mac, ETH_P_IP, &reply))
                    } else { None }
                }
                ip::PROTO_TCP => {
                    let (tcp_hdr, tcp_payload) = tcp::parse_tcp(ip_payload)?;
                    let src_port = tcp_hdr.src_port();
                    let dst_port = tcp_hdr.dst_port();

                    // Deliver to socket layer
                    if let Some(reply_ip) = socket::deliver_tcp(
                        src_ip, src_port, dst_port, tcp_hdr, tcp_payload) {
                        let dst_mac = arp::cache_lookup(&src_ip).unwrap_or([0xFF; 6]);
                        return Some(build_ethernet_frame(dst_mac, mac, ETH_P_IP, &reply_ip));
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn poll() {
    while let Some(pkt) = crate::drivers::virtio_net::recv_packet() {
        let hdr_size = core::mem::size_of::<crate::drivers::virtio_net::VirtioNetHdr>();
        if pkt.data.len() > hdr_size {
            if let Some(reply) = rx_dispatch(&pkt.data[hdr_size..]) {
                let _ = crate::drivers::virtio_net::send_packet(&reply);
            }
        }
    }
}
