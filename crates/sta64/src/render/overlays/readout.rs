//! Armada II hover readout bars (issue 1046): who shows bars, and in which colours.
//!
//! Rules (grades: "static" is read from the Armada II executable, "measured" from the running
//! original at normal speed, "data" from the install's files):
//! - Bars are up only while [`HoverReadout::bars_up`]: the pointer has rested on a unit of one
//!   team for more than 1.0 s [static, measured]. Selection alone shows no bars [measured].
//! - A unit shows bars when it is the hovered unit, or when it is selected and on the hovered
//!   unit's team [static, measured]. Hovering an enemy shows none on a selected own ship [measured].
//!   A neutral (unowned) unit has no team: hovered or selected it shows no bars, and "nothing hovered"
//!   is the same team [static; measured on an unowned planet]. Owned planets: not checked.
//! - A class with the ODF key `is_starbase` is skipped unless it is itself the hovered unit [static].
//! - A bar exists only where its maximum is above 0.1: shield, hull, special energy [static]. STA64's
//!   shield maximum is the sum of the four facings, the original's a single pool. Rows
//!   from the box top: shield at -4 px, hull at 0, energy at +4 px; 2 px high [static, in virtual
//!   viewport pixels; the on-screen rows at native resolution were not measured].
//! - Each bar is a full-width (0.2, 0.2, 0.2) background plus the fill, both drawn additive [static,
//!   read from data]. `f` is the fade of [`HoverReadout`], so 0.5 to 1. Fill colours are in
//!   [`shield_colour`], [`hull_colour`] and [`energy_colour`] [static]. The constants are display
//!   values; STA64 blends them in Bevy's linear path, which was not checked against the original.
//! - Art: STA64 reads the sprite keys [`SPRITES`] from `sprites/gui_global.spr` through the mounted
//!   VFS at run time, so a mod that redefines them changes the bars [read from data]. The energy bar
//!   is UV-cropped to its share, the others are narrowed. A missing sprite leaves that bar untextured.
//! - Interim: no readout bars while a cineractive runs (hover tracking goes on then); whether the
//!   original draws them there is not established.
//! - The control-group number is in [`super::extras`]. Not implemented: the damaged-system icons, the original's pick rules.

use sta_content::ContentKey;
use sta_sim::{PlayerId, UnitView};

use bevy::prelude::*;

use crate::input::hover::HoverReadout;
use crate::viewer::Mounted;
use crate::ui::gui_sprites::{self, GuiImageSampling, GuiSprites};
use crate::sim_link::ContentRes;
use crate::ui::profile::UiProfile;

/// A bar is drawn only where its maximum is above this [static].
pub const MAX_ABOVE: f32 = 0.1;
/// The background drawn under every fill [static].
pub const BACKGROUND: f32 = 0.2;

/// The one gate for "the Armada II look" in the selection overlays: the hover readout and the
/// selection bubble share it. Fleet Operations 4 (Roots), Armada I and the Remaster look are outside
/// it. Fleet Operations 3.x runs under this profile today (`UiProfile::from_game`), so it is inside
/// it until its own look is checked.
pub fn armada2_look(profile: Option<&UiProfile>) -> bool {
    profile.is_some_and(|p| *p == UiProfile::Armada2)
}

/// The class key `is_starbase` of `def`; false when the content is unknown.
pub fn is_starbase(content: Option<&ContentRes>, def: ContentKey) -> bool {
    content.and_then(|c| c.0.unit(def)).is_some_and(|d| d.is_starbase)
}

/// Whether unit `id` of team `owner` shows bars under `readout`.
pub fn shows(readout: &HoverReadout, id: sta_sim::EntityId, owner: PlayerId, selected: bool, starbase: bool) -> bool {
    // `bars_up` needs a hovered unit with a team: nothing and neutral units never show bars.
    if !readout.bars_up() {
        return false;
    }
    if readout.unit == Some(id) {
        return true;
    }
    selected && readout.team == Some(owner) && !starbase
}

/// The sprite keys of the three bars, in row order: shield, hull, special energy.
pub const SPRITES: [&str; 3] = ["select_shield_bar", "select_hull_bar", "select_energy_bar"];

/// One bar's art: the page and the pixel rectangle of its sprite.
#[derive(Clone)]
pub struct RowArt {
    pub image: Handle<Image>,
    pub rect: Rect,
    pub size: UVec2,
}

/// The bars' art, per row, when the Armada II look is on and the install defines the sprites.
#[derive(Resource, Default)]
pub struct BarArt {
    pub rows: [Option<RowArt>; 3],
    tried_for_mount: bool,
}

pub(super) fn load_art(
    mut art: ResMut<BarArt>,
    profile: Option<Res<UiProfile>>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
) {
    let (Some(mounted), Some(mut images)) = (mounted, images) else {
        if art.rows.iter().any(Option::is_some) {
            art.rows = Default::default();
        }
        return;
    };
    if !armada2_look(profile.as_deref()) {
        if art.rows.iter().any(Option::is_some) || art.tried_for_mount {
            art.rows = Default::default();
            art.tried_for_mount = false;
        }
        return;
    }
    if mounted.is_changed() {
        art.tried_for_mount = false;
    }
    if art.tried_for_mount {
        return;
    }
    art.tried_for_mount = true;
    let table = gui_sprites::read_table(&mounted.0, "sprites/gui_global.spr");
    let mut cache = GuiSprites::default();
    art.rows = SPRITES.map(|name| {
        let sprite = table.sprite(name)?;
        let (image, size) = gui_sprites::page(sprite, GuiImageSampling::Linear, &mounted.0, &mut cache, &mut images, bevy::image::CompressedImageFormats::empty())?;
        let rect = gui_sprites::crop(sprite, size)?;
        Some(RowArt { image, rect, size })
    });
}

/// The fill material of one bar: additive, textured when its sprite exists.
pub fn fill_material(art: Option<&RowArt>) -> StandardMaterial {
    let mut m = StandardMaterial { base_color: Color::BLACK, unlit: true, cull_mode: None, alpha_mode: AlphaMode::Add, ..default() };
    if let Some(a) = art {
        m.base_color_texture = Some(a.image.clone());
        m.uv_transform = super::marker::uv_of(a.rect, a.size);
    }
    m
}

/// The special-energy bar shows only the first `share` of its sprite (UV-cropped, not narrowed).
pub fn energy_uv(art: &RowArt, share: f32) -> bevy::math::Affine2 {
    let mut r = art.rect;
    r.max.x = r.min.x + r.width() * share.clamp(0.0, 1.0);
    super::marker::uv_of(r, art.size)
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
    fn the_maximum_threshold_is_tested_on_both_sides() {
        let f = |x: f64| Fixed::from_f64(x);
        let mut u = UnitView {
            id: EntityId(1),
            def: ContentKey::NONE,
            owner: PlayerId(1),
            pos: sta_sim::Vec3::ZERO,
            heading: (Fixed::ONE, Fixed::ZERO),
            speed: Fixed::ZERO,
            hull: [f(0.03125), f(0.0625)],
            shields: [[f(0.03125), f(0.0625)], [Fixed::ZERO; 2], [Fixed::ZERO; 2], [Fixed::ZERO; 2]],
            crew: None,
            energy: Some([f(0.03125), f(0.0625)]),
            autonomy: None,
            flags: Default::default(),
            progress: None,
        };
        assert_eq!(shares(&u), [None, None, None], "maxima of 0.0625: below the threshold");
        u.hull = [f(0.125), f(0.25)];
        u.shields[0] = [f(0.125), f(0.25)];
        u.energy = Some([f(0.125), f(0.25)]);
        assert_eq!(shares(&u), [Some(0.5), Some(0.5), Some(0.5)], "maxima of 0.25: above it");
    }

    #[test]
    fn neutral_and_nothing_show_no_bars() {
        // The readout never carries a team for a neutral unit (see `tick_readout`).
        let neutral_hovered = HoverReadout { unit: Some(EntityId(9)), team: None, fade: 1.0 };
        assert!(!shows(&neutral_hovered, EntityId(9), sta_sim::world::NEUTRAL, false, false), "hovered neutral");
        assert!(!shows(&neutral_hovered, EntityId(8), sta_sim::world::NEUTRAL, true, false), "selected neutral, another neutral hovered");
        assert!(!shows(&neutral_hovered, EntityId(1), PlayerId(1), true, false), "selected own unit, neutral hovered");
    }

    #[test]
    fn art_rows_come_from_the_install_data_and_fall_back_without_it() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("sta64-bar-art-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let data = root.join("Data");
        fs::create_dir_all(data.join("sprites")).unwrap();
        fs::create_dir_all(data.join("textures/rgb")).unwrap();
        fs::write(data.join("sprites/gui_global.spr"), b"select_hull_bar bars 0 0 8 2
select_energy_bar bars 0 2 8 2
").unwrap();
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 8;
        tga[14] = 4;
        tga[16] = 24;
        for _ in 0..32 {
            tga.extend_from_slice(&[255, 255, 255]);
        }
        fs::write(data.join("textures/rgb/bars.tga"), tga).unwrap();
        let vfs = sta_vfs::Vfs::mount_install(&root, None).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<BarArt>()
            .insert_resource(UiProfile::Armada2)
            .insert_resource(Mounted(vfs))
            .add_systems(Update, load_art);
        app.update();
        let art = app.world().resource::<BarArt>();
        assert!(art.rows[0].is_none(), "the shield sprite is not defined: untextured");
        let hull = art.rows[1].clone().expect("hull sprite");
        assert_eq!(hull.rect, Rect::new(0.0, 0.0, 8.0, 2.0));
        let energy = art.rows[2].clone().expect("energy sprite");
        assert_eq!(energy.rect, Rect::new(0.0, 2.0, 8.0, 4.0));
        // The energy crop keeps the first share of the sprite.
        let uv = energy_uv(&energy, 0.25);
        assert_eq!(uv.transform_point2(Vec2::new(1.0, 1.0)), Vec2::new(0.25, 1.0));
        assert!(fill_material(Some(&hull)).base_color_texture.is_some());
        assert!(fill_material(None).base_color_texture.is_none());
        // Another look clears the art.
        app.insert_resource(UiProfile::Armada1);
        app.update();
        assert!(app.world().resource::<BarArt>().rows.iter().all(Option::is_none));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_armada_ii_profile_takes_the_readout() {
        assert!(armada2_look(Some(&UiProfile::Armada2)));
        for p in [UiProfile::Lcars, UiProfile::Armada1, UiProfile::FleetOps] {
            assert!(!armada2_look(Some(&p)), "{p:?} keeps today's bars");
        }
        assert!(!armada2_look(None));
    }
}
