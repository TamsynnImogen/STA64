//! The cineractive shot model (UI-36): one reducer, fed the mission's presentation
//! requests, shared by the mission runtime (which writes the engine variables
//! `isCineractiveDone` / `isCineractiveCancel` the converted Armada I / II scripts wait
//! on) and the client camera (which plays the same shots). Armada I, Armada II and the
//! Fleet Operations / MMM Lua missions all reach it: their calls arrive as
//! `Presentation` events (A1/A2 `CineractiveShot` and `CineractiveCameraOffset` as
//! `Call`s, MMM `Cineractive:lookFrom` / `lookAt` / `setCameraOffset` as `Call`,
//! `Shot` and `CameraOffset`).
//!
//! # The model (A1 `Armada.exe` and A2 `Armada2.exe` read statically, the same code in
//! both; FO Roots runs the A2 engine code; nothing measured on a running game)
//!
//! - A cineractive has an **eye** track and a **target** track (or no target: the camera
//!   looks along the eye track's direction), plus an **eye offset**. A track is a fixed
//!   point, an object (its place and heading, followed), or a map path (`.bzn` AiPath by
//!   exact name) followed at a speed. The speed is **path segments per second of game
//!   time**: a path of `n` points takes `(n - 1) / speed` seconds, whatever its length.
//!   Between points the path is a cubic Hermite curve, tangents from the neighbours
//!   (ends reflected) scaled by the segment length.
//! - `CineractiveShot` (12 overloads: eye = object, `(path, int)` point, `(path, float)`
//!   path at speed; target the same or `bool` = no target) replaces eye and target,
//!   restarts both, resets the offset to zero and constant speed off: a hard cut. A call
//!   naming an unknown path or point is ignored.
//! - `CineractiveCameraOffset(x, y, z, rel)`: a fixed eye offset, in the eye track's frame
//!   (x right, y up, z forward) when `rel`, world axes otherwise. `(path, speed, rel)`:
//!   the offset moves along the path relative to its first point (`(n - 2) / speed` s).
//! - A2 `CineractiveConstantSpeed(true)` (after the shot): the eye path moves at a constant
//!   world speed of `speed * len(first segment)`.
//! - MMM `lookFrom(Path, speed)` is `CineractiveShot(name, speed, true)` (eye path, no
//!   target, offset reset); `lookFrom` / `lookAt` of an entity or a vector set one track
//!   and nothing else; `setCameraOffset(v)` is the relative fixed offset.
//! - **Done**: every track has finished (a path at its end; points and objects at once;
//!   a path of speed 0 never). It is written each sim tick while a cineractive runs, and
//!   reads 0 on the tick of a shot (the shot call writes 0; the next update writes the
//!   tracks' state). **Cancel**: the player skipped (the skip key `cineractiveCancel` /
//!   A1 `switch_camera_shot`, Space); cleared when a cineractive starts. Neither ends the
//!   cineractive: the script reads them and calls `CineractiveEnd`.
//!
//! # Time
//!
//! Shots advance on **sim ticks** (game time, as the original's cineractive update runs
//! with the game step): the mission decides Done from the tick count, the client places
//! the camera from the same state at the interpolated sim time, so a headless run, the
//! stand-in host and the client finish a shot on the same tick.

use sta_core::{EntityId, Fixed, TICK_HZ};
use sta_sim::present::{Arg, Presentation, Spot};

pub type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn len(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn norm(a: V3) -> V3 {
    let l = len(a);
    if l > 1e-12 { scale(a, 1.0 / l) } else { [0.0; 3] }
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn num(f: Fixed) -> f64 {
    f.to_bits() as f64 / 4_294_967_296.0
}

fn v3(p: &sta_core::Vec3) -> V3 {
    [num(p.x), num(p.y), num(p.z)]
}

/// A path followed at `speed` segments per second.
#[derive(Debug, Clone, PartialEq)]
pub struct PathRun {
    pub points: Vec<V3>,
    pub speed: f64,
}

impl PathRun {
    fn segments(&self) -> usize {
        self.points.len().saturating_sub(1)
    }

    /// Seconds each segment lasts: `1 / speed`, or with constant speed `len_i / (speed * len_0)`.
    fn durations(&self, constant: bool) -> Vec<f64> {
        let n = self.segments();
        let l0 = if n > 0 { len(sub(self.points[1], self.points[0])) } else { 0.0 };
        (0..n)
            .map(|i| {
                if constant && l0 > 1e-9 {
                    len(sub(self.points[i + 1], self.points[i])) / (self.speed * l0)
                } else {
                    1.0 / self.speed
                }
            })
            .collect()
    }

    /// The path parameter (segments travelled, `0..=n-1`) after `secs`.
    pub fn param(&self, secs: f64, constant: bool) -> f64 {
        let n = self.segments();
        if n == 0 || self.speed <= 0.0 || secs <= 0.0 {
            return 0.0;
        }
        let mut left = secs;
        for (i, d) in self.durations(constant).into_iter().enumerate() {
            if left < d {
                return i as f64 + left / d;
            }
            left -= d;
        }
        n as f64
    }

    /// Whether the run has reached its last point after `secs`.
    pub fn done(&self, secs: f64, constant: bool) -> bool {
        let n = self.segments();
        if n == 0 {
            return true;
        }
        if self.speed <= 0.0 {
            return false;
        }
        let total: f64 = self.durations(constant).iter().sum();
        secs >= total
    }

    /// Place and direction at parameter `t`.
    pub fn at(&self, t: f64) -> (V3, V3) {
        hermite(&self.points, t)
    }
}

/// The point at parameter `t` (`i + u`) of the Hermite curve through `p`, and its derivative.
pub fn hermite(p: &[V3], t: f64) -> (V3, V3) {
    match p.len() {
        0 => return ([0.0; 3], [0.0, 0.0, 1.0]),
        1 => return (p[0], [0.0, 0.0, 1.0]),
        _ => {}
    }
    let n = p.len();
    let t = t.clamp(0.0, (n - 1) as f64);
    let i = (t.floor() as usize).min(n - 2);
    let u = t - i as f64;
    let get = |k: isize| -> V3 {
        if k < 0 {
            sub(scale(p[0], 2.0), p[1])
        } else if k as usize >= n {
            sub(scale(p[n - 1], 2.0), p[n - 2])
        } else {
            p[k as usize]
        }
    };
    let (a, b) = (p[i], p[i + 1]);
    let seg = len(sub(b, a));
    let m0 = scale(norm(sub(b, get(i as isize - 1))), seg);
    let m1 = scale(norm(sub(get(i as isize + 2), a)), seg);
    let (u2, u3) = (u * u, u * u * u);
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    let pos = add(add(scale(a, h00), scale(m0, h10)), add(scale(b, h01), scale(m1, h11)));
    let d00 = 6.0 * u2 - 6.0 * u;
    let d10 = 3.0 * u2 - 4.0 * u + 1.0;
    let d01 = -6.0 * u2 + 6.0 * u;
    let d11 = 3.0 * u2 - 2.0 * u;
    let dir = add(add(scale(a, d00), scale(m0, d10)), add(scale(b, d01), scale(m1, d11)));
    (pos, if len(dir) > 1e-12 { dir } else { sub(b, a) })
}

/// What a track follows.
#[derive(Debug, Clone, PartialEq)]
pub enum Track {
    Fixed(V3),
    Object(EntityId),
    Path(PathRun),
}

/// A track and the tick it (re)started on.
#[derive(Debug, Clone, PartialEq)]
pub struct Timed<T> {
    pub what: T,
    pub since: u64,
}

/// The eye offset.
#[derive(Debug, Clone, PartialEq)]
pub enum Offset {
    Fixed(V3),
    /// A path relative to its first point: the whole path's curve from point 1 on, minus
    /// point 0 (`(n - 2) / speed` seconds).
    Path(PathRun),
}

impl Offset {
    /// The offset and its direction `secs` after it started.
    fn at(&self, secs: f64) -> (V3, V3) {
        match self {
            Offset::Fixed(v) => (*v, [0.0, 0.0, 1.0]),
            Offset::Path(run) if run.points.len() >= 2 => {
                let t = (1.0 + run.speed.max(0.0) * secs.max(0.0)).min((run.points.len() - 1) as f64);
                let (p, d) = run.at(t);
                (sub(p, run.points[0]), d)
            }
            Offset::Path(_) => ([0.0; 3], [0.0, 0.0, 1.0]),
        }
    }

    fn done(&self, secs: f64) -> bool {
        match self {
            Offset::Fixed(_) => true,
            Offset::Path(run) => {
                let segments = run.points.len().saturating_sub(2) as f64;
                segments == 0.0 || (run.speed > 0.0 && run.speed * secs >= segments)
            }
        }
    }
}

/// The current shot.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub eye: Timed<Track>,
    /// `None`: no target; the camera looks along the eye track, or along the offset track
    /// when [`Shot::along_offset`] (the `bool` of the shot's no-target form).
    pub target: Option<Timed<Track>>,
    pub along_offset: bool,
    pub offset: Timed<Offset>,
    /// The offset is in the eye track's frame.
    pub relative: bool,
    /// A2 constant world speed on the eye path.
    pub constant_speed: bool,
}

impl Shot {
    fn origin(tick: u64) -> Shot {
        Shot {
            eye: Timed { what: Track::Fixed([0.0; 3]), since: tick },
            target: Some(Timed { what: Track::Fixed([0.0; 3]), since: tick }),
            along_offset: false,
            offset: Timed { what: Offset::Fixed([0.0; 3]), since: tick },
            relative: false,
            constant_speed: false,
        }
    }
}

/// The longest a cineractive holds the player before a host releases it (seconds of game
/// time in headless runs, real seconds in the client): a script that never ends one (a path of
/// speed 0, a lost reply) cannot hold the camera for good. The longest stock intro read, A2
/// fed10's first shot, takes 184 s to its end (shot model, static).
pub const MAX_HOLD_SECS: u32 = 300;

fn secs(ticks: f64) -> f64 {
    ticks / f64::from(TICK_HZ)
}

/// Where an object is and which way it faces (game space), for the camera.
pub type Locate<'a> = &'a dyn Fn(EntityId) -> Option<(V3, V3)>;

/// The cineractive state: the reducer of every presentation request that concerns it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cineractive {
    pub active: bool,
    /// The script has placed a shot since the start (before it, the camera holds).
    pub shot_placed: bool,
    pub shot: Option<Shot>,
    /// The tick of the last `CineractiveShot` (Done reads 0 on it).
    pub shot_tick: u64,
    /// The player skipped this cineractive.
    pub cancel: bool,
    /// A skip the mission object has not answered yet (MMM's `cineractiveFinish(true)`).
    pub skip_pending: bool,
    /// The tick this cineractive started.
    pub started: u64,
    /// Values still to write once after a release ended it in the same breath (the cancel
    /// the script may wait on, an A2 step it holds).
    pub final_values: Option<(i64, i64)>,
}

fn path<'a>(paths: &'a [(String, Vec<[f32; 3]>)], name: &str) -> Option<&'a [[f32; 3]]> {
    paths.iter().find(|(n, _)| n == name).map(|(_, p)| p.as_slice())
}

fn points(p: &[[f32; 3]]) -> Vec<V3> {
    p.iter().map(|q| [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]).collect()
}

/// `(path, int)` = that point, `(path, float)` = the path at that speed.
fn path_track(paths: &[(String, Vec<[f32; 3]>)], name: &str, n: &Arg) -> Option<Track> {
    let p = path(paths, name)?;
    match n {
        Arg::Int(i) => p.get(usize::try_from(*i).ok()?).map(|q| Track::Fixed([f64::from(q[0]), f64::from(q[1]), f64::from(q[2])])),
        Arg::Num(s) => Some(Track::Path(PathRun { points: points(p), speed: num(*s) })),
        _ => None,
    }
}

/// One side of a `CineractiveShot`.
enum Side {
    Track(Track),
    /// No target; `true` looks along the offset track.
    NoTarget(bool),
}

/// One side of a `CineractiveShot` and how many arguments it used.
fn shot_side(paths: &[(String, Vec<[f32; 3]>)], args: &[Arg]) -> Option<(Side, usize)> {
    match args {
        [Arg::Entity(id), ..] => Some((Side::Track(Track::Object(*id)), 1)),
        [Arg::Int(id), ..] => Some((Side::Track(Track::Object(EntityId(u32::try_from(*id).ok()?))), 1)),
        [Arg::Text(name), n @ (Arg::Int(_) | Arg::Num(_)), ..] => Some((Side::Track(path_track(paths, name, n)?), 2)),
        [Arg::Bool(b), ..] => Some((Side::NoTarget(*b), 1)),
        _ => None,
    }
}

fn spot_track(paths: &[(String, Vec<[f32; 3]>)], s: &Spot) -> Option<Track> {
    match s {
        Spot::Entity(id) => Some(Track::Object(*id)),
        Spot::Position(p) => Some(Track::Fixed(v3(p))),
        Spot::PathPoint { path: name, index } => path_track(paths, name, &Arg::Int(i64::from(*index))),
        _ => None,
    }
}

fn arg_track(a: &Arg) -> Option<Track> {
    match a {
        Arg::Entity(id) => Some(Track::Object(*id)),
        Arg::Position(p) => Some(Track::Fixed(v3(p))),
        _ => None,
    }
}

impl Cineractive {
    /// Feed one presentation request, raised on `tick`, with the map's paths.
    pub fn observe(&mut self, p: &Presentation, tick: u64, paths: &[(String, Vec<[f32; 3]>)]) {
        match p {
            Presentation::Cineractive { on, .. } => {
                if *on && !self.active {
                    // A new cineractive: tracks at the origin and done, skip state cleared.
                    *self = Cineractive { active: true, shot: Some(Shot::origin(tick)), shot_tick: tick, started: tick, ..Cineractive::default() };
                } else if !*on && self.active {
                    // A cancel set in the same breath as the end (a host's release) has not been
                    // written yet: it is, once.
                    if self.cancel {
                        self.final_values = Some((i64::from(self.done_at(tick.saturating_add(1))), 1));
                    }
                    self.active = false;
                    self.skip_pending = false;
                }
            }
            _ if !self.active => {}
            Presentation::Call { name, args } => {
                let call = name.split_once('.').map_or(name.as_str(), |x| x.1);
                match (name.split_once('.').map_or("", |x| x.0), call) {
                    ("A1" | "A2", "CineractiveShot") => self.shot_call(args, tick, paths),
                    ("A1" | "A2", "CineractiveCameraOffset") => self.offset_call(args, tick, paths),
                    ("A2", "CineractiveConstantSpeed") => {
                        if let (Some(s), [Arg::Bool(b)]) = (self.shot.as_mut(), args.as_slice()) {
                            s.constant_speed = *b && matches!(s.eye.what, Track::Path(_));
                        }
                    }
                    ("Cineractive", "lookFrom") => match args.as_slice() {
                        [Arg::Text(path_name), speed @ (Arg::Num(_) | Arg::Int(_)), ..] => {
                            // `CineractiveShot(name, speed, true)`: eye path, no target (looking
                            // along the offset track), offset reset. The speed is a float.
                            let speed = match speed {
                                Arg::Int(i) => Fixed::from_int(i32::try_from(*i).unwrap_or(0)),
                                Arg::Num(n) => *n,
                                _ => Fixed::ZERO,
                            };
                            if let Some(t) = path_track(paths, path_name, &Arg::Num(speed)) {
                                self.cut(t, Side::NoTarget(true), tick);
                            }
                        }
                        [a] => {
                            if let (Some(t), Some(s)) = (arg_track(a), self.shot.as_mut()) {
                                s.eye = Timed { what: t, since: tick };
                                self.shot_placed = true;
                            }
                        }
                        _ => {}
                    },
                    ("Cineractive", "lookAt") => {
                        if let ([a], Some(s)) = (args.as_slice(), self.shot.as_mut())
                            && let Some(t) = arg_track(a)
                        {
                            s.target = Some(Timed { what: t, since: tick });
                        }
                    }
                    _ => {}
                }
            }
            // MMM's simple pair: `lookFrom(x)` then `lookAt(y)`, one track each.
            Presentation::Shot { from, at, .. } => {
                if let (Some(f), Some(a), Some(s)) = (spot_track(paths, from), spot_track(paths, at), self.shot.as_mut()) {
                    s.eye = Timed { what: f, since: tick };
                    s.target = Some(Timed { what: a, since: tick });
                    self.shot_placed = true;
                }
            }
            // MMM `setCameraOffset(v)`: `CineractiveCameraOffset(x, y, z, true)`.
            Presentation::CameraOffset { offset, relative } => {
                if let Some(s) = self.shot.as_mut() {
                    s.offset = Timed { what: Offset::Fixed(v3(offset)), since: tick };
                    s.relative = *relative;
                }
            }
            _ => {}
        }
    }

    fn cut(&mut self, eye: Track, target: Side, tick: u64) {
        let (target, along_offset) = match target {
            Side::Track(what) => (Some(Timed { what, since: tick }), false),
            Side::NoTarget(b) => (None, b),
        };
        self.shot = Some(Shot {
            eye: Timed { what: eye, since: tick },
            target,
            along_offset,
            offset: Timed { what: Offset::Fixed([0.0; 3]), since: tick },
            relative: false,
            constant_speed: false,
        });
        self.shot_placed = true;
        self.shot_tick = tick;
    }

    fn shot_call(&mut self, args: &[Arg], tick: u64, paths: &[(String, Vec<[f32; 3]>)]) {
        // An invalid call still writes Done = 0 (static), and changes nothing else.
        self.shot_tick = tick;
        let Some((Side::Track(eye), used)) = shot_side(paths, args) else { return };
        let Some((target, rest)) = shot_side(paths, &args[used..]) else { return };
        if used + rest != args.len() {
            return;
        }
        self.cut(eye, target, tick);
    }

    fn offset_call(&mut self, args: &[Arg], tick: u64, paths: &[(String, Vec<[f32; 3]>)]) {
        let Some(s) = self.shot.as_mut() else { return };
        let n = |a: &Arg| match a {
            Arg::Num(f) => Some(num(*f)),
            Arg::Int(i) => Some(*i as f64),
            _ => None,
        };
        match args {
            [x, y, z, Arg::Bool(rel)] => {
                if let (Some(x), Some(y), Some(z)) = (n(x), n(y), n(z)) {
                    s.offset = Timed { what: Offset::Fixed([x, y, z]), since: tick };
                    s.relative = *rel;
                }
            }
            [Arg::Text(name), speed, Arg::Bool(rel)] => {
                if let (Some(p), Some(speed)) = (path(paths, name), n(speed))
                    && !p.is_empty()
                {
                    s.offset = Timed { what: Offset::Path(PathRun { points: points(p), speed }), since: tick };
                    s.relative = *rel;
                }
            }
            _ => {}
        }
    }

    /// Every track has finished at `tick` (and the last shot is at least one update old).
    pub fn done_at(&self, tick: u64) -> bool {
        let Some(s) = &self.shot else { return true };
        if tick <= self.shot_tick {
            return false;
        }
        let el = |since: u64| secs(tick.saturating_sub(since) as f64);
        let track = |t: &Timed<Track>, constant: bool| match &t.what {
            Track::Path(run) => run.done(el(t.since), constant),
            _ => true,
        };
        let offset = s.offset.what.done(el(s.offset.since));
        track(&s.eye, s.constant_speed) && s.target.as_ref().is_none_or(|t| track(t, false)) && offset
    }

    /// The engine variables at `tick` while a cineractive runs: `(isCineractiveDone,
    /// isCineractiveCancel)`; `None` when none runs (the variables keep their values).
    pub fn engine_values(&self, tick: u64) -> Option<(i64, i64)> {
        self.active.then(|| (i64::from(self.done_at(tick)), i64::from(self.cancel)))
    }

    /// [`Cineractive::engine_values`], or the values a release left to write (taken once).
    pub fn take_engine_values(&mut self, tick: u64) -> Option<(i64, i64)> {
        self.engine_values(tick).or_else(|| self.final_values.take())
    }

    /// The player asked to skip (Space): Cancel reads 1 from the next update.
    pub fn skip(&mut self) {
        if self.active {
            self.cancel = true;
            self.skip_pending = true;
        }
    }

    fn track_at(t: &Timed<Track>, now: f64, constant: bool, locate: Locate) -> Option<(V3, V3)> {
        match &t.what {
            Track::Fixed(p) => Some((*p, [0.0, 0.0, 1.0])),
            Track::Object(id) => locate(*id),
            Track::Path(run) => Some(run.at(run.param(secs(now - t.since as f64), constant))),
        }
    }

    /// The camera at sim time `now` (in ticks, fractional): eye and the point it looks at,
    /// in game space. `None` before the script placed a shot or when a followed object is gone.
    pub fn pose(&self, now: f64, locate: Locate) -> Option<(V3, V3)> {
        if !self.active || !self.shot_placed {
            return None;
        }
        let s = self.shot.as_ref()?;
        let (eye_base, eye_dir) = Self::track_at(&s.eye, now, s.constant_speed, locate)?;
        let (off, off_dir) = s.offset.what.at(secs(now - s.offset.since as f64));
        let off = if s.relative {
            let f = norm(eye_dir);
            let f = if len(f) < 0.5 { [0.0, 0.0, 1.0] } else { f };
            let mut right = norm(cross([0.0, 1.0, 0.0], f));
            if len(right) < 0.5 {
                right = [1.0, 0.0, 0.0];
            }
            let up = cross(f, right);
            add(add(scale(right, off[0]), scale(up, off[1])), scale(f, off[2]))
        } else {
            off
        };
        let eye = add(eye_base, off);
        let look = match &s.target {
            Some(t) => Self::track_at(t, now, false, locate)?.0,
            None => {
                let dir = if s.along_offset { off_dir } else { eye_dir };
                let dir = if len(dir) > 1e-9 { norm(dir) } else { [0.0, 0.0, 1.0] };
                add(eye, dir)
            }
        };
        Some((eye, look))
    }
}

#[cfg(test)]
#[path = "cineractive/tests.rs"]
mod tests;
