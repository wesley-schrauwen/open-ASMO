//! UDP transport for LAN play: the host binds one socket, each client its own.
//!
//! Each direction is a reliable, ordered channel ([`Channel`]): every message gets the next
//! sequence number, and a packet carries the sender's unacknowledged messages plus `ack`, the
//! next number it expects. Messages not acknowledged within [`RESEND_AFTER`] go again; the
//! receiver drops duplicates and holds early ones back, so both ends see each message once, in
//! the order sent. An idle side sends an empty packet every [`KEEPALIVE`]; a side not heard
//! from for [`TIMEOUT`] is dropped.
//!
//! A client's first message is [`ToHost::Hello`] with its player id. The host keys the client by
//! its address from then on; a second address with the same player id replaces the first.
//! There is no authentication: on a LAN, players are trusted to send their own id.
//!
//! The wire encoding is JSON, one packet per datagram of at most [`MAX_PACKET`] bytes. A
//! message larger than that is not sent (and logged); the largest today, a character update,
//! is a few kilobytes.
//!
//! Each end runs a thread that owns the socket; the transport handle talks to it over channels,
//! so its methods never block. Dropping the handle sends a [`ToHost::Bye`] or [`ToClient::Bye`]
//! and stops the thread.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arcanum_sim::{LedgerDelta, PlayerId, PlayerIntent};
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{ClientTransport, HostTransport, TransportError};

/// Bumped when the wire format changes; packets of another version are ignored.
pub const PROTOCOL: u32 = 1;
/// The largest datagram sent.
pub const MAX_PACKET: usize = 60_000;
/// How long a message waits for its acknowledgement before it is sent again.
pub const RESEND_AFTER: Duration = Duration::from_millis(150);
/// The longest a side stays silent.
pub const KEEPALIVE: Duration = Duration::from_millis(500);
/// A side not heard from for this long is gone.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// How long the socket thread waits for a packet before it looks at its queues again.
const POLL: Duration = Duration::from_millis(2);

/// What a client sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToHost {
    /// Always the first message: who this client plays.
    Hello { player: PlayerId },
    Intent(PlayerIntent),
    /// The client is leaving.
    Bye,
}

/// What the host sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToClient {
    Delta(LedgerDelta),
    /// The host is closing.
    Bye,
}

/// A datagram: the sender's next expected sequence number and some of its messages.
#[derive(Debug, Deserialize)]
struct Packet<T> {
    protocol: u32,
    ack: u64,
    msgs: Vec<(u64, T)>,
}

/// A message waiting for its acknowledgement, already encoded.
#[derive(Debug)]
struct Outgoing {
    seq: u64,
    json: String,
    sent: Option<Instant>,
}

/// One side of a reliable, ordered channel over datagrams (see the module docs).
#[derive(Debug)]
pub struct Channel<In> {
    next_out: u64,
    unacked: VecDeque<Outgoing>,
    next_in: u64,
    early: BTreeMap<u64, In>,
    ack_due: bool,
    last_sent: Instant,
    last_heard: Instant,
}

/// The fixed part of a packet's encoding, with room for the numbers.
const PACKET_OVERHEAD: usize = 64;

impl<In: DeserializeOwned> Channel<In> {
    pub fn new(now: Instant) -> Self {
        Self { next_out: 0, unacked: VecDeque::new(), next_in: 0, early: BTreeMap::new(), ack_due: false, last_sent: now, last_heard: now }
    }

    /// Queues `msg`. Fails, queuing nothing, if it cannot fit in a packet.
    pub fn queue(&mut self, msg: &impl Serialize) -> Result<(), usize> {
        let json = serde_json::to_string(msg).expect("messages always serialise");
        if json.len() + PACKET_OVERHEAD > MAX_PACKET {
            return Err(json.len());
        }
        self.unacked.push_back(Outgoing { seq: self.next_out, json, sent: None });
        self.next_out += 1;
        Ok(())
    }

    /// Decodes a datagram; `None` if it is not a packet of this protocol.
    pub fn decode(data: &[u8]) -> Option<Vec<(u64, In)>> {
        Self::decode_packet(data).map(|p| p.msgs)
    }

    fn decode_packet(data: &[u8]) -> Option<Packet<In>> {
        serde_json::from_slice::<Packet<In>>(data).ok().filter(|p| p.protocol == PROTOCOL)
    }

    /// Takes a datagram from the other side. Returns the messages now deliverable, in order.
    pub fn receive(&mut self, data: &[u8], now: Instant) -> Vec<In> {
        let Some(packet) = Self::decode_packet(data) else { return Vec::new() };
        self.last_heard = now;
        while self.unacked.front().is_some_and(|o| o.seq < packet.ack) {
            self.unacked.pop_front();
        }
        for (seq, msg) in packet.msgs {
            self.ack_due = true;
            if seq >= self.next_in {
                self.early.entry(seq).or_insert(msg);
            }
        }
        let mut ready = Vec::new();
        while let Some(msg) = self.early.remove(&self.next_in) {
            ready.push(msg);
            self.next_in += 1;
        }
        ready
    }

    /// The datagrams to send now: messages never sent or due again, an acknowledgement owed,
    /// or a keepalive.
    pub fn packets(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let ack = self.next_in;
        let encode = |batch: &str| format!("{{\"protocol\":{PROTOCOL},\"ack\":{ack},\"msgs\":[{batch}]}}").into_bytes();
        let mut packets = Vec::new();
        let mut batch = String::new();
        for o in self.unacked.iter_mut().filter(|o| o.sent.is_none_or(|t| now.duration_since(t) >= RESEND_AFTER)) {
            let entry = format!("[{},{}]", o.seq, o.json);
            if !batch.is_empty() && batch.len() + entry.len() + 1 + PACKET_OVERHEAD > MAX_PACKET {
                packets.push(encode(&batch));
                batch.clear();
            }
            if !batch.is_empty() {
                batch.push(',');
            }
            batch.push_str(&entry);
            o.sent = Some(now);
        }
        if !batch.is_empty() || (packets.is_empty() && (self.ack_due || now.duration_since(self.last_sent) >= KEEPALIVE)) {
            packets.push(encode(&batch));
        }
        if !packets.is_empty() {
            self.last_sent = now;
            self.ack_due = false;
        }
        packets
    }

    /// Whether the other side has been silent for [`TIMEOUT`].
    pub fn timed_out(&self, now: Instant) -> bool {
        now.duration_since(self.last_heard) >= TIMEOUT
    }

    /// Messages sent and not yet acknowledged.
    pub fn unacknowledged(&self) -> usize {
        self.unacked.len()
    }
}

fn idle(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

/// Waits up to [`POLL`] for the first datagram, then takes whatever else has arrived.
fn receive_all(socket: &UdpSocket, buf: &mut [u8], mut each: impl FnMut(&[u8], SocketAddr)) {
    let _ = socket.set_nonblocking(false);
    let _ = socket.set_read_timeout(Some(POLL));
    loop {
        match socket.recv_from(buf) {
            Ok((n, from)) => each(&buf[..n], from),
            // A closed port on the other side can surface as a reset (Windows); keep going.
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
            Err(e) if idle(&e) => return,
            Err(e) => {
                log::warn!("udp receive failed: {e}");
                return;
            }
        }
        let _ = socket.set_nonblocking(true);
    }
}

/// The host's end: one socket for every client.
pub struct UdpHost {
    addr: SocketAddr,
    out: Option<Sender<(PlayerId, ToClient)>>,
    intents: Receiver<(PlayerId, PlayerIntent)>,
    players: Arc<RwLock<HashSet<PlayerId>>>,
    thread: Option<JoinHandle<()>>,
}

struct Peer {
    player: PlayerId,
    channel: Channel<ToHost>,
}

impl UdpHost {
    /// Listens on `addr` (port 0 picks a free one; see [`UdpHost::local_addr`]).
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        let addr = socket.local_addr()?;
        let (out_tx, out_rx) = unbounded();
        let (in_tx, in_rx) = unbounded();
        let players = Arc::new(RwLock::new(HashSet::new()));
        let shared = players.clone();
        let thread = std::thread::Builder::new().name("udp host".into()).spawn(move || host_loop(&socket, &out_rx, &in_tx, &shared))?;
        Ok(Self { addr, out: Some(out_tx), intents: in_rx, players, thread: Some(thread) })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Whether `player` is connected.
    pub fn has_player(&self, player: PlayerId) -> bool {
        self.players.read().unwrap().contains(&player)
    }
}

fn host_loop(socket: &UdpSocket, out: &Receiver<(PlayerId, ToClient)>, intents: &Sender<(PlayerId, PlayerIntent)>, players: &RwLock<HashSet<PlayerId>>) {
    let mut peers: HashMap<SocketAddr, Peer> = HashMap::new();
    let mut by_player: HashMap<PlayerId, SocketAddr> = HashMap::new();
    let mut buf = vec![0u8; 65_536];
    loop {
        let now = Instant::now();
        let mut leaving = Vec::new();
        receive_all(socket, &mut buf, |data, from| {
            if !peers.contains_key(&from) {
                // A new client: its packet must hold its Hello.
                let Some(msgs) = Channel::<ToHost>::decode(data) else { return };
                let Some(player) = msgs.iter().find_map(|(seq, m)| match m {
                    ToHost::Hello { player } if *seq == 0 => Some(*player),
                    _ => None,
                }) else {
                    return;
                };
                if let Some(old) = by_player.insert(player, from) {
                    peers.remove(&old);
                }
                peers.insert(from, Peer { player, channel: Channel::new(now) });
                players.write().unwrap().insert(player);
                log::info!("player {} joined from {from}", player.0);
            }
            let peer = peers.get_mut(&from).expect("added above");
            for msg in peer.channel.receive(data, now) {
                match msg {
                    ToHost::Hello { .. } => {}
                    ToHost::Intent(intent) => {
                        let _ = intents.send((peer.player, intent));
                    }
                    ToHost::Bye => leaving.push(from),
                }
            }
        });
        let now = Instant::now();
        loop {
            match out.try_recv() {
                Ok((player, msg)) => {
                    if let Some(peer) = by_player.get(&player).and_then(|a| peers.get_mut(a))
                        && let Err(size) = peer.channel.queue(&msg)
                    {
                        log::warn!("not sent to player {}: a {size}-byte message does not fit in a packet", player.0);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The handle is gone: say goodbye once and stop.
                    for (addr, peer) in &mut peers {
                        let _ = peer.channel.queue(&ToClient::Bye);
                        for packet in peer.channel.packets(now) {
                            let _ = socket.send_to(&packet, addr);
                        }
                    }
                    return;
                }
            }
        }
        for (addr, peer) in &mut peers {
            for packet in peer.channel.packets(now) {
                if let Err(e) = socket.send_to(&packet, addr) {
                    log::warn!("udp send to {addr} failed: {e}");
                }
            }
            if peer.channel.timed_out(now) {
                leaving.push(*addr);
            }
        }
        for addr in leaving {
            if let Some(peer) = peers.remove(&addr) {
                by_player.remove(&peer.player);
                players.write().unwrap().remove(&peer.player);
                log::info!("player {} left ({addr})", peer.player.0);
            }
        }
    }
}

impl Drop for UdpHost {
    fn drop(&mut self) {
        self.out.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl HostTransport for UdpHost {
    fn send_delta(&self, player: PlayerId, delta: LedgerDelta) -> Result<(), TransportError> {
        if !self.has_player(player) {
            return Err(TransportError::UnknownPlayer(player));
        }
        let out = self.out.as_ref().ok_or(TransportError::Disconnected)?;
        out.send((player, ToClient::Delta(delta))).map_err(|_| TransportError::Disconnected)
    }

    fn recv_intents(&self) -> Vec<(PlayerId, PlayerIntent)> {
        self.intents.try_iter().collect()
    }
}

/// A client's end, connected to one host.
pub struct UdpClient {
    player: PlayerId,
    host: SocketAddr,
    out: Option<Sender<ToHost>>,
    deltas: Receiver<LedgerDelta>,
    /// The host has answered.
    heard: Arc<AtomicBool>,
    /// The host has not said goodbye or gone silent.
    alive: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl UdpClient {
    /// Connects to the host at `host` as `player`. Nothing is waited for: the host is reached
    /// in the background ([`UdpClient::heard_from_host`]).
    pub fn connect(host: impl ToSocketAddrs, player: PlayerId) -> io::Result<Self> {
        let host = host.to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
        let local: SocketAddr = if host.is_ipv4() { (Ipv4Addr::UNSPECIFIED, 0).into() } else { (Ipv6Addr::UNSPECIFIED, 0).into() };
        let socket = UdpSocket::bind(local)?;
        let (out_tx, out_rx) = unbounded();
        let (in_tx, in_rx) = unbounded();
        out_tx.send(ToHost::Hello { player }).expect("the receiver is held above");
        let heard = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let (h, a) = (heard.clone(), alive.clone());
        let thread = std::thread::Builder::new().name("udp client".into()).spawn(move || client_loop(&socket, host, &out_rx, &in_tx, &h, &a))?;
        Ok(Self { player, host, out: Some(out_tx), deltas: in_rx, heard, alive, thread: Some(thread) })
    }

    pub fn host(&self) -> SocketAddr {
        self.host
    }

    /// Whether any packet has come back from the host.
    pub fn heard_from_host(&self) -> bool {
        self.heard.load(Ordering::Relaxed)
    }

    /// False once the host has said goodbye or been silent for [`TIMEOUT`].
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

fn client_loop(socket: &UdpSocket, host: SocketAddr, out: &Receiver<ToHost>, deltas: &Sender<LedgerDelta>, heard: &AtomicBool, alive: &AtomicBool) {
    let mut channel = Channel::<ToClient>::new(Instant::now());
    let mut buf = vec![0u8; 65_536];
    loop {
        let now = Instant::now();
        let mut bye = false;
        receive_all(socket, &mut buf, |data, from| {
            if from != host {
                return;
            }
            for msg in channel.receive(data, now) {
                heard.store(true, Ordering::Relaxed);
                match msg {
                    ToClient::Delta(delta) => {
                        let _ = deltas.send(delta);
                    }
                    ToClient::Bye => bye = true,
                }
            }
            heard.store(true, Ordering::Relaxed);
        });
        let now = Instant::now();
        let mut closing = false;
        loop {
            match out.try_recv() {
                Ok(msg) => {
                    if let Err(size) = channel.queue(&msg) {
                        log::warn!("not sent to the host: a {size}-byte message does not fit in a packet");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = channel.queue(&ToHost::Bye);
                    closing = true;
                    break;
                }
            }
        }
        for packet in channel.packets(now) {
            if let Err(e) = socket.send_to(&packet, host) {
                log::warn!("udp send to {host} failed: {e}");
            }
        }
        if bye || closing || channel.timed_out(now) {
            if bye {
                log::info!("the host at {host} closed the session");
            } else if !closing {
                log::warn!("the host at {host} stopped answering");
            }
            alive.store(false, Ordering::Relaxed);
            return;
        }
    }
}

impl Drop for UdpClient {
    fn drop(&mut self) {
        self.out.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl ClientTransport for UdpClient {
    fn player(&self) -> PlayerId {
        self.player
    }

    fn send_intent(&self, intent: PlayerIntent) -> Result<(), TransportError> {
        if !self.alive() {
            return Err(TransportError::Disconnected);
        }
        let out = self.out.as_ref().ok_or(TransportError::Disconnected)?;
        out.send(ToHost::Intent(intent)).map_err(|_| TransportError::Disconnected)
    }

    fn recv_deltas(&self) -> Vec<LedgerDelta> {
        self.deltas.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use arcanum_formats::map::TileLoc;
    use arcanum_sim::{CharacterRequest, NetId};

    use super::*;

    fn at(ms: u64, start: Instant) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// Delivers every packet from `from` to `to`, except those `drop` picks (by index).
    fn deliver<A: DeserializeOwned, B: DeserializeOwned>(from: &mut Channel<A>, to: &mut Channel<B>, now: Instant, drop: impl Fn(usize) -> bool) -> Vec<B> {
        from.packets(now).iter().enumerate().filter(|(i, _)| !drop(*i)).flat_map(|(_, p)| to.receive(p, now)).collect()
    }

    #[test]
    fn messages_arrive_once_in_order_through_loss_and_reordering() {
        let t0 = Instant::now();
        // The host side (receives ToHost) sends to the client side (receives ToClient).
        let mut a = Channel::<ToHost>::new(t0);
        let mut b = Channel::<ToClient>::new(t0);
        let msg = |n: u64| ToClient::Delta(LedgerDelta::Tick(n));
        for n in 0..3 {
            a.queue(&msg(n)).unwrap();
        }
        // The first packet is lost.
        assert!(deliver(&mut a, &mut b, at(0, t0), |_| true).is_empty());
        a.queue(&msg(3)).unwrap();
        // Only message 3 goes now (0-2 are not due again yet): held back, not delivered early.
        let pkts = a.packets(at(10, t0));
        assert_eq!(pkts.len(), 1);
        assert!(b.receive(&pkts[0], at(10, t0)).is_empty(), "3 waits for 0-2");
        // The resend fills the gap; 3 follows.
        let got = deliver(&mut a, &mut b, at(200, t0), |_| false);
        let got: Vec<_> = got.into_iter().map(|m| serde_json::to_string(&m).unwrap()).collect();
        let want: Vec<_> = (0..4).map(|n| serde_json::to_string(&msg(n)).unwrap()).collect();
        assert_eq!(got, want);
        // b acknowledges; a stops resending.
        assert_eq!(a.unacknowledged(), 4);
        for p in b.packets(at(201, t0)) {
            assert!(a.receive(&p, at(201, t0)).is_empty());
        }
        assert_eq!(a.unacknowledged(), 0);
        // A duplicate of an old packet delivers nothing.
        a.queue(&msg(4)).unwrap();
        let p = a.packets(at(300, t0)).remove(0);
        assert_eq!(b.receive(&p, at(300, t0)).len(), 1);
        assert!(b.receive(&p, at(301, t0)).is_empty(), "duplicate");
    }

    #[test]
    fn idle_sides_keep_alive_and_silent_ones_time_out() {
        let t0 = Instant::now();
        let mut a = Channel::<ToHost>::new(t0);
        let mut b = Channel::<ToHost>::new(t0);
        assert!(a.packets(at(100, t0)).is_empty(), "nothing to say yet");
        let keepalive = a.packets(t0 + KEEPALIVE);
        assert_eq!(keepalive.len(), 1);
        assert!(b.timed_out(t0 + TIMEOUT));
        assert!(b.receive(&keepalive[0], t0 + TIMEOUT).is_empty());
        assert!(!b.timed_out(t0 + TIMEOUT + Duration::from_secs(1)), "heard from");
        assert!(b.timed_out(t0 + TIMEOUT * 2));
    }

    #[test]
    fn big_batches_split_and_oversized_messages_are_refused() {
        let t0 = Instant::now();
        let mut a = Channel::<ToHost>::new(t0);
        let path: Vec<TileLoc> = (0..1000).map(|i| TileLoc { x: 100_000 + i, y: 200_000 + i }).collect();
        for seq in 0..5 {
            a.queue(&ToHost::Intent(PlayerIntent::Move { seq, path: path.clone() })).unwrap();
        }
        let packets = a.packets(t0);
        assert!(packets.len() > 1, "{} bytes do not fit in one", packets.iter().map(Vec::len).sum::<usize>());
        assert!(packets.iter().all(|p| p.len() <= MAX_PACKET));
        let mut b = Channel::<ToHost>::new(t0);
        let got: usize = packets.iter().map(|p| b.receive(p, t0).len()).sum();
        assert_eq!(got, 5);
        let huge: Vec<TileLoc> = (0..10_000).map(|i| TileLoc { x: 100_000 + i, y: 200_000 + i }).collect();
        assert!(a.queue(&ToHost::Intent(PlayerIntent::Move { seq: 9, path: huge })).is_err());
    }

    fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_client_and_host_talk_over_a_real_socket() {
        let host = UdpHost::bind("127.0.0.1:0").unwrap();
        let p = PlayerId(42);
        let client = UdpClient::connect(host.local_addr(), p).unwrap();
        client.send_intent(PlayerIntent::Character { seq: 1, request: CharacterRequest::New }).unwrap();
        let got = wait_for(|| Some(host.recv_intents()).filter(|v| !v.is_empty()));
        assert_eq!(got, vec![(p, PlayerIntent::Character { seq: 1, request: CharacterRequest::New })]);
        assert!(host.has_player(p));
        assert_eq!(host.send_delta(PlayerId(9), LedgerDelta::Tick(1)), Err(TransportError::UnknownPlayer(PlayerId(9))));
        let deltas: Vec<LedgerDelta> = (0..50).map(|n| LedgerDelta::Despawn { net_id: NetId(n) }).collect();
        for d in &deltas {
            host.send_delta(p, d.clone()).unwrap();
        }
        let mut got = Vec::new();
        wait_for(|| {
            got.extend(client.recv_deltas());
            (got.len() == deltas.len()).then_some(())
        });
        assert_eq!(got, deltas, "in order");
        assert!(client.heard_from_host());

        // The client leaves: the host forgets the player.
        drop(client);
        wait_for(|| (!host.has_player(p)).then_some(()));
    }

    #[test]
    fn a_closing_host_says_goodbye() {
        let host = UdpHost::bind("127.0.0.1:0").unwrap();
        let client = UdpClient::connect(host.local_addr(), PlayerId(1)).unwrap();
        wait_for(|| host.has_player(PlayerId(1)).then_some(()));
        drop(host);
        wait_for(|| (!client.alive()).then_some(()));
        assert_eq!(client.send_intent(PlayerIntent::Move { seq: 1, path: vec![] }), Err(TransportError::Disconnected));
    }
}
