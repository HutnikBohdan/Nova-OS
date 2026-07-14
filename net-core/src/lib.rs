#![no_std]

pub const ETHERNET_HEADER: usize = 14;
pub const IPV4_MIN_HEADER: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress(pub [u8; 6]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Address(pub [u8; 4]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    Truncated,
    Invalid,
    Unsupported,
    BufferTooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthernetFrame<'a> {
    pub destination: MacAddress,
    pub source: MacAddress,
    pub ethertype: u16,
    pub payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NetError> {
        if bytes.len() < ETHERNET_HEADER {
            return Err(NetError::Truncated);
        }
        Ok(Self {
            destination: MacAddress(bytes[0..6].try_into().unwrap()),
            source: MacAddress(bytes[6..12].try_into().unwrap()),
            ethertype: u16::from_be_bytes([bytes[12], bytes[13]]),
            payload: &bytes[ETHERNET_HEADER..],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Packet<'a> {
    pub source: Ipv4Address,
    pub destination: Ipv4Address,
    pub protocol: u8,
    pub payload: &'a [u8],
}

impl<'a> Ipv4Packet<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NetError> {
        if bytes.len() < IPV4_MIN_HEADER {
            return Err(NetError::Truncated);
        }
        if bytes[0] >> 4 != 4 {
            return Err(NetError::Unsupported);
        }
        let header_len = ((bytes[0] & 0x0f) as usize) * 4;
        let total_len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if header_len < IPV4_MIN_HEADER || total_len < header_len || total_len > bytes.len() {
            return Err(NetError::Invalid);
        }
        if internet_checksum(&bytes[..header_len]) != 0 {
            return Err(NetError::Invalid);
        }
        Ok(Self {
            source: Ipv4Address(bytes[12..16].try_into().unwrap()),
            destination: Ipv4Address(bytes[16..20].try_into().unwrap()),
            protocol: bytes[9],
            payload: &bytes[header_len..total_len],
        })
    }
}

pub fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = bytes.chunks_exact(2);
    for pair in &mut chunks {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    if let Some(last) = chunks.remainder().first() {
        sum += (*last as u32) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Writes an Ethernet II + IPv4 + UDP frame with both header and UDP
/// pseudo-header checksums.
pub fn write_udp_ipv4_frame(
    out: &mut [u8],
    destination_mac: MacAddress,
    source_mac: MacAddress,
    source_ip: Ipv4Address,
    destination_ip: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    identification: u16,
    payload: &[u8],
) -> Result<usize, NetError> {
    let udp_length = 8usize.checked_add(payload.len()).ok_or(NetError::Invalid)?;
    let ip_length = IPV4_MIN_HEADER
        .checked_add(udp_length)
        .ok_or(NetError::Invalid)?;
    let frame_length = ETHERNET_HEADER
        .checked_add(ip_length)
        .ok_or(NetError::Invalid)?;
    if udp_length > u16::MAX as usize || ip_length > u16::MAX as usize || out.len() < frame_length {
        return Err(NetError::BufferTooSmall);
    }
    out[..6].copy_from_slice(&destination_mac.0);
    out[6..12].copy_from_slice(&source_mac.0);
    out[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    let ip = &mut out[14..34];
    ip.fill(0);
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&identification.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 17;
    ip[12..16].copy_from_slice(&source_ip.0);
    ip[16..20].copy_from_slice(&destination_ip.0);
    let checksum = internet_checksum(ip);
    ip[10..12].copy_from_slice(&checksum.to_be_bytes());
    let udp = &mut out[34..42];
    udp[0..2].copy_from_slice(&source_port.to_be_bytes());
    udp[2..4].copy_from_slice(&destination_port.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_length as u16).to_be_bytes());
    udp[6..8].fill(0);
    out[42..frame_length].copy_from_slice(payload);
    let udp_checksum = udp_checksum(source_ip, destination_ip, &out[34..frame_length]);
    out[40..42].copy_from_slice(&udp_checksum.to_be_bytes());
    Ok(frame_length)
}

fn udp_checksum(source: Ipv4Address, destination: Ipv4Address, udp: &[u8]) -> u16 {
    let mut sum = 0u32;
    for pair in source.0.chunks_exact(2) {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    for pair in destination.0.chunks_exact(2) {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    sum += 17;
    sum += udp.len() as u32;
    let mut chunks = udp.chunks_exact(2);
    for pair in &mut chunks {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    if let Some(last) = chunks.remainder().first() {
        sum += (*last as u32) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let checksum = !(sum as u16);
    if checksum == 0 { 0xffff } else { checksum }
}

pub fn write_arp_request(
    out: &mut [u8],
    source_mac: MacAddress,
    source_ip: Ipv4Address,
    target_ip: Ipv4Address,
) -> Result<usize, NetError> {
    if out.len() < 42 {
        return Err(NetError::BufferTooSmall);
    }
    out[..6].fill(0xff);
    out[6..12].copy_from_slice(&source_mac.0);
    out[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
    out[14..22].copy_from_slice(&[0x00, 0x01, 0x08, 0x00, 6, 4, 0, 1]);
    out[22..28].copy_from_slice(&source_mac.0);
    out[28..32].copy_from_slice(&source_ip.0);
    out[32..38].fill(0);
    out[38..42].copy_from_slice(&target_ip.0);
    Ok(42)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpReply {
    pub sender_mac: MacAddress,
    pub sender_ip: Ipv4Address,
    pub target_mac: MacAddress,
    pub target_ip: Ipv4Address,
}

pub fn parse_arp_reply(bytes: &[u8]) -> Result<ArpReply, NetError> {
    if bytes.len() < 28 || bytes[0..8] != [0, 1, 8, 0, 6, 4, 0, 2] {
        return Err(NetError::Invalid);
    }
    Ok(ArpReply {
        sender_mac: MacAddress(bytes[8..14].try_into().unwrap()),
        sender_ip: Ipv4Address(bytes[14..18].try_into().unwrap()),
        target_mac: MacAddress(bytes[18..24].try_into().unwrap()),
        target_ip: Ipv4Address(bytes[24..28].try_into().unwrap()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    SynSent,
    Established,
    FinWait,
    TimeWait,
}

pub struct TcpConnection {
    state: TcpState,
    send_sequence: u32,
    receive_sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpSegment<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence: u32,
    pub acknowledgment: u32,
    pub flags: u16,
    pub window: u16,
    pub payload: &'a [u8],
}

impl<'a> TcpSegment<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NetError> {
        if bytes.len() < 20 {
            return Err(NetError::Truncated);
        }
        let header_length = ((bytes[12] >> 4) as usize) * 4;
        if header_length < 20 || header_length > bytes.len() {
            return Err(NetError::Invalid);
        }
        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            sequence: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
            acknowledgment: u32::from_be_bytes(bytes[8..12].try_into().unwrap()),
            flags: (((bytes[12] & 1) as u16) << 8) | bytes[13] as u16,
            window: u16::from_be_bytes([bytes[14], bytes[15]]),
            payload: &bytes[header_length..],
        })
    }
}

pub fn write_tcp_ipv4_frame(
    out: &mut [u8],
    destination_mac: MacAddress,
    source_mac: MacAddress,
    source_ip: Ipv4Address,
    destination_ip: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgment: u32,
    flags: u16,
    identification: u16,
    payload: &[u8],
) -> Result<usize, NetError> {
    let tcp_length = 20usize
        .checked_add(payload.len())
        .ok_or(NetError::Invalid)?;
    let ip_length = 20usize.checked_add(tcp_length).ok_or(NetError::Invalid)?;
    let frame_length = 14usize.checked_add(ip_length).ok_or(NetError::Invalid)?;
    if tcp_length > u16::MAX as usize || ip_length > u16::MAX as usize || out.len() < frame_length {
        return Err(NetError::BufferTooSmall);
    }
    out[..6].copy_from_slice(&destination_mac.0);
    out[6..12].copy_from_slice(&source_mac.0);
    out[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    let ip = &mut out[14..34];
    ip.fill(0);
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&identification.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 6;
    ip[12..16].copy_from_slice(&source_ip.0);
    ip[16..20].copy_from_slice(&destination_ip.0);
    let ip_checksum = internet_checksum(ip);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    let tcp = &mut out[34..54];
    tcp.fill(0);
    tcp[0..2].copy_from_slice(&source_port.to_be_bytes());
    tcp[2..4].copy_from_slice(&destination_port.to_be_bytes());
    tcp[4..8].copy_from_slice(&sequence.to_be_bytes());
    tcp[8..12].copy_from_slice(&acknowledgment.to_be_bytes());
    tcp[12] = 5 << 4 | ((flags >> 8) as u8 & 1);
    tcp[13] = flags as u8;
    tcp[14..16].copy_from_slice(&64240u16.to_be_bytes());
    out[54..frame_length].copy_from_slice(payload);
    let checksum = transport_checksum(source_ip, destination_ip, 6, &out[34..frame_length]);
    out[50..52].copy_from_slice(&checksum.to_be_bytes());
    Ok(frame_length)
}

fn transport_checksum(
    source: Ipv4Address,
    destination: Ipv4Address,
    protocol: u8,
    bytes: &[u8],
) -> u16 {
    let mut sum = 0u32;
    for pair in source.0.chunks_exact(2) {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    for pair in destination.0.chunks_exact(2) {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    sum += protocol as u32;
    sum += bytes.len() as u32;
    let mut chunks = bytes.chunks_exact(2);
    for pair in &mut chunks {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    if let Some(last) = chunks.remainder().first() {
        sum += (*last as u32) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let checksum = !(sum as u16);
    if checksum == 0 { 0xffff } else { checksum }
}

impl TcpConnection {
    pub const fn new() -> Self {
        Self {
            state: TcpState::Closed,
            send_sequence: 0,
            receive_sequence: 0,
        }
    }
    pub fn connect(&mut self, initial_sequence: u32) -> bool {
        if self.state != TcpState::Closed {
            return false;
        }
        self.send_sequence = initial_sequence;
        self.state = TcpState::SynSent;
        true
    }
    pub fn accept_syn_ack(&mut self, acknowledgment: u32, peer_sequence: u32) -> bool {
        if self.state != TcpState::SynSent || acknowledgment != self.send_sequence.wrapping_add(1) {
            return false;
        }
        self.send_sequence = acknowledgment;
        self.receive_sequence = peer_sequence.wrapping_add(1);
        self.state = TcpState::Established;
        true
    }
    pub fn sent(&mut self, bytes: usize) -> bool {
        if self.state != TcpState::Established {
            return false;
        }
        self.send_sequence = self.send_sequence.wrapping_add(bytes as u32);
        true
    }
    pub fn close(&mut self) -> bool {
        if self.state != TcpState::Established {
            return false;
        }
        self.state = TcpState::FinWait;
        true
    }
    pub const fn state(&self) -> TcpState {
        self.state
    }
    pub const fn send_sequence(&self) -> u32 {
        self.send_sequence
    }
}

impl Default for TcpConnection {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpPacket<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: &'a [u8],
}

impl<'a> UdpPacket<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NetError> {
        if bytes.len() < 8 {
            return Err(NetError::Truncated);
        }
        let length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        if length < 8 || length > bytes.len() {
            return Err(NetError::Invalid);
        }
        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            payload: &bytes[8..length],
        })
    }
}

pub fn write_dhcp_discover(
    out: &mut [u8],
    transaction: u32,
    mac: MacAddress,
) -> Result<usize, NetError> {
    if out.len() < 256 {
        return Err(NetError::BufferTooSmall);
    }
    out[..256].fill(0);
    out[0] = 1;
    out[1] = 1;
    out[2] = 6;
    out[4..8].copy_from_slice(&transaction.to_be_bytes());
    out[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    out[28..34].copy_from_slice(&mac.0);
    out[236..240].copy_from_slice(&[99, 130, 83, 99]);
    out[240..244].copy_from_slice(&[53, 1, 1, 55]);
    out[244] = 3;
    out[245..248].copy_from_slice(&[1, 3, 6]);
    out[248] = 255;
    Ok(249)
}

pub fn write_dhcp_request(
    out: &mut [u8],
    transaction: u32,
    mac: MacAddress,
    requested: Ipv4Address,
    server: Ipv4Address,
) -> Result<usize, NetError> {
    if out.len() < 270 {
        return Err(NetError::BufferTooSmall);
    }
    out[..270].fill(0);
    out[0] = 1;
    out[1] = 1;
    out[2] = 6;
    out[4..8].copy_from_slice(&transaction.to_be_bytes());
    out[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    out[28..34].copy_from_slice(&mac.0);
    out[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut at = 240;
    out[at..at + 3].copy_from_slice(&[53, 1, 3]);
    at += 3;
    out[at..at + 2].copy_from_slice(&[50, 4]);
    at += 2;
    out[at..at + 4].copy_from_slice(&requested.0);
    at += 4;
    out[at..at + 2].copy_from_slice(&[54, 4]);
    at += 2;
    out[at..at + 4].copy_from_slice(&server.0);
    at += 4;
    out[at..at + 5].copy_from_slice(&[55, 3, 1, 3, 6]);
    at += 5;
    out[at] = 255;
    Ok(at + 1)
}

pub fn dhcp_message_type(bytes: &[u8], transaction: u32) -> Result<u8, NetError> {
    if bytes.len() < 240
        || bytes[0] != 2
        || bytes[4..8] != transaction.to_be_bytes()
        || bytes[236..240] != [99, 130, 83, 99]
    {
        return Err(NetError::Invalid);
    }
    let mut at = 240;
    while at < bytes.len() {
        let kind = bytes[at];
        at += 1;
        if kind == 255 {
            break;
        }
        if kind == 0 {
            continue;
        }
        let length = *bytes.get(at).ok_or(NetError::Truncated)? as usize;
        at += 1;
        if at + length > bytes.len() {
            return Err(NetError::Truncated);
        }
        if kind == 53 && length == 1 {
            return Ok(bytes[at]);
        }
        at += length;
    }
    Err(NetError::Invalid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DhcpOffer {
    pub address: Ipv4Address,
    pub server: Option<Ipv4Address>,
    pub router: Option<Ipv4Address>,
    pub dns: Option<Ipv4Address>,
}

pub fn parse_dhcp_offer(bytes: &[u8], transaction: u32) -> Result<DhcpOffer, NetError> {
    if bytes.len() < 240
        || bytes[0] != 2
        || bytes[4..8] != transaction.to_be_bytes()
        || bytes[236..240] != [99, 130, 83, 99]
    {
        return Err(NetError::Invalid);
    }
    let mut offer = DhcpOffer {
        address: Ipv4Address(bytes[16..20].try_into().unwrap()),
        server: None,
        router: None,
        dns: None,
    };
    let mut message_type = None;
    let mut at = 240;
    while at < bytes.len() {
        let kind = bytes[at];
        at += 1;
        if kind == 255 {
            break;
        }
        if kind == 0 {
            continue;
        }
        if at >= bytes.len() {
            return Err(NetError::Truncated);
        }
        let length = bytes[at] as usize;
        at += 1;
        if at + length > bytes.len() {
            return Err(NetError::Truncated);
        }
        match (kind, length) {
            (53, 1) => message_type = Some(bytes[at]),
            (54, 4) => offer.server = Some(Ipv4Address(bytes[at..at + 4].try_into().unwrap())),
            (3, 4..) => offer.router = Some(Ipv4Address(bytes[at..at + 4].try_into().unwrap())),
            (6, 4..) => offer.dns = Some(Ipv4Address(bytes[at..at + 4].try_into().unwrap())),
            _ => {}
        }
        at += length;
    }
    if message_type != Some(2) {
        return Err(NetError::Invalid);
    }
    Ok(offer)
}

pub fn write_dns_a_query(out: &mut [u8], id: u16, host: &str) -> Result<usize, NetError> {
    if out.len() < 17 || host.is_empty() {
        return Err(NetError::BufferTooSmall);
    }
    out[..12].fill(0);
    out[0..2].copy_from_slice(&id.to_be_bytes());
    out[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
    out[4..6].copy_from_slice(&1u16.to_be_bytes());
    let mut at = 12;
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 || at + 1 + label.len() + 5 > out.len() {
            return Err(NetError::Invalid);
        }
        out[at] = label.len() as u8;
        at += 1;
        out[at..at + label.len()].copy_from_slice(label.as_bytes());
        at += label.len();
    }
    out[at] = 0;
    at += 1;
    out[at..at + 2].copy_from_slice(&1u16.to_be_bytes());
    at += 2;
    out[at..at + 2].copy_from_slice(&1u16.to_be_bytes());
    at += 2;
    Ok(at)
}

pub fn parse_dns_a_response(bytes: &[u8], expected_id: u16) -> Result<Ipv4Address, NetError> {
    if bytes.len() < 12
        || u16::from_be_bytes([bytes[0], bytes[1]]) != expected_id
        || bytes[3] & 0x0f != 0
    {
        return Err(NetError::Invalid);
    }
    let questions = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let answers = u16::from_be_bytes([bytes[6], bytes[7]]) as usize;
    let mut at = 12;
    for _ in 0..questions {
        at = skip_dns_name(bytes, at)?;
        at = at
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or(NetError::Truncated)?;
    }
    for _ in 0..answers {
        at = skip_dns_name(bytes, at)?;
        if at + 10 > bytes.len() {
            return Err(NetError::Truncated);
        }
        let record_type = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        let class = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]);
        let length = u16::from_be_bytes([bytes[at + 8], bytes[at + 9]]) as usize;
        at += 10;
        if at + length > bytes.len() {
            return Err(NetError::Truncated);
        }
        if record_type == 1 && class == 1 && length == 4 {
            return Ok(Ipv4Address(bytes[at..at + 4].try_into().unwrap()));
        }
        at += length;
    }
    Err(NetError::Invalid)
}

fn skip_dns_name(bytes: &[u8], mut at: usize) -> Result<usize, NetError> {
    loop {
        let length = *bytes.get(at).ok_or(NetError::Truncated)?;
        if length == 0 {
            return Ok(at + 1);
        }
        if length & 0xc0 == 0xc0 {
            if at + 1 >= bytes.len() {
                return Err(NetError::Truncated);
            }
            return Ok(at + 2);
        }
        if length & 0xc0 != 0 {
            return Err(NetError::Invalid);
        }
        at = at
            .checked_add(1 + length as usize)
            .filter(|next| *next <= bytes.len())
            .ok_or(NetError::Truncated)?;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetransmitTimer {
    sequence: u32,
    deadline_ms: u64,
    timeout_ms: u32,
    attempts: u8,
    armed: bool,
}

impl RetransmitTimer {
    pub const fn new() -> Self {
        Self {
            sequence: 0,
            deadline_ms: 0,
            timeout_ms: 250,
            attempts: 0,
            armed: false,
        }
    }
    pub fn arm(&mut self, sequence: u32, now_ms: u64) {
        self.sequence = sequence;
        self.timeout_ms = 250;
        self.deadline_ms = now_ms + 250;
        self.attempts = 0;
        self.armed = true;
    }
    pub fn acknowledge(&mut self, acknowledgment: u32) -> bool {
        if self.armed && acknowledgment > self.sequence {
            self.armed = false;
            true
        } else {
            false
        }
    }
    pub fn poll(&mut self, now_ms: u64) -> Option<u32> {
        if !self.armed || now_ms < self.deadline_ms || self.attempts >= 6 {
            return None;
        }
        self.attempts += 1;
        self.timeout_ms = self.timeout_ms.saturating_mul(2).min(8000);
        self.deadline_ms = now_ms + self.timeout_ms as u64;
        Some(self.sequence)
    }
}

impl Default for RetransmitTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ethernet_frame() {
        let mut bytes = [0u8; 16];
        bytes[0..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        bytes[6..12].copy_from_slice(&[7, 8, 9, 10, 11, 12]);
        bytes[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        assert_eq!(EthernetFrame::parse(&bytes).unwrap().ethertype, 0x0800);
    }

    #[test]
    fn writes_broadcast_arp_request() {
        let mut bytes = [0u8; 42];
        assert_eq!(
            write_arp_request(
                &mut bytes,
                MacAddress([1; 6]),
                Ipv4Address([10, 0, 2, 15]),
                Ipv4Address([10, 0, 2, 2])
            )
            .unwrap(),
            42
        );
        assert_eq!(&bytes[..6], &[0xff; 6]);
        assert_eq!(&bytes[38..42], &[10, 0, 2, 2]);
    }

    #[test]
    fn parses_arp_reply() {
        let bytes = [
            0, 1, 8, 0, 6, 4, 0, 2, 0x52, 0x54, 0, 0x12, 0x35, 2, 10, 0, 2, 2, 1, 2, 3, 4, 5, 6,
            10, 0, 2, 15,
        ];
        let reply = parse_arp_reply(&bytes).unwrap();
        assert_eq!(reply.sender_ip, Ipv4Address([10, 0, 2, 2]));
        assert_eq!(reply.sender_mac, MacAddress([0x52, 0x54, 0, 0x12, 0x35, 2]));
    }

    #[test]
    fn validates_ipv4_checksum() {
        let mut packet = [0u8; 20];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&20u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = 6;
        packet[12..16].copy_from_slice(&[10, 0, 0, 1]);
        packet[16..20].copy_from_slice(&[10, 0, 0, 2]);
        let checksum = internet_checksum(&packet);
        packet[10..12].copy_from_slice(&checksum.to_be_bytes());
        assert_eq!(Ipv4Packet::parse(&packet).unwrap().protocol, 6);
    }

    #[test]
    fn writes_parseable_udp_ipv4_frame() {
        let mut frame = [0u8; 128];
        let length = write_udp_ipv4_frame(
            &mut frame,
            MacAddress([0xff; 6]),
            MacAddress([1, 2, 3, 4, 5, 6]),
            Ipv4Address([0, 0, 0, 0]),
            Ipv4Address([255, 255, 255, 255]),
            68,
            67,
            9,
            b"nova",
        )
        .unwrap();
        let ethernet = EthernetFrame::parse(&frame[..length]).unwrap();
        let ipv4 = Ipv4Packet::parse(ethernet.payload).unwrap();
        let udp = UdpPacket::parse(ipv4.payload).unwrap();
        assert_eq!(udp.source_port, 68);
        assert_eq!(udp.destination_port, 67);
        assert_eq!(udp.payload, b"nova");
    }

    #[test]
    fn tcp_handshake_rejects_wrong_ack() {
        let mut tcp = TcpConnection::new();
        assert!(tcp.connect(100));
        assert!(!tcp.accept_syn_ack(100, 50));
        assert!(tcp.accept_syn_ack(101, 50));
        assert_eq!(tcp.state(), TcpState::Established);
        assert!(tcp.sent(20));
        assert_eq!(tcp.send_sequence(), 121);
    }

    #[test]
    fn writes_and_parses_tcp_syn() {
        let mut frame = [0u8; 96];
        let length = write_tcp_ipv4_frame(
            &mut frame,
            MacAddress([2; 6]),
            MacAddress([1; 6]),
            Ipv4Address([10, 0, 2, 15]),
            Ipv4Address([10, 0, 2, 2]),
            49153,
            8080,
            100,
            0,
            0x02,
            1,
            &[],
        )
        .unwrap();
        let ethernet = EthernetFrame::parse(&frame[..length]).unwrap();
        let ipv4 = Ipv4Packet::parse(ethernet.payload).unwrap();
        let tcp = TcpSegment::parse(ipv4.payload).unwrap();
        assert_eq!(tcp.source_port, 49153);
        assert_eq!(tcp.destination_port, 8080);
        assert_eq!(tcp.sequence, 100);
        assert_eq!(tcp.flags, 0x02);
    }

    #[test]
    fn dhcp_discover_and_offer_round_trip_fields() {
        let mut discover = [0u8; 300];
        assert_eq!(
            write_dhcp_discover(&mut discover, 0x12345678, MacAddress([1, 2, 3, 4, 5, 6])).unwrap(),
            249
        );
        let mut offer = [0u8; 256];
        offer[0] = 2;
        offer[4..8].copy_from_slice(&0x12345678u32.to_be_bytes());
        offer[16..20].copy_from_slice(&[10, 0, 2, 15]);
        offer[236..240].copy_from_slice(&[99, 130, 83, 99]);
        offer[240..249].copy_from_slice(&[53, 1, 2, 3, 4, 10, 0, 2, 2]);
        offer[249] = 255;
        let parsed = parse_dhcp_offer(&offer, 0x12345678).unwrap();
        assert_eq!(parsed.address, Ipv4Address([10, 0, 2, 15]));
        assert_eq!(parsed.router, Some(Ipv4Address([10, 0, 2, 2])));
    }

    #[test]
    fn writes_dhcp_request_and_reads_ack_type() {
        let mut request = [0u8; 300];
        let length = write_dhcp_request(
            &mut request,
            7,
            MacAddress([1; 6]),
            Ipv4Address([10, 0, 2, 15]),
            Ipv4Address([10, 0, 2, 2]),
        )
        .unwrap();
        assert!(
            request[..length]
                .windows(6)
                .any(|part| part == [50, 4, 10, 0, 2, 15])
        );
        let mut ack = [0u8; 244];
        ack[0] = 2;
        ack[4..8].copy_from_slice(&7u32.to_be_bytes());
        ack[236..240].copy_from_slice(&[99, 130, 83, 99]);
        ack[240..244].copy_from_slice(&[53, 1, 5, 255]);
        assert_eq!(dhcp_message_type(&ack, 7).unwrap(), 5);
    }

    #[test]
    fn dns_query_and_compressed_a_response() {
        let mut query = [0u8; 64];
        let len = write_dns_a_query(&mut query, 7, "nova.local").unwrap();
        let mut response = [0u8; 64];
        response[..len].copy_from_slice(&query[..len]);
        response[2] = 0x81;
        response[3] = 0x80;
        response[6..8].copy_from_slice(&1u16.to_be_bytes());
        let mut at = len;
        response[at..at + 2].copy_from_slice(&[0xc0, 0x0c]);
        at += 2;
        response[at..at + 10].copy_from_slice(&[0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
        at += 10;
        response[at..at + 4].copy_from_slice(&[192, 0, 2, 1]);
        at += 4;
        assert_eq!(
            parse_dns_a_response(&response[..at], 7).unwrap(),
            Ipv4Address([192, 0, 2, 1])
        );
    }

    #[test]
    fn retransmit_timer_uses_exponential_backoff() {
        let mut timer = RetransmitTimer::new();
        timer.arm(100, 0);
        assert_eq!(timer.poll(249), None);
        assert_eq!(timer.poll(250), Some(100));
        assert_eq!(timer.poll(749), None);
        assert_eq!(timer.poll(750), Some(100));
        assert!(timer.acknowledge(101));
        assert_eq!(timer.poll(10_000), None);
    }
}
