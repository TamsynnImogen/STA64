//! Playtest capture (TOOL-14): a folder that holds what a fix agent needs to
//! reproduce a moment of a real game, and the code that reads it back and
//! checks it still reproduces (docs/TESTBENCH.md, "Playtest captures").
//!
//! The game writes a capture on a hotkey (`sta64/src/capture.rs`); everything
//! about the file layout lives here, Bevy-free, so `cargo test` and the
//! `sta-bench capture` commands share it. Nothing here is a new format for
//! game state: the replay is a [`Replay`] file (`replay.star`, the start
//! configuration, the command log and digests) and the snapshot is a
//! [`Sim::snapshot`] (`snapshot.sta`). The index adds what those do not know:
//! where the player was looking, what was selected, which mods were mounted.
//!
//! ```text
//! <data dir>/sta64/captures/<YYYYMMDD-HHMMSS>-t<tick>/
//!   capture.json   the index (machine-readable; `CaptureIndex`)
//!   capture.md     the same, for people
//!   replay.star    the game from tick 0 to the capture tick
//!   snapshot.sta   the sim at the capture tick (absent when the source cannot)
//!   note.txt       the player's words (STA64_CAPTURE_NOTE, else empty)
//! ```
//!
//! A capture is read-only data about a game; taking one never steps or edits
//! the sim, so it cannot change a digest.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, OdfIndex, Races};
use sta_sim::replay::{Replay, ReplayError};
use sta_sim::snapshot::SnapshotError;
use sta_sim::{Sim, Tick};

/// Bumped when [`CaptureIndex`] changes incompatibly.
pub const FORMAT: u32 = 1;

pub const INDEX_FILE: &str = "capture.json";
pub const REPORT_FILE: &str = "capture.md";
pub const REPLAY_FILE: &str = "replay.star";
pub const SNAPSHOT_FILE: &str = "snapshot.sta";
pub const NOTE_FILE: &str = "note.txt";

/// How the game's sim was made from its setup, which the replay's header does
/// not say. A skirmish places each seat's starting forces
/// (`Sim::from_setup_with_starts`); the map view and the bench do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartMode {
    /// `Sim::from_setup`.
    Plain,
    /// `Sim::from_setup_with_starts` over the install's races.
    Starts,
}

/// One seat of the game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeatInfo {
    pub id: u8,
    pub team: u8,
    pub race: String,
    /// `human`, `remote`, `ai:<name>` or `idle`.
    pub control: String,
}

/// One mounted layer of the VFS, top first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountLayer {
    pub name: String,
    pub root: String,
}

/// What was mounted, enough to mount it again on this machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MountInfo {
    /// The install root (the folder holding `Data`), when it could be told.
    pub install_root: Option<String>,
    /// The active mod (a folder under `Data/Mods`), `None` for stock.
    pub active_mod: Option<String>,
    /// Every layer, highest priority first: the mount order.
    pub layers: Vec<MountLayer>,
}

/// `p` as an absolute path, so a capture checked from another directory finds it.
fn absolute(p: &Path) -> String {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).display().to_string()
}

impl MountInfo {
    /// Reads the mount back from a VFS's layers, `(name, root)` highest
    /// priority first. A mod chain is `Data/Mods/<name>` over `Data`; a flat
    /// root is the single layer `root`.
    pub fn from_layers<'a>(layers: impl IntoIterator<Item = (&'a str, &'a Path)>) -> MountInfo {
        let layers: Vec<(&str, &Path)> = layers.into_iter().collect();
        let active_mod = layers
            .iter()
            .find(|(n, _)| !matches!(*n, "Data" | "root"))
            .map(|(n, _)| (*n).to_owned());
        let install_root = layers
            .iter()
            .find(|(n, _)| *n == "Data")
            .and_then(|(_, r)| r.parent())
            .or_else(|| layers.iter().find(|(n, _)| *n == "root").map(|(_, r)| *r))
            .or_else(|| {
                // A standalone mod mounts without `Data`: `<install>/Data/Mods/<name>`.
                layers.first().and_then(|(_, r)| r.ancestors().nth(3))
            })
            .map(absolute);
        MountInfo {
            install_root,
            active_mod,
            layers: layers
                .into_iter()
                .map(|(n, r)| MountLayer { name: n.to_owned(), root: absolute(r) })
                .collect(),
        }
    }
}

/// The program that wrote the capture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub version: String,
    /// The git commit it was built from, when the build knew it.
    pub commit: Option<String>,
    pub os: String,
    pub arch: String,
    pub debug: bool,
}

/// A selected (or hovered) unit, as the sim's view had it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitInfo {
    pub id: u32,
    /// The unit definition's ODF name; `None` for a unit not made from content.
    pub def: Option<String>,
    pub owner: u8,
    pub race: Option<String>,
    pub pos: [f32; 3],
    pub heading: [f32; 2],
    /// `[now, max]`.
    pub hull: [f32; 2],
    /// `UnitView::flags` bits, and their names.
    pub flags: u8,
    pub flag_names: Vec<String>,
    pub progress: Option<f32>,
}

/// Where the camera was, in the renderer's space (Bevy: Y up, Z mirrored from
/// the sim's) and as the map camera's orbit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraPose {
    pub translation: [f32; 3],
    /// Quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub focus: Option<[f32; 3]>,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
    pub distance: Option<f32>,
}

/// Everything a capture says besides the replay and the snapshot. The 64-bit
/// values are hex strings (`0x...`): JSON numbers do not hold them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureIndex {
    pub format: u32,
    /// Seconds since the Unix epoch, UTC.
    pub created_unix: u64,
    /// The tick the capture was taken at: the sim had stepped this many times.
    pub tick: Tick,
    /// `Sim::digest` at [`CaptureIndex::tick`].
    pub digest: String,
    pub map: String,
    pub seed: String,
    pub content_hash: String,
    /// The content profile (`Fo4`, `Sta2Classic`, ...).
    pub profile: String,
    pub start: StartMode,
    pub players: Vec<SeatInfo>,
    /// The seat the player controlled, if any.
    pub local_seat: Option<u8>,
    pub mount: MountInfo,
    pub build: BuildInfo,
    pub units_total: usize,
    pub selection: Vec<UnitInfo>,
    pub hovered: Option<UnitInfo>,
    /// Where the cursor pointed on the ground plane, sim map space.
    pub cursor_world: Option<[f32; 3]>,
    pub camera: Option<CameraPose>,
    /// Commands in the replay.
    pub commands: usize,
    pub has_snapshot: bool,
}

impl CaptureIndex {
    /// An index for `replay` as it stands at `tick` with `digest`: the game's
    /// own facts filled in from the replay's header and `content`, the
    /// player's view (selection, cursor, camera) empty for the caller to set.
    pub fn new(replay: &Replay, content: &ContentDb, tick: Tick, digest: u64, start: StartMode, created_unix: u64, build: BuildInfo) -> CaptureIndex {
        use sta_sim::replay::ControlRec;
        CaptureIndex {
            format: FORMAT,
            created_unix,
            tick,
            digest: hex(digest),
            map: replay.map.name.clone(),
            seed: hex(replay.seed),
            content_hash: hex(replay.content_hash),
            profile: format!("{:?}", content.profile()),
            start,
            players: replay
                .players
                .iter()
                .map(|p| SeatInfo {
                    id: p.id.0,
                    team: p.team.0,
                    race: p.race.clone(),
                    control: match &p.control {
                        ControlRec::Human => "human".into(),
                        ControlRec::Remote => "remote".into(),
                        ControlRec::Ai(n) | ControlRec::RemoteAi(n) => format!("ai:{n}"),
                        ControlRec::Idle => "idle".into(),
                    },
                })
                .collect(),
            local_seat: replay.players.iter().find(|p| p.control == ControlRec::Human).map(|p| p.id.0),
            mount: MountInfo::default(),
            build,
            units_total: 0,
            selection: Vec::new(),
            hovered: None,
            cursor_world: None,
            camera: None,
            commands: replay.commands.len(),
            has_snapshot: false,
        }
    }
}

/// Seconds since the Unix epoch now.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `0x` and 16 hex digits.
pub fn hex(v: u64) -> String {
    format!("{v:#018x}")
}

/// The value of [`hex`] (or a plain decimal).
pub fn parse_hex(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Why a capture could not be written, read or reproduced.
#[derive(Debug)]
pub enum CaptureError {
    Io(String),
    /// `capture.json` is missing or unreadable.
    Index(String),
    Replay(ReplayError),
    Snapshot(SnapshotError),
    /// A skirmish capture needs the install's races, and none were given.
    NeedsRaces,
    /// The snapshot is not the sim the index says it is.
    SnapshotDigest { expected: u64, found: u64 },
    /// Mounting the install or loading its content failed.
    Content(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Io(e) => write!(f, "{e}"),
            CaptureError::Index(e) => write!(f, "capture index: {e}"),
            CaptureError::Replay(e) => write!(f, "{e}"),
            CaptureError::Snapshot(e) => write!(f, "snapshot: {e}"),
            CaptureError::NeedsRaces => write!(f, "this capture is of a skirmish: it needs the install's races to place the starting forces"),
            CaptureError::SnapshotDigest { expected, found } => {
                write!(f, "snapshot digest {found:#018x}, the index says {expected:#018x}")
            }
            CaptureError::Content(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<ReplayError> for CaptureError {
    fn from(e: ReplayError) -> Self {
        CaptureError::Replay(e)
    }
}

/// A capture, in memory.
#[derive(Debug, Clone)]
pub struct Capture {
    pub index: CaptureIndex,
    pub replay: Replay,
    pub snapshot: Option<Vec<u8>>,
    pub note: String,
}

/// `$STA64_CAPTURE_DIR`, else `$XDG_DATA_HOME/sta64/captures`, `~/.local/share/sta64/captures`,
/// `%LOCALAPPDATA%\sta64\captures` or `~/Library/Application Support/sta64/captures`.
/// Never inside a game install.
pub fn default_capture_dir() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    // `STA64_CAPTURE_DIR` names the folder that holds the captures themselves.
    if let Some(dir) = var("STA64_CAPTURE_DIR") {
        return dir;
    }
    let base = var("XDG_DATA_HOME")
        .or_else(|| var("LOCALAPPDATA"))
        .or_else(|| {
            var("HOME").map(|h| {
                if cfg!(target_os = "macos") { h.join("Library/Application Support") } else { h.join(".local/share") }
            })
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("sta64/captures")
}

/// `20261112-143015` for a Unix time (UTC).
pub fn stamp(unix: u64) -> String {
    let (days, secs) = (unix / 86_400, unix % 86_400);
    // Civil date from days since 1970-01-01 (Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

impl Capture {
    /// Writes the capture under `root` as a new `<stamp>-t<tick>` folder
    /// (`-2`, `-3`, ... when that exists) and returns it.
    pub fn write(&self, root: &Path) -> Result<PathBuf, CaptureError> {
        let io = |what: &str, e: std::io::Error| CaptureError::Io(format!("{what}: {e}"));
        std::fs::create_dir_all(root).map_err(|e| io(&root.display().to_string(), e))?;
        let base = format!("{}-t{}", stamp(self.index.created_unix), self.index.tick);
        let mut dir = root.join(&base);
        let mut n = 1;
        // `create_dir` (not `create_dir_all`) so two captures in one second never share.
        while let Err(e) = std::fs::create_dir(&dir) {
            if e.kind() != std::io::ErrorKind::AlreadyExists || n > 99 {
                return Err(io(&dir.display().to_string(), e));
            }
            n += 1;
            dir = root.join(format!("{base}-{n}"));
        }
        self.write_into(&dir)?;
        Ok(dir)
    }

    /// Writes the files into the existing folder `dir`.
    pub fn write_into(&self, dir: &Path) -> Result<(), CaptureError> {
        let put = |name: &str, bytes: &[u8]| {
            std::fs::write(dir.join(name), bytes).map_err(|e| CaptureError::Io(format!("{}: {e}", dir.join(name).display())))
        };
        let mut index = self.index.clone();
        index.has_snapshot = self.snapshot.is_some();
        index.commands = self.replay.commands.len();
        put(REPLAY_FILE, &self.replay.encode())?;
        if let Some(s) = &self.snapshot {
            put(SNAPSHOT_FILE, s)?;
        }
        put(NOTE_FILE, self.note.as_bytes())?;
        put(REPORT_FILE, report(&index).as_bytes())?;
        let json = json5::to_string(&index).map_err(|e| CaptureError::Index(e.to_string()))?;
        // The index goes last: a folder without it is an unfinished capture.
        put(INDEX_FILE, json.as_bytes())
    }

    /// Reads a capture folder.
    pub fn load(dir: &Path) -> Result<Capture, CaptureError> {
        let read = |name: &str| std::fs::read(dir.join(name)).map_err(|e| CaptureError::Io(format!("{}: {e}", dir.join(name).display())));
        let text = read(INDEX_FILE).map_err(|e| CaptureError::Index(e.to_string()))?;
        let index: CaptureIndex = json5::from_str(&String::from_utf8_lossy(&text)).map_err(|e| CaptureError::Index(e.to_string()))?;
        if index.format != FORMAT {
            return Err(CaptureError::Index(format!("format {}, expected {FORMAT}", index.format)));
        }
        let replay = Replay::decode(&read(REPLAY_FILE)?)?;
        let snapshot = index.has_snapshot.then(|| read(SNAPSHOT_FILE)).transpose()?;
        let note = std::fs::read_to_string(dir.join(NOTE_FILE)).unwrap_or_default();
        Ok(Capture { index, replay, snapshot, note })
    }

    /// The recorded digest at the capture tick.
    pub fn digest(&self) -> Result<u64, CaptureError> {
        parse_hex(&self.index.digest).ok_or_else(|| CaptureError::Index(format!("digest `{}`", self.index.digest)))
    }

    /// The game's sim at the capture tick, made by replaying from tick 0.
    ///
    /// `content` must be the content the game ran on (its hash is checked
    /// against the replay's). `races` is needed for a skirmish capture
    /// ([`StartMode::Starts`]). Every recorded digest up to the capture tick
    /// is checked on the way, and the result's digest against the index's.
    pub fn replay_to_tick(&self, content: Arc<ContentDb>, races: Option<&Races>) -> Result<Sim, CaptureError> {
        let setup = self.replay.setup(content)?;
        let sim = match self.index.start {
            StartMode::Plain => Sim::from_setup(&setup),
            StartMode::Starts => Sim::from_setup_with_starts(&setup, races.ok_or(CaptureError::NeedsRaces)?).0,
        };
        let sim = self.replay.play_from(sim, self.index.tick)?;
        let expected = self.digest()?;
        let found = sim.digest();
        if found != expected {
            return Err(CaptureError::Replay(ReplayError::Desync { tick: self.index.tick, expected, found }));
        }
        Ok(sim)
    }

    /// The sim restored from the snapshot, with its digest checked against the
    /// index's. `None` when the capture has no snapshot.
    pub fn sim_from_snapshot(&self, content: Arc<ContentDb>) -> Result<Option<Sim>, CaptureError> {
        let Some(bytes) = &self.snapshot else { return Ok(None) };
        let sim = Sim::from_snapshot(bytes, Some(content)).map_err(CaptureError::Snapshot)?;
        let expected = self.digest()?;
        let found = sim.digest();
        if found != expected {
            return Err(CaptureError::SnapshotDigest { expected, found });
        }
        Ok(Some(sim))
    }

    /// Reproduces the capture: replays to its tick and compares digests, and
    /// restores the snapshot and compares that too.
    pub fn verify(&self, content: Arc<ContentDb>, races: Option<&Races>) -> Result<Verified, CaptureError> {
        let sim = self.replay_to_tick(content.clone(), races)?;
        let snapshot_checked = self.sim_from_snapshot(content)?.is_some();
        Ok(Verified { tick: self.index.tick, digest: sim.digest(), commands: self.replay.commands.len(), snapshot_checked })
    }
}

/// What [`Capture::verify`] confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub tick: Tick,
    pub digest: u64,
    pub commands: usize,
    pub snapshot_checked: bool,
}

/// The content and races of an install, as the game loads them
/// (`ContentDb::load`, `Races::load` over `Vfs::mount_install`).
pub fn load_install(root: &Path, active_mod: Option<&str>) -> Result<(Arc<ContentDb>, Races), CaptureError> {
    let vfs = sta_vfs::Vfs::mount_install(root, active_mod).map_err(|e| CaptureError::Content(format!("mounting {}: {e}", root.display())))?;
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).map_err(|e| CaptureError::Content(format!("content: {e}")))?;
    let (races, _) = Races::load(&vfs, &index);
    Ok((Arc::new(db), races))
}

/// `capture.md`.
pub fn report(i: &CaptureIndex) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# STA64 playtest capture\n");
    let _ = writeln!(s, "- taken: {} UTC, at tick {} (digest {})", stamp(i.created_unix), i.tick, i.digest);
    let _ = writeln!(s, "- map: `{}`, seed {}, profile {}, content {}", i.map, i.seed, i.profile, i.content_hash);
    let _ = writeln!(s, "- start: {:?}; {} units; {} commands in the replay", i.start, i.units_total, i.commands);
    let _ = writeln!(
        s,
        "- build: {} {} ({}/{}{})",
        i.build.version,
        i.build.commit.as_deref().unwrap_or("commit unknown"),
        i.build.os,
        i.build.arch,
        if i.build.debug { ", debug" } else { "" }
    );
    let _ = writeln!(s, "\n## Players\n");
    for p in &i.players {
        let me = if i.local_seat == Some(p.id) { "  <- the player" } else { "" };
        let _ = writeln!(s, "- seat {} team {} race {} ({}){me}", p.id, p.team, p.race, p.control);
    }
    let _ = writeln!(s, "\n## Mount (highest priority first)\n");
    let _ = writeln!(
        s,
        "install `{}`, mod `{}`\n",
        i.mount.install_root.as_deref().unwrap_or("?"),
        i.mount.active_mod.as_deref().unwrap_or("(stock)")
    );
    for l in &i.mount.layers {
        let _ = writeln!(s, "1. {} (`{}`)", l.name, l.root);
    }
    let unit = |u: &UnitInfo| {
        format!(
            "#{} {} owner {}{} at ({:.0}, {:.0}, {:.0}) hull {:.0}/{:.0}{}",
            u.id,
            u.def.as_deref().unwrap_or("(no definition)"),
            u.owner,
            u.race.as_ref().map(|r| format!(" ({r})")).unwrap_or_default(),
            u.pos[0],
            u.pos[1],
            u.pos[2],
            u.hull[0],
            u.hull[1],
            if u.flag_names.is_empty() { String::new() } else { format!(" [{}]", u.flag_names.join(", ")) }
        )
    };
    let _ = writeln!(s, "\n## Selection ({})\n", i.selection.len());
    for u in &i.selection {
        let _ = writeln!(s, "- {}", unit(u));
    }
    if let Some(u) = &i.hovered {
        let _ = writeln!(s, "\nUnder the cursor: {}", unit(u));
    }
    if let Some(c) = i.cursor_world {
        let _ = writeln!(s, "\nCursor on the ground: ({:.0}, {:.0}, {:.0}) in map space.", c[0], c[1], c[2]);
    }
    if let Some(c) = &i.camera {
        let _ = writeln!(s, "\nCamera: at {:?}, rotation {:?}", c.translation, c.rotation);
        if let (Some(f), Some(d)) = (c.focus, c.distance) {
            let _ = writeln!(s, "focus {f:?}, distance {d:.0}, yaw {:?}, pitch {:?}", c.yaw, c.pitch);
        }
    }
    let _ = writeln!(
        s,
        "\n## Reproduce\n\n```sh\nsta-bench capture check <this folder>   # replays to tick {} and compares the digest\n```\n\nSee docs/TESTBENCH.md, \"Playtest captures\", for turning it into a regression test. \
         `note.txt` holds the player's description.",
        i.tick
    );
    s
}

/// Capture folders under `root`, newest first.
pub fn list(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(INDEX_FILE).is_file())
        .collect();
    // Names start with a UTC stamp, so name order is time order.
    dirs.sort();
    dirs.reverse();
    dirs
}
