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
use std::fmt;
use std::io::{ErrorKind, Write};
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

/// How many close records the relay keeps; the oldest is dropped first.
pub const CLOSE_LOG: usize = 64;

/// A request a client may not make in its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Misuse {
    /// The first message was not a create or a join.
    NotOpenOrJoin,
    /// A create or join from a connection that already holds a seat.
    RepeatedOpenOrJoin,
    /// A kick or an open/close request from a seat that is not the host.
    HostOnly,
}

/// Why the relay ended a client connection (one case per place in the relay
/// where a connection is dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseCause {
    /// A read failed with something other than a timeout.
    ReadFailed { kind: ErrorKind, os: Option<i32> },
    /// A length prefix of zero or over the maximum.
    BadLength(usize),
    /// An envelope that did not decode; the decoder's text.
    DecodeFailed(String),
    /// A request the client may not make.
    Misuse(Misuse),
    /// The relay refused the first message (room full, wrong password, ...).
    Refused(RefuseReason),
    /// No create or join within the handshake deadline.
    HandshakeTimeout,
    /// A half-sent envelope was not completed within its deadline.
    FrameTimeout,
    /// Nothing received within the idle deadline.
    IdleTimeout,
    /// Over the byte or frame rate for longer than the throttle limit.
    RateLimit,
    /// Writing to the client failed.
    WriteFailed { kind: ErrorKind, os: Option<i32> },
    /// Writing to the client timed out.
    WriteTimeout,
    /// The client ended its stream (EOF).
    ClientClosed,
    /// The client sent a leave request.
    ClientLeft,
    /// The host kicked this seat.
    Kicked,
    /// The client's outgoing queue was full: it could not keep up.
    TooSlow,
    /// The seat was removed from its room by another path.
    Removed,
    /// The relay was shutting down.
    RelayShutdown,
    /// Not admitted: the connection limits were reached.
    NotAdmitted,
    /// The socket could not be set up (options, clones, writer thread).
    SetupFailed,
}

impl CloseCause {
    /// Ends that are part of normal operation and not logged (a client
    /// that closes in the middle of an envelope is logged all the same).
    pub fn is_routine(&self) -> bool {
        matches!(
            self,
            CloseCause::ClientClosed | CloseCause::ClientLeft | CloseCause::RelayShutdown
        )
    }

    fn read_failed(e: &std::io::Error) -> CloseCause {
        CloseCause::ReadFailed {
            kind: e.kind(),
            os: e.raw_os_error(),
        }
    }

    fn write_failed(e: &std::io::Error) -> CloseCause {
        if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
            CloseCause::WriteTimeout
        } else {
            CloseCause::WriteFailed {
                kind: e.kind(),
                os: e.raw_os_error(),
            }
        }
    }

    /// Ends where the relay sends no refusal first.
    fn silent(&self) -> bool {
        matches!(
            self,
            CloseCause::HandshakeTimeout
                | CloseCause::FrameTimeout
                | CloseCause::IdleTimeout
                | CloseCause::RelayShutdown
        )
    }
}

impl fmt::Display for CloseCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let os = |os: &Option<i32>| os.map_or(String::new(), |c| format!(" (os error {c})"));
        match self {
            CloseCause::ReadFailed { kind, os: c } => write!(f, "read failed: {kind:?}{}", os(c)),
            CloseCause::BadLength(n) => write!(f, "bad length prefix {n}"),
            CloseCause::DecodeFailed(e) => write!(f, "envelope did not decode: {e}"),
            CloseCause::Misuse(m) => write!(f, "protocol misuse: {m:?}"),
            CloseCause::Refused(r) => write!(f, "refused: {r}"),
            CloseCause::HandshakeTimeout => f.write_str("handshake deadline"),
            CloseCause::FrameTimeout => f.write_str("half-sent envelope deadline"),
            CloseCause::IdleTimeout => f.write_str("idle deadline"),
            CloseCause::RateLimit => f.write_str("over the rate limit too long"),
            CloseCause::WriteFailed { kind, os: c } => write!(f, "write failed: {kind:?}{}", os(c)),
            CloseCause::WriteTimeout => f.write_str("write timed out"),
            CloseCause::ClientClosed => f.write_str("the client closed the stream"),
            CloseCause::ClientLeft => f.write_str("the client left"),
            CloseCause::Kicked => f.write_str("kicked by the host"),
            CloseCause::TooSlow => f.write_str("outgoing queue full, the client is too slow"),
            CloseCause::Removed => f.write_str("removed from its room"),
            CloseCause::RelayShutdown => f.write_str("the relay shut down"),
            CloseCause::NotAdmitted => f.write_str("connection limit reached"),
            CloseCause::SetupFailed => f.write_str("socket setup failed"),
        }
    }
}

/// One ended client connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseRecord {
    /// The connection's number (0 when it was not admitted).
    pub conn: u64,
    pub ip: IpAddr,
    /// The seat it held, once it had one.
    pub seat: Option<u8>,
    pub cause: CloseCause,
    /// How long the connection lived.
    pub lived: Duration,
    /// Time since the relay started when it ended.
    pub at: Duration,
    /// Bytes read but not yet parsed into an envelope when it ended.
    pub unparsed: usize,
    /// Reads that timed out over the connection's life.
    pub read_timeouts: u64,
    /// Reads that failed with the pending-I/O error (997) that counts as a
    /// timeout, on Windows.
    pub spurious_pending: u64,
}

impl fmt::Display for CloseRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "connection {} from {}", self.conn, self.ip)?;
        if let Some(s) = self.seat {
            write!(f, " (seat {s})")?;
        }
        write!(
            f,
            " closed at {} ms after {} ms: {}; {} bytes unparsed, {} read timeouts, {} pending-I/O errors",
            self.at.as_millis(),
            self.lived.as_millis(),
            self.cause,
            self.unparsed,
            self.read_timeouts,
            self.spurious_pending
        )
    }
}

/// The last [`CLOSE_LOG`] close records.
#[derive(Default)]
struct CloseLog(VecDeque<CloseRecord>);

impl CloseLog {
    fn push(&mut self, r: CloseRecord) {
        if self.0.len() >= CLOSE_LOG {
            self.0.pop_front();
        }
        self.0.push_back(r);
    }
}

/// Reads the relay's close records from another thread.
#[derive(Clone)]
pub struct CloseLogHandle(Arc<Shared>);

impl CloseLogHandle {
    /// The last [`CLOSE_LOG`] ended connections and why, oldest first.
    pub fn snapshot(&self) -> Vec<CloseRecord> {
        let log = self.0.closes.lock().unwrap_or_else(PoisonError::into_inner);
        log.0.iter().cloned().collect()
    }
}

/// The cause another thread (the host's kick, a full queue, the writer)
/// gave a connection's end; the first one wins.
type CauseSlot = Mutex<Option<CloseCause>>;

fn set_cause(slot: &CauseSlot, cause: CloseCause) {
    slot.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(cause);
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
    cause: Arc<CauseSlot>,
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
    started: Instant,
    closes: Mutex<CloseLog>,
}

impl Shared {
    /// Keeps and logs a close record. Never panics, whatever the locks'
    /// state or stderr's.
    fn record(&self, r: CloseRecord) {
        if !r.cause.is_routine() || (r.cause == CloseCause::ClientClosed && r.unparsed > 0) {
            let _ = writeln!(std::io::stderr(), "relay: {r}");
        }
        self.closes.lock().unwrap_or_else(PoisonError::into_inner).push(r);
    }
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
            started: Instant::now(),
            closes: Mutex::new(CloseLog::default()),
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

    /// The last [`CLOSE_LOG`] ended connections and why, oldest first.
    pub fn closes(&self) -> Vec<CloseRecord> {
        self.close_log().snapshot()
    }

    /// A handle on the close records that outlives borrowing the relay.
    pub fn close_log(&self) -> CloseLogHandle {
        CloseLogHandle(self.shared.clone())
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
fn depart(
    st: &mut State,
    cfg: &RelayConfig,
    code: RoomCode,
    seat: usize,
    conn: ConnId,
    why: Option<CloseCause>,
) {
    let mut work = vec![(code, seat, conn, why)];
    while let Some((code, seat, conn, why)) = work.pop() {
        let Some(room) = st.rooms.get_mut(&code) else {
            continue;
        };
        if room.members[seat].as_ref().is_none_or(|m| m.conn != conn) {
            continue;
        }
        let Some(gone) = room.members[seat].take() else {
            continue;
        };
        if let Some(why) = why {
            set_cause(&gone.cause, why);
        }
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
                work.push((code, i, m.conn, Some(CloseCause::TooSlow)));
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
        depart(st, cfg, code, i, c, Some(CloseCause::TooSlow));
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
    cause: &'a Arc<CauseSlot>,
}

fn open_room(st: &mut State, cfg: &RelayConfig, c: Conn<'_>, password: Vec<u8>) -> Joined {
    let Conn {
        ip,
        id: conn,
        tx,
        queued,
        sock,
        cause,
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
        cause: cause.clone(),
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
        cause,
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
        cause: cause.clone(),
    });
    Joined::Yes { code, seat }
}

fn writer(
    mut sock: TcpStream,
    rx: &Receiver<Out>,
    keepalive: Duration,
    queued: &AtomicUsize,
    cause: &CauseSlot,
) {
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
        if let Err(e) = sock.write_all(&bytes) {
            set_cause(cause, CloseCause::write_failed(&e));
            break;
        }
    }
    let _ = sock.shutdown(Shutdown::Both);
}

/// Reads until a whole envelope arrives, a deadline passes, or the relay
/// stops; the error says which.
fn read_msg(
    sock: &mut TcpStream,
    reader: &mut EnvelopeReader,
    cfg: &RelayConfig,
    shared: &Shared,
    deadline: Option<Instant>,
    last_rx: &mut Instant,
) -> Result<ClientMsg, CloseCause> {
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return Err(CloseCause::RelayShutdown);
        }
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            return Err(CloseCause::HandshakeTimeout);
        }
        if now.duration_since(*last_rx) > cfg.idle_timeout {
            return Err(CloseCause::IdleTimeout);
        }
        if reader.partial_age().is_some_and(|a| a > cfg.frame_timeout) {
            return Err(CloseCause::FrameTimeout);
        }
        match reader.poll(sock) {
            Ok(Polled::Payload(p)) => {
                *last_rx = Instant::now();
                return ClientMsg::decode(&p).map_err(|e| CloseCause::DecodeFailed(e.to_string()));
            }
            Ok(Polled::Idle) => {}
            Err(ReadError::Closed) => return Err(CloseCause::ClientClosed),
            Err(ReadError::Io(e)) => return Err(CloseCause::read_failed(&e)),
            Err(ReadError::BadLength(n)) => return Err(CloseCause::BadLength(n)),
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

/// What ended one admitted connection.
struct Ended {
    held: Option<(RoomCode, usize)>,
    seat: Option<usize>,
    cause: CloseCause,
    unparsed: usize,
    timeouts: u64,
    spurious: u64,
}

fn serve(stream: TcpStream, ip: IpAddr, shared: &Arc<Shared>) {
    let ip = limit_key(ip);
    let cfg = &shared.cfg;
    let born = Instant::now();
    let record = |conn: ConnId, e: Ended| {
        shared.record(CloseRecord {
            conn,
            ip,
            seat: e.seat.map(|s| s as u8),
            cause: e.cause,
            lived: born.elapsed(),
            at: shared.started.elapsed(),
            unparsed: e.unparsed,
            read_timeouts: e.timeouts,
            spurious_pending: e.spurious,
        });
    };
    let bare = |cause: CloseCause| Ended {
        held: None,
        seat: None,
        cause,
        unparsed: 0,
        timeouts: 0,
        spurious: 0,
    };
    let _ = stream.set_nodelay(true);
    // The listener is non-blocking (so `accept` can poll the stop flag); a
    // stream accepted from it inherits that on Windows and the BSDs, where
    // the read timeout below would then be ignored and every connection's
    // reader would spin instead of sleeping in the kernel.
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(POLL)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        record(0, bare(CloseCause::SetupFailed));
        return;
    }
    // Admission.
    let conn = {
        let mut st = lock(&shared.state);
        if st.connections >= cfg.max_connections
            || st.ips.get(&ip).is_some_and(|s| s.connections >= cfg.max_connections_per_ip)
        {
            drop(st);
            record(0, bare(CloseCause::NotAdmitted));
            return;
        }
        st.connections += 1;
        st.ips.entry(ip).or_default().connections += 1;
        st.next_conn += 1;
        st.next_conn
    };
    let ended = serve_admitted(stream, ip, conn, shared);
    let held = ended.held;
    record(conn, ended);
    let mut st = lock(&shared.state);
    if let Some((code, seat)) = held {
        depart(&mut st, cfg, code, seat, conn, None);
    }
    st.connections = st.connections.saturating_sub(1);
    if let Some(ipst) = st.ips.get_mut(&ip) {
        ipst.connections = ipst.connections.saturating_sub(1);
        if ipst.connections == 0 && ipst.rooms == 0 && ipst.failed.is_empty() {
            st.ips.remove(&ip);
        }
    }
}

/// Serves one admitted connection; says what ended it and the seat it still
/// holds, if any.
fn serve_admitted(stream: TcpStream, ip: IpAddr, conn: ConnId, shared: &Arc<Shared>) -> Ended {
    let cfg = &shared.cfg;
    let setup_failed = || Ended {
        held: None,
        seat: None,
        cause: CloseCause::SetupFailed,
        unparsed: 0,
        timeouts: 0,
        spurious: 0,
    };
    let Ok(mut sock) = stream.try_clone() else {
        return setup_failed();
    };
    let (tx, rx) = sync_channel::<Out>(cfg.outgoing_queue.max(8));
    let Ok(wsock) = stream.try_clone() else {
        return setup_failed();
    };
    let keepalive = (cfg.idle_timeout / 3).max(Duration::from_millis(50));
    let queued = Arc::new(AtomicUsize::new(0));
    let q = queued.clone();
    let cause: Arc<CauseSlot> = Arc::new(Mutex::new(None));
    let wc = cause.clone();
    let Ok(wthread) = thread::Builder::new()
        .name("relay-write".into())
        .spawn(move || writer(wsock, &rx, keepalive, &q, &wc))
    else {
        return setup_failed();
    };
    let mut reader = EnvelopeReader::new();
    let end = serve_loop(&mut sock, &mut reader, &stream, ip, conn, shared, &tx, &queued, &cause);
    // Whatever ended the loop, the writer must end too, and at once: nothing
    // is flushed to a departing seat, so a slow reader cannot delay its own
    // departure marker. The departure (if any) is made by the caller under
    // the lock. A cause another thread already gave (a kick, a full queue, a
    // failed write) is the real one; the loop's own is only what it saw of
    // it, and from here on it outranks anything the writer meets while
    // closing.
    set_cause(&cause, end.cause);
    let _ = tx.try_send(Out::Close);
    if end.held.is_some() {
        let _ = stream.shutdown(Shutdown::Both);
    }
    let _ = wthread.join();
    let cause = cause
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or(CloseCause::Removed);
    let (timeouts, spurious) = reader.read_timeouts();
    Ended {
        held: end.held,
        seat: end.seat,
        cause,
        unparsed: reader.buffered(),
        timeouts,
        spurious,
    }
}

struct LoopEnd {
    held: Option<(RoomCode, usize)>,
    seat: Option<usize>,
    cause: CloseCause,
}

#[allow(clippy::too_many_arguments)]
fn serve_loop(
    sock: &mut TcpStream,
    reader: &mut EnvelopeReader,
    stream: &TcpStream,
    ip: IpAddr,
    conn: ConnId,
    shared: &Arc<Shared>,
    tx: &SyncSender<Out>,
    queued: &Arc<AtomicUsize>,
    cause: &Arc<CauseSlot>,
) -> LoopEnd {
    let cfg = &shared.cfg;
    let c = Conn {
        ip,
        id: conn,
        tx,
        queued,
        sock: stream,
        cause,
    };
    let mut last_rx = Instant::now();
    // The writer delivers the refusal before it sees the close that follows.
    let refuse = |why: RefuseReason, cause: CloseCause| -> LoopEnd {
        let _ = send_to(tx, &ServerMsg::Refused(why));
        LoopEnd {
            held: None,
            seat: None,
            cause,
        }
    };

    // Handshake: the first message opens or joins a room, within a deadline.
    let first = read_msg(
        sock,
        reader,
        cfg,
        shared,
        Some(Instant::now() + cfg.handshake_timeout),
        &mut last_rx,
    );
    let (code, seat) = match first {
        Ok(ClientMsg::Create { password }) => {
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
                    return refuse(why, CloseCause::Refused(why));
                }
            }
        }
        Ok(ClientMsg::Join {
            code,
            reclaim,
            password,
        }) => {
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
                    return refuse(why, CloseCause::Refused(why));
                }
            }
        }
        Ok(_) => {
            return refuse(RefuseReason::BadRequest, CloseCause::Misuse(Misuse::NotOpenOrJoin));
        }
        Err(cause) if cause.silent() => {
            return LoopEnd {
                held: None,
                seat: None,
                cause,
            };
        }
        Err(cause) => return refuse(RefuseReason::BadRequest, cause),
    };

    let held = Some((code, seat));
    let end = |cause: CloseCause| LoopEnd {
        held,
        seat: Some(seat),
        cause,
    };
    let mut bucket = TokenBucket::new(cfg.bytes_per_sec, cfg.burst_bytes.max(70_000));
    let mut frames = TokenBucket::new(cfg.frames_per_sec, cfg.burst_frames.max(1));
    loop {
        let msg = match read_msg(sock, reader, cfg, shared, None, &mut last_rx) {
            Ok(m) => m,
            Err(cause) => return end(cause),
        };
        let size = match &msg {
            ClientMsg::Data { frame, .. } => frame.len() + 8,
            _ => 8,
        };
        // Over the byte rate: stop reading (TCP throttles the sender) until
        // the budget refills; cut off only if that goes on too long.
        let throttled = Instant::now();
        while !(bucket.take(size) && frames.take(1)) {
            if shared.stop.load(Ordering::SeqCst) {
                return end(CloseCause::RelayShutdown);
            }
            if throttled.elapsed() > cfg.throttle_limit {
                return end(CloseCause::RateLimit);
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
                    return LoopEnd {
                        held: None,
                        seat: Some(seat),
                        cause: CloseCause::Removed,
                    };
                }
            }
            ClientMsg::Keepalive => {}
            ClientMsg::Leave => return end(CloseCause::ClientLeft),
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
                    depart(&mut st, cfg, code, target, c, Some(CloseCause::Kicked));
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
            ClientMsg::Create { .. } | ClientMsg::Join { .. } => {
                return end(CloseCause::Misuse(Misuse::RepeatedOpenOrJoin));
            }
            ClientMsg::Kick(_) | ClientMsg::SetOpen(_) => {
                return end(CloseCause::Misuse(Misuse::HostOnly));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(conn: u64) -> CloseRecord {
        CloseRecord {
            conn,
            ip: IpAddr::from([127, 0, 0, 1]),
            seat: None,
            cause: CloseCause::ClientClosed,
            lived: Duration::ZERO,
            at: Duration::ZERO,
            unparsed: 0,
            read_timeouts: 0,
            spurious_pending: 0,
        }
    }

    #[test]
    fn the_close_log_drops_the_oldest() {
        let mut log = CloseLog::default();
        for i in 0..(CLOSE_LOG as u64 + 6) {
            log.push(rec(i));
        }
        assert_eq!(log.0.len(), CLOSE_LOG);
        assert_eq!(log.0.front().map(|r| r.conn), Some(6));
        assert_eq!(log.0.back().map(|r| r.conn), Some(CLOSE_LOG as u64 + 5));
    }

    #[test]
    fn a_poisoned_lock_neither_panics_the_relay_nor_loses_the_log() {
        let relay = Relay::bind("127.0.0.1:0", RelayConfig::default()).unwrap();
        let shared = relay.shared.clone();
        let _ = thread::spawn(move || {
            let _held = shared.closes.lock().unwrap();
            panic!("poison the close log");
        })
        .join();
        relay.shared.record(rec(1));
        assert_eq!(relay.closes().len(), 1);
    }

    #[test]
    fn a_cause_slot_keeps_the_first_cause() {
        let slot: CauseSlot = Mutex::new(None);
        set_cause(&slot, CloseCause::Kicked);
        set_cause(&slot, CloseCause::ClientClosed);
        assert_eq!(slot.lock().unwrap().take(), Some(CloseCause::Kicked));
    }
}
