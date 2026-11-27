//! UI-46: reveal cheats for testing. Lift the shroud and the fog of war, each on its own, in a
//! running single-player game.
//!
//! - By default, Alt+F toggles the fog (explored ground counts as in sight, so enemy units are drawn everywhere),
//!   Alt+V toggles the shroud (unexplored ground counts as explored: no black). All three keys
//!   are rebindable; `--reveal
//!   fog,shroud,all` starts with them lifted. A notice says what changed.
//! - **Drawing only.** [`Reveal`] is read by the fog layers and the minimap picture
//!   ([`FogGrid::lifted`](super::fog::FogGrid::lifted)) and, for the fog, by
//!   [`SimLink::set_fog_lifted`], which shows the client every unit. The sim's own views, the
//!   controllers (AI and mission scripts), the replay log, snapshots, digests and captures never see
//!   it, so a revealed game plays and records exactly as an unrevealed one.
//! - Refused in a network game (a [`NetGame`] exists): the toggles do nothing there (a notice says so)
//!   and anything lifted is put back. The chords act only while a match runs (not in the menus), and the
//!   lifted state carries from one single-player match to the next until toggled.
//! - With the fog lifted and the shroud not, the 3D view draws only the units whose footprint touches
//!   explored ground (rule: the original's fog off and shroud on, read from the executable, static; not
//!   measured; the cheat follows it as STA64's own stated rule): the shroud layer is drawn under the scene and
//!   hides nothing. Units on never-explored ground are still in the unit lists and the minimap blips (open);
//!   lift both to see everything.
//!
//! Keys: the original input maps carry two fog and shroud toggles on Alt+F and Alt+V (read from data);
//! which of them toggles which layer there is not established, so the split here is STA64's own.

use bevy::prelude::*;

use crate::app::GameSet;
use crate::game::net_match::NetGame;
use crate::input::keys::{Action, KeyBindings};
use crate::sim_link::SimLink;
use crate::ui::notices::{Notice, Severity};

/// Which layers the player has lifted (`true` = lifted). Local to this client, never saved.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Reveal {
    pub fog: bool,
    pub shroud: bool,
}

/// A `--reveal` value: a comma separated list of `fog`, `shroud` and `all`, as `(fog, shroud)`.
pub fn parse_reveal(v: &str) -> Option<(bool, bool)> {
    let (mut fog, mut shroud) = (false, false);
    for part in v.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "fog" => fog = true,
            "shroud" => shroud = true,
            "all" => (fog, shroud) = (true, true),
            _ => return None,
        }
    }
    (fog || shroud).then_some((fog, shroud))
}

/// Apply this frame's chords to `reveal`; in a network game they are refused (with a notice) and
/// anything lifted is put back. Returns the notices to show.
fn toggle(reveal: &mut Reveal, keys: &ButtonInput<KeyCode>, bindings: &KeyBindings, network: bool) -> Vec<String> {
    let mut said = Vec::new();
    let modifier = bindings.pressed(Action::RevealModifier, keys);
    let fog = modifier && bindings.just_pressed(Action::RevealFog, keys);
    let shroud = modifier && bindings.just_pressed(Action::RevealShroud, keys);
    if network {
        *reveal = Reveal::default();
        if fog || shroud {
            said.push("Cheat refused: not in a network game".to_owned());
        }
        return said;
    }
    if !modifier {
        return said;
    }
    let mut flip = |on: &mut bool, what: &str| {
        *on = !*on;
        said.push(format!("Cheat: {what} {}", if *on { "lifted" } else { "restored" }));
    };
    if fog {
        flip(&mut reveal.fog, "fog of war");
    }
    if shroud {
        flip(&mut reveal.shroud, "shroud");
    }
    said
}

/// Read the chords and hand the fog to the link. Runs while a match runs only (not in the front end's
/// menus, where the modifier may be held while typing); a network game is refused even before its link exists.
fn update_reveal(keys: Option<Res<ButtonInput<KeyCode>>>, bindings: Res<KeyBindings>, net: Option<Res<NetGame>>, editor: Option<Res<crate::editor::Editing>>, mut reveal: ResMut<Reveal>, link: Option<ResMut<SimLink>>, mut notices: MessageWriter<Notice>) {
    let said = match keys {
        Some(k) => toggle(&mut reveal, &k, &bindings, net.is_some()),
        None => toggle(&mut reveal, &ButtonInput::default(), &bindings, net.is_some()),
    };
    for t in said {
        notices.write(Notice::new(Severity::Info, t));
    }
    let lifted = reveal.fog || (editor.is_some() && net.is_none());
    if let Some(mut link) = link
        && link.fog_lifted() != lifted
    {
        link.set_fog_lifted(lifted);
    }
}

pub struct RevealPlugin;

impl Plugin for RevealPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Reveal>().init_resource::<KeyBindings>().add_message::<Notice>().add_systems(Update, update_reveal.in_set(GameSet::Input).run_if(crate::game::flow::match_running).run_if(crate::ui::ingame_menu::menu_closed).run_if(crate::cheats::input::game_input_free));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::fog::{FogGrid, Layer};
    use crate::sim_link::all_seen;
    use std::sync::Arc;
    use sta_sim::view::{PlayerView, UnitFlags, UnitView};
    use sta_sim::{ContentKey, EntityId, Fixed, PlayerId, TeamId, WorldView};

    fn unit(id: u32, owner: u8, x: i32) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: ContentKey::NONE,
            owner: PlayerId(owner),
            pos: sta_sim::Vec3 { x: Fixed::from_int(x), y: Fixed::from_int(0), z: Fixed::from_int(0) },
            heading: (Fixed::from_int(0), Fixed::from_int(1)),
            speed: Fixed::from_int(0),
            hull: [Fixed::from_int(100); 2],
            shields: [[Fixed::from_int(0); 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: UnitFlags::SELECTABLE,
            progress: None,
        }
    }

    fn player(id: u8, team: u8) -> PlayerView {
        PlayerView { crew: None, id: PlayerId(id), team: TeamId(team), resources: vec![], officers: [Fixed::from_int(0); 2], eliminated: false, relations: vec![] }
    }

    /// Player 1 owns unit 1 at x = 0; player 2's unit 2 is far away and unseen.
    fn view() -> Arc<WorldView> {
        Arc::new(WorldView {
            players: vec![player(1, 1), player(2, 2)],
            units: vec![unit(1, 1, 0), unit(2, 2, 3000)],
            visibility: vec![(PlayerId(1), vec![EntityId(1)]), (PlayerId(2), vec![EntityId(2)])],
            ..Default::default()
        })
    }

    #[test]
    fn parse_names_the_layers() {
        assert_eq!(parse_reveal("fog"), Some((true, false)));
        assert_eq!(parse_reveal("shroud"), Some((false, true)));
        assert_eq!(parse_reveal("fog, shroud"), Some((true, true)));
        assert_eq!(parse_reveal("all"), Some((true, true)));
        assert_eq!(parse_reveal("fogg"), None);
        assert_eq!(parse_reveal(""), None);
    }

    #[test]
    fn the_shown_view_has_every_unit_visible_and_the_real_one_is_untouched() {
        let real = view();
        assert_eq!(real.visible_to(PlayerId(1)).len(), 1);
        let shown = all_seen(&real);
        assert_eq!(shown.visible_to(PlayerId(1)).len(), 2);
        assert_eq!(real.visible_to(PlayerId(1)).len(), 1, "the sim's view is not changed");
        assert_eq!((shown.tick, shown.digest, shown.units.len()), (real.tick, real.digest, real.units.len()));
    }

    #[test]
    fn lifting_the_fog_is_independent_of_the_shroud_and_restores_the_memory() {
        let mut g = FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]);
        g.observe(&view(), PlayerId(1), None);
        // Player 1's unit sees a disc around x = 0; the far unit is theirs to see only if lifted.
        let off = g.lifted(false, false);
        assert_eq!(off.mask(Layer::Fog), g.mask(Layer::Fog), "no lifting leaves the sight as it is");
        assert!(off.explored_at(0.0, 0.0) && !off.explored_at(3000.0, 0.0));
        let fog = g.lifted(true, false);
        assert!(fog.visible_at(0.0, 0.0));
        assert!(!fog.visible_at(3000.0, 0.0) && !fog.explored_at(3000.0, 0.0), "the fog alone leaves the unexplored ground black");
        let shroud = g.lifted(false, true);
        assert!(shroud.explored_at(3000.0, 0.0) && !shroud.visible_at(3000.0, 0.0), "the shroud alone leaves the fog");
        let both = g.lifted(true, true);
        assert!(both.visible_at(3000.0, 0.0) && both.visible_at(-4000.0, 4000.0));
        assert!(!g.visible_at(3000.0, 0.0) && !g.explored_at(3000.0, 0.0), "the real memory is kept");
        assert!(both.mask(Layer::Fog).iter().all(|b| *b == 0) && both.mask(Layer::Shroud).iter().all(|b| *b == 0));
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(RevealPlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(SimLink::new(Box::new(Fixed1(view())), Box::new(sta_sim::command::LocalRouter::new()), PlayerId(1)));
        app
    }

    struct Fixed1(Arc<WorldView>);
    impl sta_sim::WorldSource for Fixed1 {
        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
        fn step(&mut self, _: &[sta_sim::command::Command]) -> sta_sim::TickOutput {
            unreachable!("the test never steps")
        }
    }

    fn chord(app: &mut App, key: KeyCode) {
        chord_with_modifier(app, KeyCode::AltLeft, key);
    }

    fn chord_with_modifier(app: &mut App, modifier: KeyCode, key: KeyCode) {
        let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        k.release_all();
        k.clear();
        k.press(modifier);
        k.press(key);
        app.update();
    }

    #[test]
    fn the_chords_toggle_the_fog_and_the_shroud_on_their_own() {
        let mut app = app();
        chord(&mut app, KeyCode::KeyF);
        assert_eq!(*app.world().resource::<Reveal>(), Reveal { fog: true, shroud: false });
        assert_eq!(app.world().resource::<SimLink>().curr().visible_to(PlayerId(1)).len(), 2, "the client draws every unit");
        assert_eq!(app.world().resource::<SimLink>().real_curr().visible_to(PlayerId(1)).len(), 1, "the sim's view is untouched");
        chord(&mut app, KeyCode::KeyV);
        assert_eq!(*app.world().resource::<Reveal>(), Reveal { fog: true, shroud: true });
        chord(&mut app, KeyCode::KeyF);
        assert_eq!(*app.world().resource::<Reveal>(), Reveal { fog: false, shroud: true });
        assert_eq!(app.world().resource::<SimLink>().curr().visible_to(PlayerId(1)).len(), 1, "restored");
    }

    #[test]
    fn turning_off_the_cheat_keeps_instant_actions_fog_setting() {
        let world = view();
        let mut link = SimLink::new(Box::new(Fixed1(world)), Box::new(sta_sim::command::LocalRouter::new()), PlayerId(1))
            .with_match_fog_lifted(true);
        link.set_fog_lifted(true);
        link.set_fog_lifted(false);

        assert_eq!(link.curr().visible_to(PlayerId(1)).len(), 2);
        assert_eq!(link.real_curr().visible_to(PlayerId(1)).len(), 1);
    }

    #[test]
    fn a_chord_needs_alt() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyF);
        let mut r = Reveal::default();
        assert!(toggle(&mut r, &keys, &KeyBindings::default(), false).is_empty());
        assert_eq!(r, Reveal::default());
    }

    #[test]
    fn reveal_system_reads_rebound_actions_from_the_resource() {
        let mut app = app();
        let mut bindings = app.world_mut().resource_mut::<KeyBindings>();
        bindings.rebind(Action::RevealModifier, KeyCode::ControlRight);
        bindings.rebind(Action::RevealFog, KeyCode::KeyG);
        bindings.rebind(Action::RevealShroud, KeyCode::KeyH);
        drop(bindings);
        chord(&mut app, KeyCode::KeyF);
        chord(&mut app, KeyCode::KeyV);
        assert_eq!(*app.world().resource::<Reveal>(), Reveal::default());
        chord_with_modifier(&mut app, KeyCode::ControlRight, KeyCode::KeyG);
        assert_eq!(*app.world().resource::<Reveal>(), Reveal { fog: true, shroud: false });
        chord_with_modifier(&mut app, KeyCode::ControlRight, KeyCode::KeyH);
        assert_eq!(*app.world().resource::<Reveal>(), Reveal { fog: true, shroud: true });
    }

    #[test]
    fn rebound_chords_require_their_own_modifier_and_fresh_key_presses() {
        let mut bindings = KeyBindings::default();
        bindings.rebind(Action::RevealModifier, KeyCode::ControlRight);
        bindings.rebind(Action::RevealFog, KeyCode::KeyG);
        bindings.rebind(Action::RevealShroud, KeyCode::KeyH);
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut reveal = Reveal::default();

        // The old Alt+F/V chords and the new keys without their modifier do nothing.
        for key in [KeyCode::KeyF, KeyCode::KeyV, KeyCode::KeyG, KeyCode::KeyH] {
            keys.release_all();
            keys.clear();
            keys.press(KeyCode::AltLeft);
            keys.press(key);
            assert!(toggle(&mut reveal, &keys, &bindings, false).is_empty());
        }
        keys.release_all();
        keys.clear();
        keys.press(KeyCode::KeyG);
        assert!(toggle(&mut reveal, &keys, &bindings, false).is_empty());

        keys.release_all();
        keys.clear();
        keys.press(KeyCode::ControlRight);
        keys.press(KeyCode::KeyG);
        assert_eq!(toggle(&mut reveal, &keys, &bindings, false), ["Cheat: fog of war lifted"]);
        assert_eq!(reveal, Reveal { fog: true, shroud: false });
        keys.clear();
        keys.press(KeyCode::KeyG); // Held key and repeat are not a fresh press.
        assert!(toggle(&mut reveal, &keys, &bindings, false).is_empty());
        keys.press(KeyCode::KeyH);
        assert_eq!(toggle(&mut reveal, &keys, &bindings, false), ["Cheat: shroud lifted"]);
        assert_eq!(reveal, Reveal { fog: true, shroud: true });
        keys.clear();
        keys.release(KeyCode::ControlRight);
        keys.release(KeyCode::KeyG);
        keys.press(KeyCode::KeyG);
        assert!(toggle(&mut reveal, &keys, &bindings, false).is_empty());
        assert_eq!(reveal, Reveal { fog: true, shroud: true });
    }

    #[test]
    fn a_network_game_refuses_the_chords_and_puts_a_lifted_layer_back() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::AltLeft);
        keys.press(KeyCode::KeyV);
        let mut r = Reveal { fog: true, shroud: false };
        assert_eq!(toggle(&mut r, &keys, &KeyBindings::default(), true).len(), 1, "the refusal is said");
        assert_eq!(r, Reveal::default());
    }
}
