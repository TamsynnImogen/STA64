//! Armada II hover readout bars (issue 1046): who shows bars, and in which colours.
//!
//! Rules (grades: "static" is read from the Armada II executable, "measured" from the running
//! original at normal speed, "data" from the install's files):
//! - Bars are up only while [`HoverReadout::bars_up`]: the pointer has rested on a unit of one
//!   team for more than 1.0 s [static, measured]. Selection alone shows no bars [measured].
//! - A unit shows bars when it is the hovered unit, or when it is selected and on the hovered
//!   unit's team [static, measured]. Hovering an enemy shows none on a selected own ship [measured].
//! - A class with the ODF key `is_starbase` is skipped unless it is itself the hovered unit [static].
//! - A bar exists only where its maximum is above 0.1: shield, hull, special energy [static]. Rows
//!   from the box top: shield at -4 px, hull at 0, energy at +4 px; 2 px high [static, in virtual
//!   viewport pixels; the on-screen rows at native resolution were not measured].
//! - Each bar is a full-width (0.2, 0.2, 0.2) background plus the fill, both additive [static; the
//!   bars' sprite material is additive, data]. `f` is the fade of [`HoverReadout`], so 0.5 to 1.
//!   Fill colours are in [`shield_colour`], [`hull_colour`] and [`energy_colour`] [static].
//! - Interim: the bars are drawn untextured. The original takes their art from the sprites
//!   `select_shield_bar`, `select_hull_bar` and `select_energy_bar` of `gui_global.spr`; a mod that
//!   redefines them is not honoured yet.
//! - Not implemented: the damaged-system icons below the box, group numbers, the original's pick rules.

use sta_content::ContentKey;
use sta_sim::{PlayerId, UnitView};

use crate::input::hover::HoverReadout;
use crate::sim_link::ContentRes;
use crate::ui::profile::UiProfile;

/// A bar is drawn only where its maximum is above this [static].
pub const MAX_ABOVE: f32 = 0.1;
/// The background drawn under every fill [static].
pub const BACKGROUND: f32 = 0.2;

/// Whether the Armada II readout replaces the permanent bars. Fleet Operations 4 (Roots), Armada I
/// and the Remaster look keep today's bars. Fleet Operations 3.x runs under this profile today
/// (`UiProfile::from_game`), so it takes this rule too until its own look is measured.
pub fn wanted(profile: Option<&UiProfile>) -> bool {
    profile.is_some_and(|p| *p == UiProfile::Armada2)
}

/// The class key `is_starbase` of `def`; false when the content is unknown.
pub fn is_starbase(content: Option<&ContentRes>, def: ContentKey) -> bool {
    content.and_then(|c| c.0.unit(def)).is_some_and(|d| d.is_starbase)
}

/// Whether unit `id` of team `owner` shows bars under `readout`.
pub fn shows(readout: &HoverReadout, id: sta_sim::EntityId, owner: PlayerId, selected: bool, starbase: bool) -> bool {
    if !readout.bars_up() {
        return false;
    }
    if readout.unit == Some(id) {
        return true;
    }
    selected && readout.team == Some(owner) && !starbase
}

/// Shares for the three rows, `None` where the bar does not exist.
pub fn shares(u: &UnitView) -> [Option<f32>; 3] {
    let shield = {
        let (now, max) = u.shields.iter().fold((0.0, 0.0), |(n, m), s| (n + s[0].to_f32(), m + s[1].to_f32()));
        (max > MAX_ABOVE).then(|| (now / max).clamp(0.0, 1.0))
    };
    let pair = |p: [sta_sim::Fixed; 2]| (p[1].to_f32() > MAX_ABOVE).then(|| (p[0].to_f32() / p[1].to_f32()).clamp(0.0, 1.0));
    [shield, pair(u.hull), u.energy.and_then(pair)]
}

fn lift(f: f32) -> f32 {
    (f - 0.5).max(0.0)
}

/// Shield fill: ((1-s)(f-1/2), s(f-1/2), 0) [static].
pub fn shield_colour(s: f32, f: f32) -> [f32; 3] {
    [(1.0 - s) * lift(f), s * lift(f), 0.0]
}

/// Hull fill: ((1-h)(f-1/2), (1-h)(f-1/2), h f) [static].
pub fn hull_colour(h: f32, f: f32) -> [f32; 3] {
    [(1.0 - h) * lift(f), (1.0 - h) * lift(f), h * f]
}

/// Special energy fill: (f, f, 0) [static].
pub fn energy_colour(f: f32) -> [f32; 3] {
    [f, f, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::{EntityId, Fixed};

    fn up(unit: u32, team: u8) -> HoverReadout {
        HoverReadout { unit: Some(EntityId(unit)), team: Some(PlayerId(team)), fade: 0.75 }
    }

    #[test]
    fn four_states_of_selection_and_hover() {
        let (me, other) = (EntityId(1), EntityId(2));
        // Neither: nothing.
        assert!(!shows(&HoverReadout::default(), me, PlayerId(1), false, false));
        // Selected, not hovered (nothing hovered): no bars.
        assert!(!shows(&HoverReadout::default(), me, PlayerId(1), true, false));
        // Selected and hovered; not selected and hovered: bars.
        assert!(shows(&up(1, 1), me, PlayerId(1), true, false));
        assert!(shows(&up(1, 1), me, PlayerId(1), false, false));
        // Selected, another unit of the same team hovered: bars.
        assert!(shows(&up(2, 1), me, PlayerId(1), true, false));
        // Same team but not selected, not hovered: none.
        assert!(!shows(&up(2, 1), me, PlayerId(1), false, false));
        // Selected unit of another team than the hovered one: none.
        assert!(!shows(&up(2, 2), me, PlayerId(1), true, false));
        let _ = other;
    }

    #[test]
    fn bars_wait_for_the_threshold() {
        let mut r = up(1, 1);
        r.fade = 0.5;
        assert!(!shows(&r, EntityId(1), PlayerId(1), false, false), "exactly 1.0 s: not yet");
        r.fade = 0.51;
        assert!(shows(&r, EntityId(1), PlayerId(1), false, false));
    }

    #[test]
    fn starbase_class_shows_bars_only_when_hovered() {
        let r = up(2, 1);
        assert!(!shows(&r, EntityId(1), PlayerId(1), true, true), "selected starbase, another unit hovered");
        assert!(shows(&r, EntityId(2), PlayerId(1), false, true), "a hovered starbase does");
    }

    #[test]
    fn colour_formulas_at_the_corners() {
        // f = 0.5 (bars just appeared): the lifted terms vanish.
        assert_eq!(shield_colour(1.0, 0.5), [0.0, 0.0, 0.0]);
        assert_eq!(hull_colour(1.0, 0.5), [0.0, 0.0, 0.5]);
        assert_eq!(energy_colour(0.5), [0.5, 0.5, 0.0]);
        // f = 1: full shield is green, full hull is blue, an empty hull is yellow.
        assert_eq!(shield_colour(1.0, 1.0), [0.0, 0.5, 0.0]);
        assert_eq!(shield_colour(0.0, 1.0), [0.5, 0.0, 0.0]);
        assert_eq!(hull_colour(1.0, 1.0), [0.0, 0.0, 1.0]);
        assert_eq!(hull_colour(0.0, 1.0), [0.5, 0.5, 0.0]);
        assert_eq!(hull_colour(0.5, 1.0), [0.25, 0.25, 0.5]);
        assert_eq!(energy_colour(1.0), [1.0, 1.0, 0.0]);
    }

    #[test]
    fn bars_exist_only_above_the_maximum_threshold() {
        let mut u = UnitView {
            id: EntityId(1),
            def: ContentKey::NONE,
            owner: PlayerId(1),
            pos: sta_sim::Vec3::ZERO,
            heading: (Fixed::ONE, Fixed::ZERO),
            speed: Fixed::ZERO,
            hull: [Fixed::from_int(50), Fixed::from_int(100)],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: Default::default(),
            progress: None,
        };
        assert_eq!(shares(&u), [None, Some(0.5), None]);
        u.shields[0] = [Fixed::from_int(10), Fixed::from_int(10)];
        u.energy = Some([Fixed::from_int(1), Fixed::from_int(4)]);
        assert_eq!(shares(&u), [Some(1.0), Some(0.5), Some(0.25)]);
        u.hull = [Fixed::ZERO, Fixed::ZERO];
        u.energy = Some([Fixed::ZERO, Fixed::ZERO]);
        assert_eq!(shares(&u), [Some(1.0), None, None]);
    }

    #[test]
    fn only_the_armada_ii_profile_takes_the_readout() {
        assert!(wanted(Some(&UiProfile::Armada2)));
        for p in [UiProfile::Lcars, UiProfile::Armada1, UiProfile::FleetOps] {
            assert!(!wanted(Some(&p)), "{p:?} keeps today's bars");
        }
        assert!(!wanted(None));
    }
}
