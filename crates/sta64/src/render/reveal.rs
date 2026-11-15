//! UI-46: reveal cheats for testing. Lift the shroud and the fog of war, each on its own, in a
//! running single-player game.
//!
//! - Alt+F toggles the fog (explored ground counts as in sight, so enemy units are drawn everywhere),
//!   Alt+V toggles the shroud (unexplored ground counts as explored: no black); `--reveal
//!   fog,shroud,all` starts with them lifted. A notice says what changed.
//! - **Drawing only.** [`Reveal`] is read by the fog layers and the minimap picture
//!   ([`FogGrid::lifted`](super::fog::FogGrid::lifted)) and, for the fog, by
//!   [`SimLink::set_fog_lifted`], which shows the client every unit. The sim's own views, the
//!   controllers (AI and mission scripts), the replay log, snapshots, digests and captures never see
//!   it, so a revealed game plays and records exactly as an unrevealed one.
//! - Refused in a network game (a [`NetGame`] exists): the toggles do nothing there and anything
//!   lifted is put back.
//! - Limit: with the fog lifted and the shroud not, units on never-explored ground are shown to the
//!   unit lists and the minimap blips, but the shroud still blacks the ground under them in the 3D
//!   view and on the minimap picture; lift both to see everything.
//!
//! Keys: the original input maps carry two fog and shroud toggles on Alt+F and Alt+V (read from data);
//! which of them toggles which layer there is not established, so the split here is STA64's own.

use bevy::prelude::*;

use crate::app::GameSet;
use crate::game::net_match::NetGame;
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

fn alt(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)
}

/// Apply this frame's chords to `reveal`; in a network game they are refused and anything lifted is put
/// back. Returns the notices to show.
fn toggle(reveal: &mut Reveal, keys: &ButtonInput<KeyCode>, network: bool) -> Vec<String> {
    let mut said = Vec::new();
    if network {
        *reveal = Reveal::default();
        return said;
    }
    if !alt(keys) {
        return said;
    }
    let mut flip = |on: &mut bool, what: &str| {
        *on = !*on;
        said.push(format!("Cheat: {what} {}", if *on { "lifted" } else { "restored" }));
    };
    if keys.just_pressed(KeyCode::KeyF) {
        flip(&mut reveal.fog, "fog of war");
    }
    if keys.just_pressed(KeyCode::KeyV) {
        flip(&mut reveal.shroud, "shroud");
    }
    said
}

/// Read the chords and hand the fog to the link.
fn update_reveal(keys: Option<Res<ButtonInput<KeyCode>>>, net: Option<Res<NetGame>>, mut reveal: ResMut<Reveal>, link: Option<ResMut<SimLink>>, mut notices: MessageWriter<Notice>) {
    let Some(mut link) = link else { return };
    let before = *reveal;
    let said = match keys {
        Some(k) => toggle(&mut reveal, &k, net.is_some()),
        None => toggle(&mut reveal, &ButtonInput::default(), net.is_some()),
    };
    if *reveal != before || !said.is_empty() {
        for t in said {
            notices.write(Notice::new(Severity::Info, t));
        }
    }
    if link.fog_lifted() != reveal.fog {
        link.set_fog_lifted(reveal.fog);
    }
}

pub struct RevealPlugin;

impl Plugin for RevealPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Reveal>().add_message::<Notice>().add_systems(Update, update_reveal.in_set(GameSet::Input));
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
        assert_eq!(off, g);
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
        let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        k.release_all();
        k.clear();
        k.press(KeyCode::AltLeft);
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
    fn a_chord_needs_alt() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyF);
        let mut r = Reveal::default();
        assert!(toggle(&mut r, &keys, false).is_empty());
        assert_eq!(r, Reveal::default());
    }

    #[test]
    fn a_network_game_refuses_the_chords_and_puts_a_lifted_layer_back() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::AltLeft);
        keys.press(KeyCode::KeyV);
        let mut r = Reveal { fog: true, shroud: false };
        assert!(toggle(&mut r, &keys, true).is_empty());
        assert_eq!(r, Reveal::default());
    }
}
