//! The front end's scene (UI-06): a live map behind the menus, on one map under one
//! sun, with named views, a decloak and a departure to warp.
//!
//! Follows Armada II Refit's menu scene (`render3d/core/src/menu/mod.rs`, the map
//! `render3d/content/menu/map.ini`; see `assets/menu/NOTICE.md`). The screens ask for a
//! **view** by name (`main`, and on the campaign selection `tutorials`, `federation`,
//! `klingon`, `borg`) and the camera moves over to it; a group with a `CloakView` is
//! cloaked until that view is asked for, then its ships decloak one after the other; a
//! **departure** sends the view's ships to warp, front first, while the camera swings
//! behind them.
//!
//! # Layers
//!
//! - [`MenuMap`]: `map.ini` parsed (no Bevy). The built-in copy is `assets/menu/map.ini`;
//!   a `Menu/map.ini` in the mounted install takes precedence, as in Refit.
//! - [`Director`]: the scene's clock and state (no Bevy): the camera with its sway and
//!   the blend between views, each ship's pose, cloak and departure, and the marks of
//!   what happened. Deterministic in the scene's time.
//! - [`build`]: the map's ships as a [`MapDef`] over a [`ContentDb`], a [`GameSetup`] and
//!   a sim, so the ships are drawn by the same path as a match's (RND-02's
//!   `sync_units`, the models, the cloak fade). A ship whose ODF the content does not
//!   have is stood in for by one it does.
//! - [`MenuScenePlugin`]: the scene in the Bevy app: sky, stars, sun, planet and lights,
//!   the camera, and a system that poses the ships and the camera from the [`Director`]
//!   every frame and writes its marks into [`RenderMarks`].
//!
//! # For INT-M4
//!
//! Add [`MenuScenePlugin`] once. Insert a [`MenuSceneConfig`] when the front end opens
//! and remove it when a match starts (the scene tears itself down). Ask for views with
//! the [`MenuRequest`] message. `--headless` with `STA64_MENU_SCENE=1` runs the scene on
//! its own for the bench and the frame tests ([`run_headless`]).

#![allow(dead_code)] // the screens (INT-M4) are the plugin's users; until they land only `run_headless` is

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::image::CompressedImageFormats;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_content::{ContentDb, MapDef, MapObject, OdfIndex};
use sta_sim::command::LocalRouter;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{EntityId, Fixed, PlayerId, Sim, TICK_HZ, TeamId, Tick, WorldSource};

use crate::app::GameSet;
use crate::bench::marks::RenderMarks;
use crate::clock::AnimClock;
use crate::render::fx::specials::{CLOAKED_ALPHA, ManualFade, UnitFade};
use crate::render::planet::{KEY_LUX, PlanetMaterial};
use crate::render::model_scene::PartCache;
use crate::render::sprites::SpriteTables;
use crate::render::textures::{Loader, TextureCache};
use crate::render::units::{FromView, LocalPlayer, NeedsModel, to_bevy};
use crate::sim_link::{ContentRes, SimEntity, SimLink};
use crate::viewer::Mounted;

/// The built-in map: Refit's `render3d/content/menu/map.ini`, unchanged.
pub const BUILTIN_MAP: &str = include_str!("../../../../assets/menu/map.ini");

const DEG: f32 = std::f32::consts::PI / 180.0;

// ---------------------------------------------------------------------------------
// The map file
// ---------------------------------------------------------------------------------

/// Sections of an ini in file order, keys lower-cased.
struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    fn parse(text: &str) -> Ini {
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for line in text.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                sections.push((name.trim().to_ascii_lowercase(), Vec::new()));
            } else if let (Some((k, v)), Some(sec)) = (line.split_once('='), sections.last_mut()) {
                sec.1.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
            }
        }
        Ini { sections }
    }

    fn sec(&self, name: &str) -> Option<&Vec<(String, String)>> {
        self.sections.iter().find(|s| s.0 == name).map(|s| &s.1)
    }
}

fn get<'a>(sec: &'a [(String, String)], key: &str) -> Option<&'a str> {
    sec.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn num(sec: &[(String, String)], key: &str, default: f32) -> f32 {
    get(sec, key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn vec3(sec: &[(String, String)], key: &str) -> Option<Vec3> {
    let v: Vec<f32> = get(sec, key)?.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

/// `[Warp]`: a departure's timings, seconds unless said.
#[derive(Debug, Clone, PartialEq)]
pub struct WarpDef {
    pub delay: f32,
    pub stagger: f32,
    pub ramp: f32,
    pub stretch: f32,
    pub speed: f32,
    pub turn: f32,
    pub follow: f32,
    pub tunnel_at: f32,
    pub tunnel_in: f32,
    pub dim: f32,
    pub widen: f32,
    pub flash: f32,
    pub flash_time: f32,
}

/// `[Planet]`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanetDef {
    pub class: String,
    pub radius: f32,
    pub at: Vec3,
    pub axis: Vec3,
    pub spin: f32,
    pub clouds: f32,
    pub orbit: f32,
}

/// `[Group.*]`: a place on the map; its ships, gas and views are in its frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub at: Vec3,
    /// Radians.
    pub yaw: f32,
    /// The view on whose arrival the group's ships decloak.
    pub cloak_view: Option<String>,
    pub cloak_delay: f32,
    pub cloak_time: f32,
    pub cloak_stagger: f32,
    /// Stays cloaked while its view is locked.
    pub locked_cloak: bool,
    pub locked_yaw: f32,
}

impl Group {
    /// A point of the group's frame, on the map.
    pub fn place(&self, p: Vec3) -> Vec3 {
        let (s, c) = self.yaw.sin_cos();
        self.at + Vec3::new(p.x * c + p.z * s, p.y, -p.x * s + p.z * c)
    }
}

/// `[Ship.*]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipDef {
    pub name: String,
    pub odf: String,
    pub group: usize,
    /// Which of its group's ships it is, for the cloak's stagger.
    pub nth: usize,
    pub at: Vec3,
    pub yaw: f32,
    pub bob: f32,
    pub phase: f32,
    /// Goes to warp when its group departs (`Warp=0`: a station stays).
    pub warp: bool,
    pub locked_at: Option<Vec3>,
    pub locked_yaw: Option<f32>,
}

/// `[Nebula.*]`: kept for the gas renderer (RND-05); the scene does not draw it yet.
#[derive(Debug, Clone, PartialEq)]
pub struct NebulaDef {
    pub name: String,
    pub odf: String,
    pub group: usize,
    pub at: Vec3,
    pub size: Vec3,
}

/// `[View.*]`: a camera of a group's frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewDef {
    pub name: String,
    pub group: usize,
    pub eye: Vec3,
    pub target: Vec3,
    pub fov: f32,
    pub sway: Vec3,
    pub sway_period: f32,
    /// Radians a second about the vertical axis through the target.
    pub orbit: f32,
    pub warp_eye: Option<Vec3>,
    pub warp_target: Option<Vec3>,
}

/// The menu's map.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuMap {
    pub sky: String,
    /// Radians a second about +x.
    pub sky_turn: f32,
    pub stars: u32,
    pub star_size: f32,
    pub star_seed: u32,
    /// Where the key light travels (the sun is where it comes from).
    pub key_axis: Vec3,
    pub key_colour: Vec3,
    pub fill_axis: Vec3,
    pub fill_colour: Vec3,
    /// Seconds a change of view takes.
    pub blend: f32,
    pub warp: WarpDef,
    pub planet: Option<PlanetDef>,
    /// Group 0 is the map's origin, for sections that name none.
    pub groups: Vec<Group>,
    pub ships: Vec<ShipDef>,
    pub nebulae: Vec<NebulaDef>,
    pub views: Vec<ViewDef>,
}

impl MenuMap {
    /// The built-in map (Refit's, copied).
    pub fn builtin() -> MenuMap {
        MenuMap::parse(BUILTIN_MAP)
    }

    /// The install's `Menu/map.ini` when it has one, else the built-in map.
    pub fn from_vfs(vfs: &sta_vfs::Vfs) -> MenuMap {
        for path in ["menu/map.ini", "Menu/map.ini", "Menu/Map.ini"] {
            if let Some(Ok(bytes)) = vfs.read(path) {
                return MenuMap::parse(&String::from_utf8_lossy(&bytes));
            }
        }
        MenuMap::builtin()
    }

    pub fn parse(text: &str) -> MenuMap {
        let ini = Ini::parse(text);
        let empty = Vec::new();
        let scene = ini.sec("scene").unwrap_or(&empty);
        let w = ini.sec("warp").unwrap_or(&empty);
        let blend = num(scene, "blend", 2.0).max(0.0);
        let named = |sec: &str, kind: &str| sec.strip_prefix(kind).and_then(|r| r.strip_prefix('.')).map(str::to_owned);

        let mut groups = vec![Group {
            name: String::new(),
            at: Vec3::ZERO,
            yaw: 0.0,
            cloak_view: None,
            cloak_delay: 0.0,
            cloak_time: 1.0,
            cloak_stagger: 0.0,
            locked_cloak: false,
            locked_yaw: 0.0,
        }];
        for (sec, kv) in &ini.sections {
            let Some(name) = named(sec, "group") else { continue };
            groups.push(Group {
                name,
                at: vec3(kv, "at").unwrap_or(Vec3::ZERO),
                yaw: num(kv, "yaw", 0.0) * DEG,
                cloak_view: get(kv, "cloakview").map(str::to_ascii_lowercase),
                cloak_delay: num(kv, "cloakdelay", blend * 0.7).max(0.0),
                cloak_time: num(kv, "cloaktime", 2.5).max(0.05),
                cloak_stagger: num(kv, "cloakstagger", 0.5).max(0.0),
                locked_cloak: num(kv, "lockedcloak", 0.0) != 0.0,
                locked_yaw: num(kv, "lockedyaw", 0.0) * DEG,
            });
        }
        let group_of = |kv: &[(String, String)]| {
            let g = get(kv, "group").map(str::to_ascii_lowercase).unwrap_or_default();
            groups.iter().position(|x| x.name == g).unwrap_or(0)
        };

        let mut ships: Vec<ShipDef> = Vec::new();
        let mut nebulae = Vec::new();
        let mut views = Vec::new();
        for (sec, kv) in &ini.sections {
            if let Some(name) = named(sec, "ship") {
                let Some(odf) = get(kv, "odf") else { continue };
                let group = group_of(kv);
                let nth = ships.iter().filter(|s| s.group == group).count();
                ships.push(ShipDef {
                    name,
                    odf: odf.to_owned(),
                    group,
                    nth,
                    at: vec3(kv, "at").unwrap_or(Vec3::ZERO),
                    yaw: num(kv, "yaw", 0.0) * DEG,
                    bob: num(kv, "bob", 1.0),
                    phase: num(kv, "phase", ships.len() as f32 * 1.7),
                    warp: num(kv, "warp", 1.0) != 0.0,
                    locked_at: vec3(kv, "lockedat"),
                    locked_yaw: get(kv, "lockedyaw").and_then(|v| v.parse::<f32>().ok()).map(|d| d * DEG),
                });
            } else if let Some(name) = named(sec, "nebula") {
                let Some(odf) = get(kv, "odf") else { continue };
                nebulae.push(NebulaDef {
                    name,
                    odf: odf.to_owned(),
                    group: group_of(kv),
                    at: vec3(kv, "at").unwrap_or(Vec3::ZERO),
                    size: vec3(kv, "size").unwrap_or(Vec3::new(1500.0, 0.0, 1500.0)),
                });
            } else if let Some(name) = named(sec, "view") {
                let gi = group_of(kv);
                let g = &groups[gi];
                let placed = |k: &str| vec3(kv, k).map(|p| g.place(p));
                views.push(ViewDef {
                    name,
                    group: gi,
                    eye: placed("eye").unwrap_or_else(|| g.place(Vec3::new(0.0, 50.0, 400.0))),
                    target: placed("target").unwrap_or_else(|| g.place(Vec3::ZERO)),
                    fov: num(kv, "fov", 40.0),
                    sway: vec3(kv, "sway").unwrap_or(Vec3::new(8.0, 4.0, 6.0)),
                    sway_period: num(kv, "swayperiod", 40.0).max(1.0),
                    orbit: num(kv, "orbit", 0.0) * DEG,
                    warp_eye: placed("warpeye"),
                    warp_target: placed("warptarget"),
                });
            }
        }
        let planet = ini.sec("planet").map(|p| PlanetDef {
            class: get(p, "class").unwrap_or("").to_owned(),
            radius: num(p, "radius", 3000.0),
            at: vec3(p, "at").unwrap_or(Vec3::ZERO),
            axis: vec3(p, "axis").unwrap_or(Vec3::X).normalize_or(Vec3::Y),
            spin: num(p, "spin", 0.0),
            clouds: num(p, "clouds", 0.0),
            orbit: num(p, "orbit", 0.0),
        });
        MenuMap {
            sky: get(scene, "sky").unwrap_or("").to_owned(),
            sky_turn: num(scene, "skyturn", 0.0),
            stars: num(scene, "stars", 0.0).max(0.0) as u32,
            star_size: num(scene, "starsize", 0.001),
            star_seed: num(scene, "starseed", 1.0) as u32,
            key_axis: vec3(scene, "keyaxis").unwrap_or(Vec3::new(0.5, -0.6, 0.6)).normalize_or(Vec3::NEG_Y),
            key_colour: vec3(scene, "keycolour").unwrap_or(Vec3::ONE),
            fill_axis: vec3(scene, "fillaxis").unwrap_or(Vec3::new(-0.5, 0.3, 0.8)).normalize_or(Vec3::Z),
            fill_colour: vec3(scene, "fillcolour").unwrap_or(Vec3::new(0.1, 0.12, 0.18)),
            blend,
            warp: WarpDef {
                delay: num(w, "delay", 0.9),
                stagger: num(w, "stagger", 0.18),
                ramp: num(w, "ramp", 0.75).max(0.05),
                stretch: num(w, "stretch", 30.0),
                speed: num(w, "speed", 120.0),
                turn: num(w, "turn", 1.3).max(0.05),
                follow: num(w, "follow", 260.0),
                tunnel_at: num(w, "tunnelat", 0.1),
                tunnel_in: num(w, "tunnelin", 0.9).max(0.05),
                dim: num(w, "dim", 0.85),
                widen: num(w, "widen", 18.0),
                flash: num(w, "flash", 2.2).max(0.0),
                flash_time: num(w, "flashtime", 0.45).max(0.05),
            },
            planet,
            groups,
            ships,
            nebulae,
            views,
        }
    }

    pub fn view_index(&self, name: &str) -> Option<usize> {
        self.views.iter().position(|v| v.name.eq_ignore_ascii_case(name))
    }

    /// The names of the views, in file order.
    pub fn view_names(&self) -> Vec<&str> {
        self.views.iter().map(|v| v.name.as_str()).collect()
    }

    /// The group a ship sits in.
    fn group_of(&self, ship: &ShipDef) -> &Group {
        &self.groups[ship.group]
    }
}

// ---------------------------------------------------------------------------------
// The director
// ---------------------------------------------------------------------------------

/// A camera: eye, target (the map's space, +y up, left-handed) and vertical field in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cam {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov: f32,
}

impl Cam {
    fn lerp(self, to: Cam, k: f32) -> Cam {
        Cam { eye: self.eye.lerp(to.eye, k), target: self.target.lerp(to.target, k), fov: self.fov + (to.fov - self.fov) * k }
    }
}

fn smooth(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One ship, posed.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipPose {
    /// The map's space.
    pub pos: Vec3,
    /// About +y, radians: the nose along (sin, 0, cos).
    pub yaw: f32,
    pub roll: f32,
    pub pitch: f32,
    /// Along the nose: the warp stretch.
    pub stretch: f32,
    /// `None`: whole. `Some(k)`: `k` of the hull is back (0: cloaked).
    pub cloak: Option<f32>,
    /// Held cloaked while its campaign is locked: drawn in the cloak's glass.
    pub held: bool,
    /// Left in a departure; its flash was there.
    pub gone: bool,
}

impl ShipPose {
    /// How much of the hull to draw, 0..1, as the hull fade's opacity.
    pub fn alpha(&self) -> f32 {
        if self.gone {
            return 0.0;
        }
        if self.held {
            return CLOAKED_ALPHA;
        }
        match self.cloak {
            None => 1.0,
            Some(k) => k.clamp(0.0, 1.0),
        }
    }
}

/// A departure's tunnel: how far in it is, 0..1, and the scene's time inside the departure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tunnel {
    pub k: f32,
    pub time: f32,
}

/// What the director says about one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub cam: Cam,
    pub ships: Vec<ShipPose>,
    pub tunnel: Option<Tunnel>,
    /// The planet's turn and the sky's, radians.
    pub planet_turn: f32,
    pub sky_turn: f32,
}

/// A mark the director made, before the host stamps it with a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectorMark {
    pub time: f32,
    pub kind: &'static str,
    pub fields: Vec<(&'static str, String)>,
}

impl DirectorMark {
    /// The sim tick at which the scene's time falls.
    pub fn tick(&self) -> Tick {
        (self.time.max(0.0) * TICK_HZ as f32).round() as Tick
    }
}

#[derive(Debug, Clone)]
struct WarpRun {
    t0: f32,
    from: Option<Cam>,
    /// Ships that have started, and gone, for the marks.
    started: Vec<bool>,
    gone: Vec<bool>,
    tunnel: bool,
}

#[derive(Debug, Clone, Default)]
struct ShipState {
    cloak: Option<Option<f32>>,
}

/// The menu scene's state: which view is shown, the camera between views, the cloaks
/// and a departure.
#[derive(Debug, Clone)]
pub struct Director {
    map: Arc<MenuMap>,
    view: usize,
    prev: Option<usize>,
    since: f32,
    /// The camera of the last frame, and the one a change of view blends from.
    cam: Option<Cam>,
    from: Option<Cam>,
    /// Per group: its campaign is locked.
    locked: Vec<bool>,
    warp: Option<WarpRun>,
    last: Vec<ShipState>,
    marks: Vec<DirectorMark>,
    /// The scene's time at the last frame.
    now: f32,
}

impl Director {
    /// The scene opens on `view` (the first view of the map when it has no such one) at time 0.
    pub fn new(map: Arc<MenuMap>, view: &str) -> Director {
        let v = map.view_index(view).unwrap_or(0);
        let groups = map.groups.len();
        let ships = map.ships.len();
        let mut d = Director {
            map,
            view: v,
            prev: None,
            since: 0.0,
            cam: None,
            from: None,
            locked: vec![false; groups],
            warp: None,
            last: vec![ShipState::default(); ships],
            marks: Vec::new(),
            now: 0.0,
        };
        d.mark(0.0, "render.menu.view", vec![("view", d.view_name().to_owned()), ("from", "-".to_owned())]);
        d
    }

    pub fn map(&self) -> &MenuMap {
        &self.map
    }

    pub fn view_name(&self) -> &str {
        self.map.views.get(self.view).map_or("", |v| v.name.as_str())
    }

    fn mark(&mut self, time: f32, kind: &'static str, fields: Vec<(&'static str, String)>) {
        self.marks.push(DirectorMark { time, kind, fields });
    }

    /// The marks made since the last call.
    pub fn take_marks(&mut self) -> Vec<DirectorMark> {
        std::mem::take(&mut self.marks)
    }

    /// Ask for another view at scene time `t`: the camera moves over from where it is.
    /// False (and nothing changes) when the map has no such view.
    pub fn request_view(&mut self, name: &str, t: f32) -> bool {
        let Some(v) = self.map.view_index(name) else { return false };
        if v == self.view {
            return true;
        }
        let from = self.view_name().to_owned();
        self.prev = Some(self.view);
        self.view = v;
        self.since = t;
        self.from = self.cam;
        let to = self.view_name().to_owned();
        self.mark(t, "render.menu.view", vec![("view", to), ("from", from)]);
        true
    }

    /// Which campaigns are locked: the views named, and so the groups they look at.
    pub fn set_locked(&mut self, views: &[String]) {
        self.locked = vec![false; self.map.groups.len()];
        for v in self.map.views.iter().filter(|v| views.iter().any(|l| l.eq_ignore_ascii_case(&v.name))) {
            self.locked[v.group] = true;
        }
    }

    /// Begin a departure from the view shown, at scene time `t` (a mission is loading).
    pub fn depart(&mut self, t: f32) {
        if self.warp.is_some() {
            return;
        }
        let n = self.map.ships.len();
        let view = self.view_name().to_owned();
        self.warp = Some(WarpRun { t0: t, from: self.cam, started: vec![false; n], gone: vec![false; n], tunnel: false });
        self.mark(t, "render.menu.warp", vec![("state", "start".to_owned()), ("view", view)]);
    }

    pub fn departing(&self) -> bool {
        self.warp.is_some()
    }

    /// The camera of a view at `t`, with its sway: Refit's `Map::camera`.
    fn camera(v: &ViewDef, t: f32) -> Cam {
        let w = std::f32::consts::TAU / v.sway_period;
        let s = v.sway;
        let d = Vec3::new(s.x * (w * t).sin(), s.y * (w * t * 0.73 + 1.1).sin(), s.z * (w * t * 0.51 + 2.3).sin());
        let (mut eye, target) = (v.eye + d, v.target + d * 0.3);
        if v.orbit != 0.0 {
            let (sn, cs) = (v.orbit * t).sin_cos();
            let r = eye - target;
            eye = target + Vec3::new(r.x * cs + r.z * sn, r.y, -r.x * sn + r.z * cs);
        }
        Cam { eye, target, fov: v.fov }
    }

    /// How far the ship `nth` of a decloaking group is back: Refit's `cloak_of`.
    fn cloak_of(&self, g: &Group, nth: usize, t: f32) -> Option<f32> {
        let cv = g.cloak_view.as_deref()?;
        let gone = t - self.since;
        let shown = self.view_name().to_ascii_lowercase();
        if cv == shown {
            let k = (gone - g.cloak_delay - nth as f32 * g.cloak_stagger) / g.cloak_time;
            return if k >= 1.0 { None } else { Some(k.max(0.0)) };
        }
        // Leaving its view, the group stays as it was until the camera has gone.
        let prev = self.prev.and_then(|p| self.map.views.get(p)).map(|v| v.name.to_ascii_lowercase());
        if prev.as_deref() == Some(cv) && gone < self.map.blend {
            return None;
        }
        Some(0.0)
    }

    /// The departure `tw` seconds in: each leaving ship's clock, the way out, the camera
    /// behind them and the tunnel. Refit's `Map::departure`.
    fn departure(&self, vd: &ViewDef, tw: f32) -> Departure {
        let m = &self.map;
        let g = &m.groups[vd.group];
        let wd = &m.warp;
        let going: Vec<usize> = (0..m.ships.len()).filter(|&i| m.ships[i].group == vd.group && m.ships[i].warp).collect();
        let place = |i: usize| g.place(m.ships[i].at);
        let nose = |i: usize| {
            let yw = m.ships[i].yaw + g.yaw;
            Vec3::new(yw.sin(), 0.0, yw.cos())
        };
        let mut dir = Vec3::ZERO;
        let mut centre = Vec3::ZERO;
        for &i in &going {
            dir += nose(i);
            centre += place(i);
        }
        let mut tau = vec![None; m.ships.len()];
        let (eye, target, last);
        if going.is_empty() {
            dir = (vd.target - vd.eye).normalize_or(Vec3::Z);
            eye = vd.eye;
            target = vd.target;
            last = wd.delay;
        } else {
            dir = if dir.length() > 1e-3 { dir.normalize() } else { Vec3::Z };
            centre /= going.len() as f32;
            let spread = going.iter().map(|&i| (place(i) - centre).length()).fold(0.0f32, f32::max);
            let mut order = going.clone();
            order.sort_by(|&a, &b| place(b).dot(dir).partial_cmp(&place(a).dot(dir)).unwrap_or(std::cmp::Ordering::Equal));
            for (k, &i) in order.iter().enumerate() {
                let t0 = wd.delay + k as f32 * wd.stagger;
                if tw >= t0 {
                    tau[i] = Some(tw - t0);
                }
            }
            last = wd.delay + (order.len() - 1) as f32 * wd.stagger + wd.ramp;
            eye = vd.warp_eye.unwrap_or(centre - dir * (spread + 520.0) + Vec3::new(0.0, spread * 0.25 + 90.0, 0.0));
            target = vd.warp_target.unwrap_or(centre + dir * (spread + 900.0));
        }
        let swing = smooth(tw / wd.turn);
        let tf = (tw - wd.delay - wd.ramp * 0.3).max(0.0);
        let follow = 0.5 * wd.follow * tf * tf;
        let k = smooth((tw - last - wd.tunnel_at) / wd.tunnel_in);
        Departure { dir, tau, eye, target, swing, follow, tunnel: (k > 0.0).then_some(Tunnel { k, time: tw }) }
    }

    /// The scene at time `t` (never going back). Makes the marks of what changed since the
    /// frame before.
    pub fn frame(&mut self, t: f32) -> Frame {
        self.now = t;
        let map = self.map.clone();
        let Some(vd) = map.views.get(self.view) else {
            let cam = Cam { eye: Vec3::new(0.0, 50.0, 400.0), target: Vec3::ZERO, fov: 40.0 };
            return Frame { cam, ships: Vec::new(), tunnel: None, planet_turn: 0.0, sky_turn: 0.0 };
        };
        let mut cam = Self::camera(vd, t);
        if let Some(from) = self.from {
            let k = if map.blend > 0.0 { smooth((t - self.since) / map.blend) } else { 1.0 };
            cam = from.lerp(cam, k);
            if k >= 1.0 {
                self.from = None;
            }
        }
        let dep = self.warp.as_ref().map(|w| {
            let tw = t - w.t0;
            let d = self.departure(vd, tw);
            let from = w.from.unwrap_or(cam);
            // Over the top as it swings, so the camera does not pass through the ships.
            let lift = Vec3::new(0.0, (d.swing * std::f32::consts::PI).sin() * 320.0, 0.0);
            let off = d.dir * d.follow;
            let wk = d.tunnel.map_or(0.0, |tn| tn.k);
            let swung = Cam {
                eye: from.eye.lerp(d.eye, d.swing) + lift + off,
                target: from.target.lerp(d.target, d.swing) + off,
                fov: from.fov + (vd.fov - from.fov) * d.swing + map.warp.widen * wk,
            };
            (d, swung)
        });
        if let Some((_, swung)) = &dep {
            cam = *swung;
        } else {
            self.cam = Some(cam);
        }
        let locked: Vec<bool> = if dep.is_some() { vec![false; map.groups.len()] } else { self.locked.clone() };

        let mut ships = Vec::with_capacity(map.ships.len());
        for (si, s) in map.ships.iter().enumerate() {
            let g = map.group_of(s);
            let lk = locked.get(s.group).copied().unwrap_or(false);
            // Each ship bobs, rolls and pitches a little on a period of its own.
            let ph = s.phase + t * 0.21;
            let bob = Vec3::new(0.0, (ph).sin() * 1.5, (ph * 0.7).sin() * 0.8) * s.bob;
            let roll = (ph * 0.8 + 0.5).sin() * 0.02 * s.bob;
            let pitch = (ph * 0.6 + 1.3).sin() * 0.012 * s.bob;
            let own = if lk { s.locked_yaw.unwrap_or(s.yaw + g.locked_yaw) } else { s.yaw };
            let mut pos = g.place(if lk { s.locked_at.unwrap_or(s.at) } else { s.at }) + bob;
            let (mut stretch, mut gone) = (1.0, false);
            if let Some((d, _)) = &dep
                && let Some(tau) = d.tau[si]
            {
                let wd = &map.warp;
                let tr = tau.min(wd.ramp);
                pos += d.dir * (0.5 * wd.speed * tr * tr);
                if tau >= wd.ramp {
                    gone = true;
                } else {
                    let b = smooth(tau / wd.ramp);
                    stretch = 1.0 + wd.stretch * b * b;
                }
            }
            let held = lk && g.locked_cloak;
            let cloak = if held { Some(0.0) } else { self.cloak_of(g, s.nth, t) };
            ships.push(ShipPose { pos, yaw: own + g.yaw, roll, pitch, stretch, cloak, held, gone });
        }
        self.note_changes(&ships, dep.as_ref().map(|(d, _)| d), t);

        let planet_turn = map.planet.as_ref().map_or(0.0, |p| -p.orbit * t * (1.0 + p.spin));
        Frame { cam, ships, tunnel: dep.and_then(|(d, _)| d.tunnel), planet_turn, sky_turn: map.sky_turn * t }
    }

    /// Marks for what the ships did since the last frame: a decloak begun and done, a
    /// cloak again, a ship away to warp and gone, the tunnel.
    fn note_changes(&mut self, ships: &[ShipPose], dep: Option<&Departure>, t: f32) {
        let map = self.map.clone();
        let hidden = |k: Option<f32>| matches!(k, Some(k) if k <= 0.0);
        let mut out: Vec<(&'static str, Vec<(&'static str, String)>)> = Vec::new();
        for (i, p) in ships.iter().enumerate() {
            let s = &map.ships[i];
            let gname = map.group_of(s).name.clone();
            if let Some(last) = self.last[i].cloak
                && !p.held
            {
                let was_hidden = hidden(last);
                let now_hidden = hidden(p.cloak);
                let ship = |state: &str| vec![("ship", s.name.clone()), ("group", gname.clone()), ("state", state.to_owned())];
                if was_hidden && !now_hidden {
                    out.push(("render.menu.decloak", ship("start")));
                }
                if last.is_some() && p.cloak.is_none() {
                    out.push(("render.menu.decloak", ship("done")));
                }
                if !was_hidden && now_hidden {
                    out.push(("render.menu.decloak", ship("cloak")));
                }
            }
            self.last[i].cloak = Some(p.cloak);
        }
        if let (Some(run), Some(d)) = (self.warp.as_mut(), dep) {
            for (i, p) in ships.iter().enumerate() {
                let s = &map.ships[i];
                if d.tau[i].is_some() && !run.started[i] {
                    run.started[i] = true;
                    out.push(("render.menu.warp", vec![("state", "ship".to_owned()), ("ship", s.name.clone())]));
                }
                if p.gone && !run.gone[i] {
                    run.gone[i] = true;
                    out.push(("render.menu.warp", vec![("state", "gone".to_owned()), ("ship", s.name.clone())]));
                }
            }
            if d.tunnel.is_some() && !run.tunnel {
                run.tunnel = true;
                out.push(("render.menu.warp", vec![("state", "tunnel".to_owned())]));
            }
        }
        for (kind, fields) in out {
            self.mark(t, kind, fields);
        }
    }
}

struct Departure {
    dir: Vec3,
    /// Seconds each ship has been leaving, once it has begun.
    tau: Vec<Option<f32>>,
    eye: Vec3,
    target: Vec3,
    swing: f32,
    follow: f32,
    tunnel: Option<Tunnel>,
}

// ---------------------------------------------------------------------------------
// The map as a sim
// ---------------------------------------------------------------------------------

/// A ship of the map and the sim entity that is drawn for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub ship: usize,
    pub id: EntityId,
}

/// The scene built over a content database.
pub struct Built {
    pub setup: GameSetup,
    pub sim: Sim,
    pub slots: Vec<Slot>,
    /// Ships whose ODF the content does not have, and the one drawn for each.
    pub substituted: Vec<(String, String)>,
}

fn fixed(v: Vec3) -> sta_sim::Vec3 {
    sta_sim::Vec3 { x: Fixed::from_f64(v.x as f64), y: Fixed::from_f64(v.y as f64), z: Fixed::from_f64(v.z as f64) }
}

/// The ODF to draw for `odf`: itself when the content has it, else one of the biggest ships (or,
/// for a ship that stays, stations) of its race that the content does have (Armada II names
/// `fgalaxy`, `kbattle`, `bbattle3` after their race's first letter; Fleet Operations calls them
/// `fed_*`, `kli_*`, `bor_*`), `n` ships in, so a fleet is not one hull. Any biggest hull when no
/// unit of the race exists.
fn resolve_odf(content: &ContentDb, odf: &str, stays: bool, n: usize) -> Option<String> {
    if content.key(odf).is_some_and(|k| content.unit(k).is_some()) {
        return Some(odf.to_owned());
    }
    let moves = |u: &sta_content::UnitDef| u.movement.max_speed > Fixed::ZERO;
    let race = match odf.chars().next() {
        Some('f') => "fed_",
        Some('k') => "kli_",
        Some('b') => "bor_",
        _ => "",
    };
    let pool = |prefix: &str| {
        let mut c: Vec<_> = content
            .units()
            .filter(|u| u.hull > Fixed::ZERO && moves(u) != stays && (prefix.is_empty() || u.odf.to_ascii_lowercase().starts_with(prefix)))
            .collect();
        c.sort_by(|a, b| b.hull.cmp(&a.hull).then_with(|| a.odf.cmp(&b.odf)));
        // One of each model: variants of a ship share one.
        let mut seen = std::collections::BTreeSet::new();
        c.retain(|u| seen.insert(u.model.to_ascii_lowercase()));
        c
    };
    let mut c = pool(race);
    if c.is_empty() {
        c = pool("");
    }
    if c.is_empty() {
        c = content.units().filter(|u| u.hull > Fixed::ZERO).collect();
        c.sort_by(|a, b| a.odf.cmp(&b.odf));
    }
    let top = c.len().min(6);
    (top > 0).then(|| c[n % top].odf.clone())
}

/// The map's ships as a `MapDef`, `GameSetup` and sim over `content`. `base` gives the
/// map's name and scenery (and its bounds, grown to take the ships in).
pub fn build(menu: &MenuMap, content: &Arc<ContentDb>, base: &MapDef) -> Built {
    let mut map = base.clone();
    map.objects.clear();
    map.regions.clear();
    map.starts.clear();
    let mut substituted = Vec::new();
    let mut placed: Vec<(usize, MapObject)> = Vec::new();
    let (mut lo, mut hi) = map.bounds;
    for (i, s) in menu.ships.iter().enumerate() {
        let Some(odf) = resolve_odf(content, &s.odf, !s.warp, substituted.len()) else { continue };
        if !odf.eq_ignore_ascii_case(&s.odf) {
            substituted.push((s.odf.clone(), odf.clone()));
        }
        let g = menu.group_of(s);
        let pos = fixed(g.place(s.at));
        let yaw = s.yaw + g.yaw;
        let heading = (Fixed::from_f64(yaw.sin() as f64), Fixed::from_f64(yaw.cos() as f64));
        lo =sta_sim::Vec3 { x: lo.x.min(pos.x), y: lo.y.min(pos.y), z: lo.z.min(pos.z) };
        hi = sta_sim::Vec3 { x: hi.x.max(pos.x), y: hi.y.max(pos.y), z: hi.z.max(pos.z) };
        placed.push((i, MapObject { odf, owner: Some(0), pos, heading, label: Some(s.name.clone()) }));
    }
    map.bounds = (lo, hi);
    map.objects = placed.iter().map(|(_, o)| o.clone()).collect();
    let setup = GameSetup {
        seed: 1,
        players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: String::new(), control: Control::Idle }],
        map: Arc::new(map),
        content: content.clone(),
        options: GameOptions::default(),
    };
    let (sim, _warnings) = Sim::from_setup_with_warnings(&setup);
    // Each ship finds its entity again by what it was placed as: definition and place.
    let view = sim.view();
    let mut taken: Vec<EntityId> = Vec::new();
    let mut slots = Vec::new();
    for (ship, o) in &placed {
        let Some(key) = content.key(&o.odf) else { continue };
        let found = view.units.iter().find(|u| u.def == key && u.pos == o.pos && !taken.contains(&u.id));
        if let Some(u) = found {
            taken.push(u.id);
            slots.push(Slot { ship: *ship, id: u.id });
        }
    }
    Built { setup, sim, slots, substituted }
}

/// The mount's content database, for the scene over an install.
pub fn load_install_content(vfs: &sta_vfs::Vfs) -> Result<Arc<ContentDb>, String> {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).map_err(|e| e.to_string())?;
    Ok(Arc::new(db))
}

// ---------------------------------------------------------------------------------
// The Bevy side
// ---------------------------------------------------------------------------------

/// What the scene is built from. Insert it to open the scene, remove it to close it.
#[derive(Resource, Clone)]
pub struct MenuSceneConfig {
    pub content: Arc<ContentDb>,
    pub map: Arc<MenuMap>,
    /// Gives the sim map its name and scenery; `MapDef::empty()` will do.
    pub base: MapDef,
    /// The view the scene opens on: `main`, else the map's first.
    pub view: String,
}

impl MenuSceneConfig {
    pub fn new(content: Arc<ContentDb>, map: MenuMap, base: MapDef) -> MenuSceneConfig {
        let view = if map.view_index("main").is_some() { "main".to_owned() } else { map.view_names().first().map_or_else(String::new, |v| (*v).to_owned()) };
        MenuSceneConfig { content, map: Arc::new(map), base, view }
    }
}

/// What the screens ask of the scene.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum MenuRequest {
    /// Move the camera to a named view.
    View(String),
    /// A mission is loading from the view shown: its ships go to warp.
    Depart,
    /// Stop a departure.
    Arrive,
    /// The views whose campaigns are locked.
    Locked(Vec<String>),
}

/// The scene while it is open.
#[derive(Resource)]
pub struct MenuScene {
    pub director: Director,
    slots: Vec<Slot>,
    /// Ships drawn by a stand-in, for the log.
    pub substituted: Vec<(String, String)>,
    /// Scene time zero, in the anim clock's.
    t0: f64,
}

impl MenuScene {
    pub fn view(&self) -> &str {
        self.director.view_name()
    }

    /// Scene seconds at the anim clock's `now`.
    fn time(&self, now: f64) -> f32 {
        (now - self.t0).max(0.0) as f32
    }
}

/// The director's last frame.
#[derive(Resource, Default)]
pub struct MenuFrame(pub Option<Frame>);

/// Everything the scene spawned, to take down again.
#[derive(Component)]
struct SceneEntity;

/// The scene's camera.
#[derive(Component)]
pub struct MenuCamera;

/// A sky that stays about the camera, and the turning part of it.
#[derive(Component)]
struct SkyFollow;

#[derive(Component)]
struct SkyTurn;

#[derive(Component)]
struct PlanetBody;

/// A ship drawn as a box, where there is no model to draw (no install mounted).
#[derive(Component)]
struct StandIn;

/// The scene without a renderer: its sim, its director and its marks. What the headless
/// bench runs; [`MenuScenePlugin`] adds it.
pub struct MenuSceneCorePlugin;

impl Plugin for MenuSceneCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderMarks>()
            .init_resource::<MenuFrame>()
            .add_message::<MenuRequest>()
            .add_systems(Update, open.in_set(GameSet::Input).run_if(resource_exists::<MenuSceneConfig>).run_if(not(resource_exists::<MenuScene>)))
            .add_systems(Update, close.in_set(GameSet::Input).run_if(not(resource_exists::<MenuSceneConfig>)).run_if(resource_exists::<MenuScene>))
            .add_systems(Update, step.in_set(GameSet::Render).run_if(resource_exists::<MenuScene>));
    }
}

/// The scene in a window or an image: [`MenuSceneCorePlugin`], and the sky, planet, lights,
/// camera and ships on screen. Needs the renderer's plugins (`DefaultPlugins`).
pub struct MenuScenePlugin;

impl Plugin for MenuScenePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MenuSceneCorePlugin>() {
            app.add_plugins(MenuSceneCorePlugin);
        }
        if !app.is_plugin_added::<crate::render::planet::PlanetPlugin>() {
            app.add_plugins(crate::render::planet::PlanetPlugin);
        }
        if !app.is_plugin_added::<crate::render::units::UnitsRenderPlugin>() {
            app.add_plugins(crate::render::units::UnitsRenderPlugin);
        }
        // The planet's surface systems read the mount; with no install it is empty.
        if !app.world().contains_resource::<Mounted>() {
            app.insert_resource(Mounted(sta_vfs::Vfs::default()));
        }
        if !app.world().contains_resource::<SpriteTables>() {
            app.insert_resource(SpriteTables(Arc::new(sta_assets::load_sprite_table(&sta_vfs::Vfs::default()))));
        }
        app.init_resource::<TextureCache>()
            .init_resource::<PartCache>()
            .add_systems(Update, spawn_visuals.in_set(GameSet::Input).after(open).run_if(resource_added::<MenuScene>))
            .add_systems(Update, despawn_visuals.in_set(GameSet::Input).run_if(resource_removed::<MenuScene>))
            .add_systems(Update, stand_ins.in_set(GameSet::Interpolate).run_if(resource_exists::<MenuScene>))
            .add_systems(Update, pose.in_set(GameSet::Render).after(step).run_if(resource_exists::<MenuScene>));
    }
}

/// Build the sim of the map's ships and link it, and start the director.
fn open(mut commands: Commands, config: Res<MenuSceneConfig>, clock: Res<AnimClock>) {
    let built = build(&config.map, &config.content, &config.base);
    if !built.substituted.is_empty() {
        info!("menu scene: {} ships drawn by stand-ins (the content has no {:?}...)", built.substituted.len(), built.substituted[0].0);
    }
    commands.insert_resource(ContentRes(config.content.clone()));
    commands.insert_resource(LocalPlayer(None));
    commands.insert_resource(SimLink::new(Box::new(built.sim), Box::new(LocalRouter::new()), PlayerId::BENCH));
    let director = Director::new(config.map.clone(), &config.view);
    commands.insert_resource(MenuScene { director, slots: built.slots, substituted: built.substituted, t0: clock.now() });
}

fn close(mut commands: Commands, mut frame: ResMut<MenuFrame>) {
    commands.remove_resource::<MenuScene>();
    commands.remove_resource::<SimLink>();
    commands.remove_resource::<ContentRes>();
    frame.0 = None;
}

/// Per frame: the requests, the director's frame, and its marks.
fn step(clock: Res<AnimClock>, mut scene: ResMut<MenuScene>, mut requests: MessageReader<MenuRequest>, mut marks: ResMut<RenderMarks>, mut frame: ResMut<MenuFrame>) {
    let t = scene.time(clock.now());
    for r in requests.read() {
        match r {
            MenuRequest::View(name) => {
                if !scene.director.request_view(name, t) {
                    warn!("menu scene: the map has no view {name:?}");
                }
            }
            MenuRequest::Depart => scene.director.depart(t),
            MenuRequest::Arrive => scene.director.warp = None,
            MenuRequest::Locked(v) => scene.director.set_locked(v),
        }
    }
    frame.0 = Some(scene.director.frame(t));
    for m in scene.director.take_marks() {
        let fields = m.fields.iter().map(|(k, v)| (*k, sta_bench::marks::Val::Text(v.clone()))).collect();
        marks.0.push(sta_bench::marks::Mark { tick: m.tick(), kind: m.kind, fields });
    }
}

/// A deterministic generator for the stars.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

/// `n` small quads on a sphere of `radius`, each turned to face the centre, as one mesh.
fn star_mesh(n: u32, size: f32, seed: u32, radius: f32) -> Mesh {
    let mut rng = Lcg(u64::from(seed).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let (mut pos, mut idx) = (Vec::new(), Vec::new());
    let half = size * radius * 0.5;
    for i in 0..n {
        let z = rng.next() * 2.0 - 1.0;
        let a = rng.next() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let dir = Vec3::new(r * a.cos(), z, r * a.sin());
        let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
        let right = dir.cross(up).normalize();
        let up = right.cross(dir).normalize();
        // Some stars are bigger than others.
        let big = 0.6 + rng.next() * 0.9;
        let c = dir * radius;
        let (rx, uy) = (right * half * big, up * half * big);
        let base = i * 4;
        pos.extend([(c - rx - uy).to_array(), (c + rx - uy).to_array(), (c + rx + uy).to_array(), (c - rx + uy).to_array()]);
        idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_indices(Indices::U32(idx))
}

/// The sky, the sun, the planet, the lights and the camera.
#[allow(clippy::too_many_arguments)]
fn spawn_visuals(
    mut commands: Commands,
    config: Res<MenuSceneConfig>,
    vfs: Option<Res<Mounted>>,
    device: Option<Res<RenderDevice>>,
    mut cache: ResMut<TextureCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut planets: ResMut<Assets<PlanetMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    target: Option<Res<FrameTarget>>,
) {
    let menu = config.map.clone();
    let vfs = vfs.filter(|v| !v.0.layers().is_empty());
    let formats = device.as_deref().map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or(CompressedImageFormats::BC);
    ambient.color = Color::WHITE;
    ambient.brightness = 250.0;
    let key_dir = to_bevy(menu.key_axis.to_array());
    let fill_dir = to_bevy(menu.fill_axis.to_array());
    let colour = |c: Vec3| {
        let m = c.max_element().max(1e-3);
        (Color::srgb(c.x / m, c.y / m, c.z / m), m.min(1.0))
    };
    let (kc, _) = colour(menu.key_colour);
    let (fc, fk) = colour(menu.fill_colour);
    commands.spawn((
        SceneEntity,
        DirectionalLight { illuminance: KEY_LUX, color: kc, shadow_maps_enabled: false, ..default() },
        Transform::IDENTITY.looking_to(key_dir, Vec3::Y),
    ));
    commands.spawn((
        SceneEntity,
        DirectionalLight { illuminance: KEY_LUX * fk.max(0.1) * 0.3, color: fc, shadow_maps_enabled: false, ..default() },
        Transform::IDENTITY.looking_to(fill_dir, Vec3::Y),
    ));

    let unlit = |c: Color| StandardMaterial { base_color: c, unlit: true, cull_mode: None, double_sided: true, ..default() };
    // Sky: the stars and the sun about the camera. The sky turns with the fleet's orbit.
    let follow = commands.spawn((SceneEntity, SkyFollow, Transform::IDENTITY, Visibility::default())).id();
    let turn = commands.spawn((SceneEntity, SkyTurn, Transform::IDENTITY, Visibility::default(), ChildOf(follow))).id();
    let sky_radius = 60_000.0;
    if menu.stars > 0 {
        commands.spawn((
            SceneEntity,
            Mesh3d(meshes.add(star_mesh(menu.stars, menu.star_size.max(0.0004), menu.star_seed, sky_radius * 0.9))),
            MeshMaterial3d(materials.add(unlit(Color::WHITE))),
            Transform::IDENTITY,
            ChildOf(turn),
            bevy::light::NotShadowCaster,
        ));
    }
    // The sun stands where the key comes from, the same from every view.
    commands.spawn((
        SceneEntity,
        Mesh3d(meshes.add(Sphere::new(sky_radius * 0.012).mesh().ico(3).expect("a sphere"))),
        MeshMaterial3d(materials.add(unlit(Color::srgb(menu.key_colour.x * 4.0, menu.key_colour.y * 4.0, menu.key_colour.z * 4.0)))),
        Transform::from_translation(-key_dir * sky_radius * 0.8),
        ChildOf(follow),
        bevy::light::NotShadowCaster,
    ));
    if let Some(v) = vfs.as_deref()
        && !menu.sky.is_empty()
    {
        let sky = commands.spawn((SceneEntity, Transform::from_scale(Vec3::splat(sky_radius * 0.95 / 100.0)), Visibility::default(), ChildOf(turn))).id();
        let mut loader = Loader { vfs: &v.0, cache: &mut cache, images: &mut images, formats };
        let faces = crate::render::space::sky_faces(&mut commands, &mut meshes, &mut materials, &mut loader, &menu.sky, sky);
        info!("menu scene: sky {} ({faces}/6 faces)", menu.sky);
        // Without a texture a face is a white wall: no sky is better.
        if faces == 0 {
            commands.entity(sky).despawn();
        }
    }

    // The planet at the map's centre; the scenery's first planet when the map has none.
    let planet = menu.planet.clone().or_else(|| {
        config.base.scenery.planets.first().map(|p| PlanetDef {
            class: String::new(),
            radius: p.radius,
            at: Vec3::from(p.pos),
            axis: Vec3::Y,
            spin: 0.3,
            clouds: 0.002,
            orbit: 0.004,
        })
    });
    if let Some(p) = planet {
        let body = commands
            .spawn((SceneEntity, PlanetBody, Transform::from_translation(to_bevy(p.at.to_array())).with_rotation(planet_rotation(&p, 0.0)), Visibility::default()))
            .id();
        let (ground, atmo) = planet_textures(&p, vfs.as_deref());
        let empty = sta_vfs::Vfs::default();
        let mut loader = Loader { vfs: vfs.as_deref().map_or(&empty, |m| &m.0), cache: &mut cache, images: &mut images, formats };
        let missing = crate::render::planet::spawn(&mut commands, &mut meshes, &mut planets, &mut loader, &ground, &atmo, p.radius, body);
        if !missing.is_empty() {
            debug!("menu scene: planet textures not found: {}", missing.join(", "));
        }
    }

    let fov = menu.views.first().map_or(40.0, |v| v.fov);
    let projection = Projection::Perspective(PerspectiveProjection { near: 2.0, far: 400_000.0, fov: fov * DEG, ..default() });
    let mut camera = commands.spawn((SceneEntity, MenuCamera, Camera3d::default(), projection, Transform::default()));
    // A frame run draws into the capture's image, with no multisampling to resolve.
    if let Some(t) = target {
        camera.insert((bevy::camera::RenderTarget::from(t.0.clone()), Msaa::Off));
    }
}

/// The ground and cloud textures of the planet's class: its ODF from the install when
/// there is one, else stock's atmosphere alone (as Refit does when the ODF names none).
fn planet_textures(p: &PlanetDef, vfs: Option<&Mounted>) -> (String, String) {
    let mut ground = String::new();
    let mut atmo = String::from("PA_ECFR0");
    if let Some(v) = vfs.filter(|_| !p.class.is_empty()) {
        let mut legacy = sta_content::legacy::Legacy::default();
        let db = legacy.db(&v.0, [p.class.as_str()]);
        if let Some(odf) = db.key(&p.class).and_then(|k| db.unit(k)).map(|d| d.raw.clone()) {
            ground = odf.str("groundTextureName").unwrap_or("").to_owned();
            if let Some(a) = odf.str("atmosphereTextureName").filter(|a| !a.is_empty()) {
                atmo = a.to_owned();
            }
        }
    }
    (ground, atmo)
}

/// The planet's pole along its axis, and the ground turned about it by `turn` radians.
fn planet_rotation(p: &PlanetDef, turn: f32) -> Quat {
    let pole = to_bevy(p.axis.to_array()).normalize_or(Vec3::Y);
    Quat::from_rotation_arc(Vec3::Y, pole) * Quat::from_rotation_y(turn)
}

fn despawn_visuals(mut commands: Commands, scene: Query<Entity, With<SceneEntity>>, mut seen: ResMut<crate::sim_link::SimEntities>) {
    for e in &scene {
        commands.entity(e).despawn();
    }
    // The ships are the link's: they go with it.
    for (_, e) in std::mem::take(&mut seen.0) {
        commands.entity(e).try_despawn();
    }
}

/// A ship with no model to draw (no install mounted, or the install has none for it) is a box.
#[allow(clippy::type_complexity)]
fn stand_ins(
    mut commands: Commands,
    pending: Query<Entity, (With<FromView>, Without<NeedsModel>, Without<Children>, Without<StandIn>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for e in &pending {
        let mesh = meshes.add(Cuboid::new(40.0, 16.0, 110.0));
        let mat = materials.add(StandardMaterial { base_color: Color::srgb(0.62, 0.66, 0.74), perceptual_roughness: 0.6, ..default() });
        commands.entity(e).insert(StandIn);
        commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), Transform::IDENTITY, ChildOf(e)));
    }
}

/// The camera, the sky and the planet, and each ship's pose and cloak, from the director's frame.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pose(
    mut commands: Commands,
    scene: Res<MenuScene>,
    frame: Res<MenuFrame>,
    config: Res<MenuSceneConfig>,
    entities: Res<crate::sim_link::SimEntities>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<MenuCamera>>,
    mut follow: Query<&mut Transform, (With<SkyFollow>, Without<MenuCamera>, Without<SkyTurn>, Without<PlanetBody>, Without<SimEntity>)>,
    mut turn: Query<&mut Transform, (With<SkyTurn>, Without<MenuCamera>, Without<SkyFollow>, Without<PlanetBody>, Without<SimEntity>)>,
    mut planet: Query<&mut Transform, (With<PlanetBody>, Without<MenuCamera>, Without<SkyFollow>, Without<SkyTurn>, Without<SimEntity>)>,
    mut ships: Query<(&mut Transform, &mut Visibility), (With<SimEntity>, Without<MenuCamera>, Without<SkyFollow>, Without<SkyTurn>, Without<PlanetBody>)>,
    fades: Query<&UnitFade>,
) {
    let Some(frame) = &frame.0 else { return };
    let eye = to_bevy(frame.cam.eye.to_array());
    let look = to_bevy(frame.cam.target.to_array());
    if let Ok((mut tf, mut proj)) = cameras.single_mut() {
        *tf = Transform::from_translation(eye).looking_at(look, Vec3::Y);
        if let Projection::Perspective(p) = &mut *proj {
            p.fov = frame.cam.fov * DEG;
        }
    }
    for mut tf in &mut follow {
        tf.translation = eye;
    }
    for mut tf in &mut turn {
        tf.rotation = Quat::from_rotation_x(frame.sky_turn);
    }
    if let Some(p) = config.map.planet.as_ref() {
        for mut tf in &mut planet {
            tf.rotation = planet_rotation(p, frame.planet_turn);
        }
    }
    for slot in &scene.slots {
        let Some(&e) = entities.0.get(&slot.id) else { continue };
        let Some(p) = frame.ships.get(slot.ship) else { continue };
        let Ok((mut tf, mut vis)) = ships.get_mut(e) else { continue };
        tf.translation = to_bevy(p.pos.to_array());
        let nose = Vec3::new(p.yaw.sin(), 0.0, -p.yaw.cos());
        tf.rotation = Transform::IDENTITY.looking_to(nose, Vec3::Y).rotation * Quat::from_euler(EulerRot::XYZ, p.pitch, 0.0, p.roll);
        tf.scale = Vec3::new(1.0, 1.0, p.stretch);
        let a = p.alpha();
        let want = if a <= 0.0 { Visibility::Hidden } else { Visibility::Inherited };
        if *vis != want {
            *vis = want;
        }
        // The cloak is the scene's own: the sim never cloaks these ships.
        let fade = UnitFade { cloak: a.clamp(0.0, 1.0), build: 1.0 };
        if fades.get(e).map_or(true, |f| *f != fade) {
            commands.entity(e).insert((ManualFade, fade));
        }
    }
}

// ---------------------------------------------------------------------------------
// Running the scene on its own (the bench and the frame tests)
// ---------------------------------------------------------------------------------

/// Is this launch the scene on its own: `--headless` with `STA64_MENU_SCENE=1`?
pub fn requested(cfg: &crate::app::LaunchConfig) -> bool {
    cfg.headless && std::env::var("STA64_MENU_SCENE").as_deref() == Ok("1")
}

/// The views to ask for, from `STA64_MENU_VIEWS="klingon@10,main@25"` (view at scene
/// seconds), and `STA64_MENU_DEPART=<seconds>`.
fn script() -> (Vec<(f32, String)>, Option<f32>) {
    let views = std::env::var("STA64_MENU_VIEWS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|p| {
            let (v, at) = p.split_once('@')?;
            Some((at.trim().parse().ok()?, v.trim().to_owned()))
        })
        .collect();
    let depart = std::env::var("STA64_MENU_DEPART").ok().and_then(|v| v.parse().ok());
    (views, depart)
}

#[derive(Resource)]
struct Script {
    views: Vec<(f32, String)>,
    depart: Option<f32>,
}

fn run_script(clock: Res<AnimClock>, mut script: ResMut<Script>, mut out: MessageWriter<MenuRequest>, scene: Option<Res<MenuScene>>) {
    let Some(scene) = scene else { return };
    let t = scene.time(clock.now());
    while script.views.first().is_some_and(|(at, _)| *at <= t) {
        let (_, v) = script.views.remove(0);
        out.write(MenuRequest::View(v));
    }
    if script.depart.is_some_and(|at| at <= t) {
        script.depart = None;
        out.write(MenuRequest::Depart);
    }
}

/// Run the clock the way the bench does: the sim link steps from it.
#[derive(Resource)]
struct Budget(Tick);

#[derive(Resource)]
struct FrameTarget(Handle<Image>);

/// Hold the clock while a frame settles and is read back; stop after the last one.
fn gate_frames(mut capture: ResMut<crate::bench::frames::Capture>, mut clock: ResMut<AnimClock>, mut commands: Commands, mut exit: MessageWriter<AppExit>) {
    use crate::bench::frames::Gate;
    match capture.gate(clock.tick(), &mut commands) {
        Gate::Hold => clock.set_paused(true),
        Gate::Run(limit) => {
            if capture.done() {
                exit.write(AppExit::Success);
            }
            let stop = clock.tick() >= limit;
            clock.set_paused(stop);
        }
    }
}

fn ticked_out(clock: Res<AnimClock>, budget: Res<Budget>, mut exit: MessageWriter<AppExit>) {
    if clock.tick() >= budget.0 {
        exit.write(AppExit::Success);
    }
}

fn write_marks(marks: Res<RenderMarks>, mut done: MessageReader<AppExit>) {
    if done.read().next().is_some() {
        crate::bench::marks::write_render_if_requested(&marks);
    }
}

/// The scene alone, headless: `sta64 --headless --clock bench --ticks N` runs the scene
/// for `N` ticks and writes its marks to `$STA64_BENCH_OUT/marks`; with `--frames` it
/// draws frames at those ticks (needs a GPU; exits 77 without one). The install is read
/// when `--install-root` names one that mounts; otherwise the scene is built on the fake
/// content and `two_bases()`. `STA64_MENU_VIEW`, `STA64_MENU_VIEWS` and `STA64_MENU_DEPART`
/// script it. Returns the process exit code.
pub fn run_headless(cfg: &crate::app::LaunchConfig) -> i32 {
    if cfg.frames.is_empty() {
        return run_inner(cfg);
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_inner(cfg))) {
        Ok(code) => code,
        Err(e) => {
            let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_default();
            if msg.contains("Unable to find a GPU") {
                eprintln!("no GPU adapter: {msg}");
                crate::bench::frames::NO_ADAPTER
            } else {
                eprintln!("headless render failed: {msg}");
                101
            }
        }
    }
}

fn run_inner(cfg: &crate::app::LaunchConfig) -> i32 {
    use std::time::Duration;
    let vfs = cfg.install_root.as_ref().and_then(|root| match sta_vfs::Vfs::mount_install(root, cfg.active_mod.as_deref()) {
        Ok(vfs) => Some(vfs),
        Err(e) => {
            eprintln!("menu scene: no install at {} ({e}); fake content and two_bases()", root.display());
            None
        }
    });
    let (content, map, base, mounted) = match vfs {
        Some(vfs) => match load_install_content(&vfs) {
            Ok(db) => (db, MenuMap::from_vfs(&vfs), MapDef::empty(), Some(vfs)),
            Err(e) => {
                eprintln!("content: {e}");
                return 2;
            }
        },
        None => (Arc::new(sta_content::fake::db()), MenuMap::builtin(), sta_content::fake::two_bases(), None),
    };
    let mut config = MenuSceneConfig::new(content, map, base);
    if let Ok(v) = std::env::var("STA64_MENU_VIEW") {
        config.view = v;
    }
    let (views, depart) = script();
    let frames = !cfg.frames.is_empty();
    let mut app = App::new();
    let runner = bevy::app::ScheduleRunnerPlugin::run_loop(Duration::from_millis(1));
    if frames {
        use bevy::render::RenderPlugin;
        use bevy::window::ExitCondition;
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<bevy::winit::WinitPlugin>(),
        )
        .add_plugins(runner);
        if let Some(info) = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>()
            && crate::bench::frames::is_software_adapter(info.device_type, &info.name)
        {
            eprintln!("no GPU adapter: only a software adapter is available ({})", info.name);
            return crate::bench::frames::NO_ADAPTER;
        }
    } else {
        app.add_plugins(MinimalPlugins.set(runner));
    }
    crate::app::configure_sets(&mut app);
    app.insert_resource(AnimClock::new(cfg.clock_mode(), 0))
        .add_plugins(crate::sim_link::SimLinkPlugin)
        .insert_resource(Script { views, depart })
        .insert_resource(Budget(cfg.ticks.unwrap_or(0).max(cfg.frames.iter().copied().max().unwrap_or(0))))
        .insert_resource(config)
        .add_systems(Update, run_script.in_set(GameSet::Bench));
    if let Some(vfs) = mounted {
        app.insert_resource(SpriteTables(Arc::new(sta_assets::load_sprite_table(&vfs))));
        app.insert_resource(Mounted(vfs));
    }
    if frames {
        let capture = crate::bench::frames::Capture::new(cfg, &mut app.world_mut().resource_mut::<Assets<Image>>());
        app.insert_resource(FrameTarget(capture.target().clone()))
            .insert_resource(capture)
            .insert_resource(ClearColor(Color::BLACK))
            .add_plugins(crate::render::fx::FxMaterialsPlugin)
            .add_plugins(crate::render::fx::specials::SpecialsFxPlugin)
            .add_plugins(crate::render::hulls::HullPlugin)
            .add_plugins(MenuScenePlugin)
            .add_systems(Update, gate_frames.in_set(GameSet::Bench));
    } else {
        app.add_plugins(MenuSceneCorePlugin).add_systems(Update, ticked_out.in_set(GameSet::Bench));
    }
    app.add_systems(Last, write_marks);
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(c) => i32::from(c.get()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    fn builtin() -> Arc<MenuMap> {
        Arc::new(MenuMap::builtin())
    }

    fn marks_of(d: &mut Director) -> Vec<String> {
        d.take_marks().iter().map(|m| format!("{} {}", m.kind, m.fields.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "))).collect()
    }

    #[test]
    fn the_builtin_map_is_refits() {
        let m = MenuMap::builtin();
        assert_eq!(m.view_names(), ["main", "federation", "tutorials", "klingon", "borg"]);
        assert_eq!(m.sky, "mbgblue");
        assert_eq!(m.stars, 1400);
        assert_eq!(m.blend, 1.6);
        assert_eq!(m.warp.delay, 0.9);
        assert_eq!(m.warp.tunnel_in, 0.9);
        let p = m.planet.as_ref().unwrap();
        assert_eq!((p.class.as_str(), p.radius), ("PB_CLSSM", 4200.0));
        // Group 0 is the origin; then fleet, station, klingon, borg, borgshadow.
        assert_eq!(m.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["", "fleet", "station", "klingon", "borg", "borgshadow"]);
        assert_eq!(m.ships.len(), 6 + 3 + 5 + 2);
        assert_eq!(m.nebulae.len(), 2);
        let klingon = &m.groups[3];
        assert_eq!(klingon.cloak_view.as_deref(), Some("klingon"));
        assert_eq!((klingon.cloak_time, klingon.cloak_stagger), (2.5, 0.45));
        assert!(klingon.locked_cloak);
        // Views are given in their group's frame: `main` is the fleet's eye, placed.
        let main = &m.views[0];
        assert_eq!(main.eye, m.groups[1].place(Vec3::new(125.0, 25.0, 285.0)));
        // The starbase stays when the station departs.
        assert!(!m.ships.iter().find(|s| s.name == "starbase").unwrap().warp);
        // The borg cube sits elsewhere while its campaign is locked.
        assert!(m.ships.iter().find(|s| s.name == "fusion").unwrap().locked_at.is_some());
    }

    #[test]
    fn a_group_places_points_by_its_yaw() {
        let g = Group { name: "g".into(), at: Vec3::new(10.0, 0.0, 0.0), yaw: 90.0 * DEG, cloak_view: None, cloak_delay: 0.0, cloak_time: 1.0, cloak_stagger: 0.0, locked_cloak: false, locked_yaw: 0.0 };
        // +z forward turned a quarter about +y points along +x.
        let p = g.place(Vec3::new(0.0, 5.0, 100.0));
        assert!((p - Vec3::new(110.0, 5.0, 0.0)).length() < 1e-3, "{p}");
    }

    #[test]
    fn the_camera_blends_between_views_and_sways() {
        let m = builtin();
        let mut d = Director::new(m.clone(), "main");
        let a = d.frame(0.0).cam;
        let main = &m.views[0];
        assert!((a.eye - main.eye).length() < main.sway.length() + 1e-3);
        assert_eq!(a.fov, 40.0);
        d.frame(10.0);
        assert!(d.request_view("federation", 10.0));
        assert!(!d.request_view("nowhere", 10.0));
        // Just after the request the camera is where it was; after the blend it is there.
        let before = d.frame(10.0).cam;
        let mid = d.frame(10.0 + m.blend / 2.0).cam;
        let settled = d.frame(10.0 + m.blend).cam;
        let fed = &m.views[1];
        assert!((settled.eye - fed.eye).length() < fed.sway.length() + 1e-3, "{settled:?}");
        assert_eq!(settled.fov, 42.0);
        assert!((before.eye - main.eye).length() < 40.0);
        // Halfway it is between the two.
        assert!(mid.fov > 40.0 && mid.fov < 42.0, "{}", mid.fov);
    }

    #[test]
    fn a_view_switch_makes_a_mark_and_asking_again_makes_none() {
        let mut d = Director::new(builtin(), "main");
        assert_eq!(marks_of(&mut d), ["render.menu.view view=main from=-"]);
        d.request_view("borg", 3.0);
        d.request_view("borg", 4.0);
        d.request_view("tutorials", 5.0);
        assert_eq!(marks_of(&mut d), ["render.menu.view view=borg from=main", "render.menu.view view=tutorials from=borg"]);
        assert_eq!(d.view_name(), "tutorials");
    }

    #[test]
    fn klingon_ships_decloak_in_turn_when_their_view_is_asked_for() {
        let m = builtin();
        let mut d = Director::new(m.clone(), "main");
        let kl: Vec<usize> = (0..m.ships.len()).filter(|&i| m.groups[m.ships[i].group].name == "klingon").collect();
        let alpha = |f: &Frame| kl.iter().map(|&i| f.ships[i].alpha()).collect::<Vec<_>>();
        // Cloaked, and not drawn at all, while the camera is elsewhere.
        let f = d.frame(1.0);
        assert!(alpha(&f).iter().all(|a| *a == 0.0), "{:?}", alpha(&f));
        d.take_marks();
        d.request_view("klingon", 2.0);
        let mut t = 2.0;
        while t < 12.0 {
            d.frame(t);
            t += 1.0 / 30.0;
        }
        let marks = marks_of(&mut d);
        let starts: Vec<&String> = marks.iter().filter(|l| l.contains("state=start")).collect();
        let dones: Vec<&String> = marks.iter().filter(|l| l.contains("state=done")).collect();
        assert_eq!(starts.len(), 5, "{marks:?}");
        assert_eq!(dones.len(), 5, "{marks:?}");
        // In file order, one stagger apart; the view's own mark came first.
        assert!(starts[0].contains("ship=negh'var"), "{starts:?}");
        assert!(starts[1].contains("ship=vor'cha"));
        assert!(marks[0].starts_with("render.menu.view view=klingon"));
        assert!(alpha(&d.frame(12.0)).iter().all(|a| *a == 1.0));
        // Leaving: whole until the camera has gone, then cloaked again.
        d.request_view("main", 13.0);
        assert!(alpha(&d.frame(13.0)).iter().all(|a| *a == 1.0));
        assert!(alpha(&d.frame(13.0 + m.blend + 0.1)).iter().all(|a| *a == 0.0));
        assert!(marks_of(&mut d).iter().any(|l| l.contains("state=cloak")));
    }

    #[test]
    fn a_locked_campaign_keeps_its_ships_in_the_glass() {
        let m = builtin();
        let mut d = Director::new(m.clone(), "main");
        d.set_locked(&["Klingon".to_owned()]);
        d.request_view("klingon", 1.0);
        let f = d.frame(30.0);
        let neghvar = m.ships.iter().position(|s| s.name == "negh'var").unwrap();
        assert!(f.ships[neghvar].held);
        assert_eq!(f.ships[neghvar].alpha(), CLOAKED_ALPHA);
        // Never decloaks: no decloak mark.
        assert!(!marks_of(&mut d).iter().any(|l| l.contains("decloak")));
    }

    #[test]
    fn a_departure_sends_the_front_ship_first_and_the_station_stays() {
        let m = builtin();
        let mut d = Director::new(m.clone(), "main");
        d.frame(0.0);
        d.take_marks();
        d.depart(1.0);
        d.depart(1.5);
        let mut t = 1.0;
        let mut last = d.frame(t);
        while t < 9.0 {
            t += 1.0 / 30.0;
            last = d.frame(t);
        }
        let marks = marks_of(&mut d);
        assert_eq!(marks[0], "render.menu.warp state=start view=main");
        let away: Vec<&String> = marks.iter().filter(|l| l.contains("state=ship")).collect();
        let gone: Vec<&String> = marks.iter().filter(|l| l.contains("state=gone")).collect();
        assert_eq!(away.len(), 6, "{marks:?}");
        assert_eq!(gone.len(), 6);
        // Front first: the galaxy sits at the group's origin, the rest behind it (-z).
        assert!(away[0].contains("ship=galaxy"), "{away:?}");
        assert!(marks.iter().any(|l| l == "render.menu.warp state=tunnel"));
        assert!(last.tunnel.is_some_and(|t| t.k > 0.9));
        // The camera widened its field for the tunnel.
        assert!(last.cam.fov > 40.0 + m.warp.widen * 0.9);

        // From the tutorials the starbase (`Warp=0`) stays and the others go.
        let mut d = Director::new(m.clone(), "tutorials");
        d.frame(0.0);
        d.depart(1.0);
        let mut t = 1.0;
        let mut f = d.frame(t);
        while t < 6.0 {
            t += 1.0 / 30.0;
            f = d.frame(t);
        }
        let base = m.ships.iter().position(|s| s.name == "starbase").unwrap();
        let venture = m.ships.iter().position(|s| s.name == "venture").unwrap();
        assert!(!f.ships[base].gone);
        assert!(f.ships[venture].gone);
    }

    #[test]
    fn a_departure_is_repeatable_frame_for_frame() {
        let run = || {
            let mut d = Director::new(builtin(), "klingon");
            let mut out = Vec::new();
            for i in 0..600 {
                let t = i as f32 / 30.0;
                if i == 200 {
                    d.depart(t);
                }
                if i == 100 {
                    d.request_view("borg", t);
                }
                out.push(d.frame(t));
            }
            (out, d.take_marks())
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn marks_are_stamped_with_sim_ticks() {
        let m = DirectorMark { time: 2.5, kind: "render.menu.view", fields: vec![] };
        assert_eq!(m.tick(), 75);
    }

    #[test]
    fn the_scene_builds_on_two_bases_and_the_fake_content() {
        let content = Arc::new(sta_content::fake::db());
        let menu = MenuMap::builtin();
        let base = sta_content::fake::two_bases();
        let built = build(&menu, &content, &base);
        // Every ship of the map is drawn by something the fake content has.
        assert_eq!(built.slots.len(), menu.ships.len());
        assert_eq!(built.substituted.len(), menu.ships.len());
        let by = |odf: &str| built.substituted.iter().find(|(o, _)| o == odf).map(|(_, s)| s.as_str());
        assert_eq!(by("fgalaxy"), Some("fx_cruiser"));
        assert_eq!(by("fbase"), Some("fx_station"));
        // The map's name and scenery come from the base; its own objects (the moon) do not.
        assert_eq!(built.setup.map.name, "fx_two_bases");
        assert_eq!(built.setup.map.scenery.planets.len(), 1);
        assert!(built.setup.map.objects.iter().all(|o| o.odf != "fx_moon"));
        let (lo, hi) = built.setup.map.bounds;
        assert!(lo.z < Fixed::from_int(-14000) && hi.z > Fixed::from_int(8000), "the bounds take the ships in");
        // The sim holds them where the map put them, one entity per slot.
        let view = built.sim.view();
        assert_eq!(view.units.len(), menu.ships.len());
        let galaxy = menu.ships.iter().position(|s| s.name == "galaxy").unwrap();
        let slot = built.slots.iter().find(|s| s.ship == galaxy).unwrap();
        let at = menu.groups[1].place(Vec3::ZERO);
        let u = view.unit(slot.id).unwrap();
        assert!((u.pos.x.to_f32() - at.x).abs() < 0.01 && (u.pos.y.to_f32() - at.y).abs() < 0.01);
        let mut ids: Vec<_> = built.slots.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), menu.ships.len());
    }

    #[test]
    fn a_map_without_a_planet_takes_the_bases_scenery() {
        let menu = MenuMap::parse("[View.main]\nEye=0,0,100\n[Ship.a]\nOdf=fx_cruiser\n");
        assert!(menu.planet.is_none());
        let content = Arc::new(sta_content::fake::db());
        let built = build(&menu, &content, &sta_content::fake::two_bases());
        assert_eq!((built.slots.len(), built.substituted.len()), (1, 0));
        assert_eq!(built.setup.map.scenery.planets[0].radius, 6000.0);
    }

    /// The scene in an app without a renderer.
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 30.0)))
            .insert_resource(AnimClock::new(crate::clock::ClockMode::Bench { step: 1.0 / 30.0 }, 0))
            .add_plugins(crate::sim_link::SimLinkPlugin)
            .add_plugins(MenuSceneCorePlugin);
        crate::app::configure_sets(&mut app);
        app.insert_resource(MenuSceneConfig::new(Arc::new(sta_content::fake::db()), MenuMap::builtin(), sta_content::fake::two_bases()));
        app
    }

    fn run(app: &mut App, ticks: u32) {
        for _ in 0..ticks {
            app.update();
        }
    }

    #[test]
    fn the_scene_loads_headless_and_marks_its_view_switches() {
        let mut app = app();
        run(&mut app, 3);
        assert!(app.world().contains_resource::<MenuScene>());
        assert!(app.world().contains_resource::<SimLink>(), "the ships' sim is linked");
        assert_eq!(app.world().resource::<SimLink>().curr().units.len(), MenuMap::builtin().ships.len());
        assert_eq!(app.world().resource::<MenuScene>().view(), "main");
        app.world_mut().write_message(MenuRequest::View("klingon".into()));
        run(&mut app, 3);
        app.world_mut().write_message(MenuRequest::View("borg".into()));
        run(&mut app, 3);
        let marks = app.world().resource::<RenderMarks>().lines();
        let views: Vec<&String> = marks.iter().filter(|l| l.starts_with("render.menu.view")).collect();
        assert_eq!(views, ["render.menu.view view=main from=-", "render.menu.view view=klingon from=main", "render.menu.view view=borg from=klingon"]);
        // The file's text: sim ticks first, ascending.
        let text = app.world().resource::<RenderMarks>().text();
        let ticks: Vec<u64> = text.lines().map(|l| l.split_whitespace().next().unwrap().parse().unwrap()).collect();
        assert!(ticks.windows(2).all(|w| w[0] <= w[1]), "{text}");
    }

    #[test]
    fn decloak_marks_come_through_the_app_and_closing_takes_the_scene_down() {
        let mut app = app();
        run(&mut app, 2);
        app.world_mut().write_message(MenuRequest::View("klingon".into()));
        run(&mut app, 30 * 12);
        let marks = app.world().resource::<RenderMarks>().lines();
        assert_eq!(marks.iter().filter(|l| l.starts_with("render.menu.decloak") && l.contains("state=start")).count(), 5, "{marks:?}");
        app.world_mut().write_message(MenuRequest::Depart);
        run(&mut app, 30 * 8);
        let marks = app.world().resource::<RenderMarks>().lines();
        assert!(marks.iter().any(|l| l == "render.menu.warp state=start view=klingon"), "{marks:?}");
        assert!(marks.iter().any(|l| l == "render.menu.warp state=tunnel"));
        app.world_mut().remove_resource::<MenuSceneConfig>();
        run(&mut app, 2);
        assert!(!app.world().contains_resource::<MenuScene>());
        assert!(!app.world().contains_resource::<SimLink>());
        // Opened again, it starts over.
        app.insert_resource(MenuSceneConfig::new(Arc::new(sta_content::fake::db()), MenuMap::builtin(), sta_content::fake::two_bases()));
        run(&mut app, 2);
        assert!(app.world().contains_resource::<MenuScene>());
    }

    /// With `STA64_TEST_INSTALL` (+ optional `STA64_TEST_MOD`): the scene builds over the install's
    /// content, every ship of the map drawn by a ship the install has.
    #[test]
    fn install_content_has_the_menus_ships() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        let content = load_install_content(&vfs).unwrap();
        let menu = MenuMap::from_vfs(&vfs);
        assert!(menu.view_index("main").is_some());
        let built = build(&menu, &content, &MapDef::empty());
        println!("{} ships, {} slots, stand-ins: {:?}", menu.ships.len(), built.slots.len(), built.substituted);
        assert_eq!(built.slots.len(), menu.ships.len());
        // Armada II's ODFs are the map's (`fgalaxy`); Fleet Operations renamed them (`fed_*`), and the
        // scene then draws the biggest ships of each race instead.
        assert!(built.substituted.len() <= menu.ships.len());
        let drawn: std::collections::BTreeSet<&str> = built.setup.map.objects.iter().map(|o| o.odf.as_str()).collect();
        assert!(!drawn.is_empty());
    }
}
