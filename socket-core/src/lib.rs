#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free asynchronous socket service for Nova OS.
//!
//! This layer owns policy, socket state and bounded queues.  A driver/service
//! below it translates [`TxRequest`] and [`RxPacket`] to and from `net-core`;
//! this crate deliberately does not parse or construct NIC frames.

#[cfg(test)]
extern crate std;

use core::cmp;

pub const PAYLOAD_CAPACITY: usize = 512;
pub const DNS_NAME_CAPACITY: usize = 63;
const EPHEMERAL_FIRST: u16 = 49_152;
const EPHEMERAL_LAST: u16 = 65_535;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const UNSPECIFIED: Self = Self([0; 4]);
    pub const BROADCAST: Self = Self([255; 4]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }
    pub const fn as_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }
    pub const fn is_unspecified(self) -> bool {
        self.as_u32() == 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Endpoint {
    pub address: Ipv4Addr,
    pub port: u16,
}

impl Endpoint {
    pub const fn new(address: Ipv4Addr, port: u16) -> Self {
        Self { address, port }
    }
}

pub type ProcessId = u32;
pub type Tick = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketHandle {
    pub slot: u16,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Udp,
    Tcp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    Established,
    CloseWait,
    FinWait,
    TimeWait,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketState {
    UdpBound,
    Tcp(TcpState),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities(u8);

impl Capabilities {
    pub const NONE: Self = Self(0);
    pub const NETWORK: Self = Self(1);
    pub const LISTEN: Self = Self(2);
    pub const RAW_ROUTE: Self = Self(4);
    pub const ALL: Self = Self(7);
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for Capabilities {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    НемаєМісця,
    ХибнийДескриптор,
    НемаєДозволу,
    ПеревищеноКвоту,
    АдресаЗайнята,
    НеПідключено,
    УжеПідключено,
    ЧергаЗаповнена,
    ДаніЗавеликі,
    СпробуйтеПізніше,
    НемаєМаршруту,
    ХибнийСтан,
    ІмяЗавелике,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Payload {
    bytes: [u8; PAYLOAD_CAPACITY],
    len: u16,
}

impl Payload {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; PAYLOAD_CAPACITY],
            len: 0,
        }
    }
    pub fn from_slice(data: &[u8]) -> Result<Self, Error> {
        if data.len() > PAYLOAD_CAPACITY {
            return Err(Error::ДаніЗавеликі);
        }
        let mut result = Self::empty();
        result.bytes[..data.len()].copy_from_slice(data);
        result.len = data.len() as u16;
        Ok(result)
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for Payload {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RxPacket {
    pub source: Endpoint,
    pub destination: Endpoint,
    pub payload: Payload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxKind {
    Udp,
    TcpData,
    TcpSyn,
    TcpAck,
    TcpFin,
    DnsQuery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxRequest {
    pub socket: Option<SocketHandle>,
    pub kind: TxKind,
    pub source: Endpoint,
    pub destination: Endpoint,
    pub payload: Payload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Читання,
    Запис,
    Підключено,
    НовеЗєднання,
    Закрито,
    Помилка,
    DnsГотово,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event {
    pub socket: Option<SocketHandle>,
    pub kind: EventKind,
}

#[derive(Clone, Copy)]
struct Ring<T: Copy, const N: usize> {
    items: [Option<T>; N],
    head: usize,
    len: usize,
}

impl<T: Copy, const N: usize> Ring<T, N> {
    const fn new() -> Self {
        Self {
            items: [None; N],
            head: 0,
            len: 0,
        }
    }
    fn push(&mut self, value: T) -> Result<(), Error> {
        if self.len == N {
            return Err(Error::ЧергаЗаповнена);
        }
        let at = (self.head + self.len) % N;
        self.items[at] = Some(value);
        self.len += 1;
        Ok(())
    }
    fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.items[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        value
    }
    const fn is_empty(&self) -> bool {
        self.len == 0
    }
    const fn is_full(&self) -> bool {
        self.len == N
    }
}

#[derive(Clone, Copy)]
struct Socket<const Q: usize> {
    owner: ProcessId,
    protocol: Protocol,
    state: SocketState,
    local: Endpoint,
    remote: Endpoint,
    backlog: Ring<Endpoint, Q>,
    rx: Ring<RxPacket, Q>,
    tx: Ring<TxRequest, Q>,
    deadline: Tick,
    retries: u8,
}

#[derive(Clone, Copy)]
struct Slot<const Q: usize> {
    generation: u16,
    socket: Option<Socket<Q>>,
}

impl<const Q: usize> Slot<Q> {
    const fn empty() -> Self {
        Self {
            generation: 1,
            socket: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessPolicy {
    pub process: ProcessId,
    pub capabilities: Capabilities,
    pub socket_limit: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Interface {
    pub id: u8,
    pub up: bool,
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
    pub mtu: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Route {
    pub network: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Ipv4Addr,
    pub interface: u8,
    pub metric: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DnsName {
    bytes: [u8; DNS_NAME_CAPACITY],
    len: u8,
}

impl DnsName {
    pub fn new(name: &str) -> Result<Self, Error> {
        if name.is_empty() || name.len() > DNS_NAME_CAPACITY {
            return Err(Error::ІмяЗавелике);
        }
        let mut bytes = [0; DNS_NAME_CAPACITY];
        for (at, byte) in name.bytes().enumerate() {
            bytes[at] = byte.to_ascii_lowercase();
        }
        Ok(Self {
            bytes,
            len: name.len() as u8,
        })
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsState {
    Empty,
    Pending,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DnsEntry {
    pub state: DnsState,
    pub name: Option<DnsName>,
    pub address: Ipv4Addr,
    pub expires: Tick,
    pub next_retry: Tick,
    pub retries: u8,
    pub owner: ProcessId,
}

impl DnsEntry {
    const fn empty() -> Self {
        Self {
            state: DnsState::Empty,
            name: None,
            address: Ipv4Addr::UNSPECIFIED,
            expires: 0,
            next_retry: 0,
            retries: 0,
            owner: 0,
        }
    }
}

pub struct SocketService<
    const S: usize,
    const Q: usize,
    const P: usize,
    const I: usize,
    const R: usize,
    const D: usize,
    const E: usize,
> {
    sockets: [Slot<Q>; S],
    policies: [Option<ProcessPolicy>; P],
    interfaces: [Option<Interface>; I],
    routes: [Option<Route>; R],
    dns: [DnsEntry; D],
    events: Ring<Event, E>,
    next_ephemeral: u16,
    now: Tick,
}

impl<
    const S: usize,
    const Q: usize,
    const P: usize,
    const I: usize,
    const R: usize,
    const D: usize,
    const E: usize,
> SocketService<S, Q, P, I, R, D, E>
{
    pub const fn new() -> Self {
        Self {
            sockets: [Slot::empty(); S],
            policies: [None; P],
            interfaces: [None; I],
            routes: [None; R],
            dns: [DnsEntry::empty(); D],
            events: Ring::new(),
            next_ephemeral: EPHEMERAL_FIRST,
            now: 0,
        }
    }

    pub const fn now(&self) -> Tick {
        self.now
    }

    pub fn set_policy(&mut self, policy: ProcessPolicy) -> Result<(), Error> {
        if let Some(slot) = self
            .policies
            .iter_mut()
            .find(|p| p.is_some_and(|v| v.process == policy.process))
        {
            *slot = Some(policy);
            return Ok(());
        }
        let slot = self
            .policies
            .iter_mut()
            .find(|p| p.is_none())
            .ok_or(Error::НемаєМісця)?;
        *slot = Some(policy);
        Ok(())
    }

    pub fn set_interface(&mut self, interface: Interface) -> Result<(), Error> {
        if let Some(slot) = self
            .interfaces
            .iter_mut()
            .find(|i| i.is_some_and(|v| v.id == interface.id))
        {
            *slot = Some(interface);
            return Ok(());
        }
        *self
            .interfaces
            .iter_mut()
            .find(|i| i.is_none())
            .ok_or(Error::НемаєМісця)? = Some(interface);
        Ok(())
    }

    pub fn add_route(&mut self, route: Route) -> Result<(), Error> {
        *self
            .routes
            .iter_mut()
            .find(|r| r.is_none())
            .ok_or(Error::НемаєМісця)? = Some(route);
        Ok(())
    }

    pub fn route(&self, address: Ipv4Addr) -> Option<Route> {
        let mut best: Option<Route> = None;
        for route in self.routes.iter().flatten().copied() {
            let bits = cmp::min(route.prefix_len, 32);
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            if address.as_u32() & mask != route.network.as_u32() & mask {
                continue;
            }
            if best.is_none_or(|old| {
                route.prefix_len > old.prefix_len
                    || (route.prefix_len == old.prefix_len && route.metric < old.metric)
            }) {
                best = Some(route);
            }
        }
        best
    }

    fn policy(&self, process: ProcessId) -> Result<ProcessPolicy, Error> {
        self.policies
            .iter()
            .flatten()
            .copied()
            .find(|p| p.process == process)
            .ok_or(Error::НемаєДозволу)
    }

    fn require(
        &self,
        process: ProcessId,
        capability: Capabilities,
    ) -> Result<ProcessPolicy, Error> {
        let policy = self.policy(process)?;
        if !policy.capabilities.contains(capability) {
            return Err(Error::НемаєДозволу);
        }
        Ok(policy)
    }

    pub fn open(&mut self, process: ProcessId, protocol: Protocol) -> Result<SocketHandle, Error> {
        let policy = self.require(process, Capabilities::NETWORK)?;
        let used = self
            .sockets
            .iter()
            .filter(|s| s.socket.is_some_and(|v| v.owner == process))
            .count();
        if used >= policy.socket_limit as usize {
            return Err(Error::ПеревищеноКвоту);
        }
        let (index, slot) = self
            .sockets
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.socket.is_none())
            .ok_or(Error::НемаєМісця)?;
        let state = match protocol {
            Protocol::Udp => SocketState::UdpBound,
            Protocol::Tcp => SocketState::Tcp(TcpState::Closed),
        };
        slot.socket = Some(Socket {
            owner: process,
            protocol,
            state,
            local: Endpoint::default(),
            remote: Endpoint::default(),
            backlog: Ring::new(),
            rx: Ring::new(),
            tx: Ring::new(),
            deadline: 0,
            retries: 0,
        });
        Ok(SocketHandle {
            slot: index as u16,
            generation: slot.generation,
        })
    }

    fn socket(&self, handle: SocketHandle) -> Result<&Socket<Q>, Error> {
        let slot = self
            .sockets
            .get(handle.slot as usize)
            .ok_or(Error::ХибнийДескриптор)?;
        if slot.generation != handle.generation {
            return Err(Error::ХибнийДескриптор);
        }
        slot.socket.as_ref().ok_or(Error::ХибнийДескриптор)
    }

    fn socket_mut(&mut self, handle: SocketHandle) -> Result<&mut Socket<Q>, Error> {
        let slot = self
            .sockets
            .get_mut(handle.slot as usize)
            .ok_or(Error::ХибнийДескриптор)?;
        if slot.generation != handle.generation {
            return Err(Error::ХибнийДескриптор);
        }
        slot.socket.as_mut().ok_or(Error::ХибнийДескриптор)
    }

    fn authorize(&self, process: ProcessId, handle: SocketHandle) -> Result<(), Error> {
        if self.socket(handle)?.owner != process {
            return Err(Error::НемаєДозволу);
        }
        Ok(())
    }

    fn port_in_use(&self, protocol: Protocol, address: Ipv4Addr, port: u16) -> bool {
        self.sockets.iter().filter_map(|s| s.socket).any(|s| {
            s.protocol == protocol
                && s.local.port == port
                && (s.local.address.is_unspecified()
                    || address.is_unspecified()
                    || s.local.address == address)
        })
    }

    fn ephemeral(&mut self, protocol: Protocol, address: Ipv4Addr) -> Result<u16, Error> {
        let attempts = (EPHEMERAL_LAST - EPHEMERAL_FIRST) as usize + 1;
        for _ in 0..attempts {
            let candidate = self.next_ephemeral;
            self.next_ephemeral = if candidate == EPHEMERAL_LAST {
                EPHEMERAL_FIRST
            } else {
                candidate + 1
            };
            if !self.port_in_use(protocol, address, candidate) {
                return Ok(candidate);
            }
        }
        Err(Error::НемаєМісця)
    }

    pub fn bind(
        &mut self,
        process: ProcessId,
        handle: SocketHandle,
        mut local: Endpoint,
    ) -> Result<Endpoint, Error> {
        self.authorize(process, handle)?;
        let protocol = self.socket(handle)?.protocol;
        if local.port == 0 {
            local.port = self.ephemeral(protocol, local.address)?;
        }
        if self.port_in_use(protocol, local.address, local.port) {
            return Err(Error::АдресаЗайнята);
        }
        self.socket_mut(handle)?.local = local;
        Ok(local)
    }

    pub fn listen(&mut self, process: ProcessId, handle: SocketHandle) -> Result<(), Error> {
        self.require(process, Capabilities::LISTEN)?;
        self.authorize(process, handle)?;
        let socket = self.socket_mut(handle)?;
        if socket.protocol != Protocol::Tcp || socket.local.port == 0 {
            return Err(Error::ХибнийСтан);
        }
        socket.state = SocketState::Tcp(TcpState::Listen);
        Ok(())
    }

    pub fn inject_connection(&mut self, local: Endpoint, remote: Endpoint) -> Result<(), Error> {
        let (index, slot) = self
            .sockets
            .iter_mut()
            .enumerate()
            .find(|(_, s)| {
                s.socket.is_some_and(|v| {
                    v.state == SocketState::Tcp(TcpState::Listen) && v.local.port == local.port
                })
            })
            .ok_or(Error::НеПідключено)?;
        slot.socket.as_mut().unwrap().backlog.push(remote)?;
        let handle = SocketHandle {
            slot: index as u16,
            generation: slot.generation,
        };
        let _ = self.events.push(Event {
            socket: Some(handle),
            kind: EventKind::НовеЗєднання,
        });
        Ok(())
    }

    pub fn accept(
        &mut self,
        process: ProcessId,
        listener: SocketHandle,
    ) -> Result<(SocketHandle, Endpoint), Error> {
        self.authorize(process, listener)?;
        let (remote, local) = {
            let s = self.socket_mut(listener)?;
            if s.state != SocketState::Tcp(TcpState::Listen) {
                return Err(Error::ХибнийСтан);
            }
            (s.backlog.pop().ok_or(Error::СпробуйтеПізніше)?, s.local)
        };
        let child = self.open(process, Protocol::Tcp)?;
        let socket = self.socket_mut(child)?;
        socket.local = local;
        socket.remote = remote;
        socket.state = SocketState::Tcp(TcpState::Established);
        Ok((child, remote))
    }

    pub fn connect(
        &mut self,
        process: ProcessId,
        handle: SocketHandle,
        remote: Endpoint,
    ) -> Result<(), Error> {
        self.authorize(process, handle)?;
        if self.route(remote.address).is_none() {
            return Err(Error::НемаєМаршруту);
        }
        let protocol = self.socket(handle)?.protocol;
        if self.socket(handle)?.local.port == 0 {
            let local = Endpoint::new(
                Ipv4Addr::UNSPECIFIED,
                self.ephemeral(protocol, Ipv4Addr::UNSPECIFIED)?,
            );
            self.socket_mut(handle)?.local = local;
        }
        let now = self.now;
        let socket = self.socket_mut(handle)?;
        if protocol == Protocol::Tcp && socket.state != SocketState::Tcp(TcpState::Closed) {
            return Err(Error::УжеПідключено);
        }
        socket.remote = remote;
        if protocol == Protocol::Tcp {
            socket.state = SocketState::Tcp(TcpState::SynSent);
            socket.deadline = now + 1_000;
            socket.retries = 0;
            socket.tx.push(TxRequest {
                socket: Some(handle),
                kind: TxKind::TcpSyn,
                source: socket.local,
                destination: remote,
                payload: Payload::empty(),
            })?;
        }
        Ok(())
    }

    pub fn complete_connect(&mut self, handle: SocketHandle) -> Result<(), Error> {
        let socket = self.socket_mut(handle)?;
        if socket.state != SocketState::Tcp(TcpState::SynSent) {
            return Err(Error::ХибнийСтан);
        }
        socket.state = SocketState::Tcp(TcpState::Established);
        socket.deadline = 0;
        let _ = self.events.push(Event {
            socket: Some(handle),
            kind: EventKind::Підключено,
        });
        Ok(())
    }

    pub fn send(
        &mut self,
        process: ProcessId,
        handle: SocketHandle,
        destination: Option<Endpoint>,
        data: &[u8],
    ) -> Result<usize, Error> {
        self.authorize(process, handle)?;
        let payload = Payload::from_slice(data)?;
        let socket = self.socket_mut(handle)?;
        let destination = destination.unwrap_or(socket.remote);
        if destination.port == 0 {
            return Err(Error::НеПідключено);
        }
        let kind = match socket.protocol {
            Protocol::Udp => TxKind::Udp,
            Protocol::Tcp if socket.state == SocketState::Tcp(TcpState::Established) => {
                TxKind::TcpData
            }
            Protocol::Tcp => return Err(Error::НеПідключено),
        };
        socket.tx.push(TxRequest {
            socket: Some(handle),
            kind,
            source: socket.local,
            destination,
            payload,
        })?;
        Ok(data.len())
    }

    pub fn next_tx(&mut self) -> Option<TxRequest> {
        for slot in &mut self.sockets {
            if let Some(socket) = &mut slot.socket
                && let Some(tx) = socket.tx.pop()
            {
                return Some(tx);
            }
        }
        None
    }

    pub fn inject_rx(&mut self, protocol: Protocol, packet: RxPacket) -> Result<(), Error> {
        let (index, slot) = self
            .sockets
            .iter_mut()
            .enumerate()
            .find(|(_, s)| {
                s.socket.is_some_and(|v| {
                    v.protocol == protocol
                        && v.local.port == packet.destination.port
                        && (v.remote.port == 0 || v.remote == packet.source)
                })
            })
            .ok_or(Error::НеПідключено)?;
        let socket = slot.socket.as_mut().unwrap();
        socket.rx.push(packet)?;
        let handle = SocketHandle {
            slot: index as u16,
            generation: slot.generation,
        };
        let _ = self.events.push(Event {
            socket: Some(handle),
            kind: EventKind::Читання,
        });
        Ok(())
    }

    pub fn recv(&mut self, process: ProcessId, handle: SocketHandle) -> Result<RxPacket, Error> {
        self.authorize(process, handle)?;
        self.socket_mut(handle)?
            .rx
            .pop()
            .ok_or(Error::СпробуйтеПізніше)
    }

    pub fn close(&mut self, process: ProcessId, handle: SocketHandle) -> Result<(), Error> {
        self.authorize(process, handle)?;
        let slot = &mut self.sockets[handle.slot as usize];
        slot.socket = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        let _ = self.events.push(Event {
            socket: Some(handle),
            kind: EventKind::Закрито,
        });
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Event> {
        self.events.pop()
    }

    pub fn readiness(&self, handle: SocketHandle) -> Result<(bool, bool), Error> {
        let socket = self.socket(handle)?;
        Ok((
            !socket.rx.is_empty() || !socket.backlog.is_empty(),
            !socket.tx.is_full(),
        ))
    }

    pub fn dns_lookup(
        &mut self,
        process: ProcessId,
        name: &str,
    ) -> Result<Option<Ipv4Addr>, Error> {
        self.require(process, Capabilities::NETWORK)?;
        let name = DnsName::new(name)?;
        if let Some(entry) = self
            .dns
            .iter()
            .find(|e| e.name == Some(name) && e.state == DnsState::Ready && e.expires > self.now)
        {
            return Ok(Some(entry.address));
        }
        if self
            .dns
            .iter()
            .any(|e| e.name == Some(name) && e.state == DnsState::Pending)
        {
            return Ok(None);
        }
        let at = self
            .dns
            .iter()
            .position(|e| e.state == DnsState::Empty || e.expires <= self.now)
            .ok_or(Error::НемаєМісця)?;
        self.dns[at] = DnsEntry {
            state: DnsState::Pending,
            name: Some(name),
            address: Ipv4Addr::UNSPECIFIED,
            expires: 0,
            next_retry: self.now,
            retries: 0,
            owner: process,
        };
        Ok(None)
    }

    pub fn complete_dns(
        &mut self,
        name: &str,
        address: Ipv4Addr,
        ttl_ticks: Tick,
    ) -> Result<(), Error> {
        let name = DnsName::new(name)?;
        let entry = self
            .dns
            .iter_mut()
            .find(|e| e.name == Some(name) && e.state == DnsState::Pending)
            .ok_or(Error::ХибнийСтан)?;
        entry.state = DnsState::Ready;
        entry.address = address;
        entry.expires = self.now.saturating_add(ttl_ticks);
        let _ = self.events.push(Event {
            socket: None,
            kind: EventKind::DnsГотово,
        });
        Ok(())
    }

    pub fn tick(&mut self, now: Tick) {
        self.now = now;
        for (index, slot) in self.sockets.iter_mut().enumerate() {
            let Some(socket) = &mut slot.socket else {
                continue;
            };
            if socket.state == SocketState::Tcp(TcpState::SynSent)
                && socket.deadline != 0
                && now >= socket.deadline
            {
                let handle = SocketHandle {
                    slot: index as u16,
                    generation: slot.generation,
                };
                if socket.retries < 4 && !socket.tx.is_full() {
                    let _ = socket.tx.push(TxRequest {
                        socket: Some(handle),
                        kind: TxKind::TcpSyn,
                        source: socket.local,
                        destination: socket.remote,
                        payload: Payload::empty(),
                    });
                    socket.retries += 1;
                    socket.deadline = now.saturating_add(1_000u64 << socket.retries);
                } else if socket.retries >= 4 {
                    socket.state = SocketState::Tcp(TcpState::Closed);
                    socket.deadline = 0;
                    let _ = self.events.push(Event {
                        socket: Some(handle),
                        kind: EventKind::Помилка,
                    });
                }
            }
        }
        for entry in &mut self.dns {
            if entry.state != DnsState::Pending || now < entry.next_retry {
                continue;
            }
            if entry.retries >= 4 {
                entry.state = DnsState::Failed;
                let _ = self.events.push(Event {
                    socket: None,
                    kind: EventKind::Помилка,
                });
                continue;
            }
            entry.retries += 1;
            entry.next_retry = now.saturating_add(500u64 << entry.retries);
        }
    }

    pub fn dns_query_due(&self) -> Option<(ProcessId, DnsName, u8)> {
        self.dns
            .iter()
            .find(|e| e.state == DnsState::Pending && e.next_retry > self.now)
            .and_then(|e| e.name.map(|n| (e.owner, n, e.retries)))
    }
}

impl<
    const S: usize,
    const Q: usize,
    const P: usize,
    const I: usize,
    const R: usize,
    const D: usize,
    const E: usize,
> Default for SocketService<S, Q, P, I, R, D, E>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type Service = SocketService<8, 3, 3, 2, 4, 3, 12>;
    const PID: u32 = 7;

    fn service() -> Service {
        let mut s = Service::new();
        s.set_policy(ProcessPolicy {
            process: PID,
            capabilities: Capabilities::ALL,
            socket_limit: 4,
        })
        .unwrap();
        s.set_interface(Interface {
            id: 1,
            up: true,
            address: Ipv4Addr::new(10, 0, 2, 15),
            prefix_len: 24,
            gateway: Ipv4Addr::new(10, 0, 2, 2),
            dns: Ipv4Addr::new(10, 0, 2, 3),
            mtu: 1500,
        })
        .unwrap();
        s.add_route(Route {
            network: Ipv4Addr::UNSPECIFIED,
            prefix_len: 0,
            gateway: Ipv4Addr::new(10, 0, 2, 2),
            interface: 1,
            metric: 10,
        })
        .unwrap();
        s
    }

    #[test]
    fn stale_generation_is_rejected() {
        let mut s = service();
        let h = s.open(PID, Protocol::Udp).unwrap();
        s.close(PID, h).unwrap();
        assert_eq!(s.recv(PID, h), Err(Error::ХибнийДескриптор));
    }
    #[test]
    fn quota_is_enforced() {
        let mut s = service();
        for _ in 0..4 {
            s.open(PID, Protocol::Udp).unwrap();
        }
        assert_eq!(s.open(PID, Protocol::Udp), Err(Error::ПеревищеноКвоту));
    }
    #[test]
    fn capability_is_enforced() {
        let mut s = Service::new();
        s.set_policy(ProcessPolicy {
            process: PID,
            capabilities: Capabilities::NETWORK,
            socket_limit: 2,
        })
        .unwrap();
        let h = s.open(PID, Protocol::Tcp).unwrap();
        s.bind(PID, h, Endpoint::new(Ipv4Addr::UNSPECIFIED, 80))
            .unwrap();
        assert_eq!(s.listen(PID, h), Err(Error::НемаєДозволу));
    }
    #[test]
    fn ephemeral_ports_are_unique() {
        let mut s = service();
        let a = s.open(PID, Protocol::Udp).unwrap();
        let b = s.open(PID, Protocol::Udp).unwrap();
        let a = s.bind(PID, a, Endpoint::default()).unwrap();
        let b = s.bind(PID, b, Endpoint::default()).unwrap();
        assert_ne!(a.port, b.port);
    }
    #[test]
    fn udp_send_receive_and_backpressure() {
        let mut s = service();
        let h = s.open(PID, Protocol::Udp).unwrap();
        let local = s.bind(PID, h, Endpoint::default()).unwrap();
        let peer = Endpoint::new(Ipv4Addr::new(1, 1, 1, 1), 53);
        assert_eq!(s.send(PID, h, Some(peer), b"nova").unwrap(), 4);
        assert_eq!(s.next_tx().unwrap().payload.as_slice(), b"nova");
        let packet = RxPacket {
            source: peer,
            destination: local,
            payload: Payload::from_slice(b"ok").unwrap(),
        };
        s.inject_rx(Protocol::Udp, packet).unwrap();
        assert_eq!(s.recv(PID, h).unwrap().payload.as_slice(), b"ok");
    }
    #[test]
    fn tcp_connect_retries_then_fails() {
        let mut s = service();
        let h = s.open(PID, Protocol::Tcp).unwrap();
        s.connect(PID, h, Endpoint::new(Ipv4Addr::new(8, 8, 8, 8), 443))
            .unwrap();
        for t in [1000, 3000, 7000, 15000, 31000] {
            while s.next_tx().is_some() {}
            s.tick(t);
        }
        assert!(matches!(
            s.poll(),
            Some(Event {
                kind: EventKind::Помилка,
                ..
            })
        ));
    }
    #[test]
    fn tcp_connect_completion() {
        let mut s = service();
        let h = s.open(PID, Protocol::Tcp).unwrap();
        s.connect(PID, h, Endpoint::new(Ipv4Addr::new(1, 1, 1, 1), 443))
            .unwrap();
        s.complete_connect(h).unwrap();
        assert_eq!(s.poll().unwrap().kind, EventKind::Підключено);
        assert_eq!(s.send(PID, h, None, b"GET").unwrap(), 3);
    }
    #[test]
    fn listen_accept_is_bounded() {
        let mut s = service();
        let h = s.open(PID, Protocol::Tcp).unwrap();
        let local = s
            .bind(PID, h, Endpoint::new(Ipv4Addr::UNSPECIFIED, 8080))
            .unwrap();
        s.listen(PID, h).unwrap();
        let peer = Endpoint::new(Ipv4Addr::new(10, 0, 2, 2), 44000);
        s.inject_connection(local, peer).unwrap();
        let (child, actual) = s.accept(PID, h).unwrap();
        assert_eq!(actual, peer);
        assert_eq!(
            s.socket(child).unwrap().state,
            SocketState::Tcp(TcpState::Established)
        );
    }
    #[test]
    fn longest_route_wins() {
        let mut s = service();
        s.add_route(Route {
            network: Ipv4Addr::new(10, 1, 0, 0),
            prefix_len: 16,
            gateway: Ipv4Addr::UNSPECIFIED,
            interface: 1,
            metric: 1,
        })
        .unwrap();
        assert_eq!(s.route(Ipv4Addr::new(10, 1, 2, 3)).unwrap().prefix_len, 16);
    }
    #[test]
    fn dns_cache_normalizes_and_expires() {
        let mut s = service();
        assert_eq!(s.dns_lookup(PID, "Nova.OS").unwrap(), None);
        s.tick(1);
        assert!(s.dns_query_due().is_some());
        s.complete_dns("nova.os", Ipv4Addr::new(10, 0, 2, 9), 100)
            .unwrap();
        assert_eq!(
            s.dns_lookup(PID, "NOVA.OS").unwrap(),
            Some(Ipv4Addr::new(10, 0, 2, 9))
        );
        s.tick(102);
        assert_eq!(s.dns_lookup(PID, "nova.os").unwrap(), None);
    }
    #[test]
    fn payload_limit_is_safe() {
        let data = [1u8; PAYLOAD_CAPACITY + 1];
        assert_eq!(Payload::from_slice(&data), Err(Error::ДаніЗавеликі));
    }
}
