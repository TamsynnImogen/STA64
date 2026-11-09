//! Music: playlists, calm/battle switching from sim events, menu music (AUD-04).
//!
//! What the original games do (read from the installs, not guessed):
//!
//! - Armada I: each race ODF has a `// Music play list.` of `track0`, `track1`,
//!   `track2` (e.g. `mfed300.wav`, `mfed115.wav`, `mfed100.wav`, named by their
//!   length in seconds). There is no battle or calm data in any ODF.
//! - Armada II / Fleet Operations: each race's system ODF has one
//!   `singleplayermusic` and one `multiplayermusic` file (`Federation.mp3`,
//!   `borg.mp3` ...), and `sounds/music/` holds `Shell.ogg` (front end) and
//!   `Loading.mp3`. Again no battle data.
//!
//! So the playlist comes from the data ([`playlist_from_odf`]) and the
//! calm/battle split is STA64's own: [`MusicConfig::battle`] is an optional second
//! list, and with none configured the calm list simply keeps playing. The logic
//! lives in [`Director`], a pure state machine fed seconds and engagement weights,
//! so tests need no clock, no device and no Bevy; [`MusicPlugin`] only wires it to
//! [`SimMsg`], the app state and the mixer backend (observed through `NullAudio`).

use std::collections::HashMap;

use bevy::prelude::*;
use sta_sim::event::SimEvent;
use sta_sim::{EntityId, PlayerId};

use super::{Mixer, Resolver};
use crate::app::{AppState, GameSet};
use crate::render::units::LocalPlayer;
use crate::sim_link::{Bus, SimLink, SimMsg};

/// What the player is looking at, which picks the playlist family.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Context {
    /// Nothing plays (boot, model viewer).
    #[default]
    Silent,
    /// Front end and loading screens: the menu playlist.
    Menu,
    /// A match: calm, or battle when things heat up.
    Game,
}

/// What is playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Menu,
    Calm,
    Battle,
}

/// Playlists and the battle thresholds. Defaults play nothing.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct MusicConfig {
    pub menu: Vec<String>,
    pub calm: Vec<String>,
    /// Optional; empty means battle never starts a new track.
    pub battle: Vec<String>,
    /// Threat score at which calm turns into battle.
    pub battle_enter: f32,
    /// Threat lost per second.
    pub threat_decay: f32,
    /// Seconds without any engagement before battle returns to calm.
    pub battle_hold: f32,
    /// Seconds to wait after no track could be loaded, before trying again.
    pub retry: f32,
}

impl Default for MusicConfig {
    fn default() -> Self {
        Self {
            menu: Vec::new(),
            calm: Vec::new(),
            battle: Vec::new(),
            battle_enter: 3.0,
            threat_decay: 0.5,
            battle_hold: 20.0,
            retry: 10.0,
        }
    }
}

/// The playlist an ODF names: `track0`, `track1`, ... (Armada I), else the
/// single `singleplayermusic` / `multiplayermusic` file (Armada II), else empty.
/// `get` looks a key up (case-insensitively is the caller's business).
pub fn playlist_from_odf(multiplayer: bool, get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let clean = |s: String| {
        let s = s.trim().trim_matches('"').trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    let tracks: Vec<String> = (0..).map_while(|i| get(&format!("track{i}")).and_then(clean)).collect();
    if !tracks.is_empty() {
        return tracks;
    }
    let key = if multiplayer { "multiplayermusic" } else { "singleplayermusic" };
    get(key).and_then(clean).into_iter().collect()
}

/// What the director asks the backend to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Play { name: String, mode: Mode },
    Stop,
}

/// Round-robin over a list; deterministic, no shuffle.
#[derive(Debug, Clone, Default, PartialEq)]
struct Cursor {
    next: usize,
}

impl Cursor {
    fn pick<'a>(&mut self, list: &'a [String]) -> Option<&'a str> {
        let s = list.get(self.next % list.len().max(1))?;
        self.next = (self.next + 1) % list.len();
        Some(s)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Playing {
    name: String,
    ends_at: f32,
}

/// The music state machine.
///
/// Time is the sum of `dt` given to [`Director::advance`]. Starting a track asks
/// the caller for its length so the next one starts when it ends.
#[derive(Debug, Clone)]
pub struct Director {
    cfg: MusicConfig,
    ctx: Context,
    mode: Option<Mode>,
    now: f32,
    threat: f32,
    last_engagement: f32,
    playing: Option<Playing>,
    retry_at: Option<f32>,
    cursors: [Cursor; 3],
}

impl Director {
    pub fn new(cfg: MusicConfig) -> Self {
        Self {
            cfg,
            ctx: Context::Silent,
            mode: None,
            now: 0.0,
            threat: 0.0,
            last_engagement: f32::NEG_INFINITY,
            playing: None,
            retry_at: None,
            cursors: Default::default(),
        }
    }

    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }

    pub fn threat(&self) -> f32 {
        self.threat
    }

    pub fn now_playing(&self) -> Option<&str> {
        self.playing.as_ref().map(|p| p.name.as_str())
    }

    pub fn set_config(&mut self, cfg: MusicConfig) {
        self.cfg = cfg;
    }

    /// Switch what the player is looking at; the new playlist starts on the next [`Director::advance`].
    pub fn set_context(&mut self, ctx: Context) {
        if ctx != self.ctx {
            self.ctx = ctx;
            if ctx == Context::Game {
                self.threat = 0.0;
                self.last_engagement = f32::NEG_INFINITY;
            }
        }
    }

    /// Something happened to or by the local player; `weight` is how much it matters.
    pub fn engage(&mut self, weight: f32) {
        if weight > 0.0 && self.ctx == Context::Game {
            self.threat += weight;
            self.last_engagement = self.now;
        }
    }

    fn list(&self, m: Mode) -> &[String] {
        match m {
            Mode::Menu => &self.cfg.menu,
            Mode::Calm => &self.cfg.calm,
            Mode::Battle => &self.cfg.battle,
        }
    }

    fn wanted(&self) -> Option<Mode> {
        match self.ctx {
            Context::Silent => None,
            Context::Menu => Some(Mode::Menu),
            Context::Game => {
                let battle = !self.cfg.battle.is_empty()
                    && match self.mode {
                        Some(Mode::Battle) => self.now - self.last_engagement < self.cfg.battle_hold,
                        _ => self.threat >= self.cfg.battle_enter,
                    };
                Some(if battle { Mode::Battle } else { Mode::Calm })
            }
        }
    }

    /// Move time on by `dt` seconds. `length` gives a track's length in seconds, or
    /// `None` when it cannot be loaded (it is skipped). Returns what to do, in order.
    pub fn advance(&mut self, dt: f32, length: &mut dyn FnMut(&str) -> Option<f32>) -> Vec<Out> {
        self.now += dt;
        self.threat = (self.threat - self.cfg.threat_decay * dt).max(0.0);
        let mut out = Vec::new();
        let want = self.wanted();
        let changed = want != self.mode;
        if changed {
            if self.playing.take().is_some() {
                out.push(Out::Stop);
            }
            self.mode = want;
            self.retry_at = None;
        }
        let Some(mode) = self.mode else {
            return out;
        };
        let ended = self.playing.as_ref().is_some_and(|p| self.now >= p.ends_at);
        let retry = self.retry_at.is_some_and(|t| self.now >= t);
        if ended {
            self.playing = None;
        }
        if self.playing.is_none() && (changed || ended || retry || self.retry_at.is_none()) {
            self.retry_at = None;
            let len = self.list(mode).len();
            for _ in 0..len {
                let mut cursor = std::mem::take(&mut self.cursors[mode as usize]);
                let name = cursor.pick(self.list(mode)).map(str::to_string);
                self.cursors[mode as usize] = cursor;
                let Some(name) = name else { break };
                if let Some(secs) = length(&name) {
                    self.playing = Some(Playing { name: name.clone(), ends_at: self.now + secs.max(0.1) });
                    out.push(Out::Play { name, mode });
                    return out;
                }
            }
            if len > 0 {
                self.retry_at = Some(self.now + self.cfg.retry);
            }
        }
        out
    }
}

/// How much a sim event matters to `local`. `owner` maps a unit to its player.
pub fn threat_weight(ev: &SimEvent, local: PlayerId, owner: &dyn Fn(EntityId) -> Option<PlayerId>) -> f32 {
    use sta_sim::command::Target;
    let mine = |id: EntityId| owner(id) == Some(local);
    let hostile = |a: EntityId, b: EntityId| mine(a) != mine(b) && (mine(a) || mine(b));
    match ev {
        SimEvent::Fired { shooter, target: Target::Entity(t), .. } if hostile(*shooter, *t) => 0.25,
        SimEvent::Fired { shooter, target: Target::None | Target::Point(_), .. } if mine(*shooter) => 0.25,
        SimEvent::Hit { target, by, .. } if hostile(*target, *by) => 1.0,
        SimEvent::ShieldsDown { id, .. } if mine(*id) => 1.0,
        SimEvent::Destroyed { id, by } => match by {
            Some(b) if hostile(*id, *b) => 3.0,
            None if mine(*id) => 3.0,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

#[derive(Resource)]
struct MusicState {
    director: Director,
    /// Who owned each unit, kept after it dies so `Destroyed` can be read.
    owners: HashMap<EntityId, PlayerId>,
}

/// Registers the music systems. [`MusicConfig`] and [`Context`] are resources the
/// front end and content loading set; they start empty and silent.
pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SimMsg>()
            .init_resource::<Context>()
            .init_resource::<MusicConfig>()
            .insert_resource(MusicState { director: Director::new(MusicConfig::default()), owners: HashMap::new() })
            .add_systems(Update, (context_from_state, sync_director, run_music).chain().in_set(GameSet::Audio));
    }
}

fn context_from_state(state: Option<Res<State<AppState>>>, mut ctx: ResMut<Context>) {
    let Some(state) = state else { return };
    let want = match state.get() {
        AppState::FrontEnd | AppState::Loading => Context::Menu,
        AppState::InGame => Context::Game,
        AppState::Boot | AppState::Viewer => Context::Silent,
    };
    ctx.set_if_neq(want);
}

fn sync_director(ctx: Res<Context>, cfg: Res<MusicConfig>, mut st: ResMut<MusicState>) {
    if cfg.is_changed() {
        st.director.set_config(cfg.clone());
    }
    st.director.set_context(*ctx);
}

fn run_music(
    time: Option<Res<Time<Real>>>,
    link: Option<Res<SimLink>>,
    local: Option<Res<LocalPlayer>>,
    mut msgs: MessageReader<SimMsg>,
    mut st: ResMut<MusicState>,
    mut resolver: ResMut<Resolver>,
    mut mixer: NonSendMut<Mixer>,
) {
    let st = &mut *st;
    if let Some(link) = &link {
        for u in &link.curr().units {
            st.owners.insert(u.id, u.owner);
        }
    }
    let me = local.and_then(|l| l.0).or_else(|| link.as_ref().map(|l| l.player()));
    for m in msgs.read() {
        if let Some(me) = me {
            let w = threat_weight(&m.0, me, &|id| st.owners.get(&id).copied());
            st.director.engage(w);
        }
    }
    let mut length = |name: &str| {
        let s = super::builtin(name).or_else(|| resolver.0.resolve(name))?;
        Some(s.frames.len() as f32 / s.sample_rate.max(1) as f32)
    };
    let dt = time.map_or(0.0, |t| t.delta_secs());
    for o in st.director.advance(dt, &mut length) {
        match o {
            Out::Stop => mixer.backend.stop_bus(Bus::Music),
            Out::Play { name, .. } => {
                if let Some(s) = super::builtin(&name).or_else(|| resolver.0.resolve(&name)) {
                    mixer.backend.play(&s, Bus::Music, None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Call, CallLog, NullAudio};
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use sta_sim::command::Target;
    use sta_sim::{ContentKey, Fixed, Vec3 as SimVec3};

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// Tracks named `audio-test:<hz>` last 0.4 s in the plugin; the director tests use a table.
    fn secs(name: &str) -> Option<f32> {
        match name {
            "bad" => None,
            n if n.starts_with("menu") => Some(30.0),
            n if n.starts_with("calm") => Some(60.0),
            _ => Some(40.0),
        }
    }

    fn cfg() -> MusicConfig {
        MusicConfig {
            menu: list(&["menu1"]),
            calm: list(&["calm1", "calm2", "calm3"]),
            battle: list(&["war1", "war2"]),
            ..Default::default()
        }
    }

    fn step(d: &mut Director, dt: f32) -> Vec<Out> {
        d.advance(dt, &mut secs)
    }

    fn play(name: &str, mode: Mode) -> Out {
        Out::Play { name: name.into(), mode }
    }

    #[test]
    fn silent_until_a_context_is_set() {
        let mut d = Director::new(cfg());
        assert!(step(&mut d, 1.0).is_empty());
        assert_eq!(d.mode(), None);
    }

    #[test]
    fn menu_music_loops_and_stops_for_the_match() {
        let mut d = Director::new(cfg());
        d.set_context(Context::Menu);
        assert_eq!(step(&mut d, 0.1), vec![play("menu1", Mode::Menu)]);
        assert!(step(&mut d, 29.0).is_empty());
        // The only track ends and starts again.
        assert_eq!(step(&mut d, 1.0), vec![play("menu1", Mode::Menu)]);
        d.set_context(Context::Game);
        assert_eq!(step(&mut d, 0.1), vec![Out::Stop, play("calm1", Mode::Calm)]);
    }

    #[test]
    fn calm_playlist_goes_round_robin() {
        let mut d = Director::new(cfg());
        d.set_context(Context::Game);
        let mut names = vec![];
        for _ in 0..5 {
            for o in step(&mut d, 60.0) {
                if let Out::Play { name, .. } = o {
                    names.push(name);
                }
            }
        }
        assert_eq!(names, ["calm1", "calm2", "calm3", "calm1", "calm2"]);
    }

    #[test]
    fn heavy_engagement_switches_to_battle_and_back_after_the_hold() {
        let mut d = Director::new(cfg());
        d.set_context(Context::Game);
        step(&mut d, 0.1);
        d.engage(1.0);
        assert!(step(&mut d, 0.1).is_empty(), "one hit is not a battle");
        d.engage(3.0);
        assert_eq!(step(&mut d, 0.1), vec![Out::Stop, play("war1", Mode::Battle)]);
        assert_eq!(d.mode(), Some(Mode::Battle));
        // Quiet for just under the hold: still battle.
        assert!(step(&mut d, 19.0).is_empty());
        assert_eq!(d.mode(), Some(Mode::Battle));
        // Engagement during battle restarts the hold.
        d.engage(0.25);
        assert!(step(&mut d, 19.0).is_empty());
        assert_eq!(d.mode(), Some(Mode::Battle));
        // The calm playlist resumes where it left off.
        assert_eq!(step(&mut d, 1.5), vec![Out::Stop, play("calm2", Mode::Calm)]);
    }

    #[test]
    fn battle_track_ending_advances_the_battle_list() {
        let mut d = Director::new(MusicConfig { battle_hold: 100.0, ..cfg() });
        d.set_context(Context::Game);
        step(&mut d, 0.1);
        d.engage(5.0);
        assert_eq!(step(&mut d, 0.1), vec![Out::Stop, play("war1", Mode::Battle)]);
        d.engage(1.0);
        assert_eq!(step(&mut d, 40.0), vec![play("war2", Mode::Battle)]);
    }

    #[test]
    fn threat_decays() {
        let mut d = Director::new(cfg());
        d.set_context(Context::Game);
        step(&mut d, 0.1);
        d.engage(2.0);
        step(&mut d, 4.0); // decays 2.0 at 0.5/s
        assert_eq!(d.threat(), 0.0);
        d.engage(2.0);
        assert!(step(&mut d, 0.1).is_empty());
        assert_eq!(d.mode(), Some(Mode::Calm));
    }

    #[test]
    fn engagement_outside_a_match_is_ignored() {
        let mut d = Director::new(cfg());
        d.set_context(Context::Menu);
        d.engage(10.0);
        assert_eq!(d.threat(), 0.0);
    }

    #[test]
    fn without_a_battle_list_calm_keeps_playing() {
        let mut d = Director::new(MusicConfig { battle: vec![], ..cfg() });
        d.set_context(Context::Game);
        step(&mut d, 0.1);
        d.engage(50.0);
        assert!(step(&mut d, 0.1).is_empty());
        assert_eq!(d.mode(), Some(Mode::Calm));
        assert_eq!(d.now_playing(), Some("calm1"));
    }

    #[test]
    fn unloadable_tracks_are_skipped_then_retried() {
        let mut d = Director::new(MusicConfig { calm: list(&["bad", "calm2"]), ..cfg() });
        d.set_context(Context::Game);
        assert_eq!(step(&mut d, 0.1), vec![play("calm2", Mode::Calm)]);

        let mut d = Director::new(MusicConfig { calm: list(&["bad"]), retry: 10.0, ..cfg() });
        d.set_context(Context::Game);
        assert!(step(&mut d, 0.1).is_empty());
        assert!(step(&mut d, 9.0).is_empty(), "no spinning on a bad track");
        let mut found = 0;
        let mut again = |n: &str| {
            found += 1;
            (n == "bad").then_some(5.0)
        };
        assert_eq!(d.advance(1.0, &mut again), vec![play("bad", Mode::Calm)]);
        assert_eq!(found, 1);
    }

    #[test]
    fn empty_playlists_stay_silent() {
        let mut d = Director::new(MusicConfig::default());
        d.set_context(Context::Menu);
        assert!(step(&mut d, 1.0).is_empty());
        d.set_context(Context::Game);
        assert!(step(&mut d, 1.0).is_empty());
    }

    #[test]
    fn leaving_a_match_for_the_menu_swaps_lists() {
        let mut d = Director::new(cfg());
        d.set_context(Context::Game);
        step(&mut d, 0.1);
        d.set_context(Context::Menu);
        assert_eq!(step(&mut d, 0.1), vec![Out::Stop, play("menu1", Mode::Menu)]);
        d.set_context(Context::Silent);
        assert_eq!(step(&mut d, 0.1), vec![Out::Stop]);
        assert_eq!(d.now_playing(), None);
    }

    #[test]
    fn playlists_come_from_odf_keys() {
        let a1: HashMap<&str, &str> =
            [("track0", "\"mfed300.wav\""), ("track1", "mfed115.wav"), ("track2", "mfed100.wav")].into();
        fn get(m: &HashMap<&str, &str>) -> impl Fn(&str) -> Option<String> {
            let m: HashMap<String, String> = m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            move |k: &str| m.get(k).cloned()
        }
        assert_eq!(playlist_from_odf(false, get(&a1)), ["mfed300.wav", "mfed115.wav", "mfed100.wav"]);
        let a2: HashMap<&str, &str> =
            [("singleplayermusic", "Federation.mp3"), ("multiplayermusic", "borg.mp3")].into();
        assert_eq!(playlist_from_odf(false, get(&a2)), ["Federation.mp3"]);
        assert_eq!(playlist_from_odf(true, get(&a2)), ["borg.mp3"]);
        let blank: HashMap<&str, &str> = [("singleplayermusic", "\"\"")].into();
        assert!(playlist_from_odf(false, get(&blank)).is_empty());
    }

    fn id(n: u32) -> EntityId {
        EntityId(n)
    }

    #[test]
    fn threat_weights_only_count_local_involvement() {
        let me = PlayerId(1);
        let owner = |e: EntityId| Some(PlayerId(if e.0 < 10 { 1 } else { 2 }));
        let w = |ev: SimEvent| threat_weight(&ev, me, &owner);
        let hit = |target: u32, by: u32| SimEvent::Hit {
            target: id(target),
            by: id(by),
            weapon: ContentKey::NONE,
            facing: 0,
            shield: Fixed::ZERO,
            hull: Fixed::ZERO,
            at: SimVec3::from_f32([0.0; 3]),
        };
        assert_eq!(w(hit(1, 11)), 1.0);
        assert_eq!(w(hit(11, 1)), 1.0);
        assert_eq!(w(hit(1, 2)), 0.0, "friendly fire");
        assert_eq!(w(hit(11, 12)), 0.0, "others fighting each other");
        assert_eq!(w(SimEvent::Destroyed { id: id(1), by: Some(id(11)) }), 3.0);
        assert_eq!(w(SimEvent::Destroyed { id: id(11), by: Some(id(1)) }), 3.0);
        assert_eq!(w(SimEvent::Destroyed { id: id(1), by: None }), 3.0);
        assert_eq!(w(SimEvent::Destroyed { id: id(11), by: None }), 0.0);
        assert_eq!(w(SimEvent::ShieldsDown { id: id(1), facing: 0 }), 1.0);
        assert_eq!(w(SimEvent::ShieldsDown { id: id(11), facing: 0 }), 0.0);
        let fired = |s: u32, t: Target| SimEvent::Fired { shooter: id(s), weapon: ContentKey::NONE, hardpoint: 0, target: t };
        assert_eq!(w(fired(1, Target::Entity(id(11)))), 0.25);
        assert_eq!(w(fired(11, Target::Entity(id(1)))), 0.25);
        assert_eq!(w(fired(11, Target::Entity(id(12)))), 0.0);
        assert_eq!(w(SimEvent::Spawned { id: id(1) }), 0.0);
    }

    // -- through Bevy and the null backend ------------------------------------

    fn app(cfg: MusicConfig, ctx: Context) -> (App, CallLog) {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0)))
            .insert_non_send(Mixer::new(Box::new(null)))
            .add_plugins(crate::audio::AudioPlugin::default())
            .insert_resource(cfg)
            .insert_resource(ctx);
        crate::app::configure_sets(&mut app);
        app.update();
        app.update();
        log.drain();
        (app, log)
    }

    fn plays(log: &CallLog) -> Vec<String> {
        log.drain()
            .into_iter()
            .filter_map(|c| match c {
                Call::Play { name, bus: Bus::Music, .. } => Some(name),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plugin_plays_the_menu_then_the_game_playlist_on_the_music_bus() {
        let c = MusicConfig {
            menu: list(&["audio-test:100"]),
            calm: list(&["audio-test:200", "audio-test:300"]),
            ..Default::default()
        };
        let (mut app, log) = app(c, Context::Silent);
        app.update();
        assert!(log.drain().is_empty());
        *app.world_mut().resource_mut::<Context>() = Context::Menu;
        app.update();
        assert_eq!(log.drain(), vec![Call::Play { name: "audio-test:100".into(), bus: Bus::Music, at: None }]);
        *app.world_mut().resource_mut::<Context>() = Context::Game;
        app.update();
        assert_eq!(
            log.drain(),
            vec![Call::StopBus { bus: Bus::Music }, Call::Play { name: "audio-test:200".into(), bus: Bus::Music, at: None }]
        );
        // Tones last 0.4 s, the clock moves 1 s a frame: the next track follows.
        app.update();
        assert_eq!(plays(&log), ["audio-test:300"]);
        app.update();
        assert_eq!(plays(&log), ["audio-test:200"]);
    }

    #[test]
    fn plugin_goes_to_battle_on_sim_events_about_the_local_player() {
        use crate::sim_link::SimLinkPlugin;
        use sta_sim::command::LocalRouter;
        use sta_sim::fake::ScriptedSource;

        let db = sta_content::ContentDb::builder().build().unwrap();
        let script = "{ units: [
            { id: 1, owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
            { id: 2, owner: 2, hull: 100, rails: [{ at: 0, pos: { x: 9, y: 0, z: 0 } }] },
        ] }";
        let source = ScriptedSource::from_json5(script, &db).unwrap();
        let c = MusicConfig {
            calm: list(&["audio-test:200"]),
            battle: list(&["audio-test:900"]),
            // Long tracks are not available as tones, so keep the hold short and rely on StopBus ordering.
            battle_hold: 3.0,
            ..Default::default()
        };
        let (mut app, log) = app(c, Context::Game);
        app.add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(1)));
        app.update();
        let first = plays(&log);
        assert!(first.iter().all(|n| n == "audio-test:200"), "{first:?}");

        let hit = SimEvent::Hit {
            target: id(1),
            by: id(2),
            weapon: ContentKey::NONE,
            facing: 0,
            shield: Fixed::ZERO,
            hull: Fixed::ZERO,
            at: SimVec3::from_f32([0.0; 3]),
        };
        for _ in 0..4 {
            app.world_mut().write_message(SimMsg(hit.clone()));
        }
        app.update();
        let calls = log.drain();
        assert!(calls.contains(&Call::StopBus { bus: Bus::Music }), "{calls:?}");
        assert!(calls.contains(&Call::Play { name: "audio-test:900".into(), bus: Bus::Music, at: None }), "{calls:?}");
        assert_eq!(app.world().resource::<MusicState>().director.mode(), Some(Mode::Battle));
    }
}
