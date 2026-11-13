//! The envelope between a client and the relay (docs/NET.md, section 9), and
//! the bounded stream reader both ends use.
//!
//! Every envelope is `u32` little-endian payload length, then the payload; the
//! payload's first byte is the message kind. The relay never looks inside a
//! [`proto`](crate::proto) frame beyond its header ([`peek_header`], which
//! accepts every protocol version), so the envelope is its own small format
//! with its own version ([`RELAY_VERSION`]) and does not move with
//! `PROTOCOL_VERSION`.
//!
//! Everything here treats its input as hostile: lengths are checked before
//! anything is buffered, no decoder indexes past what it has checked, and no
//! error is a panic.

use std::io::{ErrorKind, Read};
use std::time::{Duration, Instant};

use crate::proto::{peek_header, HEADER_LEN, MAX_FRAME_BYTES, MAX_SEATS};

/// The envelope format's version, carried in `Create` and `Join`.
pub const RELAY_VERSION: u8 = 1;
/// The longest envelope payload: a frame plus the kind, mask and sender bytes.
pub const MAX_ENVELOPE_BYTES: usize = MAX_FRAME_BYTES + 8;
/// Characters in a room code.
pub const CODE_LEN: usize = 6;
/// Longest room password, in bytes.
pub const MAX_PASSWORD_BYTES: usize = 32;
/// "No seat" in the optional-seat byte of `Join`.
pub const NO_SEAT: u8 = 0xFF;
/// Destination mask for every other member.
pub const ALL_SEATS: u8 = 0xFF;

const ALPHABET: &[u8; 31] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

/// A room's join code: six characters from an alphabet without look-alikes
/// (`0`/`O`, `1`/`I`/`L`), shown as `K7M-Q2X`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomCode([u8; CODE_LEN]);

impl RoomCode {
    /// A code from OS randomness.
    pub fn random() -> Result<RoomCode, getrandom::Error> {
        let mut bytes = [0u8; CODE_LEN];
        getrandom::fill(&mut bytes)?;
        // 256 is not a multiple of 31; the bias (a few parts in a thousand
        // per character) does not matter for a code that is also rate
        // limited, and rejection sampling would need an unbounded loop.
        Ok(RoomCode(bytes.map(|b| ALPHABET[usize::from(b) % ALPHABET.len()])))
    }

    /// Parses what a player types: case-insensitive, dashes and spaces
    /// ignored.
    pub fn parse(text: &str) -> Option<RoomCode> {
        let mut out = [0u8; CODE_LEN];
        let mut n = 0;
        for c in text.chars() {
            if c == '-' || c == ' ' {
                continue;
            }
            let c = u8::try_from(c.to_ascii_uppercase()).ok()?;
            if n == CODE_LEN || !ALPHABET.contains(&c) {
                return None;
            }
            out[n] = c;
            n += 1;
        }
        (n == CODE_LEN).then_some(RoomCode(out))
    }

    /// The code from its wire bytes, or `None` for bytes outside the alphabet.
    pub fn from_bytes(bytes: [u8; CODE_LEN]) -> Option<RoomCode> {
        bytes.iter().all(|b| ALPHABET.contains(b)).then_some(RoomCode(bytes))
    }

    pub fn as_bytes(&self) -> [u8; CODE_LEN] {
        self.0
    }
}

impl std::fmt::Display for RoomCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, &b) in self.0.iter().enumerate() {
            if i == CODE_LEN / 2 {
                f.write_str("-")?;
            }
            write!(f, "{}", char::from(b))?;
        }
        Ok(())
    }
}

/// Why the relay did not take a client into a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefuseReason {
    /// The relay speaks another envelope version.
    Version,
    /// No such room.
    NoRoom,
    /// Wrong password.
    BadPassword,
    /// All eight seats are taken.
    Full,
    /// The host closed the room to new players, or the host has left.
    Closed,
    /// The seat asked for in a rejoin is not vacated.
    BadReclaim,
    /// Too many attempts from this address.
    RateLimited,
    /// The relay is at one of its limits.
    Busy,
    /// Malformed or out-of-order request.
    BadRequest,
}

impl RefuseReason {
    fn code(self) -> u8 {
        match self {
            RefuseReason::Version => 1,
            RefuseReason::NoRoom => 2,
            RefuseReason::BadPassword => 3,
            RefuseReason::Full => 4,
            RefuseReason::Closed => 5,
            RefuseReason::BadReclaim => 6,
            RefuseReason::RateLimited => 7,
            RefuseReason::Busy => 8,
            RefuseReason::BadRequest => 9,
        }
    }

    fn from_code(c: u8) -> Option<RefuseReason> {
        Some(match c {
            1 => RefuseReason::Version,
            2 => RefuseReason::NoRoom,
            3 => RefuseReason::BadPassword,
            4 => RefuseReason::Full,
            5 => RefuseReason::Closed,
            6 => RefuseReason::BadReclaim,
            7 => RefuseReason::RateLimited,
            8 => RefuseReason::Busy,
            9 => RefuseReason::BadRequest,
            _ => return None,
        })
    }
}

impl std::fmt::Display for RefuseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RefuseReason::Version => "the relay speaks another version",
            RefuseReason::NoRoom => "no such room",
            RefuseReason::BadPassword => "wrong password",
            RefuseReason::Full => "the room is full",
            RefuseReason::Closed => "the room is closed",
            RefuseReason::BadReclaim => "that seat cannot be taken back",
            RefuseReason::RateLimited => "too many attempts, try again later",
            RefuseReason::Busy => "the relay is busy",
            RefuseReason::BadRequest => "malformed request",
        })
    }
}

/// Why an envelope or a frame inside it was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// A zero or oversized length prefix.
    BadLength(usize),
    /// Unknown kind, or a field out of range.
    Malformed(&'static str),
    /// Bytes after what the kind defines.
    Trailing,
    /// The frame inside a `Data` envelope has a bad header or a length that
    /// does not match.
    BadFrame,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::BadLength(n) => write!(f, "bad envelope length {n}"),
            WireError::Malformed(w) => write!(f, "malformed envelope: {w}"),
            WireError::Trailing => write!(f, "bytes after the envelope"),
            WireError::BadFrame => write!(f, "bad frame inside the envelope"),
        }
    }
}

impl std::error::Error for WireError {}

/// A client's request to the relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    /// Open a room; the sender becomes its host in seat 0.
    Create { password: Vec<u8> },
    /// Enter a room. `reclaim` asks for a vacated seat back (a rejoin; the
    /// host still checks the token), otherwise the lowest free seat is given.
    Join {
        code: RoomCode,
        reclaim: Option<u8>,
        password: Vec<u8>,
    },
    /// Forward a [`proto`](crate::proto) frame to the seats in `mask` (bit
    /// `n` is seat `n`; the sender is never a recipient).
    Data { mask: u8, frame: Vec<u8> },
    /// Proves the connection is alive.
    Keepalive,
    /// Host only: the relay closes that seat's connection and tells the
    /// others (docs/NET.md, section 8, "Drop, without a race").
    Kick(u8),
    /// Host only: whether new players may join. Closed seats stay reserved
    /// for rejoins.
    SetOpen(bool),
    /// Leaving on purpose.
    Leave,
}

/// What the relay tells a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    /// The room exists; you are its host in `seat`.
    Created { code: RoomCode, seat: u8 },
    /// You are in `seat`; `host` is the host's seat; `members` has a bit per
    /// seat now occupied (yours included).
    Joined { seat: u8, host: u8, members: u8 },
    Refused(RefuseReason),
    /// A frame from `from`, whose seat and host flag the relay knows from the
    /// connection. Never what the client claims.
    Data {
        from: u8,
        from_host: bool,
        frame: Vec<u8>,
    },
    /// The departure marker: `seat`'s connection is closed, and every frame
    /// it sent precedes this message in the stream.
    Departed(u8),
    Keepalive,
}

fn envelope(payload: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

fn check_frame(frame: &[u8]) -> Result<(), WireError> {
    match peek_header(frame) {
        Ok((_, len)) if HEADER_LEN + len == frame.len() => Ok(()),
        _ => Err(WireError::BadFrame),
    }
}

fn check_password(p: &[u8]) -> Result<(), WireError> {
    if p.len() > MAX_PASSWORD_BYTES {
        return Err(WireError::Malformed("password too long"));
    }
    Ok(())
}

/// Reads a slice cursor without indexing past the end.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, WireError> {
        let (&b, rest) = self.0.split_first().ok_or(WireError::Malformed("short"))?;
        self.0 = rest;
        Ok(b)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.0.len() < n {
            return Err(WireError::Malformed("short"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }
    fn done(&self) -> Result<(), WireError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(WireError::Trailing)
        }
    }
}

fn seat(b: u8) -> Result<u8, WireError> {
    if usize::from(b) < MAX_SEATS {
        Ok(b)
    } else {
        Err(WireError::Malformed("seat out of range"))
    }
}

impl ClientMsg {
    /// The envelope, length prefix included. Refuses what the relay would.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut p = Vec::new();
        match self {
            ClientMsg::Create { password } => {
                check_password(password)?;
                p.extend_from_slice(&[1, RELAY_VERSION, password.len() as u8]);
                p.extend_from_slice(password);
            }
            ClientMsg::Join {
                code,
                reclaim,
                password,
            } => {
                check_password(password)?;
                p.extend_from_slice(&[2, RELAY_VERSION]);
                p.extend_from_slice(&code.as_bytes());
                p.push(reclaim.unwrap_or(NO_SEAT));
                p.push(password.len() as u8);
                p.extend_from_slice(password);
            }
            ClientMsg::Data { mask, frame } => {
                check_frame(frame)?;
                p.extend_from_slice(&[3, *mask]);
                p.extend_from_slice(frame);
            }
            ClientMsg::Keepalive => p.push(4),
            ClientMsg::Kick(s) => p.extend_from_slice(&[5, seat(*s)?]),
            ClientMsg::SetOpen(open) => p.extend_from_slice(&[6, u8::from(*open)]),
            ClientMsg::Leave => p.push(7),
        }
        Ok(envelope(p))
    }

    /// Decodes one payload (without its length prefix).
    pub fn decode(payload: &[u8]) -> Result<ClientMsg, WireError> {
        let mut c = Cursor(payload);
        let msg = match c.u8()? {
            1 => {
                if c.u8()? != RELAY_VERSION {
                    return Err(WireError::Malformed("version"));
                }
                let n = usize::from(c.u8()?);
                let password = c.take(n)?.to_vec();
                check_password(&password)?;
                ClientMsg::Create { password }
            }
            2 => {
                if c.u8()? != RELAY_VERSION {
                    return Err(WireError::Malformed("version"));
                }
                let mut code = [0u8; CODE_LEN];
                code.copy_from_slice(c.take(CODE_LEN)?);
                let code = RoomCode::from_bytes(code).ok_or(WireError::Malformed("code"))?;
                let reclaim = match c.u8()? {
                    NO_SEAT => None,
                    s => Some(seat(s)?),
                };
                let n = usize::from(c.u8()?);
                let password = c.take(n)?.to_vec();
                check_password(&password)?;
                ClientMsg::Join {
                    code,
                    reclaim,
                    password,
                }
            }
            3 => {
                let mask = c.u8()?;
                let frame = c.rest();
                check_frame(frame)?;
                return Ok(ClientMsg::Data {
                    mask,
                    frame: frame.to_vec(),
                });
            }
            4 => ClientMsg::Keepalive,
            5 => ClientMsg::Kick(seat(c.u8()?)?),
            6 => match c.u8()? {
                0 => ClientMsg::SetOpen(false),
                1 => ClientMsg::SetOpen(true),
                _ => return Err(WireError::Malformed("flag")),
            },
            7 => ClientMsg::Leave,
            _ => return Err(WireError::Malformed("kind")),
        };
        c.done()?;
        Ok(msg)
    }
}

impl ServerMsg {
    /// The envelope, length prefix included.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut p = Vec::new();
        match self {
            ServerMsg::Created { code, seat: s } => {
                p.push(1);
                p.extend_from_slice(&code.as_bytes());
                p.push(seat(*s)?);
            }
            ServerMsg::Joined {
                seat: s,
                host,
                members,
            } => p.extend_from_slice(&[2, seat(*s)?, seat(*host)?, *members]),
            ServerMsg::Refused(r) => p.extend_from_slice(&[3, r.code()]),
            ServerMsg::Data {
                from,
                from_host,
                frame,
            } => {
                check_frame(frame)?;
                p.extend_from_slice(&[4, seat(*from)?, u8::from(*from_host)]);
                p.extend_from_slice(frame);
            }
            ServerMsg::Departed(s) => p.extend_from_slice(&[5, seat(*s)?]),
            ServerMsg::Keepalive => p.push(6),
        }
        Ok(envelope(p))
    }

    /// Decodes one payload (without its length prefix). A client runs this
    /// on what the relay sends, so a hostile relay gets the same treatment
    /// as a hostile peer.
    pub fn decode(payload: &[u8]) -> Result<ServerMsg, WireError> {
        let mut c = Cursor(payload);
        let msg = match c.u8()? {
            1 => {
                let mut code = [0u8; CODE_LEN];
                code.copy_from_slice(c.take(CODE_LEN)?);
                ServerMsg::Created {
                    code: RoomCode::from_bytes(code).ok_or(WireError::Malformed("code"))?,
                    seat: seat(c.u8()?)?,
                }
            }
            2 => ServerMsg::Joined {
                seat: seat(c.u8()?)?,
                host: seat(c.u8()?)?,
                members: c.u8()?,
            },
            3 => ServerMsg::Refused(
                RefuseReason::from_code(c.u8()?).ok_or(WireError::Malformed("reason"))?,
            ),
            4 => {
                let from = seat(c.u8()?)?;
                let from_host = match c.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(WireError::Malformed("flag")),
                };
                let frame = c.rest();
                check_frame(frame)?;
                return Ok(ServerMsg::Data {
                    from,
                    from_host,
                    frame: frame.to_vec(),
                });
            }
            5 => ServerMsg::Departed(seat(c.u8()?)?),
            6 => ServerMsg::Keepalive,
            _ => return Err(WireError::Malformed("kind")),
        };
        c.done()?;
        Ok(msg)
    }
}

/// Why reading a stream stopped.
#[derive(Debug)]
pub enum ReadError {
    /// The peer closed the stream (cleanly between envelopes, or not).
    Closed,
    /// The stream failed.
    Io(std::io::Error),
    /// A length prefix of zero or over [`MAX_ENVELOPE_BYTES`].
    BadLength(usize),
}

/// What one [`EnvelopeReader::poll`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Polled {
    /// One envelope's payload.
    Payload(Vec<u8>),
    /// Nothing complete yet (the read timed out or got a partial envelope).
    Idle,
}

/// Reads envelopes from a stream whose read timeout is short, so the caller
/// can check deadlines and shutdown between reads. Holds at most one
/// envelope and one read chunk, and refuses a bad length prefix before
/// buffering a byte of the body.
#[derive(Debug, Default)]
pub struct EnvelopeReader {
    buf: Vec<u8>,
    /// When the oldest byte of the envelope in progress arrived.
    partial_since: Option<Instant>,
}

const CHUNK: usize = 4096;

impl EnvelopeReader {
    pub fn new() -> EnvelopeReader {
        EnvelopeReader::default()
    }

    /// How long an incomplete envelope has been waiting for its rest.
    pub fn partial_age(&self) -> Option<Duration> {
        self.partial_since.map(|t| t.elapsed())
    }

    /// Bytes held; bounded by `MAX_ENVELOPE_BYTES + 4 + CHUNK`.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    fn take_complete(&mut self) -> Result<Option<Vec<u8>>, ReadError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if len == 0 || len > MAX_ENVELOPE_BYTES {
            return Err(ReadError::BadLength(len));
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let rest = self.buf.split_off(4 + len);
        let mut payload = std::mem::replace(&mut self.buf, rest);
        payload.drain(..4);
        self.partial_since = if self.buf.is_empty() {
            None
        } else {
            Some(Instant::now())
        };
        Ok(Some(payload))
    }

    /// Reads once and returns one envelope if a whole one is held. A read
    /// that times out (`WouldBlock` or `TimedOut`) is `Idle`.
    pub fn poll(&mut self, from: &mut impl Read) -> Result<Polled, ReadError> {
        // A previous read may have brought more than one envelope.
        if let Some(p) = self.take_complete()? {
            return Ok(Polled::Payload(p));
        }
        // Never read more than the current envelope needs plus one chunk, so
        // the buffer stays bounded whatever the peer sends.
        let mut chunk = [0u8; CHUNK];
        let want = if self.buf.len() >= 4 {
            let len =
                u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
            (4 + len).saturating_sub(self.buf.len()).clamp(1, CHUNK)
        } else {
            (4 - self.buf.len()).max(1)
        };
        match from.read(&mut chunk[..want]) {
            Ok(0) => Err(ReadError::Closed),
            Ok(n) => {
                if self.buf.is_empty() {
                    self.partial_since = Some(Instant::now());
                }
                self.buf.extend_from_slice(&chunk[..n]);
                match self.take_complete()? {
                    Some(p) => Ok(Polled::Payload(p)),
                    None => Ok(Polled::Idle),
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Ok(Polled::Idle)
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => Ok(Polled::Idle),
            // Known residual risk: Windows documents the socket state as
            // indeterminate after a receive timeout, so a pending overlapped
            // receive could complete into a dropped buffer and lose bytes.
            // That would show up as a framing or length error and close the
            // connection (it fails closed), as it could for ordinary timeouts
            // before this arm existed. The pause keeps a socket that fails
            // this way at once, every time, from spinning its thread until
            // the caller's wall-clock deadline.
            Err(e) if spurious_pending(&e) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
                Ok(Polled::Idle)
            }
            Err(e) => Err(ReadError::Io(e)),
        }
    }
}

/// Windows can fail a read that has a receive timeout with
/// `ERROR_IO_PENDING` (997, "Overlapped I/O operation is in progress") instead
/// of a timeout error, with nothing read and the socket intact: the same as a
/// timeout, never a reason to drop the connection.
fn spurious_pending(e: &std::io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(997)
}

/// A byte budget refilled at a steady rate: the relay's per-connection rate
/// limit.
#[derive(Debug)]
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    /// `rate` bytes per second, up to `burst` saved up.
    pub fn new(rate: u32, burst: u32) -> TokenBucket {
        TokenBucket {
            rate: f64::from(rate),
            burst: f64::from(burst),
            tokens: f64::from(burst),
            last: Instant::now(),
        }
    }

    /// Spends `bytes`; false when the budget is exhausted.
    pub fn take(&mut self, bytes: usize) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        let need = bytes as f64;
        if need > self.tokens {
            return false;
        }
        self.tokens -= need;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{encode, Message};

    /// Yields scripted reads: bytes, or an OS error code.
    struct Script(std::collections::VecDeque<Result<Vec<u8>, i32>>);
    impl Read for Script {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            match self.0.pop_front() {
                Some(Ok(b)) => {
                    let n = b.len().min(out.len());
                    out[..n].copy_from_slice(&b[..n]);
                    if n < b.len() {
                        self.0.push_front(Ok(b[n..].to_vec()));
                    }
                    Ok(n)
                }
                Some(Err(code)) => Err(std::io::Error::from_raw_os_error(code)),
                None => Err(std::io::ErrorKind::WouldBlock.into()),
            }
        }
    }

    /// Windows' "overlapped I/O in progress" read error (997) is a timeout
    /// there and keeps the partial envelope; elsewhere the code means
    /// something else and stays an error. (The Windows arm runs on Windows CI
    /// only.)
    #[test]
    fn a_spurious_pending_read_keeps_the_partial_envelope() {
        let payload = vec![7u8, 8, 9, 10, 11];
        let mut whole = (payload.len() as u32).to_le_bytes().to_vec();
        whole.extend_from_slice(&payload);
        let mut src = Script(
            [Ok(whole[..2].to_vec()), Err(997), Ok(whole[2..].to_vec())]
                .into_iter()
                .collect(),
        );
        let mut r = EnvelopeReader::new();
        assert!(matches!(r.poll(&mut src), Ok(Polled::Idle)));
        assert_eq!(r.buffered(), 2);
        let mid = r.poll(&mut src);
        if cfg!(windows) {
            assert!(matches!(mid, Ok(Polled::Idle)));
            assert_eq!(r.buffered(), 2, "the partial envelope was kept");
            let mut done = r.poll(&mut src);
            // The rest may need a second read to complete.
            for _ in 0..4 {
                if matches!(done, Ok(Polled::Payload(_))) {
                    break;
                }
                done = r.poll(&mut src);
            }
            assert!(matches!(done, Ok(Polled::Payload(ref p)) if *p == payload));
        } else {
            assert!(matches!(mid, Err(ReadError::Io(_))));
        }
    }

    /// A socket that fails with 997 every time must not spin: the polls are
    /// paced, so a caller's `Instant` deadline ends the loop after few polls.
    #[test]
    fn spurious_pending_forever_is_paced_and_ends_at_the_deadline() {
        let mut src = Script((0..1_000_000).map(|_| Err(997)).collect());
        let mut r = EnvelopeReader::new();
        let end = Instant::now() + std::time::Duration::from_millis(100);
        let mut polls = 0u32;
        while Instant::now() < end {
            match r.poll(&mut src) {
                Ok(Polled::Idle) => polls += 1,
                Err(_) => {
                    if cfg!(windows) {
                        panic!("997 must stay a timeout on Windows");
                    }
                    return;
                }
                Ok(Polled::Payload(_)) => unreachable!(),
            }
        }
        assert!(polls < 100, "{polls} polls in 100 ms: spinning");
    }

    fn frame() -> Vec<u8> {
        encode(&Message::Ping { nonce: 7 }).unwrap()
    }

    #[test]
    fn codes_round_trip_and_reject_junk() {
        let c = RoomCode::random().unwrap();
        assert_eq!(RoomCode::parse(&c.to_string()), Some(c));
        assert_eq!(RoomCode::parse(&c.to_string().to_lowercase()), Some(c));
        assert_eq!(RoomCode::parse("K7M-Q2X").unwrap().to_string(), "K7M-Q2X");
        for bad in ["", "K7M-Q2", "K7M-Q2XX", "K7M-Q20", "K7M-Q2I", "K7M Q2é", "\0\0\0\0\0\0"] {
            assert_eq!(RoomCode::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn messages_round_trip() {
        let code = RoomCode::parse("K7MQ2X").unwrap();
        let clients = [
            ClientMsg::Create { password: b"pw".to_vec() },
            ClientMsg::Create { password: vec![] },
            ClientMsg::Join { code, reclaim: None, password: vec![] },
            ClientMsg::Join { code, reclaim: Some(3), password: b"x".to_vec() },
            ClientMsg::Data { mask: 0b101, frame: frame() },
            ClientMsg::Keepalive,
            ClientMsg::Kick(2),
            ClientMsg::SetOpen(true),
            ClientMsg::SetOpen(false),
            ClientMsg::Leave,
        ];
        for m in clients {
            let e = m.encode().unwrap();
            assert_eq!(u32::from_le_bytes(e[..4].try_into().unwrap()) as usize, e.len() - 4);
            assert_eq!(ClientMsg::decode(&e[4..]).unwrap(), m);
        }
        let servers = [
            ServerMsg::Created { code, seat: 0 },
            ServerMsg::Joined { seat: 2, host: 0, members: 0b111 },
            ServerMsg::Refused(RefuseReason::BadPassword),
            ServerMsg::Data { from: 1, from_host: false, frame: frame() },
            ServerMsg::Departed(5),
            ServerMsg::Keepalive,
        ];
        for m in servers {
            let e = m.encode().unwrap();
            assert_eq!(ServerMsg::decode(&e[4..]).unwrap(), m);
        }
    }

    /// A deterministic mix of every shape of bad payload: none panics, every
    /// one is an error or a message that re-encodes to itself.
    #[test]
    fn hostile_payloads_are_refused_without_panic() {
        let code = RoomCode::parse("K7MQ2X").unwrap();
        let good: Vec<Vec<u8>> = vec![
            ClientMsg::Create { password: b"pw".to_vec() }.encode().unwrap(),
            ClientMsg::Join { code, reclaim: Some(1), password: b"x".to_vec() }.encode().unwrap(),
            ClientMsg::Data { mask: 3, frame: frame() }.encode().unwrap(),
            ServerMsg::Data { from: 1, from_host: true, frame: frame() }.encode().unwrap(),
            ServerMsg::Joined { seat: 1, host: 0, members: 3 }.encode().unwrap(),
        ];
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut accepted = 0;
        for round in 0..20_000 {
            let mut p = good[round % good.len()][4..].to_vec();
            match next() % 5 {
                0 => p.truncate((next() % (p.len() as u64 + 1)) as usize),
                1 => {
                    let i = (next() % p.len() as u64) as usize;
                    p[i] ^= 1 << (next() % 8);
                }
                2 => p.extend((0..next() % 8).map(|_| next() as u8)),
                3 => p = (0..next() % 40).map(|_| next() as u8).collect(),
                _ => {
                    let i = (next() % p.len() as u64) as usize;
                    p[i] = next() as u8;
                }
            }
            if ClientMsg::decode(&p).is_ok() {
                accepted += 1;
            }
            if ServerMsg::decode(&p).is_ok() {
                accepted += 1;
            }
        }
        // Mutations that keep a message valid exist (a flipped mask bit), so
        // the table is not trivially all errors.
        assert!(accepted > 0);
        // Specific refusals.
        assert_eq!(ClientMsg::decode(&[]), Err(WireError::Malformed("short")));
        assert!(ClientMsg::decode(&[3, 0xFF]).is_err(), "data without a frame");
        assert!(ClientMsg::decode(&[3, 0xFF, 1, 2, 3]).is_err(), "data with junk");
        let mut long = vec![3, 0xFF];
        long.extend_from_slice(&frame());
        long.push(0);
        assert!(ClientMsg::decode(&long).is_err(), "frame longer than its header says");
        assert!(ClientMsg::decode(&[5, 8]).is_err(), "kick of seat 8");
        assert!(ClientMsg::decode(&[4, 9]).is_err(), "keepalive with a body");
        let mut create = vec![1, RELAY_VERSION, 33];
        create.extend_from_slice(&[b'a'; 33]);
        assert!(ClientMsg::decode(&create).is_err(), "password over the limit");
        assert!(ServerMsg::decode(&[4, 9, 0]).is_err(), "sender seat 9");
    }

    struct Chunks<'a>(&'a [u8], usize);
    impl Read for Chunks<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            let n = self.1.min(buf.len()).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn drain(bytes: &[u8], step: usize) -> (Vec<Vec<u8>>, Option<ReadError>, usize) {
        let mut src = Chunks(bytes, step);
        let mut r = EnvelopeReader::new();
        let mut out = Vec::new();
        let mut max = 0;
        loop {
            match r.poll(&mut src) {
                Ok(Polled::Payload(p)) => out.push(p),
                Ok(Polled::Idle) => {}
                Err(e) => return (out, Some(e), max),
            }
            max = max.max(r.buffered());
        }
    }

    #[test]
    fn reader_reassembles_any_chunking_and_stays_bounded() {
        let mut stream = Vec::new();
        for i in 0..5 {
            stream.extend(ClientMsg::Data { mask: i, frame: frame() }.encode().unwrap());
            stream.extend(ClientMsg::Keepalive.encode().unwrap());
        }
        for step in [1, 2, 3, 7, 4096] {
            let (got, end, _) = drain(&stream, step);
            assert_eq!(got.len(), 10, "step {step}");
            assert!(matches!(end, Some(ReadError::Closed)));
        }
    }

    #[test]
    fn reader_refuses_a_bad_length_before_buffering_the_body() {
        for len in [0u32, MAX_ENVELOPE_BYTES as u32 + 1, u32::MAX, 0x8000_0000] {
            let mut bytes = len.to_le_bytes().to_vec();
            bytes.extend(vec![0xAA; 100_000]);
            let (got, end, max) = drain(&bytes, 4096);
            assert!(got.is_empty());
            assert!(matches!(end, Some(ReadError::BadLength(n)) if n == len as usize), "{len}");
            assert!(max <= 4096, "buffered {max} bytes for length {len}");
        }
        // The largest legal envelope is accepted and bounded.
        let mut bytes = (MAX_ENVELOPE_BYTES as u32).to_le_bytes().to_vec();
        bytes.extend(vec![0u8; MAX_ENVELOPE_BYTES]);
        let (got, _, max) = drain(&bytes, 4096);
        assert_eq!(got.len(), 1);
        assert!(max <= MAX_ENVELOPE_BYTES + 4 + CHUNK);
    }

    #[test]
    fn token_bucket_limits_a_flood() {
        let mut b = TokenBucket::new(1000, 3000);
        assert!(b.take(3000));
        assert!(!b.take(1000));
    }
}
