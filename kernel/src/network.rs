//! First real Nova network hardware path: PCI discovery and Intel e1000 PIO.
//!
//! QEMU's 82540EM exposes the same indexed I/O register window as physical
//! hardware. This path enables PCI I/O and bus mastering, verifies the device
//! status register and reads the hardware MAC without host networking code.

use bootloader_api::BootInfo;
use core::{
    ptr,
    sync::atomic::{Ordering, compiler_fence},
};
use driver_core::{Bar, decode_bar};
use net_core::{
    EthernetFrame, Ipv4Address, Ipv4Packet, MacAddress, TcpSegment, UdpPacket, dhcp_message_type,
    parse_arp_reply, parse_dhcp_offer, parse_dns_a_response, write_arp_request,
    write_dhcp_discover, write_dhcp_request, write_dns_a_query, write_tcp_ipv4_frame,
    write_udp_ipv4_frame,
};
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        FrameAllocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB, Translate,
    },
};

const INTEL: u16 = 0x8086;
const E1000_DEVICES: [u16; 3] = [0x100e, 0x100f, 0x10d3];
const REG_STATUS: u32 = 0x0008;
const REG_RAL: u32 = 0x5400;
const REG_RAH: u32 = 0x5404;
const REG_CTRL: u32 = 0x0000;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;
const RING_ENTRIES: usize = 8;
const DHCP_TRANSACTION: u32 = 0x4e4f_5641;

#[repr(C)]
#[derive(Clone, Copy)]
struct RxDescriptor {
    address: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TxDescriptor {
    address: u64,
    length: u16,
    checksum_offset: u8,
    command: u8,
    status: u8,
    checksum_start: u8,
    special: u16,
}

struct DmaPage {
    frame: PhysFrame<Size4KiB>,
    virtual_address: u64,
}

pub fn initialize(boot_info: &BootInfo) {
    let Some(device) = crate::pci::find_class(0x02, 0x00) else {
        crate::serial::write_str("NOVA_NET_NO_DEVICE\n");
        return;
    };
    if device.vendor != INTEL || !E1000_DEVICES.contains(&device.device) {
        crate::serial::write_str("NOVA_NET_DEVICE_UNSUPPORTED\n");
        return;
    }

    let mut mmio_base = None;
    for offset in [0x10, 0x14, 0x18, 0x1c, 0x20, 0x24] {
        if let Bar::Memory32 { address, .. } =
            decode_bar(crate::pci::read_config(device.address, offset))
        {
            mmio_base = Some(address as u64);
            break;
        }
    }
    let Some(mmio_base) = mmio_base else {
        crate::serial::write_str("NOVA_E1000_NO_MMIO_BAR\n");
        return;
    };

    let command = crate::pci::read_config(device.address, 0x04);
    crate::pci::write_config(device.address, 0x04, command | 0x0000_0005);
    let Ok(mmio_window) = map_mmio(boot_info, mmio_base) else {
        crate::serial::write_str("NOVA_E1000_MMIO_MAP_FAILED\n");
        return;
    };
    let status = read_register(mmio_window, REG_STATUS);
    crate::serial::write_fmt(format_args!(
        "NOVA_E1000_PROBE={:04x}:{:04x},mmio={:08x},status={:08x}\n",
        device.vendor, device.device, mmio_base, status
    ));
    if status == u32::MAX {
        crate::serial::write_str("NOVA_E1000_STATUS_FAILED\n");
        return;
    }
    let low = read_register(mmio_window, REG_RAL);
    let high = read_register(mmio_window, REG_RAH);
    let mac = MacAddress([
        low as u8,
        (low >> 8) as u8,
        (low >> 16) as u8,
        (low >> 24) as u8,
        high as u8,
        (high >> 8) as u8,
    ]);
    if mac.0 == [0; 6] || mac.0 == [0xff; 6] {
        crate::serial::write_str("NOVA_E1000_MAC_FAILED\n");
        return;
    }
    crate::serial::write_fmt(format_args!(
        "NOVA_E1000_READY={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\n",
        mac.0[0], mac.0[1], mac.0[2], mac.0[3], mac.0[4], mac.0[5]
    ));
    if dhcp_probe(boot_info, mmio_window, mac) {
        crate::serial::write_str("NOVA_E1000_TX_RX_OK\n");
    }
}

fn read_register(window: u64, register: u32) -> u32 {
    unsafe { ptr::read_volatile((window + register as u64) as *const u32) }
}

fn write_register(window: u64, register: u32, value: u32) {
    unsafe {
        ptr::write_volatile((window + register as u64) as *mut u32, value);
    }
}

fn allocate_dma(boot_info: &BootInfo, frames: &mut crate::memory::GlobalFrames) -> Option<DmaPage> {
    let offset = boot_info.physical_memory_offset.into_option()?;
    let frame = frames.allocate_frame()?;
    let virtual_address = offset + frame.start_address().as_u64();
    unsafe {
        ptr::write_bytes(virtual_address as *mut u8, 0, 4096);
    }
    Some(DmaPage {
        frame,
        virtual_address,
    })
}

fn dhcp_probe(boot_info: &BootInfo, window: u64, mac: MacAddress) -> bool {
    let mut frames = crate::memory::GlobalFrames;
    let Some(rx_ring) = allocate_dma(boot_info, &mut frames) else {
        return false;
    };
    let Some(tx_ring) = allocate_dma(boot_info, &mut frames) else {
        return false;
    };
    let Some(tx_buffer) = allocate_dma(boot_info, &mut frames) else {
        return false;
    };
    let mut rx_buffers: [Option<DmaPage>; RING_ENTRIES] = core::array::from_fn(|_| None);
    for slot in &mut rx_buffers {
        *slot = allocate_dma(boot_info, &mut frames);
        if slot.is_none() {
            return false;
        }
    }

    let rx_descriptors = rx_ring.virtual_address as *mut RxDescriptor;
    for (index, buffer) in rx_buffers.iter().enumerate() {
        let descriptor = RxDescriptor {
            address: buffer.as_ref().unwrap().frame.start_address().as_u64(),
            length: 0,
            checksum: 0,
            status: 0,
            errors: 0,
            special: 0,
        };
        unsafe {
            ptr::write_volatile(rx_descriptors.add(index), descriptor);
        }
    }
    let tx_descriptors = tx_ring.virtual_address as *mut TxDescriptor;
    for index in 0..RING_ENTRIES {
        unsafe {
            ptr::write_volatile(
                tx_descriptors.add(index),
                TxDescriptor {
                    address: tx_buffer.frame.start_address().as_u64(),
                    length: 0,
                    checksum_offset: 0,
                    command: 0,
                    status: 1,
                    checksum_start: 0,
                    special: 0,
                },
            );
        }
    }

    write_register(
        window,
        REG_RDBAL,
        rx_ring.frame.start_address().as_u64() as u32,
    );
    write_register(
        window,
        REG_RDBAH,
        (rx_ring.frame.start_address().as_u64() >> 32) as u32,
    );
    write_register(window, REG_RDLEN, 128);
    write_register(window, REG_RDH, 0);
    write_register(window, REG_RDT, (RING_ENTRIES - 1) as u32);
    write_register(
        window,
        REG_TDBAL,
        tx_ring.frame.start_address().as_u64() as u32,
    );
    write_register(
        window,
        REG_TDBAH,
        (tx_ring.frame.start_address().as_u64() >> 32) as u32,
    );
    write_register(window, REG_TDLEN, 128);
    write_register(window, REG_TDH, 0);
    write_register(window, REG_TDT, 0);
    write_register(window, REG_CTRL, read_register(window, REG_CTRL) | (1 << 6));
    write_register(
        window,
        REG_TCTL,
        (1 << 1) | (1 << 3) | (0x10 << 4) | (0x40 << 12),
    );
    write_register(window, REG_RCTL, (1 << 1) | (1 << 15) | (1 << 26));

    let mut dhcp = [0u8; 300];
    let Ok(dhcp_length) = write_dhcp_discover(&mut dhcp, DHCP_TRANSACTION, mac) else {
        return false;
    };
    let packet =
        unsafe { core::slice::from_raw_parts_mut(tx_buffer.virtual_address as *mut u8, 4096) };
    let Ok(packet_length) = write_udp_ipv4_frame(
        packet,
        MacAddress([0xff; 6]),
        mac,
        Ipv4Address([0, 0, 0, 0]),
        Ipv4Address([255, 255, 255, 255]),
        68,
        67,
        1,
        &dhcp[..dhcp_length],
    ) else {
        return false;
    };
    let mut tx_index = 0;
    if !transmit(window, tx_descriptors, &mut tx_index, packet_length) {
        return false;
    }

    let mut offer = None;
    if !poll_receive(window, rx_descriptors, &rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        let Ok(ipv4) = Ipv4Packet::parse(ethernet.payload) else {
            return false;
        };
        let Ok(udp) = UdpPacket::parse(ipv4.payload) else {
            return false;
        };
        if udp.source_port != 67 || udp.destination_port != 68 {
            return false;
        }
        let Ok(value) = parse_dhcp_offer(udp.payload, DHCP_TRANSACTION) else {
            return false;
        };
        offer = Some(value);
        true
    }) {
        crate::serial::write_str("NOVA_DHCP_TIMEOUT\n");
        return false;
    }
    let offer = offer.unwrap();
    crate::serial::write_fmt(format_args!(
        "NOVA_DHCP_OFFER={}.{}.{}.{}\n",
        offer.address.0[0], offer.address.0[1], offer.address.0[2], offer.address.0[3]
    ));

    let Some(server) = offer.server else {
        return true;
    };
    let Ok(request_length) =
        write_dhcp_request(&mut dhcp, DHCP_TRANSACTION, mac, offer.address, server)
    else {
        return true;
    };
    let Ok(request_frame_length) = write_udp_ipv4_frame(
        packet,
        MacAddress([0xff; 6]),
        mac,
        Ipv4Address([0, 0, 0, 0]),
        Ipv4Address([255, 255, 255, 255]),
        68,
        67,
        2,
        &dhcp[..request_length],
    ) else {
        return true;
    };
    if !transmit(window, tx_descriptors, &mut tx_index, request_frame_length) {
        return true;
    }
    if !poll_receive(window, rx_descriptors, &rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        let Ok(ipv4) = Ipv4Packet::parse(ethernet.payload) else {
            return false;
        };
        let Ok(udp) = UdpPacket::parse(ipv4.payload) else {
            return false;
        };
        udp.source_port == 67
            && udp.destination_port == 68
            && dhcp_message_type(udp.payload, DHCP_TRANSACTION) == Ok(5)
    }) {
        crate::serial::write_str("NOVA_DHCP_ACK_TIMEOUT\n");
        return true;
    }
    crate::serial::write_str("NOVA_DHCP_BOUND\n");

    let Some(dns_server) = offer.dns else {
        return true;
    };
    let Ok(arp_length) = write_arp_request(packet, mac, offer.address, dns_server) else {
        return true;
    };
    if !transmit(window, tx_descriptors, &mut tx_index, arp_length) {
        return true;
    }
    let mut router_mac = None;
    if !poll_receive(window, rx_descriptors, &rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        if ethernet.ethertype != 0x0806 {
            return false;
        }
        let Ok(reply) = parse_arp_reply(ethernet.payload) else {
            return false;
        };
        if reply.sender_ip != dns_server {
            return false;
        }
        router_mac = Some(reply.sender_mac);
        true
    }) {
        crate::serial::write_str("NOVA_ARP_TIMEOUT\n");
        return true;
    }
    let router_mac = router_mac.unwrap();
    crate::serial::write_str("NOVA_ARP_READY\n");

    crate::serial::write_fmt(format_args!(
        "NOVA_DNS_SERVER={}.{}.{}.{}\n",
        dns_server.0[0], dns_server.0[1], dns_server.0[2], dns_server.0[3]
    ));
    let mut dns_query = [0u8; 128];
    // `localhost` is resolved by the host resolver without requiring external
    // Internet access, making this a deterministic DNS transport gate.
    let Ok(query_length) = write_dns_a_query(&mut dns_query, 0x4e56, "localhost") else {
        return true;
    };
    let Ok(dns_frame_length) = write_udp_ipv4_frame(
        packet,
        router_mac,
        mac,
        offer.address,
        dns_server,
        49152,
        53,
        2,
        &dns_query[..query_length],
    ) else {
        return true;
    };
    if !transmit(window, tx_descriptors, &mut tx_index, dns_frame_length) {
        return true;
    }
    let mut resolved = None;
    if poll_receive(window, rx_descriptors, &rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        crate::serial::write_fmt(format_args!(
            "NOVA_DNS_RX_ETHERTYPE={:04x}\n",
            ethernet.ethertype
        ));
        let Ok(ipv4) = Ipv4Packet::parse(ethernet.payload) else {
            return false;
        };
        crate::serial::write_fmt(format_args!("NOVA_DNS_RX_IP_PROTOCOL={}\n", ipv4.protocol));
        let Ok(udp) = UdpPacket::parse(ipv4.payload) else {
            return false;
        };
        crate::serial::write_fmt(format_args!(
            "NOVA_DNS_RX_UDP={}:{}\n",
            udp.source_port, udp.destination_port
        ));
        if udp.source_port != 53 || udp.destination_port != 49152 {
            return false;
        }
        let Ok(address) = parse_dns_a_response(udp.payload, 0x4e56) else {
            crate::serial::write_str("NOVA_DNS_PARSE_FAILED\n");
            return false;
        };
        resolved = Some(address);
        true
    }) {
        let address = resolved.unwrap();
        crate::serial::write_fmt(format_args!(
            "NOVA_DNS_READY={}.{}.{}.{}\n",
            address.0[0], address.0[1], address.0[2], address.0[3]
        ));
    } else {
        crate::serial::write_str("NOVA_DNS_TIMEOUT\n");
    }
    if let Some(router) = offer.router {
        tcp_http_probe(
            window,
            tx_descriptors,
            &mut tx_index,
            rx_descriptors,
            &rx_buffers,
            packet,
            mac,
            offer.address,
            router,
        );
    }
    true
}

fn tcp_http_probe(
    window: u64,
    tx_descriptors: *mut TxDescriptor,
    tx_index: &mut usize,
    rx_descriptors: *mut RxDescriptor,
    rx_buffers: &[Option<DmaPage>; RING_ENTRIES],
    packet: &mut [u8],
    mac: MacAddress,
    source_ip: Ipv4Address,
    router: Ipv4Address,
) {
    let Ok(arp_length) = write_arp_request(packet, mac, source_ip, router) else {
        return;
    };
    if !transmit(window, tx_descriptors, tx_index, arp_length) {
        return;
    }
    let mut router_mac = None;
    if !poll_receive(window, rx_descriptors, rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        if ethernet.ethertype != 0x0806 {
            return false;
        }
        let Ok(reply) = parse_arp_reply(ethernet.payload) else {
            return false;
        };
        if reply.sender_ip != router {
            return false;
        }
        router_mac = Some(reply.sender_mac);
        true
    }) {
        return;
    }
    let router_mac = router_mac.unwrap();
    const CLIENT_PORT: u16 = 49153;
    const SERVER_PORT: u16 = 8080;
    const INITIAL_SEQUENCE: u32 = 0x1234_5678;
    let Ok(syn_length) = write_tcp_ipv4_frame(
        packet,
        router_mac,
        mac,
        source_ip,
        router,
        CLIENT_PORT,
        SERVER_PORT,
        INITIAL_SEQUENCE,
        0,
        0x02,
        3,
        &[],
    ) else {
        return;
    };
    if !transmit(window, tx_descriptors, tx_index, syn_length) {
        return;
    }
    let mut peer_sequence = None;
    if !poll_receive(window, rx_descriptors, rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        let Ok(ipv4) = Ipv4Packet::parse(ethernet.payload) else {
            return false;
        };
        if ipv4.protocol != 6 {
            return false;
        }
        let Ok(tcp) = TcpSegment::parse(ipv4.payload) else {
            return false;
        };
        if tcp.source_port == SERVER_PORT
            && tcp.destination_port == CLIENT_PORT
            && tcp.flags & 0x12 == 0x12
            && tcp.acknowledgment == INITIAL_SEQUENCE.wrapping_add(1)
        {
            peer_sequence = Some(tcp.sequence);
            return true;
        }
        false
    }) {
        crate::serial::write_str("NOVA_TCP_TIMEOUT\n");
        return;
    }
    crate::serial::write_str("NOVA_TCP_ESTABLISHED\n");
    let request = b"GET / HTTP/1.1\r\nHost: nova.test\r\nConnection: close\r\n\r\n";
    let Ok(request_length) = write_tcp_ipv4_frame(
        packet,
        router_mac,
        mac,
        source_ip,
        router,
        CLIENT_PORT,
        SERVER_PORT,
        INITIAL_SEQUENCE.wrapping_add(1),
        peer_sequence.unwrap().wrapping_add(1),
        0x18,
        4,
        request,
    ) else {
        return;
    };
    if !transmit(window, tx_descriptors, tx_index, request_length) {
        return;
    }
    if poll_receive(window, rx_descriptors, rx_buffers, |bytes| {
        let Ok(ethernet) = EthernetFrame::parse(bytes) else {
            return false;
        };
        let Ok(ipv4) = Ipv4Packet::parse(ethernet.payload) else {
            return false;
        };
        let Ok(tcp) = TcpSegment::parse(ipv4.payload) else {
            return false;
        };
        tcp.source_port == SERVER_PORT
            && tcp.destination_port == CLIENT_PORT
            && tcp.payload.windows(8).any(|window| window == b"HTTP/1.1")
    }) {
        crate::serial::write_str("NOVA_HTTP_RESPONSE_OK\n");
    } else {
        crate::serial::write_str("NOVA_HTTP_TIMEOUT\n");
    }
}

fn transmit(window: u64, descriptors: *mut TxDescriptor, index: &mut usize, length: usize) -> bool {
    let current = *index;
    let mut spins = 0;
    while unsafe { ptr::read_volatile(&(*descriptors.add(current)).status) } & 1 == 0 {
        if spins == 5_000_000 {
            return false;
        }
        core::hint::spin_loop();
        spins += 1;
    }
    unsafe {
        let address = (*descriptors.add(current)).address;
        ptr::write_volatile(
            descriptors.add(current),
            TxDescriptor {
                address,
                length: length as u16,
                checksum_offset: 0,
                command: 0x0b,
                status: 0,
                checksum_start: 0,
                special: 0,
            },
        );
    }
    compiler_fence(Ordering::Release);
    *index = (current + 1) % RING_ENTRIES;
    write_register(window, REG_TDT, *index as u32);
    true
}

fn poll_receive(
    window: u64,
    descriptors: *mut RxDescriptor,
    buffers: &[Option<DmaPage>; RING_ENTRIES],
    mut accept: impl FnMut(&[u8]) -> bool,
) -> bool {
    for _ in 0..10_000_000u32 {
        for index in 0..RING_ENTRIES {
            let descriptor = unsafe { ptr::read_volatile(descriptors.add(index)) };
            if descriptor.status & 1 == 0 {
                continue;
            }
            compiler_fence(Ordering::Acquire);
            let buffer = buffers[index].as_ref().unwrap();
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    buffer.virtual_address as *const u8,
                    descriptor.length as usize,
                )
            };
            let accepted = accept(bytes);
            unsafe {
                (*descriptors.add(index)).status = 0;
                (*descriptors.add(index)).length = 0;
            }
            compiler_fence(Ordering::Release);
            write_register(window, REG_RDT, index as u32);
            if accepted {
                return true;
            }
        }
        core::hint::spin_loop();
    }
    false
}

fn map_mmio(boot_info: &BootInfo, physical_base: u64) -> Result<u64, ()> {
    let offset = boot_info.physical_memory_offset.into_option().ok_or(())?;
    let (level4, _) = Cr3::read();
    let table_address = VirtAddr::new(offset + level4.start_address().as_u64());
    let table = unsafe { &mut *table_address.as_mut_ptr::<PageTable>() };
    let mapper = unsafe { OffsetPageTable::new(table, VirtAddr::new(offset)) };
    let window = offset + physical_base;
    if mapper
        .translate_addr(VirtAddr::new(window))
        .map(|address| address.as_u64())
        != Some(physical_base)
    {
        return Err(());
    }
    Ok(window)
}
