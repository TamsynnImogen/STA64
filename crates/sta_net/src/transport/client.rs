//! The client side of the relay: [`NetTransport`], the real
//! [`Transport`](crate::lockstep::Transport) (docs/NET.md, sections 1 and 9).
//!
//! One TCP connection to the relay, a reader thread and a writer thread. The
//! game thread never blocks: `send` queues, `poll` pops. Both queues are
//! bounded; a client that falls behind is closed with a reason rather than
//! allowed to grow without bound. What the relay sends is checked as
//! carefully as what a peer sends.
//!
//! The router sees only the [`Transport`] trait, so the things a lobby and a
//! stall dialog need (the room code, kicking a seat, closing the room to new
//! players, departure markers, why the connection ended) are on a cloneable
//! [`NetControl`] taken before the transport moves into a `Peer`.

use std::collections::VecDeque;
use std::io::Write;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use sta_core::PlayerId;

use super::wire::{
    ClientMsg, EnvelopeReader, Polled, ReadError, RefuseReason, RoomCode, ServerMsg, WireError,
    ALL_SEATS,
};
use crate::lockstep::{Destination, Incoming, Transport, TransportError};
use crate::proto::{self, MAX_SEATS};

/// Timeouts and bounds of a client connection.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub connect_timeout: Duration,
    /// The relay must answer `Create`/`Join` within this long.
    pub handshake_timeout: Duration,
    /// The relay must send something (it sends keepalives) within this long,
    /// or the connection is lost (docs/NET.md, 3.1: 15 s).
    pub idle_timeout: Duration,
    /// A half-sent envelope must be completed within this long.
    pub frame_timeout: Duration,
    /// Received frames not yet polled, in bytes, before the connection is
    /// closed as overflowing.
    pub max_inbox_bytes: usize,
    /// Frames waiting to be written.
    pub outgoing_queue: usize,
}

impl Default for ClientOptions {
    fn default() -> ClientOptions {
        ClientOptions {
            connect_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(15),
            frame_timeout: Duration::from_secs(10),
            max_inbox_bytes: 4 * 1024 * 1024,
            outgoing_queue: 1024,
        }
    }
}

/// Why opening or joining a room failed.
#[derive(Debug)]
pub enum ConnectError {
    /// No connection, or it failed during the handshake.
    Io(std::io::Error),
    /// The relay said no.
    Refused(RefuseReason),
    /// The relay did not answer in time.
    Timeout,
    /// The relay's answer was not valid.
    Protocol(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Io(e) => write!(f, "cannot reach the relay: {e}"),
            ConnectError::Refused(r) => write!(f, "the relay refused: {r}"),
            ConnectError::Timeout => write!(f, "the relay did not answer"),
            ConnectError::Protocol(e) => write!(f, "bad answer from the relay: {e}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// This side left or closed on purpose.
    Left,
    /// The relay closed the connection (kicked, or the relay stopped).
    RelayClosed,
    /// Nothing arrived for the idle timeout: the connection is lost.
    TimedOut,
    /// A socket error.
    Io(String),
    /// The relay sent something invalid.
    Protocol(String),
    /// The game thread did not poll fast enough.
    InboxFull,
    /// The relay did not take what was sent fast enough.
    SendQueueFull,
}

enum Item {
    Data { from: PlayerId, frame: Vec<u8> },
    Departed(PlayerId),
}

#[derive(Default)]
struct Inbox {
    queue: VecDeque<Item>,
    bytes: usize,
    /// A bit per seat whose marker was reached and not yet taken.
    departures: u8,
    closed: Option<CloseReason>,
}

enum Out {
    Bytes(Vec<u8>),
    /// Send `Leave` (if graceful), then close the socket.
    Close { graceful: bool },
}

struct Shared {
    inbox: Mutex<Inbox>,
    stop: AtomicBool,
    sock: TcpStream,
    out: SyncSender<Out>,
}

impl Shared {
    fn inbox(&self) -> MutexGuard<'_, Inbox> {
        self.inbox.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends the connection; the first reason wins.
    fn close(&self, reason: CloseReason, graceful: bool) {
        {
            let mut inbox = self.inbox();
            if inbox.closed.is_none() {
                inbox.closed = Some(reason);
            }
        }
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.out.try_send(Out::Close { graceful });
        if !graceful {
            let _ = self.sock.shutdown(Shutdown::Both);
        }
    }

    fn queue(&self, msg: &ClientMsg) -> Result<(), TransportError> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(TransportError::Closed);
        }
        let bytes = msg.encode().map_err(|e| match e {
            WireError::BadFrame => TransportError::Frame(proto::ProtoError::Invalid("frame")),
            _ => TransportError::Closed,
        })?;
        match self.out.try_send(Out::Bytes(bytes)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.close(CloseReason::SendQueueFull, false);
                Err(TransportError::Closed)
            }
            Err(TrySendError::Disconnected(_)) => Err(TransportError::Closed),
        }
    }
}

/// What a lobby and a stall dialog need beyond the [`Transport`] trait.
/// Cheap to clone; valid while any handle lives, and inert once the
/// connection has closed.
#[derive(Clone)]
pub struct NetControl {
    shared: Arc<Shared>,
    me: PlayerId,
    host: PlayerId,
    code: RoomCode,
}

impl NetControl {
    /// This client's seat. The relay's seat numbers are the game's
    /// `PlayerId`s: the host is seat 0 and joiners take the lowest free seat.
    pub fn seat(&self) -> PlayerId {
        self.me
    }

    pub fn host_seat(&self) -> PlayerId {
        self.host
    }

    pub fn is_host(&self) -> bool {
        self.me == self.host
    }

    /// The code others join with.
    pub fn room_code(&self) -> RoomCode {
        self.code
    }

    /// Host only: has the relay close `seat`'s connection. Every member, the
    /// host included, then reads a departure marker for it after the last
    /// frame it sent (docs/NET.md, section 8).
    pub fn kick(&self, seat: PlayerId) -> Result<(), TransportError> {
        self.shared.queue(&ClientMsg::Kick(seat.0))
    }

    /// Host only: whether new players may join. Close it when the game
    /// starts; a closed room keeps vacated seats for [`NetTransport::rejoin`].
    pub fn set_open(&self, open: bool) -> Result<(), TransportError> {
        self.shared.queue(&ClientMsg::SetOpen(open))
    }

    /// Seats whose departure marker has been reached. A marker is reached
    /// when [`Transport::poll`] has returned every frame the seat sent
    /// before it, so a seat listed here has no frame still to come.
    pub fn take_departures(&self) -> Vec<PlayerId> {
        let mask = std::mem::take(&mut self.shared.inbox().departures);
        (0..MAX_SEATS as u8).filter(|s| mask & (1 << s) != 0).map(PlayerId).collect()
    }

    /// Why the connection ended, once it has.
    pub fn close_reason(&self) -> Option<CloseReason> {
        self.shared.inbox().closed.clone()
    }

    /// Closes the connection without telling the relay why, as a crash or a
    /// cable pull would.
    pub fn abort(&self) {
        self.shared.close(CloseReason::Left, false);
    }

    /// Leaves the room.
    pub fn leave(&self) {
        self.shared.close(CloseReason::Left, true);
    }
}

/// The real transport: [`Transport`] over a relay connection.
pub struct NetTransport {
    control: NetControl,
}

impl NetTransport {
    /// Opens a room on the relay at `addr`; this client is its host (seat 0).
    pub fn create(
        addr: impl ToSocketAddrs,
        password: &str,
        opts: &ClientOptions,
    ) -> Result<NetTransport, ConnectError> {
        let msg = ClientMsg::Create {
            password: password.as_bytes().to_vec(),
        };
        connect(addr, &msg, opts)
    }

    /// Joins the room `code` in the lowest free seat.
    pub fn join(
        addr: impl ToSocketAddrs,
        code: RoomCode,
        password: &str,
        opts: &ClientOptions,
    ) -> Result<NetTransport, ConnectError> {
        let msg = ClientMsg::Join {
            code,
            reclaim: None,
            password: password.as_bytes().to_vec(),
        };
        connect(addr, &msg, opts)
    }

    /// Takes `seat` back after a lost connection (docs/NET.md, section 8).
    /// The relay lets a new connection into a seat only if its old one has
    /// closed in a room that was closed to new players; the host decides
    /// whether this client really is that seat (the `Rejoin` token) and can
    /// `kick` it if not.
    pub fn rejoin(
        addr: impl ToSocketAddrs,
        code: RoomCode,
        password: &str,
        seat: PlayerId,
        opts: &ClientOptions,
    ) -> Result<NetTransport, ConnectError> {
        let msg = ClientMsg::Join {
            code,
            reclaim: Some(seat.0),
            password: password.as_bytes().to_vec(),
        };
        connect(addr, &msg, opts)
    }

    /// A handle for the things the trait does not cover.
    pub fn control(&self) -> NetControl {
        self.control.clone()
    }

    /// Seats occupied when this client joined (a bit per seat).
    pub fn seat(&self) -> PlayerId {
        self.control.me
    }
}

impl Drop for NetTransport {
    fn drop(&mut self) {
        self.control.shared.close(CloseReason::Left, true);
    }
}

const POLL: Duration = Duration::from_millis(50);

fn connect(
    addr: impl ToSocketAddrs,
    request: &ClientMsg,
    opts: &ClientOptions,
) -> Result<NetTransport, ConnectError> {
    let mut last = None;
    let mut stream = None;
    for a in addr.to_socket_addrs().map_err(ConnectError::Io)? {
        match TcpStream::connect_timeout(&a, opts.connect_timeout) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let mut stream = stream.ok_or_else(|| {
        ConnectError::Io(last.unwrap_or_else(|| std::io::Error::other("no address")))
    })?;
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(POLL)).map_err(ConnectError::Io)?;
    stream
        .set_write_timeout(Some(opts.handshake_timeout))
        .map_err(ConnectError::Io)?;
    let bytes = request
        .encode()
        .map_err(|e| ConnectError::Protocol(e.to_string()))?;
    stream.write_all(&bytes).map_err(ConnectError::Io)?;

    // The answer.
    let mut reader = EnvelopeReader::new();
    let deadline = Instant::now() + opts.handshake_timeout;
    let (me, host, code) = loop {
        if Instant::now() >= deadline {
            return Err(ConnectError::Timeout);
        }
        let payload = match reader.poll(&mut stream) {
            Ok(Polled::Payload(p)) => p,
            Ok(Polled::Idle) => continue,
            Err(ReadError::Closed) => {
                return Err(ConnectError::Io(std::io::ErrorKind::UnexpectedEof.into()))
            }
            Err(ReadError::Io(e)) => return Err(ConnectError::Io(e)),
            Err(ReadError::BadLength(n)) => {
                return Err(ConnectError::Protocol(format!("envelope of {n} bytes")))
            }
        };
        match ServerMsg::decode(&payload) {
            Ok(ServerMsg::Created { code, seat }) => break (seat, seat, Some(code)),
            Ok(ServerMsg::Joined { seat, host, .. }) => {
                let code = match request {
                    ClientMsg::Join { code, .. } => Some(*code),
                    _ => None,
                };
                break (seat, host, code);
            }
            Ok(ServerMsg::Refused(why)) => return Err(ConnectError::Refused(why)),
            Ok(ServerMsg::Keepalive) => {}
            Ok(_) => return Err(ConnectError::Protocol("data before the room".into())),
            Err(e) => return Err(ConnectError::Protocol(e.to_string())),
        }
    };
    let code = code.ok_or_else(|| ConnectError::Protocol("no room code".into()))?;

    let (tx, rx) = sync_channel::<Out>(opts.outgoing_queue.max(8));
    let wsock = stream.try_clone().map_err(ConnectError::Io)?;
    let rsock = stream.try_clone().map_err(ConnectError::Io)?;
    wsock
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(ConnectError::Io)?;
    let shared = Arc::new(Shared {
        inbox: Mutex::new(Inbox::default()),
        stop: AtomicBool::new(false),
        sock: stream,
        out: tx,
    });
    let keepalive = (opts.idle_timeout / 3).max(Duration::from_millis(50));
    let s = shared.clone();
    thread::Builder::new()
        .name("net-write".into())
        .spawn(move || write_loop(wsock, &rx, keepalive, &s))
        .map_err(ConnectError::Io)?;
    let s = shared.clone();
    let o = opts.clone();
    thread::Builder::new()
        .name("net-read".into())
        .spawn(move || read_loop(rsock, reader, &o, &s))
        .map_err(ConnectError::Io)?;
    Ok(NetTransport {
        control: NetControl {
            shared,
            me: PlayerId(me),
            host: PlayerId(host),
            code,
        },
    })
}

fn write_loop(mut sock: TcpStream, rx: &Receiver<Out>, keepalive: Duration, shared: &Shared) {
    let ka = ClientMsg::Keepalive.encode().unwrap_or_default();
    loop {
        let bytes = match rx.recv_timeout(keepalive) {
            Ok(Out::Bytes(b)) => b,
            Ok(Out::Close { graceful }) => {
                if graceful {
                    let _ = sock.write_all(&ClientMsg::Leave.encode().unwrap_or_default());
                }
                break;
            }
            // A close that found the queue full is not lost: the flag says.
            Err(RecvTimeoutError::Timeout) if shared.stop.load(Ordering::SeqCst) => break,
            Err(RecvTimeoutError::Timeout) => ka.clone(),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if let Err(e) = sock.write_all(&bytes) {
            shared.close(CloseReason::Io(e.to_string()), false);
            break;
        }
    }
    let _ = sock.shutdown(Shutdown::Both);
}

fn read_loop(mut sock: TcpStream, mut reader: EnvelopeReader, opts: &ClientOptions, shared: &Shared) {
    let mut last_rx = Instant::now();
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let now = Instant::now();
        if now.duration_since(last_rx) > opts.idle_timeout {
            return shared.close(CloseReason::TimedOut, false);
        }
        if reader.partial_age().is_some_and(|a| a > opts.frame_timeout) {
            return shared.close(CloseReason::TimedOut, false);
        }
        let payload = match reader.poll(&mut sock) {
            Ok(Polled::Payload(p)) => p,
            Ok(Polled::Idle) => continue,
            Err(ReadError::Closed) => return shared.close(CloseReason::RelayClosed, false),
            Err(ReadError::Io(e)) => return shared.close(CloseReason::Io(e.to_string()), false),
            Err(ReadError::BadLength(n)) => {
                return shared.close(CloseReason::Protocol(format!("envelope of {n} bytes")), false)
            }
        };
        last_rx = Instant::now();
        let item = match ServerMsg::decode(&payload) {
            Ok(ServerMsg::Data { from, frame, .. }) => Item::Data {
                from: PlayerId(from),
                frame,
            },
            Ok(ServerMsg::Departed(s)) => Item::Departed(PlayerId(s)),
            Ok(ServerMsg::Keepalive) => continue,
            Ok(_) => {
                return shared.close(CloseReason::Protocol("unexpected message".into()), false)
            }
            Err(e) => return shared.close(CloseReason::Protocol(e.to_string()), false),
        };
        let mut inbox = shared.inbox();
        let size = match &item {
            Item::Data { frame, .. } => frame.len() + 16,
            Item::Departed(_) => 16,
        };
        if inbox.bytes + size > opts.max_inbox_bytes {
            drop(inbox);
            return shared.close(CloseReason::InboxFull, false);
        }
        inbox.bytes += size;
        inbox.queue.push_back(item);
    }
}

impl Transport for NetTransport {
    fn send(&mut self, to: Destination, msg: &proto::Message) -> Result<(), TransportError> {
        let mask = match to {
            Destination::All => ALL_SEATS,
            Destination::Seat(s) if usize::from(s.0) < MAX_SEATS && s != self.control.me => {
                1 << s.0
            }
            Destination::Seat(_) => return Err(TransportError::Closed),
        };
        let frame = proto::encode(msg).map_err(TransportError::Frame)?;
        self.control.shared.queue(&ClientMsg::Data { mask, frame })
    }

    fn poll(&mut self) -> Result<Option<Incoming>, TransportError> {
        let shared = &self.control.shared;
        let mut inbox = shared.inbox();
        loop {
            match inbox.queue.pop_front() {
                Some(Item::Departed(seat)) => {
                    inbox.bytes = inbox.bytes.saturating_sub(16);
                    inbox.departures |= 1u8.checked_shl(u32::from(seat.0)).unwrap_or(0);
                }
                Some(Item::Data { from, frame }) => {
                    inbox.bytes = inbox.bytes.saturating_sub(frame.len() + 16);
                    drop(inbox);
                    return match proto::decode(&frame) {
                        Ok(msg) => Ok(Some(Incoming { from, msg })),
                        Err(e) => Err(TransportError::Frame(e)),
                    };
                }
                None if inbox.closed.is_some() => return Err(TransportError::Closed),
                None => return Ok(None),
            }
        }
    }
}
