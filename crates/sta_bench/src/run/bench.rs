//! One running scene: the sim, its history, the command queue and the
//! answers. Everything a scenario step acts on lives here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sta_content::MapDef;
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Tick, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::{ControlRec, Logged, Replay, SeatRec};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, WorldSource};

use super::marks::{self, Mark};
use super::{RunConfig, json, to_f64};
use crate::parse::command::{CargoAmount, CloakMode, Resource, Switch, TechWho};
use crate::parse::scene::{MapObjects, Start};
use crate::parse::{BenchCommand, SceneFile, Target, TimeSpec, TimedCommand};

/// Recorded commands of a `Start=replay:` file still to be fed to the sim.
struct ReplayFeed {
    commands: Vec<(Tick, Command)>,
    next: usize,
    digests: Vec<(Tick, u64)>,
    next_digest: usize,
}

/// What a run has done so far, for `Save replay as`: the setup it started
/// from (empty or named map only), every command that reached the sim and
/// every scene or `spawn` placement as an `Edit::Spawn` on the tick it was made.
struct Rec {
    setup: GameSetup,
    commands: Vec<Logged>,
    digests: Vec<(Tick, u64)>,
}

/// Digest recorded every this many ticks, and at the end.
const REC_EVERY: Tick = 30;

pub struct Bench {
    pub cfg: Arc<RunConfig>,
    pub sim: Sim,
    base_tick: Tick,
    /// `views[i]` is the world at tick `base_tick + i`.
    views: Vec<Arc<WorldView>>,
    /// Every event, stamped with the tick of the step that emitted it (the
    /// tick the step started at).
    pub events: Vec<(Tick, SimEvent)>,
    names: BTreeMap<String, EntityId>,
    pending: Vec<Command>,
    seq: BTreeMap<PlayerId, u32>,
    issued: BTreeMap<(PlayerId, u32), String>,
    timed: Vec<(Tick, BenchCommand)>,
    healing: Vec<EntityId>,
    pub selection: Vec<EntityId>,
    paused: bool,
    pause_at: Option<Tick>,
    pub fire_gap: Tick,
    replay: Option<ReplayFeed>,
    rec: Option<Rec>,
    pub log: Vec<String>,
    logged_once: BTreeSet<String>,
    /// Files the case writes into its report directory.
    pub artifacts: BTreeMap<String, String>,
    pub label: String,
}

fn heading_of(degrees: f64) -> (Fixed, Fixed) {
    // 0 faces +z. Rounded so the platform's libm cannot move a last bit.
    let r = degrees.to_radians();
    let q = |v: f64| Fixed::from_f64((v * 1e9).round() / 1e9);
    (q(r.sin()), q(r.cos()))
}

fn point(p: [f64; 3]) -> Vec3 {
    Vec3 {
        x: Fixed::from_f64(p[0]),
        y: Fixed::from_f64(p[1]),
        z: Fixed::from_f64(p[2]),
    }
}

pub(super) fn owner_of(team: i32) -> PlayerId {
    if team <= 0 {
        NEUTRAL
    } else {
        PlayerId(team as u8)
    }
}

impl Bench {
    /// Build a scene: the start (empty map, named map, snapshot or replay),
    /// then the scene file's objects on top.
    pub fn build(cfg: Arc<RunConfig>, scene: &SceneFile) -> Result<Bench, String> {
        let s = &scene.scene;
        let cfg = match &s.profile {
            Some(name) => {
                let p = sta_content::Profile::from_name(name)
                    .ok_or_else(|| format!("Profile={name}: not a profile name"))?;
                super::with_profile(&cfg, p)
            }
            None => cfg,
        };
        let content = cfg.content.clone();
        let mut replay_feed = None;
        let mut rec_setup = None;
        let label;
        let mut log = Vec::new();

        let sim = match &s.start {
            Some(Start::Snapshot(file)) => {
                label = format!("snapshot:{file}");
                let bytes = std::fs::read(cfg.data_path(file))
                    .map_err(|e| format!("Start=snapshot:{file}: {e}"))?;
                Sim::from_snapshot(&bytes, Some(content.clone()))
                    .map_err(|e| format!("Start=snapshot:{file}: {e:?}"))?
            }
            Some(Start::Replay(file)) => {
                let bytes = std::fs::read(cfg.data_path(file))
                    .map_err(|e| format!("Start=replay:{file}: {e}"))?;
                let r =
                    Replay::decode(&bytes).map_err(|e| format!("Start=replay:{file}: {e:?}"))?;
                // The bench builds bare maps only; a race-start recording would
                // otherwise fail as a tick-0 desync.
                if r.starts != sta_sim::replay::StartRec::Bare {
                    return Err(format!("Start=replay:{file}: {}", sta_sim::replay::ReplayError::StartsNeedRaces));
                }
                let setup = r
                    .setup(content.clone())
                    .map_err(|e| format!("Start=replay:{file}: {e:?}"))?;
                label = r.map.name.clone();
                replay_feed = Some(ReplayFeed {
                    commands: r
                        .commands
                        .iter()
                        .map(|l| (l.tick, l.command.clone()))
                        .collect(),
                    next: 0,
                    digests: r.digests.clone(),
                    next_digest: 0,
                });
                Sim::from_setup(&setup)
            }
            None => {
                let mut map: MapDef = match &s.map {
                    Some(name) => cfg
                        .maps
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("Map={name}: no such map"))?,
                    None => MapDef::empty(),
                };
                if scene.map_objects() == MapObjects::Strip {
                    map.objects.clear();
                }
                label = map.label.clone().unwrap_or_else(|| map.name.clone());
                let teams: BTreeSet<u8> = scene
                    .objects
                    .iter()
                    .filter(|o| o.team > 0)
                    .map(|o| o.team as u8)
                    .collect();
                let players = teams
                    .into_iter()
                    .map(|t| PlayerSetup {
                        id: PlayerId(t),
                        team: TeamId(if s.allies.contains(&t) {
                            s.allies[0]
                        } else {
                            t
                        }),
                        race: "bench".into(),
                        control: Control::Idle,
                    })
                    .collect();
                let setup = GameSetup {
                    seed: s.seed.unwrap_or(1),
                    map: Arc::new(map),
                    content: content.clone(),
                    players,
                    options: GameOptions::default(),
                };
                let (sim, warnings) = Sim::from_setup_with_warnings(&setup);
                rec_setup = Some(setup);
                log.extend(
                    warnings
                        .iter()
                        .map(|w| format!("! map: {} {}", w.file, w.message)),
                );
                sim
            }
        };

        let base_tick = sim.tick;
        let mut b = Bench {
            cfg,
            views: vec![sim.view()],
            sim,
            base_tick,
            events: Vec::new(),
            names: BTreeMap::new(),
            pending: Vec::new(),
            seq: BTreeMap::new(),
            issued: BTreeMap::new(),
            timed: Vec::new(),
            healing: Vec::new(),
            selection: Vec::new(),
            paused: false,
            pause_at: s.pause.map(|p| p.ticks(TICK_HZ)),
            fire_gap: (scene.marks.fire_gap * f64::from(TICK_HZ)).round() as Tick,
            replay: replay_feed,
            rec: None,
            log,
            logged_once: BTreeSet::new(),
            artifacts: BTreeMap::new(),
            label,
        };
        b.log.push(format!(
            "scene built at tick {base_tick}, seed {}",
            s.seed.unwrap_or(1)
        ));
        for sys in scene.switched_systems() {
            b.log.push(format!("! unsupported: {}", sys.name()));
        }
        if s.camera.is_some() {
            b.log.push("camera: no renderer at tier 1, ignored".into());
        }
        b.rec = rec_setup.map(|setup| Rec {
            digests: vec![(0, b.sim.digest())],
            setup,
            commands: Vec::new(),
        });
        b.place_objects(scene)?;
        Ok(b)
    }

    fn place_objects(&mut self, scene: &SceneFile) -> Result<(), String> {
        if !scene.objects.is_empty() && self.replay.is_some() {
            self.log
                .push("! scene objects on a replay start change its digests".into());
        }
        let mut attacks = Vec::new();
        for o in &scene.objects {
            let key = self.cfg.key_of(&o.odf).ok_or_else(|| {
                format!(
                    "[Object.{}] Odf={}: not in the content database",
                    o.name, o.odf
                )
            })?;
            let owner = owner_of(o.team);
            let id = self
                .sim
                .spawn_def(key, owner, point(o.pos), heading_of(o.heading))
                .ok_or_else(|| format!("[Object.{}] {}: sim refused the spawn", o.name, o.odf))?;
            self.record_spawn(key, owner, point(o.pos), heading_of(o.heading));
            self.names.insert(o.name.clone(), id);
            if o.immortal {
                self.edit(Edit::Immortal(id, true), format!("immortal {}", o.name));
            }
            if o.heal {
                self.healing.push(id);
            }
            if !o.engines {
                self.edit(Edit::Engines(id, false), format!("engines {} off", o.name));
            }
            if !o.weapons {
                self.edit(Edit::Weapons(id, false), format!("weapons {} off", o.name));
            }
            if o.energy {
                self.log.push(format!("! unsupported: energy ({})", o.name));
            }
            for t in &o.tech {
                if let Some(k) = self.cfg.key_of(t) {
                    self.edit(Edit::Tech(owner, k), format!("tech {t}"));
                }
            }
            if o.vent.is_some() || o.population.is_some() {
                self.log
                    .push(format!("! unsupported: vent/population ({})", o.name));
            }
            if let Some(t) = &o.attack {
                attacks.push((o.name.clone(), t.clone()));
            }
        }
        for (who, target) in attacks {
            self.exec(&BenchCommand::Attack { who, target });
        }
        self.views[0] = self.sim.view();
        Ok(())
    }

    // ---- time -----------------------------------------------------------

    pub fn tick(&self) -> Tick {
        self.sim.tick
    }

    pub fn paused(&self) -> bool {
        self.paused || self.pause_at.is_some_and(|p| self.sim.tick >= p)
    }

    pub fn view(&self) -> &Arc<WorldView> {
        self.views.last().expect("tick 0 view")
    }

    /// The world at `tick`, which must have been reached.
    pub fn view_at(&self, tick: Tick) -> Result<&Arc<WorldView>, String> {
        if tick > self.sim.tick {
            return Err(format!(
                "tick {tick} not reached yet (now tick {})",
                self.sim.tick
            ));
        }
        tick.checked_sub(self.base_tick)
            .and_then(|i| self.views.get(i as usize))
            .ok_or_else(|| {
                format!(
                    "tick {tick} is before the run started (tick {})",
                    self.base_tick
                )
            })
    }

    pub fn digest_at(&self, tick: Tick) -> Result<u64, String> {
        Ok(self.view_at(tick)?.digest)
    }

    /// Advance one tick unless paused; false when it did not.
    pub fn advance(&mut self) -> bool {
        if self.paused() {
            return false;
        }
        let tick = self.sim.tick;
        let due: Vec<BenchCommand> = {
            let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.timed)
                .into_iter()
                .partition(|(t, _)| *t <= tick);
            self.timed = later;
            now.into_iter().map(|(_, c)| c).collect()
        };
        for c in &due {
            self.exec(c);
        }
        for id in self.healing.clone() {
            self.push(
                PlayerId::BENCH,
                CommandKind::Edit(Edit::Heal(id)),
                Some("heal".into()),
            );
        }
        if let Some(r) = &mut self.replay {
            while let Some((t, c)) = r.commands.get(r.next).filter(|(t, _)| *t <= tick) {
                let _ = t;
                self.pending.push(c.clone());
                r.next += 1;
            }
        }
        let batch = std::mem::take(&mut self.pending);
        if let Some(r) = &mut self.rec {
            let mut sorted: Vec<&Command> = batch.iter().collect();
            sorted.sort_by_key(|c| c.sort_key());
            r.commands.extend(sorted.into_iter().map(|c| Logged {
                tick,
                command: c.clone(),
            }));
        }
        let out = self.sim.step(&batch);
        for e in out.events {
            if let SimEvent::Rejected { player, seq } = e {
                let what = self
                    .issued
                    .get(&(player, seq))
                    .cloned()
                    .unwrap_or_else(|| format!("player {} seq {seq}", player.0));
                if what == "heal" {
                    // Continuous `Heal=1` is one edit a tick; stop when the sim has none.
                    if !self.healing.is_empty() {
                        self.log.push(format!(
                            "tick {tick}: ! rejected by the sim: heal (no longer issued)"
                        ));
                    }
                    self.healing.clear();
                } else if self.logged_once.insert(format!("rej {what}")) {
                    self.log
                        .push(format!("tick {tick}: ! rejected by the sim: {what}"));
                }
            }
            self.events.push((tick, e));
        }
        if let Some(r) = &mut self.rec
            && out.view.tick.is_multiple_of(REC_EVERY)
        {
            r.digests.push((out.view.tick, out.view.digest));
        }
        self.views.push(out.view);
        if let Some(r) = &mut self.replay {
            let now = self.sim.tick;
            while let Some(&(t, want)) = r.digests.get(r.next_digest).filter(|(t, _)| *t <= now) {
                r.next_digest += 1;
                if t == now && self.views.last().expect("view").digest != want {
                    self.log.push(format!(
                        "! replay desync at tick {t}: expected {want:016x}, found {:016x}",
                        self.views.last().expect("view").digest
                    ));
                }
            }
        }
        true
    }

    // ---- names ----------------------------------------------------------

    pub fn resolve(&self, name: &str) -> Result<EntityId, String> {
        if let Some(n) = name.strip_prefix('#') {
            return n
                .parse::<u32>()
                .map(EntityId)
                .map_err(|_| format!("`{name}`: not an id"));
        }
        if let Some(&id) = self.names.get(name) {
            return Ok(id);
        }
        self.sim
            .labels
            .iter()
            .find(|(_, l)| l.eq_ignore_ascii_case(name))
            .map(|(id, _)| id)
            .ok_or_else(|| format!("no object named `{name}`"))
    }

    pub fn name_of(&self, id: EntityId) -> String {
        self.names
            .iter()
            .find(|(_, v)| **v == id)
            .map_or_else(|| format!("#{}", id.0), |(k, _)| k.clone())
    }

    fn living(&self, name: &str) -> Result<EntityId, String> {
        let id = self.resolve(name)?;
        self.sim
            .unit(id)
            .map(|_| id)
            .ok_or_else(|| format!("`{name}` is not in the world"))
    }

    fn pos_of(&self, t: &Target) -> Result<Vec3, String> {
        match t {
            Target::Point(p) => Ok(point(*p)),
            Target::Object(n) => Ok(self.sim.unit(self.living(n)?).expect("living").pos),
        }
    }

    // ---- commands -------------------------------------------------------

    fn push(&mut self, player: PlayerId, kind: CommandKind, desc: Option<String>) {
        let n = self.seq.entry(player).or_insert(0);
        let seq = *n;
        *n += 1;
        if let Some(d) = desc {
            self.issued.insert((player, seq), d);
        }
        self.pending.push(Command { player, seq, kind });
    }

    fn edit(&mut self, e: Edit, desc: String) {
        self.push(PlayerId::BENCH, CommandKind::Edit(e), Some(desc));
    }

    fn order(&mut self, units: &[EntityId], o: Order, desc: String) {
        let owner = units
            .first()
            .and_then(|&u| self.sim.unit(u))
            .map_or(PlayerId::BENCH, |e| e.owner);
        let player = if owner == NEUTRAL {
            PlayerId::BENCH
        } else {
            owner
        };
        self.push(player, CommandKind::Order(o), Some(desc));
    }

    fn ids(&self, names: &[String]) -> Result<Vec<EntityId>, String> {
        names.iter().map(|n| self.living(n)).collect()
    }

    /// Run a command at once (queued for the next tick boundary when it is an
    /// order or an edit) and return its answer; `! ...` answers are failures
    /// of the command, not of the case.
    pub fn exec(&mut self, c: &BenchCommand) -> String {
        match self.exec_inner(c) {
            Ok(a) => a,
            Err(e) => format!("! {e}"),
        }
    }

    /// `at <time> <command>` waits for its tick; anything else runs now.
    pub fn exec_timed(&mut self, c: &TimedCommand) -> String {
        match c.at {
            Some(t) if t.ticks(TICK_HZ) > self.sim.tick => {
                let at = t.ticks(TICK_HZ);
                self.timed.push((at, c.command.clone()));
                format!("scheduled for tick {at}")
            }
            _ => self.exec(&c.command),
        }
    }

    fn exec_inner(&mut self, c: &BenchCommand) -> Result<String, String> {
        use BenchCommand as C;
        Ok(match c {
            C::Orbit { .. }
            | C::Glide { .. }
            | C::Spin { .. }
            | C::Camera { .. }
            | C::CameraRts
            | C::Zoom(_)
            | C::Center(_) => "ok (no renderer at tier 1)".into(),
            C::Display { system, .. } => format!("! unsupported: {}", system.name()),
            C::Pause => {
                self.paused = true;
                format!("paused at tick {}", self.sim.tick)
            }
            C::Resume => {
                self.paused = false;
                self.pause_at = None;
                format!("resumed at tick {}", self.sim.tick)
            }
            C::PauseAt(t) => {
                let at = t.ticks(TICK_HZ);
                self.pause_at = Some(at);
                format!("will pause at tick {at}")
            }
            C::Tick => format!("tick {}", self.sim.tick),
            C::Seed(n) => {
                self.edit(Edit::Seed(*n), format!("seed {n}"));
                format!("seed {n}")
            }
            C::Query => format!(
                "tick {} objects {} digest {:016x}",
                self.sim.tick,
                self.view().units.len(),
                self.view().digest
            ),
            C::Objects => {
                let doc = self.objects_jsonl();
                let n = self.view().units.len();
                self.artifacts.insert("objects.jsonl".into(), doc);
                format!("objects {n} -> objects.jsonl")
            }
            C::Marks => {
                let m = marks::derive(self);
                self.artifacts
                    .insert("marks".into(), marks::render(self, &m));
                format!("marks {} -> marks", m.len())
            }
            C::Selection => {
                if self.selection.is_empty() {
                    "selection: none".into()
                } else {
                    format!(
                        "selection: {}",
                        self.selection
                            .iter()
                            .map(|&i| self.name_of(i))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                }
            }
            C::Select { names, additive } => {
                let ids = self.ids(names)?;
                if !additive {
                    self.selection.clear();
                }
                for i in ids {
                    if !self.selection.contains(&i) {
                        self.selection.push(i);
                    }
                }
                format!("selected {}", self.selection.len())
            }
            C::Goto {
                who,
                to,
                warp,
                queue,
            } => {
                let ids = self.ids(who)?;
                let to = self.pos_of(to)?;
                self.order(
                    &ids,
                    Order::Move {
                        units: ids.clone(),
                        to,
                        queue: *queue,
                    },
                    format!("goto {}", who.join(",")),
                );
                if *warp {
                    self.log
                        .push("! warp: not supported, moved normally".into());
                }
                "ok".into()
            }
            C::Attack { who, target } => {
                let id = self.living(who)?;
                let t = self.living(target)?;
                self.order(
                    &[id],
                    Order::Attack {
                        units: vec![id],
                        target: t,
                    },
                    format!("attack {who} {target}"),
                );
                "ok".into()
            }
            C::Stop(n) => {
                let id = self.living(n)?;
                self.order(&[id], Order::Stop { units: vec![id] }, format!("stop {n}"));
                "ok".into()
            }
            C::Build { rig, odf, at } => {
                let id = self.living(rig)?;
                let what = self
                    .cfg
                    .key_of(odf)
                    .ok_or_else(|| format!("`{odf}` is not in the content database"))?;
                let at = self.pos_of(at)?;
                self.order(
                    &[id],
                    Order::Build {
                        builder: id,
                        what,
                        at: Some(at),
                    },
                    format!("build {rig} {odf}"),
                );
                "ok".into()
            }
            C::Rally { builders, at } => {
                let ids = self.ids(builders)?;
                let at = match at {
                    None => sta_sim::command::Target::None,
                    Some(Target::Point(p)) => sta_sim::command::Target::Point(point(*p)),
                    Some(Target::Object(name)) => {
                        sta_sim::command::Target::Entity(self.living(name)?)
                    }
                };
                self.order(
                    &ids,
                    Order::Rally {
                        builders: ids.clone(),
                        at,
                    },
                    format!("rally {}", builders.join(",")),
                );
                "ok".into()
            }
            C::Formation { who, kind } => {
                let ids = self.ids(who)?;
                self.order(
                    &ids,
                    Order::Formation {
                        units: ids.clone(),
                        kind: *kind as u8,
                    },
                    format!("formation {}", who.join(",")),
                );
                "ok".into()
            }
            C::Cloak { name, mode } => {
                let id = self.living(name)?;
                let on = !matches!(mode, CloakMode::Off);
                self.order(
                    &[id],
                    Order::Cloak {
                        units: vec![id],
                        on,
                    },
                    format!("cloak {name}"),
                );
                "ok".into()
            }
            C::Special { name, weapon, target } => {
                // The weapon's name picks the slot among the unit's specials.
                if target.is_some() {
                    return Err("unsupported: special with a target (cloak is the only special)".into());
                }
                let id = self.living(name)?;
                let content = &self.cfg.content;
                let def = self.sim.unit(id).and_then(|u| content.unit(u.def));
                let slot = def
                    .and_then(|d| {
                        sta_sim::specials::specials_of(content, d)
                            .position(|(_, w)| w.odf.eq_ignore_ascii_case(weapon))
                    })
                    .ok_or_else(|| format!("`{name}` has no special `{weapon}`"))?;
                self.order(
                    &[id],
                    Order::Special {
                        units: vec![id],
                        slot: slot as u8,
                        target: sta_sim::command::Target::None,
                    },
                    format!("special {name} {weapon}"),
                );
                "ok".into()
            }
            C::Spawn {
                name,
                odf,
                pos,
                heading,
                team,
            } => {
                let key = self
                    .cfg
                    .key_of(odf)
                    .ok_or_else(|| format!("`{odf}` is not in the content database"))?;
                let id = self
                    .sim
                    .spawn_def(key, owner_of(*team), point(*pos), heading_of(*heading))
                    .ok_or_else(|| "the sim refused the spawn".to_string())?;
                self.record_spawn(key, owner_of(*team), point(*pos), heading_of(*heading));
                self.names.insert(name.clone(), id);
                format!("spawned #{}", id.0)
            }
            C::Kill(n) => {
                let id = self.living(n)?;
                self.edit(Edit::Kill(id), format!("kill {n}"));
                "ok".into()
            }
            C::Team { name, team } => {
                let id = self.living(name)?;
                self.edit(
                    Edit::Team(id, owner_of(*team)),
                    format!("team {name} {team}"),
                );
                "ok".into()
            }
            C::Damage { name, amount } => {
                let id = self.living(name)?;
                self.edit(
                    Edit::Damage(id, Fixed::from_f64(*amount)),
                    format!("damage {name}"),
                );
                "ok".into()
            }
            C::Set { switch, name, on } => {
                let id = self.living(name)?;
                match switch {
                    Switch::Heal => {
                        self.healing.retain(|&h| h != id);
                        if *on {
                            self.healing.push(id);
                        }
                    }
                    Switch::Engines => self.edit(Edit::Engines(id, *on), format!("engines {name}")),
                    Switch::Weapons => self.edit(Edit::Weapons(id, *on), format!("weapons {name}")),
                    Switch::Immortal => {
                        self.edit(Edit::Immortal(id, *on), format!("immortal {name}"))
                    }
                    Switch::Energy => return Err("unsupported: energy".into()),
                }
                "ok".into()
            }
            C::Tech { who, item, on } => {
                if !on {
                    return Err("unsupported: tech off".into());
                }
                let p = match who {
                    TechWho::Team(t) => owner_of(*t),
                    TechWho::Object(n) => self.sim.unit(self.living(n)?).expect("living").owner,
                };
                let key: ContentKey = self
                    .cfg
                    .key_of(item)
                    .ok_or_else(|| format!("`{item}` is not in the content database"))?;
                self.edit(Edit::Tech(p, key), format!("tech {item}"));
                "ok".into()
            }
            C::Cargo {
                name,
                resource,
                amount,
            } => {
                let id = self.living(name)?;
                let owner = self.sim.unit(id).expect("living").owner;
                let res = match resource {
                    Resource::Dilithium => 0,
                    Resource::Latinum => 1,
                    Resource::Metal => 2,
                    Resource::Biomatter => 3,
                    _ => return Err("unsupported: cargo crew/officers".into()),
                };
                let level = match amount {
                    CargoAmount::Empty => 0.0,
                    CargoAmount::Level(v) => *v,
                    CargoAmount::Full => return Err("unsupported: cargo full".into()),
                };
                self.edit(
                    Edit::Cargo(owner, res, Fixed::from_f64(level)),
                    format!("cargo {name}"),
                );
                "ok".into()
            }
            C::Unknown { verb, text } if verb == "mine" => {
                // Runner extension: `mine <ship>... <node>`.
                let words: Vec<String> =
                    text.split_whitespace().skip(1).map(str::to_owned).collect();
                let (node, ships) = words.split_last().ok_or("usage: mine <ship>... <node>")?;
                let ids = self.ids(ships)?;
                let node = self.resolve(node)?;
                self.order(
                    &ids,
                    Order::Mine {
                        units: ids.clone(),
                        node,
                    },
                    format!("mine {}", ships.join(",")),
                );
                "ok".into()
            }
            C::Colonize { .. } | C::Vent { .. } | C::EndMission { .. } => {
                return Err("unsupported: colonize/vent/succeed/fail".into());
            }
            C::Unknown { verb, .. } => return Err(format!("unsupported: {verb}")),
        })
    }

    // ---- state ----------------------------------------------------------

    pub fn objects_jsonl(&self) -> String {
        let v = self.view();
        let db = &self.cfg.content;
        let mut out = format!(
            "{{\"schema\":1,\"build\":\"sta64-sim {} {:?}\",\"label\":{},\"tick\":{},\"digest\":\"{:016x}\"}}\n",
            env!("CARGO_PKG_VERSION"),
            db.profile(),
            json::string(&self.label),
            v.tick,
            v.digest
        );
        for u in &v.units {
            let e = self.sim.unit(u.id);
            let (odf, class) = db.unit(u.def).map_or((String::new(), String::new()), |d| {
                (d.odf.clone(), d.class.0.clone())
            });
            let kind = e.map_or("?".to_string(), |e| format!("{:?}", e.kind).to_lowercase());
            let team = if u.owner == NEUTRAL {
                0
            } else {
                u32::from(u.owner.0)
            };
            out += &format!(
                "{{\"id\":{},\"name\":{},\"odf\":{},\"class\":{},\"team\":{},\"mine\":false,\"kind\":{},\"pos\":[{},{},{}],\"hull\":[{},{}],\"shields\":[{},{}],\"selected\":{},\"group\":null,\"dying\":{}}}\n",
                u.id.0,
                json::string(&self.name_of(u.id)),
                json::string(&odf),
                json::string(&class),
                team,
                json::string(&kind),
                json::num(to_f64(u.pos.x)),
                json::num(to_f64(u.pos.y)),
                json::num(to_f64(u.pos.z)),
                json::num(to_f64(u.hull[0])),
                json::num(to_f64(u.hull[1])),
                json::num(to_f64(u.shields[0][0])),
                json::num(to_f64(u.shields[0][1])),
                self.selection.contains(&u.id),
                u.flags.contains(sta_sim::view::UnitFlags::DYING),
            );
        }
        out
    }

    pub fn marks(&self) -> Vec<Mark> {
        marks::derive(self)
    }

    /// Write the snapshot of the current state under the data directory.
    pub fn save_snapshot(&self, file: &str) -> Result<String, String> {
        let path = self.cfg.data_path(file);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&path, self.sim.snapshot())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!(
            "snapshot at tick {} -> {}",
            self.sim.tick,
            path.display()
        ))
    }

    /// Log a placement made with `Sim::spawn_def` as the `Edit::Spawn` that
    /// makes it on a replay, on the tick it happened.
    fn record_spawn(
        &mut self,
        odf: ContentKey,
        player: PlayerId,
        pos: Vec3,
        heading: (Fixed, Fixed),
    ) {
        if self.rec.is_none() {
            return;
        }
        let n = self.seq.entry(PlayerId::BENCH).or_insert(0);
        let seq = *n;
        *n += 1;
        let tick = self.sim.tick;
        let command = Command {
            player: PlayerId::BENCH,
            seq,
            kind: CommandKind::Edit(Edit::Spawn {
                odf,
                player,
                pos,
                heading,
            }),
        };
        if let Some(r) = &mut self.rec {
            r.commands.push(Logged { tick, command });
        }
    }

    /// Write what has run so far as a replay file under the data directory,
    /// and play it back from tick 0 to check that it reproduces every digest.
    pub fn save_replay(&self, file: &str) -> Result<String, String> {
        let r = self
            .rec
            .as_ref()
            .ok_or("no replay can be recorded from a snapshot or replay start".to_string())?;
        let end = self.sim.tick;
        let mut digests = r.digests.clone();
        if digests.last().map(|d| d.0) != Some(end) {
            digests.push((end, self.sim.digest()));
        }
        let replay = Replay {
            seam_rev: sta_sim::SEAM_REV,
            content_hash: r.setup.content.hash(),
            seed: r.setup.seed,
            map: (*r.setup.map).clone(),
            players: r
                .setup
                .players
                .iter()
                .map(|p| SeatRec {
                    id: p.id,
                    team: p.team,
                    race: p.race.clone(),
                    control: ControlRec::Idle,
                })
                .collect(),
            options: r.setup.options.clone(),
            starts: sta_sim::replay::StartRec::Bare,
            commands: r.commands.clone(),
            digests,
            end_tick: end,
        };
        replay
            .play(r.setup.content.clone())
            .map_err(|e| format!("the recording does not replay: {e}"))?;
        let path = self.cfg.data_path(file);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&path, replay.encode()).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!(
            "replay of {end} ticks, {} commands -> {}",
            replay.commands.len(),
            path.display()
        ))
    }

    /// Whether `t` is already behind us (an `at` time that would not wait).
    pub fn reached(&self, t: TimeSpec) -> bool {
        t.ticks(TICK_HZ) <= self.sim.tick
    }
}
