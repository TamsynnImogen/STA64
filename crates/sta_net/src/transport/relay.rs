//! The relay (docs/NET.md, section 9): rooms with join codes, a star of TCP
//! connections, no peer needs to be reachable.
//!
//! The relay forwards [`proto`](crate::proto) frames and reads only their
//! headers. It stamps each forwarded frame with the sender's seat and host
//! flag as it knows them from the connection, delivers to the destination mask
//! only, and sends every member a departure marker when a seat's connection
//! closes. All forwarding and all departures happen under one lock, so for
//! every recipient a seat's frames precede its departure marker, which is what
//! the drop protocol of NET.md section 8 relies on.
//!
//! Standard library only: one accept thread, and a reader and a writer thread
//! per connection. Every peer is hostile: bounded buffers, a handshake
//! deadline, an idle deadline, a deadline for a half-sent envelope, a byte
//! rate per connection, and limits per address on connections, rooms and
//! failed joins. A client that cannot keep up (its outgoing queue is full) is
//! disconnected rather than buffered without bound.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::wire::{
    ClientMsg, EnvelopeReader, Polled, ReadError, RefuseReason, RoomCode, ServerMsg, TokenBucket,
};
use crate::proto::MAX_SEATS;

/// The relay's limits. [`RelayConfig::default`] is the production setting;
/// tests shorten the times.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Open connections, all addresses.
    pub max_connections: usize,
    /// Open connections from one address.
    pub max_connections_per_ip: usize,
    /// Live rooms, all addresses.
    pub max_rooms: usize,
    /// Live rooms opened from one address.
    pub max_rooms_per_ip: usize,
    /// A new connection must open or join a room within this long.
    pub handshake_timeout: Duration,
    /// A connection that sends nothing (not even a keepalive) for this long
    /// is closed (docs/NET.md, 3.1: 15 s).
    pub idle_timeout: Duration,
    /// A half-sent envelope must be completed within this long.
    pub frame_timeout: Duration,
    /// Bytes per second a connection may send (Refit's server uses 128 KB/s;
    /// lockstep needs well under 5 KB/s).
    pub bytes_per_sec: u32,
    /// Bytes a connection may save up; at least one frame.
    pub burst_bytes: u32,
    /// Frames per second a connection may send, and its burst (lockstep
    /// needs about 10 a second): bounds how fast one member can fill the
    /// others' queues.
    pub frames_per_sec: u32,
    pub burst_frames: u32,
    /// Failed joins from one address within `failed_join_window` before
    /// further joins from it are refused (room codes cannot be guessed at
    /// speed).
    pub failed_joins: usize,
    pub failed_join_window: Duration,
    /// Frames waiting to be written to one connection before it is judged
    /// too slow and closed.
    pub outgoing_queue: usize,
    /// Bytes waiting to be written to one connection before it is judged too
    /// slow and closed (a trickle reader must not pin 64 MiB).
    pub outgoing_bytes: usize,
    /// How long a connection over its byte rate is throttled (not read) before
    /// it is cut off.
    pub throttle_limit: Duration,
}

impl Default for RelayConfig {
    fn default() -> RelayConfig {
        RelayConfig {
            max_connections: 512,
            max_connections_per_ip: 16,
            max_rooms: 128,
            max_rooms_per_ip: 4,
            handshake_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(15),
            frame_timeout: Duration::from_secs(10),
            bytes_per_sec: 128 * 1024,
            burst_bytes: 256 * 1024,
            frames_per_sec: 200,
            burst_frames: 400,
            failed_joins: 10,
            failed_join_window: Duration::from_secs(60),
            outgoing_queue: 1024,
            outgoing_bytes: 1024 * 1024,
            throttle_limit: Duration::from_secs(5),
        }
    }
}

/// How long a blocked read waits before the loop looks at its deadlines.
const POLL: Duration = Duration::from_millis(50);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

type ConnId = u64;

enum Out {
    Bytes(Arc<Vec<u8>>),
    /// Flush nothing more: close the socket.
    Close,
}

struct Member {
    conn: ConnId,
    tx: SyncSender<Out>,
    sock: TcpStream,
    /// Bytes queued for this member's writer, not yet written.
    queued: Arc<AtomicUsize>,
}

struct Room {
    password: Vec<u8>,
    members: [Option<Member>; MAX_SEATS],
    /// New players may join (the host's flag).
    open: bool,
    /// Seats whose connection ended while the room was closed to joins; only
    /// a rejoin may take them.
    vacated: u8,
    host_gone: bool,
    owner: IpAddr,
}

#[derive(Default)]
struct IpState {
    connections: usize,
    rooms: usize,
    failed: VecDeque<Instant>,
}

#[derive(Default)]
struct State {
    rooms: BTreeMap<RoomCode, Room>,
    ips: BTreeMap<IpAddr, IpState>,
    connections: usize,
    next_conn: ConnId,
}

struct Shared {
    cfg: RelayConfig,
    state: Mutex<State>,
    stop: AtomicBool,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A running relay.
pub struct Relay {
    shared: Arc<Shared>,
    addr: SocketAddr,
    accept: Option<JoinHandle<()>>,
}

impl Relay {
    /// Listens on `addr` (port 0 for an ephemeral one) and serves until
    /// [`Relay::shutdown`] or drop.
    pub fn bind(addr: impl ToSocketAddrs, cfg: RelayConfig) -> std::io::Result<Relay> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            cfg,
            state: Mutex::new(State::default()),
            stop: AtomicBool::new(false),
        });
        let s = shared.clone();
        let accept = thread::Builder::new()
            .name("relay-accept".into())
            .spawn(move || accept_loop(&listener, &s))?;
        Ok(Relay {
            shared,
            addr,
            accept: Some(accept),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Live rooms.
    pub fn rooms(&self) -> usize {
        lock(&self.shared.state).rooms.len()
    }

    /// Open connections.
    pub fn connections(&self) -> usize {
        lock(&self.shared.state).connections
    }

    /// Stops accepting and closes every connection; connection threads end
    /// within one poll interval.
    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Forgets addresses with nothing live and no failure inside the window, so
/// the table cannot grow with the number of addresses that ever connected.
fn prune(st: &mut State, cfg: &RelayConfig) {
    let now = Instant::now();
    st.ips.retain(|_, ip| {
        while ip
            .failed
            .front()
            .is_some_and(|t| now.duration_since(*t) > cfg.failed_join_window)
        {
            ip.failed.pop_front();
        }
        ip.connections > 0 || ip.rooms > 0 || !ip.failed.is_empty()
    });
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    let mut last_prune = Instant::now();
    while !shared.stop.load(Ordering::SeqCst) {
        if last_prune.elapsed() > Duration::from_secs(5) {
            last_prune = Instant::now();
            prune(&mut lock(&shared.state), &shared.cfg);
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let s = shared.clone();
                let spawned = thread::Builder::new()
                    .name("relay-conn".into())
                    .spawn(move || serve(stream, peer.ip(), &s));
                // No thread to spare: the stream was moved into the closure
                // and is dropped with it.
                drop(spawned);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL / 5),
            Err(_) => thread::sleep(POLL),
        }
    }
}

fn send_to(tx: &SyncSender<Out>, msg: &ServerMsg) -> bool {
    match msg.encode() {
        Ok(b) => !matches!(tx.try_send(Out::Bytes(Arc::new(b))), Err(TrySendError::Full(_))),
        Err(_) => true,
    }
}

/// Queues `bytes` for a member unless its frame or byte budget is spent.
fn push(m: &Member, bytes: &Arc<Vec<u8>>, max_bytes: usize) -> bool {
    if m.queued.load(Ordering::SeqCst) + bytes.len() > max_bytes {
        return false;
    }
    m.queued.fetch_add(bytes.len(), Ordering::SeqCst);
    match m.tx.try_send(Out::Bytes(bytes.clone())) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            m.queued.fetch_sub(bytes.len(), Ordering::SeqCst);
            false
        }
        Err(TrySendError::Disconnected(_)) => {
            m.queued.fetch_sub(bytes.len(), Ordering::SeqCst);
            true
        }
    }
}

impl Room {
    fn occupied(&self) -> u8 {
        let mut m = 0;
        for (i, s) in self.members.iter().enumerate() {
            if s.is_some() {
                m |= 1 << i;
            }
        }
        m
    }
}

/// Removes `seat` of `code` (if `conn` still holds it), tells the others,
/// and closes its socket. Members whose queue overflows while being told are
/// removed in turn. Called with the state locked.
fn depart(st: &mut State, cfg: &RelayConfig, code: RoomCode, seat: usize, conn: ConnId) {
    let mut work = vec![(code, seat, conn)];
    while let Some((code, seat, conn)) = work.pop() {
        let Some(room) = st.rooms.get_mut(&code) else {
            continue;
        };
        if room.members[seat].as_ref().is_none_or(|m| m.conn != conn) {
            continue;
        }
        let Some(gone) = room.members[seat].take() else {
            continue;
        };
        let _ = gone.tx.try_send(Out::Close);
        let _ = gone.sock.shutdown(Shutdown::Both);
        if seat == 0 {
            room.host_gone = true;
            room.open = false;
        } else if room.open {
            room.vacated &= !(1 << seat);
        } else {
            room.vacated |= 1 << seat;
        }
        let marker = Arc::new(ServerMsg::Departed(seat as u8).encode().unwrap_or_default());
        for (i, m) in room.members.iter().enumerate() {
            if let Some(m) = m
                && !push(m, &marker, cfg.outgoing_bytes + 64)
            {
                work.push((code, i, m.conn));
            }
        }
        if room.occupied() == 0 {
            let owner = room.owner;
            st.rooms.remove(&code);
            if let Some(ip) = st.ips.get_mut(&owner) {
                ip.rooms = ip.rooms.saturating_sub(1);
                if ip.connections == 0 && ip.rooms == 0 && ip.failed.is_empty() {
                    st.ips.remove(&owner);
                }
            }
        }
    }
}

/// Forwards one frame from `seat`. Recipients that cannot keep up are
/// disconnected.
fn forward(st: &mut State, cfg: &RelayConfig, code: RoomCode, seat: usize, conn: ConnId, mask: u8, frame: Vec<u8>) {
    let Some(room) = st.rooms.get(&code) else {
        return;
    };
    if room.members[seat].as_ref().is_none_or(|m| m.conn != conn) {
        return; // kicked while this frame was in flight: nothing more from it
    }
    let Ok(bytes) = (ServerMsg::Data {
        from: seat as u8,
        from_host: seat == 0,
        frame,
    })
    .encode() else {
        return;
    };
    let bytes = Arc::new(bytes);
    let mut slow = Vec::new();
    for (i, m) in room.members.iter().enumerate() {
        if i == seat || mask & (1 << i) == 0 {
            continue;
        }
        if let Some(m) = m
            && !push(m, &bytes, cfg.outgoing_bytes)
        {
            slow.push((i, m.conn));
        }
    }
    for (i, c) in slow {
        depart(st, cfg, code, i, c);
    }
}

enum Joined {
    Yes { code: RoomCode, seat: usize },
    No(RefuseReason),
}

fn same(a: &[u8], b: &[u8]) -> bool {
    // Constant time in the contents, not in the lengths.
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What the relay knows of the connection that opens or joins a room.
#[derive(Clone, Copy)]
struct Conn<'a> {
    ip: IpAddr,
    id: ConnId,
    tx: &'a SyncSender<Out>,
    queued: &'a Arc<AtomicUsize>,
    sock: &'a TcpStream,
}

fn open_room(st: &mut State, cfg: &RelayConfig, c: Conn<'_>, password: Vec<u8>) -> Joined {
    let Conn {
        ip,
        id: conn,
        tx,
        queued,
        sock,
    } = c;
    let rooms_of_ip = st.ips.get(&ip).map_or(0, |s| s.rooms);
    if st.rooms.len() >= cfg.max_rooms || rooms_of_ip >= cfg.max_rooms_per_ip {
        return Joined::No(RefuseReason::Busy);
    }
    let mut code = None;
    for _ in 0..8 {
        match RoomCode::random() {
            Ok(c) if !st.rooms.contains_key(&c) => {
                code = Some(c);
                break;
            }
            Ok(_) => {}
            Err(_) => return Joined::No(RefuseReason::Busy),
        }
    }
    let Some(code) = code else {
        return Joined::No(RefuseReason::Busy);
    };
    let Ok(sock) = sock.try_clone() else {
        return Joined::No(RefuseReason::Busy);
    };
    let mut members: [Option<Member>; MAX_SEATS] = Default::default();
    members[0] = Some(Member {
        conn,
        tx: tx.clone(),
        sock,
        queued: queued.clone(),
    });
    st.rooms.insert(
        code,
        Room {
            password,
            members,
            open: true,
            vacated: 0,
            host_gone: false,
            owner: ip,
        },
    );
    st.ips.entry(ip).or_default().rooms += 1;
    Joined::Yes { code, seat: 0 }
}

fn join_room(
    st: &mut State,
    cfg: &RelayConfig,
    c: Conn<'_>,
    code: RoomCode,
    reclaim: Option<u8>,
    password: &[u8],
) -> Joined {
    let Conn {
        ip,
        id: conn,
        tx,
        queued,
        sock,
    } = c;
    let now = Instant::now();
    let ipst = st.ips.entry(ip).or_default();
    while ipst
        .failed
        .front()
        .is_some_and(|t| now.duration_since(*t) > cfg.failed_join_window)
    {
        ipst.failed.pop_front();
    }
    if ipst.failed.len() >= cfg.failed_joins {
        return Joined::No(RefuseReason::RateLimited);
    }
    let fail = |st: &mut State, why: RefuseReason| {
        let f = &mut st.ips.entry(ip).or_default().failed;
        if f.len() < 4 * cfg.failed_joins.max(1) {
            f.push_back(now);
        }
        Joined::No(why)
    };
    let Some(room) = st.rooms.get_mut(&code) else {
        return fail(st, RefuseReason::NoRoom);
    };
    if !same(&room.password, password) {
        return fail(st, RefuseReason::BadPassword);
    }
    if room.host_gone {
        return Joined::No(RefuseReason::Closed);
    }
    let seat = match reclaim {
        Some(s) => {
            let s = usize::from(s);
            if room.vacated & (1 << s) == 0 || room.members[s].is_some() {
                return Joined::No(RefuseReason::BadReclaim);
            }
            s
        }
        None => {
            if !room.open {
                return Joined::No(RefuseReason::Closed);
            }
            match (1..MAX_SEATS).find(|&i| room.members[i].is_none()) {
                Some(s) => s,
                None => return Joined::No(RefuseReason::Full),
            }
        }
    };
    let Ok(sock) = sock.try_clone() else {
        return Joined::No(RefuseReason::Busy);
    };
    room.vacated &= !(1 << seat);
    room.members[seat] = Some(Member {
        conn,
        tx: tx.clone(),
        sock,
        queued: queued.clone(),
    });
    Joined::Yes { code, seat }
}

fn writer(mut sock: TcpStream, rx: &Receiver<Out>, keepalive: Duration, queued: &AtomicUsize) {
    let ka = ServerMsg::Keepalive.encode().unwrap_or_default();
    loop {
        let bytes = match rx.recv_timeout(keepalive) {
            Ok(Out::Bytes(b)) => b,
            Ok(Out::Close) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => Arc::new(ka.clone()),
        };
        let n = bytes.len();
        // Saturating: replies sent outside `push` are not counted.
        let mut cur = queued.load(Ordering::SeqCst);
        while let Err(now) = queued.compare_exchange(
            cur,
            cur.saturating_sub(n),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            cur = now;
        }
        if sock.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = sock.shutdown(Shutdown::Both);
}

/// Reads until a whole envelope arrives, a deadline passes, or the relay
/// stops. `Ok(None)` is a timeout or a stop; the caller closes.
fn read_msg(
    sock: &mut TcpStream,
    reader: &mut EnvelopeReader,
    cfg: &RelayConfig,
    shared: &Shared,
    deadline: Option<Instant>,
    last_rx: &mut Instant,
) -> Result<Option<ClientMsg>, ()> {
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d)
            || now.duration_since(*last_rx) > cfg.idle_timeout
            || reader.partial_age().is_some_and(|a| a > cfg.frame_timeout)
        {
            return Ok(None);
        }
        match reader.poll(sock) {
            Ok(Polled::Payload(p)) => {
                *last_rx = Instant::now();
                return ClientMsg::decode(&p).map(Some).map_err(|_| ());
            }
            Ok(Polled::Idle) => {}
            Err(ReadError::Closed | ReadError::Io(_) | ReadError::BadLength(_)) => return Err(()),
        }
    }
}

/// The limit key: an IPv6 address stands for its /64 (one subscriber can
/// hold billions of addresses), an IPv4 address for itself.
fn limit_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            let mut o = v6.octets();
            o[8..].fill(0);
            IpAddr::V6(o.into())
        }
        v4 => v4,
    }
}

fn serve(stream: TcpStream, ip: IpAddr, shared: &Arc<Shared>) {
    let ip = limit_key(ip);
    let cfg = &shared.cfg;
    let _ = stream.set_nodelay(true);
    // The listener is non-blocking (so `accept` can poll the stop flag); a
    // stream accepted from it inherits that on Windows and the BSDs, where
    // the read timeout below would then be ignored and every connection's
    // reader would spin instead of sleeping in the kernel.
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(POLL)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    // Admission.
    let conn = {
        let mut st = lock(&shared.state);
        if st.connections >= cfg.max_connections
            || st.ips.get(&ip).is_some_and(|s| s.connections >= cfg.max_connections_per_ip)
        {
            return;
        }
        st.connections += 1;
        st.ips.entry(ip).or_default().connections += 1;
        st.next_conn += 1;
        st.next_conn
    };
    let result = serve_admitted(stream, ip, conn, shared);
    let mut st = lock(&shared.state);
    if let Some((code, seat)) = result {
        depart(&mut st, cfg, code, seat, conn);
    }
    st.connections = st.connections.saturating_sub(1);
    if let Some(ipst) = st.ips.get_mut(&ip) {
        ipst.connections = ipst.connections.saturating_sub(1);
        if ipst.connections == 0 && ipst.rooms == 0 && ipst.failed.is_empty() {
            st.ips.remove(&ip);
        }
    }
}

/// Serves one admitted connection; returns the seat it still holds, if any.
fn serve_admitted(
    stream: TcpStream,
    ip: IpAddr,
    conn: ConnId,
    shared: &Arc<Shared>,
) -> Option<(RoomCode, usize)> {
    let cfg = &shared.cfg;
    let mut sock = stream.try_clone().ok()?;
    let (tx, rx) = sync_channel::<Out>(cfg.outgoing_queue.max(8));
    let wsock = stream.try_clone().ok()?;
    let keepalive = (cfg.idle_timeout / 3).max(Duration::from_millis(50));
    let queued = Arc::new(AtomicUsize::new(0));
    let q = queued.clone();
    let wthread = thread::Builder::new()
        .name("relay-write".into())
        .spawn(move || writer(wsock, &rx, keepalive, &q))
        .ok()?;
    let held = serve_loop(&mut sock, &stream, ip, conn, shared, &tx, &queued);
    // Whatever ended the loop, the writer must end too, and at once: nothing
    // is flushed to a departing seat, so a slow reader cannot delay its own
    // departure marker. The departure (if any) is made by the caller under
    // the lock.
    let _ = tx.try_send(Out::Close);
    if held.is_some() {
        let _ = stream.shutdown(Shutdown::Both);
    }
    let _ = wthread.join();
    held
}

fn serve_loop(
    sock: &mut TcpStream,
    stream: &TcpStream,
    ip: IpAddr,
    conn: ConnId,
    shared: &Arc<Shared>,
    tx: &SyncSender<Out>,
    queued: &Arc<AtomicUsize>,
) -> Option<(RoomCode, usize)> {
    let cfg = &shared.cfg;
    let c = Conn {
        ip,
        id: conn,
        tx,
        queued,
        sock: stream,
    };
    let mut reader = EnvelopeReader::new();
    let mut last_rx = Instant::now();
    // The writer delivers the refusal before it sees the close that follows.
    let refuse = |why: RefuseReason| -> Option<(RoomCode, usize)> {
        let _ = send_to(tx, &ServerMsg::Refused(why));
        None
    };

    // Handshake: the first message opens or joins a room, within a deadline.
    let first = read_msg(
        sock,
        &mut reader,
        cfg,
        shared,
        Some(Instant::now() + cfg.handshake_timeout),
        &mut last_rx,
    );
    let (code, seat) = match first {
        Ok(Some(ClientMsg::Create { password })) => {
            let mut st = lock(&shared.state);
            match open_room(&mut st, cfg, c, password) {
                Joined::Yes { code, seat } => {
                    drop(st);
                    let _ = send_to(
                        tx,
                        &ServerMsg::Created {
                            code,
                            seat: seat as u8,
                        },
                    );
                    (code, seat)
                }
                Joined::No(why) => {
                    drop(st);
                    return refuse(why);
                }
            }
        }
        Ok(Some(ClientMsg::Join {
            code,
            reclaim,
            password,
        })) => {
            let mut st = lock(&shared.state);
            match join_room(&mut st, cfg, c, code, reclaim, &password) {
                Joined::Yes { code, seat } => {
                    let members = st.rooms.get(&code).map_or(0, Room::occupied);
                    // Under the lock, so the Joined reply precedes any
                    // frame another member forwards to this one.
                    let _ = send_to(
                        tx,
                        &ServerMsg::Joined {
                            seat: seat as u8,
                            host: 0,
                            members,
                        },
                    );
                    drop(st);
                    (code, seat)
                }
                Joined::No(why) => {
                    drop(st);
                    return refuse(why);
                }
            }
        }
        Ok(Some(_)) | Err(()) => return refuse(RefuseReason::BadRequest),
        Ok(None) => return None,
    };

    let mut bucket = TokenBucket::new(cfg.bytes_per_sec, cfg.burst_bytes.max(70_000));
    let mut frames = TokenBucket::new(cfg.frames_per_sec, cfg.burst_frames.max(1));
    loop {
        let msg = match read_msg(sock, &mut reader, cfg, shared, None, &mut last_rx) {
            Ok(Some(m)) => m,
            Ok(None) | Err(()) => return Some((code, seat)),
        };
        let size = match &msg {
            ClientMsg::Data { frame, .. } => frame.len() + 8,
            _ => 8,
        };
        // Over the byte rate: stop reading (TCP throttles the sender) until
        // the budget refills; cut off only if that goes on too long.
        let throttled = Instant::now();
        while !(bucket.take(size) && frames.take(1)) {
            if throttled.elapsed() > cfg.throttle_limit || shared.stop.load(Ordering::SeqCst) {
                return Some((code, seat));
            }
            thread::sleep(Duration::from_millis(10));
        }
        last_rx = Instant::now();
        match msg {
            ClientMsg::Data { mask, frame } => {
                let mut st = lock(&shared.state);
                forward(&mut st, cfg, code, seat, conn, mask, frame);
                // Still a member? A kick removed us under the same lock.
                if st
                    .rooms
                    .get(&code)
                    .is_none_or(|r| r.members[seat].as_ref().is_none_or(|m| m.conn != conn))
                {
                    return None;
                }
            }
            ClientMsg::Keepalive => {}
            ClientMsg::Leave => return Some((code, seat)),
            ClientMsg::Kick(target) if seat == 0 => {
                let mut st = lock(&shared.state);
                let target = usize::from(target);
                if target != 0
                    && let Some(c) = st
                        .rooms
                        .get(&code)
                        .and_then(|r| r.members[target].as_ref())
                        .map(|m| m.conn)
                {
                    depart(&mut st, cfg, code, target, c);
                }
            }
            ClientMsg::SetOpen(open) if seat == 0 => {
                let mut st = lock(&shared.state);
                if let Some(room) = st.rooms.get_mut(&code) {
                    room.open = open;
                    if open {
                        room.vacated = 0;
                    }
                }
            }
            // A second Create or Join, or a host-only request from a
            // non-host: a broken or hostile client.
            _ => return Some((code, seat)),
        }
    }
}
