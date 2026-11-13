//! Positional sound effects from sim events (AUD-02).
//!
//! [`SfxPlugin`] turns [`SimMsg`] into [`PlaySound`] requests on [`Bus::Sfx`], placed
//! in map space:
//!
//! - `Fired`: the weapon's `fireSound` at the shooter.
//! - `Hit`: [`SfxNames::shield_hit`] when shields took damage, else [`SfxNames::hull_hit`],
//!   at the impact point. (`WeaponDef` has no `hitSound` yet; until CON-03 maps it these
//!   are the stock `xhit_*` files.)
//! - `Destroyed`: the unit's `sounds.explode`, at its last known position.
//! - `Built`: [`SfxNames::built_ship`] / [`SfxNames::built_facility`] (stock advisor events)
//!   at the new unit, for the local player's own builds only.
//!
//! Only things the local player can see make sound. Distance attenuation and panning
//! are the backend's (it knows the listener, see `sync_listener`); this module adds
//! what the backend cannot: [`SfxConfig::max_distance`] culls what could not be
//! heard anyway, and [`VoiceLimiter`] caps how many voices start, per sound name and
//! overall, nearest to the camera first. A big battle fires hundreds of beams a second;
//! without a cap the mix clips and the device queue grows.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy::time::Real;
use sta_assets::audio::AudioParams;
use sta_sim::event::SimEvent;
use sta_sim::{ContentKey, EntityId, Fixed, PlayerId, Vec3 as SimVec3};

use super::play_sounds;
use crate::app::GameSet;
use crate::render::units::LocalPlayer;
use crate::sim_link::{Bus, ContentRes, PlaySound, SimLink, SimMsg};

/// Sound names the sim events do not carry themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SfxNames {
    pub shield_hit: String,
    pub hull_hit: String,
    pub built_ship: String,
    pub built_facility: String,
}

impl Default for SfxNames {
    fn default() -> Self {
        Self {
            shield_hit: "xhit_pulse1.wav".into(),
            hull_hit: "xhit_pulse2.wav".into(),
            built_ship: "ShipConstructionComplete".into(),
            built_facility: "FacilityConstructionComplete".into(),
        }
    }
}

/// Tunables of the SFX layer.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct SfxConfig {
    pub names: SfxNames,
    /// Beyond this distance from the camera (map units) a sound is not started.
    pub max_distance: f32,
    /// Most voices started in any [`SfxConfig::window`] seconds.
    pub max_voices: usize,
    /// Most voices of one sound name started in any window.
    pub max_per_name: usize,
    /// Length of the limiter's window, seconds.
    pub window: f64,
}

impl Default for SfxConfig {
    fn default() -> Self {
        Self { names: SfxNames::default(), max_distance: 3000.0, max_voices: 24, max_per_name: 3, window: 0.25 }
    }
}

impl SfxConfig {
    /// `audio.odf`'s `maximumDistance3D` as the cull distance.
    pub fn from_params(p: &AudioParams) -> Self {
        Self { max_distance: p.maximum_distance_3d, ..Self::default() }
    }
}

/// Admits or refuses voices: at most `max_per_name` of a name and `max_voices` in all
/// within `window` seconds. Pure, so tests need no clock.
#[derive(Debug, Default)]
pub struct VoiceLimiter {
    started: Vec<(f64, String)>,
}

impl VoiceLimiter {
    /// Whether a voice named `name` may start at `now`; records it if so.
    pub fn admit(&mut self, now: f64, name: &str, cfg: &SfxConfig) -> bool {
        self.started.retain(|(t, _)| now - t < cfg.window);
        if self.started.len() >= cfg.max_voices {
            return false;
        }
        if self.started.iter().filter(|(_, n)| n.eq_ignore_ascii_case(name)).count() >= cfg.max_per_name {
            return false;
        }
        self.started.push((now, name.to_owned()));
        true
    }

    pub fn active(&self) -> usize {
        self.started.len()
    }
}

#[derive(Resource, Default)]
struct SfxState {
    limiter: VoiceLimiter,
    /// Last known position, definition and owner of every unit seen, for events
    /// whose unit is already gone from the view.
    seen: HashMap<EntityId, (SimVec3, ContentKey, PlayerId)>,
}

/// Adds the sim-event to [`PlaySound`] mapping with its config and voice limits.
pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SimMsg>()
            .add_message::<PlaySound>()
            .init_resource::<SfxConfig>()
            .init_resource::<SfxState>()
            .add_systems(Update, run_sfx.in_set(GameSet::Audio).before(play_sounds));
    }
}

/// Map-space position of the camera (the renderer mirrors Z).
fn camera_map_pos(cam: &Query<&GlobalTransform, crate::map::MainCamera>) -> Option<Vec3> {
    cam.iter().next().map(|t| {
        let p = t.translation();
        Vec3::new(p.x, p.y, -p.z)
    })
}

#[allow(clippy::too_many_arguments)]
fn run_sfx(
    time: Option<Res<Time<Real>>>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<LocalPlayer>>,
    cfg: Res<SfxConfig>,
    cam: Query<&GlobalTransform, crate::map::MainCamera>,
    mut msgs: MessageReader<SimMsg>,
    mut st: ResMut<SfxState>,
    mut out: MessageWriter<PlaySound>,
) {
    let (Some(link), Some(content)) = (link, content) else {
        msgs.clear();
        return;
    };
    let st = &mut *st;
    // Remember where things were: a destroyed unit is already gone from `curr`.
    for u in link.prev().units.iter().chain(link.curr().units.iter()) {
        st.seen.insert(u.id, (u.pos, u.def, u.owner));
    }
    let db = &content.0;
    let me = local.and_then(|l| l.0);
    let view = link.curr();
    let visible: Option<HashSet<EntityId>> =
        me.filter(|_| !view.visibility.is_empty()).map(|p| view.visible_to(p).into_iter().map(|u| u.id).collect());
    // Heard when the unit is the player's own or in the player's visible list; with no
    // fog data or no local player, everything is.
    let sees = |id: EntityId, owner: PlayerId| match (&visible, me) {
        (Some(v), Some(p)) => v.contains(&id) || owner == p,
        _ => true,
    };

    let mut want: Vec<(String, SimVec3)> = Vec::new();
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::Fired { shooter, weapon, .. } => {
                let (Some(&(pos, _, owner)), Some(name)) = (st.seen.get(shooter), db.weapon(*weapon).and_then(|w| w.fire_sound.clone())) else {
                    continue;
                };
                if sees(*shooter, owner) {
                    want.push((name, pos));
                }
            }
            SimEvent::Hit { target, shield, at, .. } => {
                let names = &cfg.names;
                let name = if *shield > Fixed::ZERO { &names.shield_hit } else { &names.hull_hit };
                if st.seen.get(target).is_none_or(|s| sees(*target, s.2)) {
                    want.push((name.clone(), *at));
                }
            }
            SimEvent::Destroyed { id, .. } => {
                let Some(&(pos, def, owner)) = st.seen.get(id) else { continue };
                if let Some(name) = db.unit(def).and_then(|u| u.sounds.explode.clone())
                    && sees(*id, owner)
                {
                    want.push((name, pos));
                }
            }
            SimEvent::Built { builder, id } => {
                let Some(&(pos, def, owner)) = st.seen.get(id).or_else(|| st.seen.get(builder)) else { continue };
                if me.is_some_and(|p| p != owner) {
                    continue;
                }
                let facility = db.unit(def).is_some_and(|u| u.movement.max_speed == Fixed::ZERO);
                let names = &cfg.names;
                want.push((if facility { names.built_facility.clone() } else { names.built_ship.clone() }, pos));
            }
            _ => {}
        }
    }
    if want.is_empty() {
        return;
    }
    // Nearest first: the voice budget goes to what the player is closest to.
    let cam_pos = camera_map_pos(&cam);
    let dist = |p: &SimVec3| cam_pos.map_or(0.0, |c| Vec3::from(p.to_f32()).distance(c));
    want.sort_by(|a, b| dist(&a.1).total_cmp(&dist(&b.1)));
    let now = time.map_or(0.0, |t| t.elapsed_secs_f64());
    for (name, at) in want {
        if dist(&at) > cfg.max_distance || !st.limiter.admit(now, &name, &cfg) {
            continue;
        }
        out.write(PlaySound { name, at: Some(at), bus: Bus::Sfx });
    }
    // The unit table only needs what events can still ask about.
    if st.seen.len() > 4096 {
        st.seen.retain(|id, _| link.curr().unit(*id).is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AudioPlugin, Call, CallLog, Mixer, NullAudio, Resolver, Sound, SoundResolver};
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::sync::Arc;
    use std::time::Duration;

    /// Resolves every name to a 1-frame sound named after the request.
    struct Any;
    impl SoundResolver for Any {
        fn resolve(&mut self, name: &str) -> Option<Sound> {
            Some(Sound { name: name.to_owned(), sample_rate: 8000, frames: Arc::from(vec![[0.0, 0.0]]) })
        }
    }

    /// `firing`: a cruiser at the origin shoots a frigate 600 units off (+Z) with a beam
    /// at tick 3 and a torpedo at tick 4; hits at 5 and 6; the frigate dies at 8.
    const FIRING: &str = r#"{
        units: [
          { id: 1, def: "fx_cruiser", owner: 0, hull: 800, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
          { id: 2, def: "fx_frigate", owner: 1, hull: 300, until: 8, rails: [{ at: 0, pos: { x: 0, y: 0, z: 600 } }] },
        ],
        events: [
          { at: 3, event: { Fired: { shooter: 1, weapon: BEAM, hardpoint: 0, target: { Entity: 2 } } } },
          { at: 4, event: { Fired: { shooter: 1, weapon: TORP, hardpoint: 1, target: { Entity: 2 } } } },
          { at: 5, event: { Hit: { target: 2, by: 1, weapon: BEAM, facing: 0, shield: 10, hull: 0, at: { x: 0, y: 0, z: 590 } } } },
          { at: 6, event: { Hit: { target: 2, by: 1, weapon: TORP, facing: 0, shield: 0, hull: 60, at: { x: 0, y: 0, z: 590 } } } },
        ],
    }"#;

    type Played = Vec<(String, Bus, Option<[f32; 3]>)>;

    fn scene(timeline: &str, local: Option<PlayerId>) -> (App, CallLog) {
        let db = sta_content::fake::db();
        let text = timeline.replace("BEAM", &db.key("fx_beam").unwrap().0.to_string()).replace("TORP", &db.key("fx_torpedo").unwrap().0.to_string());
        let source = ScriptedSource::from_json5(&text, &db).unwrap();
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .configure_sets(Update, (GameSet::SimLink, GameSet::Audio).chain())
            .insert_non_send(Mixer::new(Box::new(null)))
            .add_plugins((crate::sim_link::SimLinkPlugin, AudioPlugin::default()))
            .insert_resource(Resolver(Box::new(Any)))
            .insert_resource(ContentRes(Arc::new(db)))
            .insert_resource(LocalPlayer(local))
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(0)));
        app.update();
        log.drain();
        (app, log)
    }

    fn run(app: &mut App, log: &CallLog, frames: usize) -> Played {
        let mut all = Vec::new();
        for _ in 0..frames {
            app.update();
            for c in log.drain() {
                if let Call::Play { name, bus, at } = c {
                    all.push((name, bus, at));
                }
            }
        }
        all
    }

    #[test]
    fn scripted_firing_produces_the_expected_log() {
        let (mut app, log) = scene(FIRING, Some(PlayerId(0)));
        let got = run(&mut app, &log, 30);
        let sfx = |n: &str, z: f32| (n.to_owned(), Bus::Sfx, Some([0.0, 0.0, z]));
        assert_eq!(
            got,
            vec![
                sfx("fx_beam_fire", 0.0),
                sfx("fx_torpedo_fire", 0.0),
                sfx("xhit_pulse1.wav", 590.0),
                sfx("xhit_pulse2.wav", 590.0),
                sfx("fx_explode_small", 600.0),
            ]
        );
    }

    #[test]
    fn far_sounds_are_culled() {
        let (mut app, log) = scene(FIRING, Some(PlayerId(0)));
        app.world_mut().resource_mut::<SfxConfig>().max_distance = 0.5;
        app.world_mut().spawn((Camera3d::default(), GlobalTransform::from(Transform::from_xyz(5000.0, 0.0, 0.0))));
        assert!(run(&mut app, &log, 30).is_empty());
    }

    #[test]
    fn limiter_caps_per_name_and_overall_then_recovers() {
        let cfg = SfxConfig { max_voices: 4, max_per_name: 2, window: 1.0, ..Default::default() };
        let mut l = VoiceLimiter::default();
        assert!(l.admit(0.0, "a", &cfg));
        assert!(l.admit(0.1, "A", &cfg), "names compare without case");
        assert!(!l.admit(0.2, "a", &cfg), "third of one name");
        assert!(l.admit(0.2, "b", &cfg));
        assert!(l.admit(0.2, "c", &cfg));
        assert!(!l.admit(0.3, "d", &cfg), "fifth voice overall");
        assert_eq!(l.active(), 4);
        assert!(l.admit(1.05, "a", &cfg), "the window moved on");
    }

    #[test]
    fn a_barrage_is_thinned_in_the_log() {
        let mut events = String::new();
        for _ in 0..10 {
            events += "{ at: 3, event: { Fired: { shooter: 1, weapon: BEAM, hardpoint: 0, target: { Entity: 2 } } } },";
        }
        let tl = format!(
            "{{ units: [{{ id: 1, def: \"fx_cruiser\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }}], events: [{events}] }}"
        );
        let (mut app, log) = scene(&tl, Some(PlayerId(0)));
        let n = run(&mut app, &log, 16).len();
        assert_eq!(n, SfxConfig::default().max_per_name);
    }

    #[test]
    fn built_plays_only_for_the_local_player() {
        let tl = r#"{ units: [{ id: 1, def: "fx_yard", owner: 0, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
                              { id: 5, def: "fx_cruiser", owner: 0, rails: [{ at: 2, pos: { x: 0, y: 0, z: 50 } }] }],
                      events: [{ at: 2, event: { Built: { builder: 1, id: 5 } } }] }"#;
        let (mut app, log) = scene(tl, Some(PlayerId(0)));
        let got = run(&mut app, &log, 16);
        assert_eq!(got, vec![("ShipConstructionComplete".to_owned(), Bus::Sfx, Some([0.0, 0.0, 50.0]))]);
        let (mut app, log) = scene(tl, Some(PlayerId(1)));
        assert!(run(&mut app, &log, 16).is_empty());
    }
}
