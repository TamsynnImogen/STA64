//! The stock Armada II warp sounds (#1154): one when a craft starts its entry ramp, one when it starts its exit ramp.
//!
//! Rules (grades: "static" is read from the Armada II executable):
//! - The entry sound plays when the craft's warp row turns to `Entering`, the exit sound when it turns to `Leaving`; none
//!   while at warp (read from the executable, static). Both are positional, at the craft, and only for craft the local
//!   player can see (the unit drawing's visibility). Not measured: the exact tick (the sound starts on the view that
//!   shows the new phase: INTERIM, up to one view tick later than the original).
//! - Under the Stock look only; the Remaster look plays none (it did not before). Armada I plays none (it has no
//!   warp effect). The mod-family profiles (Fleet Operations) are not read for these sounds: they follow the stock
//!   reading (INTERIM, not read for the profile); a pin test names that.
//! - A ramp already running when the craft first comes into view plays no sound (INTERIM: the original's sound began
//!   with its own ramp, which the player did not see start).
//! - The files are fixed names of the stock Armada II profile ([`STOCK_A2_WARP_ENTRY`], [`STOCK_A2_WARP_EXIT`]: read
//!   from the executable, static); a mod provides its own file under the same path and the mounted data resolves it by
//!   mount order.

use std::collections::HashMap;

use sta_sim::view::{WarpPhase, WorldView};
use sta_sim::{EntityId, PlayerId, Tick, Vec3};

/// The entry sound's file, stock Armada II (read from the executable, static); a mod's own file under this path wins by
/// mount order.
pub const STOCK_A2_WARP_ENTRY: &str = "warp_speed_in.wav";
/// The exit sound's file, stock Armada II (read from the executable, static); a mod's own file under this path wins by
/// mount order.
pub const STOCK_A2_WARP_EXIT: &str = "warp_speed_out.wav";

/// Whether the content's profile plays the sounds: stock Armada II, and the mod family by the stock reading (INTERIM).
pub fn profile_plays(profile: sta_content::Profile) -> bool {
    matches!(profile, sta_content::Profile::Sta2Classic | sta_content::Profile::Fo3 | sta_content::Profile::Fo4)
}

/// The last phase seen of every craft in view: a craft in the map was seen before.
#[derive(Default)]
pub struct WarpWatch {
    tick: Option<Tick>,
    phases: HashMap<EntityId, Option<WarpPhase>>,
}

impl WarpWatch {
    /// The sounds the view `view` asks for, with their places: a craft whose row turned to `Entering` or `Leaving`
    /// since the view before. `play` is false where the look or the profile plays none (the phases are still followed);
    /// `sees` says whether the local player sees the craft.
    pub fn wanted(&mut self, view: &WorldView, play: bool, sees: &dyn Fn(EntityId, PlayerId) -> bool) -> Vec<(String, Vec3)> {
        if self.tick == Some(view.tick) {
            return Vec::new();
        }
        if self.tick.is_some_and(|t| t > view.tick) {
            self.phases.clear();
        }
        self.tick = Some(view.tick);
        let mut out = Vec::new();
        let mut next = HashMap::with_capacity(view.units.len());
        for u in &view.units {
            let now = view.warp_of(u.id).map(|r| r.phase);
            if play
                && let Some(&before) = self.phases.get(&u.id)
                && before != now
                && sees(u.id, u.owner)
            {
                match now {
                    Some(WarpPhase::Entering) => out.push((STOCK_A2_WARP_ENTRY.to_owned(), u.pos)),
                    Some(WarpPhase::Leaving) => out.push((STOCK_A2_WARP_EXIT.to_owned(), u.pos)),
                    _ => {}
                }
            }
            next.insert(u.id, now);
        }
        self.phases = next;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::view::WarpRow;
    use sta_sim::{ContentKey, Fixed, UnitView};

    fn unit(id: u32) -> UnitView {
        let mut u = sta_sim::world::Entity::new(
            sta_sim::world::EntityKind::Craft,
            ContentKey::NONE,
            PlayerId(1),
            Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
            sta_content::MoveStats::default(),
        )
        .view();
        u.id = EntityId(id);
        u
    }

    fn view(tick: Tick, rows: &[(u32, WarpPhase)], ids: &[u32]) -> WorldView {
        WorldView {
            tick,
            units: ids.iter().map(|&i| unit(i)).collect(),
            warp_rows: rows.iter().map(|&(i, phase)| WarpRow { id: EntityId(i), phase, ramp_start: tick, ramp_len: 5, stretch: Fixed::from_int(2) }).collect(),
            ..WorldView::default()
        }
    }

    const SEEN: &dyn Fn(EntityId, PlayerId) -> bool = &|_, _| true;

    #[test]
    fn the_entry_sound_plays_when_a_craft_starts_to_enter_and_the_exit_sound_when_it_starts_to_leave_and_none_in_between() {
        let mut w = WarpWatch::default();
        assert!(w.wanted(&view(0, &[], &[1]), true, SEEN).is_empty());
        let got = w.wanted(&view(1, &[(1, WarpPhase::Entering)], &[1]), true, SEEN);
        assert_eq!(got.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), [STOCK_A2_WARP_ENTRY]);
        assert!(w.wanted(&view(2, &[(1, WarpPhase::Entering)], &[1]), true, SEEN).is_empty(), "the ramp running on: no sound");
        assert!(w.wanted(&view(3, &[(1, WarpPhase::AtWarp)], &[1]), true, SEEN).is_empty(), "at warp: none");
        let got = w.wanted(&view(4, &[(1, WarpPhase::Leaving)], &[1]), true, SEEN);
        assert_eq!(got.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), [STOCK_A2_WARP_EXIT]);
        assert!(w.wanted(&view(5, &[], &[1]), true, SEEN).is_empty(), "the ramp's end: none");
    }

    #[test]
    fn a_ramp_already_running_when_the_craft_first_comes_into_view_plays_none() {
        let mut w = WarpWatch::default();
        assert!(w.wanted(&view(0, &[], &[2]), true, SEEN).is_empty());
        // Craft 1 appears (it came into view) with its ramp already running.
        assert!(w.wanted(&view(1, &[(1, WarpPhase::Entering)], &[1, 2]), true, SEEN).is_empty());
    }

    #[test]
    fn a_craft_the_player_does_not_see_and_a_look_that_plays_none_make_no_sound() {
        let mut w = WarpWatch::default();
        w.wanted(&view(0, &[], &[1]), true, SEEN);
        let unseen: &dyn Fn(EntityId, PlayerId) -> bool = &|_, _| false;
        assert!(w.wanted(&view(1, &[(1, WarpPhase::Entering)], &[1]), true, unseen).is_empty());
        // The phases are followed while none plays: leaving the quiet look does not make a late sound.
        let mut w = WarpWatch::default();
        w.wanted(&view(0, &[], &[1]), false, SEEN);
        assert!(w.wanted(&view(1, &[(1, WarpPhase::Entering)], &[1]), false, SEEN).is_empty());
        assert!(w.wanted(&view(2, &[(1, WarpPhase::Entering)], &[1]), true, SEEN).is_empty(), "no new transition");
    }

    #[test]
    fn only_the_stock_armada_ii_profile_and_the_mod_family_by_its_reading_play() {
        use sta_content::Profile;
        assert!(profile_plays(Profile::Sta2Classic));
        // INTERIM, not read for the mod family: it follows the stock reading (a later reading changes this on purpose).
        assert!(profile_plays(Profile::Fo3) && profile_plays(Profile::Fo4));
        assert!(!profile_plays(Profile::Sta1Classic), "Armada I has no warp effect");
        assert!(!profile_plays(Profile::Native));
    }

    /// Install-gated (`STA64_TEST_A2_STOCK_INSTALL`, read at run time): both sound files resolve and decode in the stock
    /// install, through the resolver the game uses (decoding only: no audio device is opened).
    #[test]
    fn install_both_warp_sounds_resolve() {
        use super::super::{BankResolver, SoundResolver};
        let Some(root) = std::env::var_os("STA64_TEST_A2_STOCK_INSTALL") else {
            eprintln!("SKIP: set STA64_TEST_A2_STOCK_INSTALL");
            return;
        };
        let mut resolver = BankResolver::mount(std::path::Path::new(&root), None).expect("the install mounts");
        for name in [STOCK_A2_WARP_ENTRY, STOCK_A2_WARP_EXIT] {
            let sound = resolver.resolve(name).expect("a warp sound resolves");
            assert!(!sound.frames.is_empty());
        }
    }
}
