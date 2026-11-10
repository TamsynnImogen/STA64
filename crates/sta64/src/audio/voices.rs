//! Unit voices: select and order acknowledgements, under-attack alerts (AUD-03).
//!
//! What a unit says comes from its ODF through `UnitDef::sounds` (CON-03): `eventSelect`
//! for selecting, `eventAcknowledge` for a move (and, until CON-03 maps `eventMove` /
//! `eventAttack`, for attacks too: [`pick_ack`] prefers `ack_attack` when the def has one
//! and falls back to `ack_move`). Each is an event name from `events.dat`; the resolver
//! (CON-10) picks the file, by the race set with [`SoundResolver::set_race`] and rotating
//! through the variants.
//!
//! - [`SelectionChanged`] plays the first selected unit's select line.
//! - [`OrderIssued`] plays the first ordered unit's ack: move and attack only; stop,
//!   build and the rest are silent (their `eventStop`, `eventRepair`, ... are unmapped).
//! - A [`SimEvent::Hit`] on one of the local player's units by another player plays
//!   [`VoiceNames::base_under_attack`] for a facility and [`VoiceNames::fleet_engaged`]
//!   for a ship (both stock advisor events), at most once per [`VoiceConfig::alert_gap`].
//!
//! Everything is on [`Bus::Voice`] and not positional. [`RateLimiter`] keeps it from
//! turning into noise: a cooldown per kind of voice plus a short gap between any two.
//!
//! Race: the trait method [`SoundResolver::set_race`] was the smallest change that works
//! for `BankResolver`, which already takes a race. `PlaySound` (a seam type) is left alone
//! and no new resource is needed: [`sync_race`] reads the race of the local player's first
//! unit from the view and passes it on when it changes. It applies to every event-named
//! sound, so advisor lines of AUD-02 and AUD-04 follow it too.

use bevy::prelude::*;
use bevy::time::Real;
use sta_content::UnitDef;
use sta_sim::event::SimEvent;
use sta_sim::{EntityId, Fixed, PlayerId};

use super::{Resolver, play_sounds};
use crate::app::GameSet;
use crate::render::units::LocalPlayer;
use crate::sim_link::{Bus, ContentRes, OrderIssued, OrderKind, PlaySound, Selection, SelectionChanged, SimLink, SimMsg};

/// Event names for alerts, which no ODF carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceNames {
    pub base_under_attack: String,
    pub fleet_engaged: String,
}

impl Default for VoiceNames {
    fn default() -> Self {
        Self { base_under_attack: "BaseUnderAttack".into(), fleet_engaged: "EnemyEngaged".into() }
    }
}

/// Tunables of the voice layer; seconds.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct VoiceConfig {
    pub names: VoiceNames,
    /// Between two select lines.
    pub select_gap: f64,
    /// Between two order acks.
    pub ack_gap: f64,
    /// Between two alerts of one kind.
    pub alert_gap: f64,
    /// Between any two unit voices (select or ack).
    pub any_gap: f64,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self { names: VoiceNames::default(), select_gap: 0.3, ack_gap: 0.8, alert_gap: 20.0, any_gap: 0.25 }
    }
}

/// A kind of voice, for rate limiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceKind {
    Select,
    Ack,
    BaseAlert,
    FleetAlert,
}

impl VoiceKind {
    const ALL: [VoiceKind; 4] = [VoiceKind::Select, VoiceKind::Ack, VoiceKind::BaseAlert, VoiceKind::FleetAlert];

    fn gap(self, cfg: &VoiceConfig) -> f64 {
        match self {
            VoiceKind::Select => cfg.select_gap,
            VoiceKind::Ack => cfg.ack_gap,
            VoiceKind::BaseAlert | VoiceKind::FleetAlert => cfg.alert_gap,
        }
    }

    fn is_unit_voice(self) -> bool {
        matches!(self, VoiceKind::Select | VoiceKind::Ack)
    }
}

/// Rate limits voices: a cooldown per kind, and a short gap between any two unit voices.
/// Pure, so tests need no clock.
#[derive(Debug, Default)]
pub struct RateLimiter {
    last: [Option<f64>; 4],
}

impl RateLimiter {
    fn slot(kind: VoiceKind) -> usize {
        VoiceKind::ALL.iter().position(|k| *k == kind).unwrap()
    }

    /// Whether `kind` may speak at `now`; records it if so.
    pub fn admit(&mut self, now: f64, kind: VoiceKind, cfg: &VoiceConfig) -> bool {
        let ready = |t: Option<f64>, gap: f64| t.is_none_or(|t| now - t >= gap);
        if !ready(self.last[Self::slot(kind)], kind.gap(cfg)) {
            return false;
        }
        if kind.is_unit_voice() {
            let recent = VoiceKind::ALL.iter().filter(|k| k.is_unit_voice()).filter_map(|k| self.last[Self::slot(*k)]).fold(f64::NEG_INFINITY, f64::max);
            if now - recent < cfg.any_gap {
                return false;
            }
        }
        self.last[Self::slot(kind)] = Some(now);
        true
    }
}

/// The line a unit says for an order, by its ODF: attack prefers `ack_attack`, anything
/// else that speaks uses `ack_move`. `variant` rotates through a list of names.
pub fn pick_ack(def: &UnitDef, kind: OrderKind, variant: usize) -> Option<&str> {
    let list = match kind {
        OrderKind::Attack if !def.sounds.ack_attack.is_empty() => &def.sounds.ack_attack,
        OrderKind::Move | OrderKind::Attack => &def.sounds.ack_move,
        _ => return None,
    };
    (!list.is_empty()).then(|| list[variant % list.len()].as_str())
}

/// The line a unit says when selected.
pub fn pick_select(def: &UnitDef, variant: usize) -> Option<&str> {
    let list = &def.sounds.select;
    (!list.is_empty()).then(|| list[variant % list.len()].as_str())
}

#[derive(Resource, Default)]
struct VoiceState {
    limiter: RateLimiter,
    variant: usize,
}

/// Adds the select / ack / alert voices and the race hook.
pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SimMsg>()
            .add_message::<OrderIssued>()
            .add_message::<SelectionChanged>()
            .add_message::<PlaySound>()
            .init_resource::<VoiceConfig>()
            .init_resource::<VoiceState>()
            .init_resource::<Selection>()
            .add_systems(Update, (sync_race, run_voices).chain().in_set(GameSet::Audio).before(play_sounds));
    }
}

/// Tell the resolver the local player's race, when it changes.
fn sync_race(
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<LocalPlayer>>,
    mut resolver: ResMut<Resolver>,
    mut sent: Local<Option<Option<String>>>,
) {
    let (Some(link), Some(content), Some(me)) = (link, content, local.and_then(|l| l.0)) else { return };
    let race = link.curr().units.iter().find(|u| u.owner == me).and_then(|u| content.0.unit(u.def)).map(|d| d.race.clone());
    if race.is_some() && sent.as_ref() != Some(&race) {
        resolver.0.set_race(race.as_deref());
        *sent = Some(race);
    }
}

fn is_facility(def: &UnitDef) -> bool {
    def.movement.max_speed == Fixed::ZERO
}

#[allow(clippy::too_many_arguments)]
fn run_voices(
    time: Option<Res<Time<Real>>>,
    link: Option<Res<SimLink>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<LocalPlayer>>,
    cfg: Res<VoiceConfig>,
    selection: Res<Selection>,
    mut selected: MessageReader<SelectionChanged>,
    mut orders: MessageReader<OrderIssued>,
    mut sim: MessageReader<SimMsg>,
    mut st: ResMut<VoiceState>,
    mut out: MessageWriter<PlaySound>,
) {
    // The menu scene's ships are scenery: no unit speaks in the front end.
    let (Some(link), Some(content), None) = (link, content, scene) else {
        selected.clear();
        orders.clear();
        sim.clear();
        return;
    };
    let st = &mut *st;
    let db = &content.0;
    let now = time.map_or(0.0, |t| t.elapsed_secs_f64());
    let me: PlayerId = local.and_then(|l| l.0).unwrap_or(link.player());
    let def_of = |id: EntityId| link.curr().unit(id).or_else(|| link.prev().unit(id)).and_then(|u| db.unit(u.def));
    let mut say = |st: &mut VoiceState, kind: VoiceKind, name: &str| {
        if st.limiter.admit(now, kind, &cfg) {
            out.write(PlaySound { name: name.to_owned(), at: None, bus: Bus::Voice });
        }
    };

    if selected.read().count() > 0 {
        st.variant = st.variant.wrapping_add(1);
        if let Some(name) = selection.0.iter().find_map(|&id| def_of(id).and_then(|d| pick_select(d, st.variant))) {
            say(st, VoiceKind::Select, name);
        }
    }
    for o in orders.read() {
        st.variant = st.variant.wrapping_add(1);
        if let Some(name) = o.units.iter().find_map(|&id| def_of(id).and_then(|d| pick_ack(d, o.kind, st.variant))) {
            say(st, VoiceKind::Ack, name);
        }
    }
    for SimMsg(ev) in sim.read() {
        let SimEvent::Hit { target, by, .. } = ev else { continue };
        let owner_of = |id: &EntityId| link.curr().unit(*id).or_else(|| link.prev().unit(*id)).map(|u| u.owner);
        if owner_of(target) != Some(me) || owner_of(by).is_none_or(|o| o == me) {
            continue;
        }
        let facility = def_of(*target).is_some_and(is_facility);
        let (kind, name) = if facility { (VoiceKind::BaseAlert, &cfg.names.base_under_attack) } else { (VoiceKind::FleetAlert, &cfg.names.fleet_engaged) };
        say(st, kind, name);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AudioPlugin, Call, CallLog, Mixer, NullAudio, Sound, SoundResolver};
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use sta_content::ContentDb;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;

    /// Resolves every name; records the race it was given.
    struct Any(Arc<Mutex<Vec<Option<String>>>>);
    impl SoundResolver for Any {
        fn resolve(&mut self, name: &str) -> Option<Sound> {
            Some(Sound { name: name.to_owned(), sample_rate: 8000, frames: Arc::from(vec![[0.0, 0.0]]) })
        }
        fn set_race(&mut self, race: Option<&str>) {
            self.0.lock().unwrap().push(race.map(str::to_owned));
        }
    }

    fn db() -> ContentDb {
        let mut cruiser = UnitDef::new("cruiser", "craft");
        cruiser.race = "federation".into();
        cruiser.movement = sta_content::MoveStats::from_f32(100.0, 1.0, 1.0, 1.0, 0.2);
        cruiser.sounds.select = vec!["CruiserSelect".into()];
        cruiser.sounds.ack_move = vec!["CruiserAck".into()];
        let mut warbird = UnitDef::new("warbird", "craft");
        warbird.race = "romulan".into();
        warbird.movement = cruiser.movement;
        warbird.sounds.select = vec!["WarbirdSelect1".into(), "WarbirdSelect2".into()];
        warbird.sounds.ack_move = vec!["WarbirdAck".into()];
        warbird.sounds.ack_attack = vec!["WarbirdAttack".into()];
        let mut mute = UnitDef::new("mute", "craft");
        mute.movement = cruiser.movement;
        let mut base = UnitDef::new("base", "starbase");
        base.race = "federation".into();
        ContentDb::builder().unit(cruiser).unit(warbird).unit(mute).unit(base).build().unwrap()
    }

    const WORLD: &str = r#"{ units: [
        { id: 1, def: "cruiser", owner: 0, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
        { id: 2, def: "warbird", owner: 0, rails: [{ at: 0, pos: { x: 10, y: 0, z: 0 } }] },
        { id: 3, def: "mute", owner: 0, rails: [{ at: 0, pos: { x: 20, y: 0, z: 0 } }] },
        { id: 4, def: "base", owner: 0, rails: [{ at: 0, pos: { x: 30, y: 0, z: 0 } }] },
        { id: 9, def: "warbird", owner: 1, rails: [{ at: 0, pos: { x: 500, y: 0, z: 0 } }] },
      ], events: [] }"#;

    struct Rig {
        app: App,
        log: CallLog,
        races: Arc<Mutex<Vec<Option<String>>>>,
    }

    fn rig(timeline: &str) -> Rig {
        let db = db();
        let source = ScriptedSource::from_json5(timeline, &db).unwrap();
        let (null, log) = NullAudio::new();
        let races = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(0.1)))
            .configure_sets(Update, (GameSet::SimLink, GameSet::Audio).chain())
            .insert_non_send(Mixer::new(Box::new(null)))
            .add_plugins((crate::sim_link::SimLinkPlugin, AudioPlugin::default()))
            .insert_resource(Resolver(Box::new(Any(races.clone()))))
            .insert_resource(ContentRes(Arc::new(db)))
            .insert_resource(LocalPlayer(Some(PlayerId(0))))
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(0)));
        app.update();
        app.update();
        log.drain();
        Rig { app, log, races }
    }

    impl Rig {
        /// One frame; the voice names played in it.
        fn frame(&mut self) -> Vec<String> {
            self.app.update();
            self.log
                .drain()
                .into_iter()
                .filter_map(|c| match c {
                    // SfxPlugin is part of AudioPlugin: leave its positional sounds out.
                    Call::Play { name, bus: Bus::Voice, at } => {
                        assert_eq!(at, None, "voices are non-positional");
                        Some(name)
                    }
                    _ => None,
                })
                .collect()
        }

        fn order(&mut self, units: &[u32], kind: OrderKind) {
            self.app.world_mut().write_message(OrderIssued { units: units.iter().map(|&u| EntityId(u)).collect(), kind });
        }

        fn select(&mut self, units: &[u32]) {
            self.app.world_mut().resource_mut::<Selection>().0 = units.iter().map(|&u| EntityId(u)).collect();
            self.app.world_mut().write_message(SelectionChanged);
        }

        /// Frames until `secs` have passed (0.1 s each), discarding the log.
        fn wait(&mut self, secs: f64) {
            for _ in 0..(secs / 0.1).ceil() as usize {
                self.frame();
            }
        }
    }

    #[test]
    fn ack_is_chosen_per_odf() {
        let mut r = rig(WORLD);
        r.order(&[1], OrderKind::Move);
        assert_eq!(r.frame(), ["CruiserAck"]);
        r.wait(1.0);
        r.order(&[2], OrderKind::Move);
        assert_eq!(r.frame(), ["WarbirdAck"]);
        r.wait(1.0);
        // Attack: the def's own attack line, else its move line.
        r.order(&[2], OrderKind::Attack);
        assert_eq!(r.frame(), ["WarbirdAttack"]);
        r.wait(1.0);
        r.order(&[1], OrderKind::Attack);
        assert_eq!(r.frame(), ["CruiserAck"]);
        r.wait(1.0);
        // The first unit that has something to say speaks; units that say nothing are skipped.
        r.order(&[3, 2, 1], OrderKind::Move);
        assert_eq!(r.frame(), ["WarbirdAck"]);
        r.wait(1.0);
        // Silent orders and silent units.
        r.order(&[1], OrderKind::Stop);
        r.order(&[1], OrderKind::Build);
        assert!(r.frame().is_empty());
        r.order(&[3], OrderKind::Move);
        assert!(r.frame().is_empty());
    }

    #[test]
    fn select_uses_the_def_and_rotates_variants() {
        let mut r = rig(WORLD);
        r.select(&[1]);
        assert_eq!(r.frame(), ["CruiserSelect"]);
        r.wait(0.5);
        r.select(&[3, 2]);
        let first = r.frame();
        assert_eq!(first.len(), 1);
        assert!(first[0].starts_with("WarbirdSelect"));
        r.wait(0.5);
        r.select(&[2]);
        let second = r.frame();
        assert_ne!(first, second, "the variant rotated");
    }

    #[test]
    fn acks_are_rate_limited() {
        let mut r = rig(WORLD);
        r.order(&[1], OrderKind::Move);
        assert_eq!(r.frame().len(), 1);
        // Ten more orders within a second (0.1 s frames) make one more ack once the gap (0.8 s) has passed.
        let mut heard = 0;
        for _ in 0..7 {
            r.order(&[1], OrderKind::Move);
            heard += r.frame().len();
        }
        assert_eq!(heard, 0, "inside the ack gap");
        r.order(&[1], OrderKind::Move);
        assert_eq!(r.frame().len(), 1, "after the gap");
        // A select right after an ack is held back by the shared gap only briefly.
        r.select(&[1]);
        assert!(r.frame().is_empty(), "any-voice gap");
        r.wait(0.3);
        r.select(&[1]);
        assert_eq!(r.frame().len(), 1);
    }

    #[test]
    fn limiter_gaps() {
        let cfg = VoiceConfig::default();
        let mut l = RateLimiter::default();
        assert!(l.admit(0.0, VoiceKind::Ack, &cfg));
        assert!(!l.admit(0.5, VoiceKind::Ack, &cfg));
        assert!(!l.admit(0.1, VoiceKind::Select, &cfg), "any-voice gap");
        assert!(l.admit(0.3, VoiceKind::Select, &cfg));
        assert!(l.admit(0.9, VoiceKind::Ack, &cfg));
        // Alerts are on their own clock, and long.
        assert!(l.admit(1.0, VoiceKind::BaseAlert, &cfg));
        assert!(l.admit(1.0, VoiceKind::FleetAlert, &cfg));
        assert!(!l.admit(15.0, VoiceKind::BaseAlert, &cfg));
        assert!(l.admit(21.5, VoiceKind::BaseAlert, &cfg));
    }

    #[test]
    fn under_attack_alerts_by_kind_and_cooldown() {
        let tl = r#"{ units: [
            { id: 1, def: "cruiser", owner: 0, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
            { id: 4, def: "base", owner: 0, rails: [{ at: 0, pos: { x: 30, y: 0, z: 0 } }] },
            { id: 9, def: "warbird", owner: 1, rails: [{ at: 0, pos: { x: 500, y: 0, z: 0 } }] },
          ],
          events: [
            { at: 20, event: { Hit: { target: 4, by: 9, weapon: 0, facing: 0, shield: 0, hull: 5, at: { x: 30, y: 0, z: 0 } } } },
            { at: 20, event: { Hit: { target: 1, by: 9, weapon: 0, facing: 0, shield: 0, hull: 5, at: { x: 0, y: 0, z: 0 } } } },
            { at: 21, event: { Hit: { target: 4, by: 9, weapon: 0, facing: 0, shield: 0, hull: 5, at: { x: 30, y: 0, z: 0 } } } },
            { at: 21, event: { Hit: { target: 9, by: 1, weapon: 0, facing: 0, shield: 0, hull: 5, at: { x: 500, y: 0, z: 0 } } } },
          ] }"#;
        let mut r = rig(tl);
        let mut all = Vec::new();
        for _ in 0..40 {
            all.extend(r.frame());
        }
        assert_eq!(all, ["BaseUnderAttack", "EnemyEngaged"], "ship and base alerts once each; our own hits on the enemy are silent");
    }

    #[test]
    fn race_follows_the_local_players_units() {
        let r = rig(WORLD);
        // Unit 1 (first owned) is a federation cruiser.
        assert_eq!(*r.races.lock().unwrap(), [Some("federation".to_owned())]);
    }
}
