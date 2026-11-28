//! Tests of the Stock look's weapon shots. The definitions, sprite tables and textures are authored
//! here; time is stepped by hand (one sim tick per frame), never read from the clock of the machine.
use super::*;
use crate::app::configure_sets;
use crate::sim_link::SimLinkPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_content::{ContentBuilder, ContentDb, DefRef, Hardpoint, ShotCfg, ShotLook, UnitDef, WeaponKind};
use sta_sim::command::LocalRouter;
use sta_sim::fake::ScriptedSource;
use sta_sim::{Fixed, PlayerId, TickOutput, WorldSource, WorldView};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// An invented table: flipbook sprites, a tiled one and an alpha-blended one.
const TABLE: &str = "sprite_table\n@referenceWidth=128\n@referenceHeight=128\n@tmaterial=additive\n\
beamspr tex_beam 0 0 32 32 @anim=flip4\n\
tilespr tex_tile 0 32 32 32 @tile_u\n\
pulsespr tex_pulse 0 64 32 32\n\
torpspr tex_torp 0 96 32 32 @anim=flip4\n\
@tmaterial=alpha\n\
alphaspr tex_alpha 0 0 32 32\n\
@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n";

fn table() -> SpriteTable {
    // The flares' sprites too: a flare sprite of its own and the impact flare's constant one.
    let text = TABLE.replacen("@tmaterial=alpha", &format!("flarespr tex_flare 0 0 32 32 @anim=flip4\n{} tex_impact 0 32 32 32 @anim=flip4\n@tmaterial=alpha", stock_flares::IMPACT_SPRITE), 1);
    SpriteTable::parse(text.as_bytes())
}

fn look(sprite: &str) -> ShotLook {
    ShotLook { sprite: Some(sprite.into()), radius: Some(4.0), ..default() }
}

fn weapon(odf: &str, class: &str, kind: WeaponKind, look: ShotLook) -> WeaponDef {
    let mut w = WeaponDef::new(odf, class, kind);
    w.look = look;
    w.range = Fixed::from_int(1000);
    if kind == WeaponKind::Projectile {
        w.shot_speed = Fixed::from_int(300);
    }
    w
}

/// A phaser with a one-second life and a fifth of it to grow and to retract.
fn phaser() -> WeaponDef {
    let flare = sta_content::ShotFlare { sprite: "flarespr".into(), radius_multiplier: 2.0 };
    weapon("w_phaser", "phaser", WeaponKind::Beam, ShotLook { life_span: Some(1.0), travel_time_fraction: Some(0.2), flare: Some(flare), ..look("beamspr") })
}
fn pulse() -> WeaponDef {
    weapon("w_pulse", "pphaser", WeaponKind::Projectile, ShotLook { length: Some(30.0), ..look("pulsespr") })
}
fn torpedo() -> WeaponDef {
    weapon("w_torp", "photontorpedo", WeaponKind::Projectile, look("torpspr"))
}
fn tractor() -> WeaponDef {
    weapon("w_tractor", "tractor_bm", WeaponKind::Beam, ShotLook { life_span: Some(1.0), ..look("beamspr") })
}

#[test]
fn which_classes_take_the_stock_path_in_each_look() {
    let t = table();
    let cases = [
        (phaser(), Some(ShotPath::Ribbon)),
        (pulse(), Some(ShotPath::Streak)),
        (torpedo(), Some(ShotPath::Sprite)),
        (weapon("a", "artillery", WeaponKind::Projectile, look("torpspr")), Some(ShotPath::Sprite)),
        (weapon("b", "plasmacannon", WeaponKind::Projectile, look("torpspr")), Some(ShotPath::Sprite)),
        (weapon("c", "polerontorpedo", WeaponKind::Projectile, look("torpspr")), Some(ShotPath::Sprite)),
        (weapon("d", "shieldremovingtorpedo", WeaponKind::Projectile, look("torpspr")), Some(ShotPath::Sprite)),
        (weapon("e", "chainpulsar", WeaponKind::Projectile, look("torpspr")), Some(ShotPath::Sprite)),
        // Every other class keeps today's drawing, whatever keys it carries: special beams, the mining
        // beam, a class nothing here knows.
        (tractor(), None),
        (weapon("f", "minebm", WeaponKind::Beam, ShotLook { life_span: Some(1.0), ..look("beamspr") }), None),
        (weapon("g", "holding_bm", WeaponKind::Beam, ShotLook { life_span: Some(1.0), ..look("beamspr") }), None),
        (weapon("h", "pdlbeam", WeaponKind::Beam, ShotLook { life_span: Some(0.2), ..look("beamspr") }), None),
        (weapon("i", "somethingnew", WeaponKind::Projectile, look("torpspr")), None),
    ];
    for (def, path) in &cases {
        assert_eq!(stock_spec(RenderProfile::Stock, def, &t).map(|s| s.path), *path, "{} under Stock", def.class.0);
        assert_eq!(stock_spec(RenderProfile::Refit, def, &t), None, "{} under Refit", def.class.0);
    }
}

#[test]
fn a_shot_without_what_the_drawing_reads_keeps_todays_drawing() {
    let t = table();
    let stock = |d: &WeaponDef| stock_spec(RenderProfile::Stock, d, &t);
    assert!(stock(&phaser()).is_some());
    // No Sprite, a sprite the table lacks, a sprite that is not additive, no Radius.
    assert!(stock(&WeaponDef { look: ShotLook { sprite: None, ..phaser().look }, ..phaser() }).is_none());
    assert!(stock(&WeaponDef { look: ShotLook { sprite: Some("nosuchsprite".into()), ..phaser().look }, ..phaser() }).is_none());
    assert!(stock(&WeaponDef { look: ShotLook { sprite: Some("alphaspr".into()), ..phaser().look }, ..phaser() }).is_none());
    assert!(stock(&WeaponDef { look: ShotLook { radius: None, ..phaser().look }, ..phaser() }).is_none());
    // A beam needs lifeSpan, a pulse needs Length.
    assert!(stock(&WeaponDef { look: ShotLook { life_span: None, ..phaser().look }, ..phaser() }).is_none());
    assert!(stock(&WeaponDef { look: ShotLook { length: None, ..pulse().look }, ..pulse() }).is_none());
    // The sprite name is looked up as the table does, not case-sensitively.
    assert!(stock(&WeaponDef { look: ShotLook { sprite: Some("BeamSpr".into()), ..phaser().look }, ..phaser() }).is_some());
}

#[test]
fn only_the_phaser_class_has_a_muzzle_flare_and_the_team_colour_classes_are_listed() {
    assert!(has_muzzle_flare("phaser"));
    for c in ["pphaser", "photontorpedo", "artillery", "tractor_bm", "pdlbeam"] {
        assert!(!has_muzzle_flare(c), "{c}");
    }
    for c in ["phaser", "pphaser", "photontorpedo", "polerontorpedo", "shieldremovingtorpedo"] {
        assert!(takes_team_colour(c), "{c}");
    }
    for c in ["artillery", "plasmacannon", "chainpulsar", "tractor_bm"] {
        assert!(!takes_team_colour(c), "{c}");
    }
}

#[test]
fn the_shots_effect_is_registered_as_stock_and_the_weapons_effect_is_not() {
    let mut app = App::new();
    app.add_plugins(StockShotsPlugin);
    let e = app.world().resource::<ProfileEffects>();
    assert!(e.has_stock(Effect::Shots));
    assert!(!e.has_stock(Effect::Weapons), "a class without a stock drawing keeps Refit's in both looks");
    assert_eq!(e.implementation(RenderProfile::Stock, Effect::Shots), RenderProfile::Stock);
    assert_eq!(e.implementation(RenderProfile::Refit, Effect::Shots), RenderProfile::Refit);
    assert_eq!(e.implementation(RenderProfile::Stock, Effect::Weapons), RenderProfile::Refit);
}

#[test]
fn a_ribbon_is_the_full_radius_wide_with_its_side_from_the_eye() {
    // The eye above the middle of a beam along +X: the side vector is along Z.
    let c = ribbon_corners(Vec3::new(5.0, 10.0, 0.0), Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), 4.0);
    let expect = [Vec3::new(0.0, 0.0, 2.0), Vec3::new(10.0, 0.0, 2.0), Vec3::new(10.0, 0.0, -2.0), Vec3::new(0.0, 0.0, -2.0)];
    for (got, want) in c.iter().zip(expect) {
        assert!(got.distance(want) < 1e-5, "{got} against {want}");
    }
    assert!((c[0].distance(c[3]) - 4.0).abs() < 1e-5, "Radius is the full width");
    // From the other side the same ribbon turns with the eye.
    let c = ribbon_corners(Vec3::new(5.0, -10.0, 0.0), Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), 4.0);
    assert!(c[0].distance(Vec3::new(0.0, 0.0, -2.0)) < 1e-5);
    // An eye on the beam's own line still gives a ribbon.
    let c = ribbon_corners(Vec3::new(-5.0, 0.0, 0.0), Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), 4.0);
    assert!((c[0].distance(c[3]) - 4.0).abs() < 1e-5);
}

#[test]
fn a_streak_runs_from_its_head_back_by_the_shorter_of_its_length_and_the_distance_flown() {
    let (head, tail) = streak_ends(Vec3::ZERO, Vec3::X, Vec3::new(100.0, 0.0, 0.0), 30.0);
    assert_eq!((head, tail), (Vec3::new(100.0, 0.0, 0.0), Vec3::new(70.0, 0.0, 0.0)));
    let (head, tail) = streak_ends(Vec3::ZERO, Vec3::X, Vec3::new(20.0, 0.0, 0.0), 30.0);
    assert_eq!((head, tail), (Vec3::new(20.0, 0.0, 0.0), Vec3::ZERO), "no longer than it has flown");
    // The texture starts at the head: the ribbon is laid out head first.
    let c = ribbon_corners(Vec3::new(50.0, 10.0, 0.0), head, tail, 6.0);
    assert!(((c[0] + c[3]) * 0.5).distance(head) < 1e-5 && ((c[1] + c[2]) * 0.5).distance(tail) < 1e-5);
}

#[test]
fn a_billboard_is_a_screen_aligned_square_of_twice_its_half_size_never_rotated() {
    let c = billboard_corners(Vec3::new(1.0, 2.0, 3.0), Vec3::X, Vec3::Y, 8.0);
    assert_eq!(c[1] - c[0], Vec3::new(16.0, 0.0, 0.0));
    assert_eq!(c[0] - c[3], Vec3::new(0.0, 16.0, 0.0));
    assert_eq!((c[0] + c[2]) * 0.5, Vec3::new(1.0, 2.0, 3.0));
    // For a camera turned about, the square follows its right and up, and stays square.
    let (r, u) = (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0));
    let c = billboard_corners(Vec3::ZERO, r, u, 8.0);
    assert_eq!(c[1] - c[0], Vec3::new(0.0, 0.0, 16.0));
}

#[test]
fn the_beam_grows_stands_and_retracts_without_a_fade() {
    let (s, e) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 100.0));
    let at = |f: f32| beam_segment(s, e, (1.0 - f) * 100.0, 100.0, 0.1);
    // f = 0.95: growing, half way out.
    let (a, b) = at(0.95);
    assert!(a.distance(s) < 1e-4 && b.distance(Vec3::new(0.0, 0.0, 50.0)) < 1e-3, "{a} {b}");
    // f = 0.5: full.
    assert_eq!(at(0.5), (s, e));
    // f = 0.05: retracting, the tail half way in.
    let (a, b) = at(0.05);
    assert!(b.distance(e) < 1e-4 && a.distance(Vec3::new(0.0, 0.0, 50.0)) < 1e-3, "{a} {b}");
    // The edges: nothing at the muzzle at the first instant, nothing at the target at the last.
    assert!(beam_segment(s, e, 0.0, 100.0, 0.1).1.distance(s) < 1e-4);
    assert!(beam_segment(s, e, 100.0, 100.0, 0.1).0.distance(e) < 1e-4);
    // It is full from the end of the grow to the start of the retract, and the grow is linear.
    for f in [0.9, 0.7, 0.3, 0.1] {
        assert_eq!(at(f), (s, e), "f = {f}");
    }
    assert!((at(0.975).1.z - 25.0).abs() < 1e-3);
    // No travel fraction: full from the first instant.
    assert_eq!(beam_segment(s, e, 0.0, 100.0, 0.0), (s, e));
}

fn body(centre: Vec3) -> Body {
    Body { centre, heading: Vec3::NEG_Z, radius: 10.0, shields_up: false, planet: false, hardpoint: None }
}

#[test]
fn a_beam_ends_on_the_shield_ellipsoid_while_shields_are_up_else_on_a_hardpoint_or_a_planets_sphere() {
    let start = Vec3::new(0.0, 0.0, 200.0);
    let centre = Vec3::ZERO;
    let (_, axes) = super::super::impacts::shield_axes(Vec3::NEG_Z, 10.0);
    // Shields up, the ordnance does not ignore them: the ellipsoid, along the heading (z) on a line in z.
    let up = Body { shields_up: true, hardpoint: Some(Vec3::new(3.0, 0.0, 4.0)), ..body(centre) };
    let end = beam_end(start, &up, false);
    assert!(end.distance(Vec3::new(0.0, 0.0, axes.z)) < 1e-3, "{end}");
    // The same shield seen from the side: the shorter axis.
    let end = beam_end(Vec3::new(200.0, 0.0, 0.0), &up, false);
    assert!(end.distance(Vec3::new(axes.x, 0.0, 0.0)) < 1e-3, "{end}");
    // On the ellipsoid in any direction.
    let end = beam_end(Vec3::new(120.0, 80.0, -60.0), &up, false);
    let p = end / axes;
    assert!((p.length() - 1.0).abs() < 1e-3, "{p}");
    // Ordnance that ignores shields, or shields down: the chosen hardpoint, else the centre.
    assert_eq!(beam_end(start, &up, true), Vec3::new(3.0, 0.0, 4.0));
    assert_eq!(beam_end(start, &Body { shields_up: false, ..up.clone() }, false), Vec3::new(3.0, 0.0, 4.0));
    assert_eq!(beam_end(start, &body(centre), false), centre);
    // A planet: the bounding sphere towards the muzzle, shields or not.
    let planet = Body { planet: true, shields_up: true, radius: 50.0, ..body(centre) };
    assert!(beam_end(start, &planet, false).distance(Vec3::new(0.0, 0.0, 50.0)) < 1e-4);
    // A muzzle inside the shield has nothing to cross.
    assert_eq!(beam_end(Vec3::new(0.0, 0.0, 1.0), &up, false), Vec3::new(0.0, 0.0, 1.0));
}

#[test]
fn a_flipbook_frame_follows_the_game_time_and_the_duration_override() {
    let t = table();
    let plain = ShotSprite::resolve(&t, "beamspr", None).unwrap();
    let fast = ShotSprite::resolve(&t, "beamspr", Some(0.5)).unwrap();
    let zero = ShotSprite::resolve(&t, "beamspr", Some(0.0)).unwrap();
    // Four frames along a row, each a rectangle wide (a quarter of the page), one second round.
    let u = |s: &ShotSprite, time: f32| s.rect_at(time)[0];
    assert!((u(&plain, 0.0) - 0.0).abs() < 1e-6 && (u(&plain, 0.3) - 0.25).abs() < 1e-6 && (u(&plain, 0.55) - 0.5).abs() < 1e-6);
    // 0.5 s round: the same time is a later frame.
    assert!((u(&fast, 0.3) - 0.5).abs() < 1e-6, "{}", u(&fast, 0.3));
    // A zero duration is no override.
    assert_eq!(zero.rect_at(0.3), plain.rect_at(0.3));
    // The rectangle keeps its size, and the loop wraps.
    assert_eq!(&plain.rect_at(0.3)[2..], &[0.25, 0.25]);
    assert_eq!(plain.rect_at(0.3), plain.rect_at(1.3));
    // A sprite without a flipbook stands.
    let still = ShotSprite::resolve(&t, "pulsespr", Some(2.0)).unwrap();
    assert_eq!(still.rect_at(0.0), still.rect_at(7.7));
    assert!(ShotSprite::resolve(&t, "nosuchsprite", None).is_none());
    assert!(ShotSprite::resolve(&t, "tilespr", None).unwrap().tile_u);
}

#[test]
fn which_ordnance_wins_a_shared_sprite_is_the_last_in_content_order_and_zero_counts_for_nothing() {
    let w = |s: &str, d: Option<f32>| WeaponDef { look: ShotLook { sprite: Some(s.into()), sprite_duration: d, ..default() }, ..WeaponDef::new("x", "phaser", WeaponKind::Beam) };
    let list = [w("a", Some(1.0)), w("A", Some(0.5)), w("a", Some(0.0)), w("b", Some(9.0)), w("a", None)];
    assert_eq!(duration_override(list.iter(), "a"), Some(0.5));
    assert_eq!(duration_override(list.iter(), "b"), Some(9.0));
    assert_eq!(duration_override(list.iter(), "c"), None);
}

#[test]
fn every_flipbook_runs_on_the_game_clock_which_stands_while_paused() {
    let t = table();
    let s = ShotSprite::resolve(&t, "torpspr", None).unwrap();
    let mut clock = AnimClock::new(crate::clock::ClockMode::Wall, 0);
    let frame = |clock: &mut AnimClock, dt: f64| {
        for _ in 0..clock.begin_frame(dt) {
            clock.ticked();
        }
        clock.end_frame();
    };
    for _ in 0..10 {
        frame(&mut clock, 0.05);
    }
    let (now, wall) = (clock.now(), clock.wall());
    clock.set_paused(true);
    let frozen = s.rect_at(clock.now() as f32);
    for _ in 0..40 {
        frame(&mut clock, 0.05);
        assert_eq!(s.rect_at(clock.now() as f32), frozen, "the flipbook moved while paused");
    }
    assert_eq!(clock.now(), now);
    assert!(clock.wall() > wall, "the wall clock, which stock's flipbooks do not use, ran on");
    clock.set_paused(false);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..60 {
        frame(&mut clock, 0.05);
        seen.insert((s.rect_at(clock.now() as f32)[0] * 100.0) as i32);
    }
    assert_eq!(seen.len(), 4, "all four frames show again once it runs");
}

#[test]
fn the_material_adds_exactly_and_writes_no_depth() {
    use bevy::render::render_resource::{ColorTargetState, DepthStencilState, FragmentState, TextureFormat};
    let mut d = RenderPipelineDescriptor {
        fragment: Some(FragmentState {
            targets: vec![Some(ColorTargetState { format: TextureFormat::Rgba8UnormSrgb, blend: Some(BlendState::ALPHA_BLENDING), write_mask: Default::default() })],
            ..default()
        }),
        depth_stencil: Some(DepthStencilState { format: TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(bevy::render::render_resource::CompareFunction::GreaterEqual), stencil: default(), bias: default() }),
        ..default()
    };
    additive_state(&mut d);
    let one = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    let target = d.fragment.as_ref().unwrap().targets[0].as_ref().unwrap();
    assert_eq!(target.blend, Some(BlendState { color: one, alpha: one }), "source ONE, destination ONE: the texture's alpha darkens nothing");
    let ds = d.depth_stencil.as_ref().unwrap();
    assert_eq!(ds.depth_write_enabled, Some(false), "depth write off");
    assert_eq!(ds.depth_compare, Some(bevy::render::render_resource::CompareFunction::GreaterEqual), "the depth test stays as the pipeline has it");
    assert_eq!(d.primitive.cull_mode, None);
    assert_eq!(StockShotMaterial { params: default(), texture: default() }.alpha_mode(), AlphaMode::Premultiplied, "a blended pass, after the opaque ones");
}

#[test]
fn the_shader_preserves_stock_colour_and_alpha_in_ldr() {
    let src = include_str!("stock_shots.wesl");
    super::super::validate_wgsl(src);
    assert!(src.contains("let emission = texel.rgb * in.color.rgb * k.tint.rgb;"));
    assert!(src.contains("vec4<f32>(emission, 0.0)"));
}

// ---------------------------------------------------------------------------------------------
// In a world
// ---------------------------------------------------------------------------------------------

fn content(rules: ShotCfg) -> ContentDb {
    content_with(rules, false, false)
}

/// [`content`], with `beam_damage_at_end` on and the phaser a beam with a life of thirty ticks when `at_end`.
fn content_with(rules: ShotCfg, at_end: bool, flight: bool) -> ContentDb {
    let mut ship = UnitDef::new("t_ship", "cruiser");
    ship.hardpoints = vec![
        Hardpoint { weapon: DefRef::new("w_phaser"), node: "hp_phaser".into(), arc: None },
        Hardpoint { weapon: DefRef::new("w_pulse"), node: "hp_pulse".into(), arc: None },
        Hardpoint { weapon: DefRef::new("w_torp"), node: "hp_torp".into(), arc: None },
        Hardpoint { weapon: DefRef::new("w_tractor"), node: "hp_tractor".into(), arc: None },
        Hardpoint { weapon: DefRef::new("w_art"), node: "hp_art".into(), arc: None },
    ];
    let mut target = UnitDef::new("t_target", "cruiser");
    target.stats.hull_target_hardpoints = vec!["hull_a".into(), "hull_b".into()];
    let mut r = sta_content::Rules::default();
    r.shots = rules;
    let mut features = sta_content::Features::default();
    features.beam_damage_at_end = at_end;
    features.ordnance_flight = flight;
    let phaser = move || {
        let mut w = phaser();
        if at_end {
            w.duration = 30;
        }
        w
    };
    ContentBuilder::default()
        .features(features)
        .weapon(phaser())
        .weapon({
            let mut w = phaser();
            w.odf = "w_phaser_ign".into();
            w.base.ignore_shield = Some(true);
            w
        })
        .weapon({
            let mut w = phaser();
            w.odf = "w_phaser_plain".into();
            w.look.flare = None;
            w
        })
        .weapon(pulse())
        .weapon({
            // under `ordnance_flight` a torpedo needs a life of its own (`lifeSpan`, seconds)
            let mut w = torpedo();
            w.base.life_span = flight.then(|| Fixed::from_int(4));
            w
        })
        .weapon(tractor())
        .weapon(weapon("w_art", "artillery", WeaponKind::Projectile, look("torpspr")))
        .unit(ship)
        .unit(target)
        .unit(UnitDef::new("t_planet", "planet"))
        .build()
        .unwrap()
        .with_rules(r)
}

/// A scripted source whose units 2 and 3 show shields.
struct Shielded {
    inner: ScriptedSource,
    shields: bool,
    /// The shields show only before this tick (they drop then), when set.
    shields_until: Option<u64>,
    /// The views of these ticks (from, until) carry unit 1's beam at unit 2 as one the sim holds (#1068).
    held: Option<(u64, u64, sta_sim::ContentKey)>,
    /// From this tick on unit 1 (the shooter) carries the dying flag.
    dying_from: Option<u64>,
    /// The views of the ticks (from, until) carry a flown torpedo of unit 1 at unit 2 (shot 1) at the place the path
    /// gives for the ticks since `from` (x, z), heading along the next place.
    flown: Vec<(u64, u64, u32, sta_sim::ContentKey, fn(i64) -> (f64, f64))>,
}

impl Shielded {
    fn patch(&self, view: Arc<WorldView>) -> Arc<WorldView> {
        if !self.shields && self.held.is_none() && self.dying_from.is_none() && self.flown.is_empty() {
            return view;
        }
        let mut v = (*view).clone();
        if self.shields && self.shields_until.is_none_or(|t| v.tick < t) {
            for u in v.units.iter_mut().filter(|u| u.id.0 >= 2) {
                u.shields = [[Fixed::from_int(50), Fixed::from_int(50)]; 4];
            }
        }
        if let Some((from, until, weapon)) = self.held
            && (from..until).contains(&v.tick)
        {
            v.beam_rows = vec![sta_sim::view::BeamRow { shooter: EntityId(1), weapon, target: EntityId(2), ends: until }];
        }
        let mut rows = Vec::new();
        for (from, until, id, weapon, path) in &self.flown {
            if !(*from..*until).contains(&v.tick) {
                continue;
            }
            let k = (v.tick - from) as i64;
            let ((x, z), (nx, nz)) = (path(k), path(k + 1));
            let heading = Vec3::new((nx - x) as f32, 0.0, (nz - z) as f32).normalize_or(Vec3::Z);
            let f = |v: f64| Fixed::from_f64(v);
            rows.push(sta_sim::view::ShotRow {
                id: *id,
                shooter: EntityId(1),
                weapon: *weapon,
                target: EntityId(2),
                pos: sta_sim::Vec3 { x: f(x), y: Fixed::ZERO, z: f(z) },
                heading: (Fixed::from_f64(heading.x as f64), Fixed::from_f64(heading.z as f64)),
            });
        }
        rows.sort_by_key(|r| r.id);
        v.shot_rows = rows;
        if let Some(from) = self.dying_from
            && v.tick >= from
        {
            for u in v.units.iter_mut().filter(|u| u.id.0 == 1) {
                u.flags = u.flags.with(sta_sim::view::UnitFlags::DYING, true);
            }
        }
        Arc::new(v)
    }
}

impl WorldSource for Shielded {
    fn step(&mut self, c: &[sta_sim::command::Command]) -> TickOutput {
        let mut o = self.inner.step(c);
        o.view = self.patch(o.view);
        o
    }
    fn view(&self) -> Arc<WorldView> {
        self.patch(self.inner.view())
    }
}

struct Setup {
    db: ContentDb,
    shields: bool,
    /// The shields drop at this tick, when set.
    shields_until: Option<u64>,
    /// Unit 2's def and a rail point (the target); `until` ends it.
    target_def: &'static str,
    target_z: i32,
    /// Where unit 2 is at tick 300 (it starts at x = 0): it moves along x.
    target_x: i32,
    target_until: Option<u64>,
    shooter_until: Option<u64>,
    events: Vec<(u64, &'static str, u8, u64)>,
    /// The ticks (from, until) whose views carry the phaser's beam as held by the sim.
    held: Option<(u64, u64)>,
    /// The shooter starts to die at this tick.
    shooter_dying: Option<u64>,
    /// A flown torpedo (ticks from, until, its path), under `ordnance_flight`.
    flown: Vec<(u64, u64, u32, fn(i64) -> (f64, f64))>,
    /// Hits scripted at a tick: (at, weapon, hull).
    hits: Vec<(u64, &'static str)>,
    /// Unit 2's height.
    target_y: i32,
}

impl Setup {
    fn new(rules: ShotCfg) -> Setup {
        Setup { db: content(rules), shields: false, shields_until: None, target_def: "t_target", target_z: 600, target_x: 0, target_until: None, shooter_until: None, events: Vec::new(), held: None, shooter_dying: None, flown: Vec::new(), hits: Vec::new(), target_y: 0 }
    }
    /// The content has `ordnance_flight` on (and the torpedo a life), and the views carry a flown torpedo on `path`
    /// from tick `from` until `until`.
    fn flight(mut self, rules: ShotCfg, from: u64, until: u64, path: fn(i64) -> (f64, f64)) -> Setup {
        self.db = content_with(rules, false, true);
        self.flown.push((from, until, 1, path));
        self
    }
    /// Another flown torpedo (shot number `id`).
    fn also_flown(mut self, from: u64, until: u64, id: u32, path: fn(i64) -> (f64, f64)) -> Setup {
        self.flown.push((from, until, id, path));
        self
    }
    /// The target is hit by `weapon` at tick `at`.
    fn hit(mut self, at: u64, weapon: &'static str) -> Self {
        self.hits.push((at, weapon));
        self
    }
    /// The phaser lands at the end of its life (`beam_damage_at_end`).
    fn at_end(mut self, rules: ShotCfg) -> Setup {
        self.db = content_with(rules, true, false);
        self
    }
    /// The views of ticks `from` up to `until` carry the beam as one the sim holds.
    fn held(mut self, from: u64, until: u64) -> Setup {
        self.held = Some((from, until));
        self
    }
    /// A beam fired at tick `at` at a place (the point the target unit stands at), not at a unit.
    fn fire_at_point(mut self, at: u64, weapon: &'static str, hardpoint: u8) -> Self {
        self.events.push((at, weapon, hardpoint, 0));
        self
    }
    /// A shot fired at tick `at`: weapon name, hardpoint, target unit.
    fn fire(mut self, at: u64, weapon: &'static str, hardpoint: u8) -> Self {
        self.events.push((at, weapon, hardpoint, 2));
        self
    }
    fn app(&self, profile: Option<RenderProfile>) -> App {
        let until = |u: Option<u64>| u.map_or(String::new(), |t| format!(", until: {t}"));
        let mut ev = String::new();
        for (at, w, hp, target) in &self.events {
            let k = self.db.key(w).unwrap().0;
            ev += &format!("{{ at: {at}, event: {{ Fired: {{ shooter: 1, weapon: {k}, hardpoint: {hp}, target: {} }} }} }},", if *target == 0 { "{ Point: { x: 0, y: 0, z: 600 } }".to_owned() } else { format!("{{ Entity: {target} }}") });
        }
        for (at, w) in &self.hits {
            let k = self.db.key(w).unwrap().0;
            ev += &format!("{{ at: {at}, event: {{ Hit: {{ target: 2, by: 1, weapon: {k}, facing: 0, shield: 0, hull: 10, at: {{ x: 0, y: 0, z: 600 }} }} }} }},");
        }
        let text = format!(
            "{{ units: [
                {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}, {{ at: 300, pos: {{ x: 60, y: 0, z: 0 }} }}]{} }},
                {{ id: 2, def: \"{}\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: {}, z: {} }} }}, {{ at: 300, pos: {{ x: {}, y: {}, z: {} }} }}]{} }},
            ], events: [{ev}] }}",
            until(self.shooter_until),
            self.target_def,
            self.target_y,
            self.target_z,
            self.target_x,
            self.target_y,
            self.target_z,
            until(self.target_until)
        );
        let inner = ScriptedSource::from_json5(&text, &self.db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / TICK_HZ as f64)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(Arc::new(self.db.clone())))
            .insert_resource(SpriteTables(Arc::new(table())))
            .insert_resource(SimLink::new(Box::new(Shielded { inner, shields: self.shields, shields_until: self.shields_until, held: self.held.map(|(a, b)| (a, b, self.db.key("w_phaser").unwrap())), dying_from: self.shooter_dying, flown: self.flown.iter().map(|(a, b, id, p)| (*a, *b, *id, self.db.key("w_torp").unwrap(), *p)).collect() }), Box::new(LocalRouter::new()), PlayerId(0)));
        if let Some(p) = profile {
            app.insert_resource(p);
        }
        app.add_plugins(super::super::weapons::WeaponFxPlugin);
        configure_sets(&mut app);
        app
    }
}

fn shots(app: &mut App) -> Vec<StockShot> {
    let mut v: Vec<StockShot> = app.world_mut().query::<&StockShot>().iter(app.world()).cloned().collect();
    v.sort_by_key(|s| (s.fired, s.weapon.0));
    v
}

fn age(app: &App, s: &StockShot) -> f32 {
    (app.world().resource::<AnimClock>().now() * TICK_HZ as f64 - s.fired as f64) as f32
}

fn fx_rows(app: &mut App) -> Vec<(u32, u32, super::super::weapons::FxKind, Tick, [u32; 3], [u32; 3], u32)> {
    let mut q = app.world_mut().query::<(&super::super::weapons::WeaponFx, &Transform, &super::super::weapons::FxAlpha)>();
    let b = |v: Vec3| v.to_array().map(f32::to_bits);
    let mut rows: Vec<_> = q.iter(app.world()).map(|(f, t, a)| (f.shooter.0, f.weapon.0, f.kind, f.fired, b(t.translation), b(t.scale), a.0.to_bits())).collect();
    rows.sort_by_key(|r| (r.0, r.1, r.2, r.3));
    rows
}

fn mixed_setup(rules: ShotCfg) -> Setup {
    let mut s = Setup::new(rules);
    s.events = vec![(5, "w_phaser", 0), (5, "w_pulse", 1), (5, "w_torp", 2), (5, "w_tractor", 3), (40, "w_phaser", 0), (40, "w_tractor", 3)]
        .into_iter()
        .map(|(a, w, h)| (a, w, h, 2))
        .collect();
    s
}

/// The stock path stays off under the Remaster look: with the profile resources set to Refit the effects
/// of a mixed scene are bit for bit those of the same code without them (today's path). What `main`
/// itself produces for the recipe colours is pinned in `weapons.rs`
/// (`the_recipe_colour_is_the_weapons_own_light_colour_as_on_main`).
#[test]
fn the_remaster_look_spawns_what_it_always_did() {
    let setup = mixed_setup(ShotCfg::default());
    let frames = 120;
    let mut today = setup.app(None);
    let mut refit = setup.app(Some(RenderProfile::Refit));
    let mut seen = 0;
    for _ in 0..frames {
        today.update();
        refit.update();
        let (a, b) = (fx_rows(&mut today), fx_rows(&mut refit));
        assert_eq!(a, b, "the Remaster look differs from today's");
        assert!(shots(&mut refit).is_empty(), "a stock shot under the Remaster look");
        seen = seen.max(a.len());
    }
    assert!(seen > 6, "the scene fired nothing ({seen})");
    // Today's muzzle flashes: one per shot of every class.
    let flashes = |app: &mut App| fx_rows(app).iter().filter(|r| r.2 == super::super::weapons::FxKind::Flash).count();
    let mut again = setup.app(Some(RenderProfile::Refit));
    let mut most = 0;
    for _ in 0..frames {
        again.update();
        most = most.max(flashes(&mut again));
    }
    assert!(most >= 4, "{most}");
}

#[test]
fn under_stock_a_class_with_a_drawing_is_a_stock_shot_and_nothing_else_gets_a_muzzle_flash() {
    let setup = mixed_setup(ShotCfg::default());
    let mut app = setup.app(Some(RenderProfile::Stock));
    let (mut max_stock, mut spawned, mut kinds) = (0, std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for _ in 0..120 {
        app.update();
        max_stock = max_stock.max(shots(&mut app).len());
        spawned.extend(shots(&mut app).iter().map(|s| (s.weapon.0, s.fired)));
        for r in fx_rows(&mut app) {
            kinds.insert((r.1, r.2));
        }
    }
    assert_eq!(spawned.len(), 4, "the phaser, the pulse and the torpedo fired together, the phaser again later");
    assert_eq!(max_stock, 3, "the first phaser has ended when the second is fired");
    let db = &setup.db;
    let tractor = db.key("w_tractor").unwrap().0;
    let todays: Vec<_> = kinds.iter().filter(|(w, _)| *w == tractor).map(|(_, k)| *k).collect();
    assert!(todays.contains(&super::super::weapons::FxKind::Beam), "a class without a stock drawing keeps today's: {todays:?}");
    assert!(!kinds.iter().any(|(_, k)| *k == super::super::weapons::FxKind::Flash), "no muzzle flash: the tractor has no flare in the original, the stock shots wait for the flare package");
    for w in ["w_phaser", "w_pulse", "w_torp"] {
        assert!(!kinds.iter().any(|(k, _)| *k == db.key(w).unwrap().0), "{w} was also drawn the Refit way");
    }
}

#[test]
fn the_beam_follows_the_timeline_from_the_hardpoint_node_which_tracks_the_ship() {
    let setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    let mut app = setup.app(Some(RenderProfile::Stock));
    // The shooter's model: a node 20 units ahead of the unit's origin (render space, -z ahead).
    let root = app.world_mut().spawn(GlobalTransform::default()).id();
    app.world_mut().spawn((Name::new("HP_Phaser"), GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -20.0)), ChildOf(root)));
    app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(1), root);
    let mut samples = Vec::new();
    for _ in 0..50 {
        app.update();
        for s in shots(&mut app) {
            samples.push((age(&app, &s), s.geom, s.life));
        }
    }
    assert!(samples.len() >= 25, "{} frames of the beam", samples.len());
    let life = samples[0].2;
    assert_eq!(life, 30.0, "lifeSpan 1 s: the data's life span from the fire event");
    let mut grew = false;
    let mut full = false;
    let mut retracted = false;
    for (a, g, _) in &samples {
        let Geom::Segment { a: p, b: q } = *g else { panic!("{g:?}") };
        // Start = the node, carried along the ship's own rail: the ship moves along +x, 60 units in 10 s.
        // (The rail time is the sim's; here only that the start sits 20 ahead and has moved with the ship.)
        assert!((p.z - -20.0).abs() < 1e-3 || *a > 24.0 || *a < 0.0, "start {p} at age {a}");
        let f = (life - a) / life;
        if f > 0.8 {
            grew = true;
            assert!(q.z < -20.0 || *a < 0.2, "growing, end {q}");
        } else if f > 0.2 {
            full = true;
            // The full length: from the node to the target's centre (600 ahead).
            assert!((q.z - -600.0).abs() < 1e-2, "full end {q} at age {a}");
            assert!((p.z - -20.0).abs() < 1e-3, "full start {p}");
        } else {
            retracted = true;
            assert!((q.z - -600.0).abs() < 1e-2, "retracting end {q}");
        }
    }
    assert!(grew && full && retracted);
    // After the life span nothing is left.
    assert!(shots(&mut app).is_empty());
    // And the start moves with the ship: the first sample's start is the ship's place then plus the node's offset.
    let first_x = samples.iter().find_map(|s| if let Geom::Segment { a, .. } = s.1 { Some(a.x) } else { None }).unwrap();
    let last_x = samples.iter().rev().find_map(|s| if let Geom::Segment { a, .. } = s.1 { Some(a.x) } else { None }).unwrap();
    assert!(last_x > first_x, "the start stood still while the ship moved ({first_x} to {last_x})");
}

#[test]
fn a_beam_stands_while_the_game_is_paused() {
    let setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    let mut app = setup.app(Some(RenderProfile::Stock));
    for _ in 0..10 {
        app.update();
    }
    app.world_mut().resource_mut::<AnimClock>().set_paused(true);
    app.update();
    let held = shots(&mut app);
    assert_eq!(held.len(), 1);
    for _ in 0..90 {
        app.update();
        let now = shots(&mut app);
        assert_eq!(now.len(), 1, "the beam ended while paused");
        assert_eq!(now[0].geom, held[0].geom, "the beam moved while paused");
    }
    app.world_mut().resource_mut::<AnimClock>().set_paused(false);
    for _ in 0..60 {
        app.update();
    }
    assert!(shots(&mut app).is_empty(), "it runs out once the game runs again");
}

#[test]
fn the_shield_ellipsoid_ends_the_beam_while_shields_are_up_else_the_hardpoint() {
    let end_after = |shields: bool, target: &'static str| {
        let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
        setup.shields = shields;
        setup.target_def = target;
        let mut app = setup.app(Some(RenderProfile::Stock));
        let mut end = None;
        for _ in 0..25 {
            app.update();
            for s in shots(&mut app) {
                let f = (s.life - age(&app, &s)) / s.life;
                if let Geom::Segment { b, .. } = s.geom
                    && (0.3..0.7).contains(&f)
                {
                    end = Some(b);
                }
            }
        }
        end.expect("a full beam was seen")
    };
    // The target is 600 ahead of the shooter, at (0, 0, -600) in render space; its shield is the default-sized ellipsoid.
    let up = end_after(true, "t_target");
    let r = super::super::impacts::DEFAULT_RADIUS;
    let gap = (up - Vec3::new(0.0, 0.0, -600.0)).length();
    assert!(gap > 0.8 * r - 1.0 && gap < 1.5 * r + 1.0, "ended {gap} from the centre with shields up");
    // Down: the centre (this model has no hardpoint node to find).
    let down = end_after(false, "t_target");
    assert!((down - Vec3::new(0.0, 0.0, -600.0)).length() < 1e-2, "{down}");
    // A planet: its bounding sphere, with or without shields.
    let planet = end_after(false, "t_planet");
    let gap = (planet - Vec3::new(0.0, 0.0, -600.0)).length();
    assert!((gap - r).abs() < 1.0, "a planet's sphere ({gap})");
}

#[test]
fn a_lost_target_keeps_the_beam_to_its_range_for_the_miss_lifetime_then_ends_it() {
    let miss = ShotCfg { beam_miss_lifetime: Some(Fixed::from_f64(0.5)), normal_weapon_team_color: None };
    let run = |cfg: ShotCfg, until: Option<u64>| {
        let mut setup = Setup::new(cfg).fire(4, "w_phaser", 0);
        setup.target_until = until;
        let mut app = setup.app(Some(RenderProfile::Stock));
        let mut rows = Vec::new();
        for _ in 0..60 {
            app.update();
            for s in shots(&mut app) {
                rows.push((age(&app, &s), s.geom));
            }
        }
        rows
    };
    // The target goes at tick 10: with 0.5 s (15 ticks) from the shot the beam stays until age 15, to its range (1000).
    let kept = run(miss, Some(10));
    let last_age = kept.iter().map(|r| r.0).fold(0.0, f32::max);
    assert!((14.0..16.0).contains(&last_age), "last drawn at age {last_age}");
    let Geom::Segment { b, .. } = kept.last().unwrap().1 else { panic!() };
    assert!((b.length() - 1000.0).abs() < 25.0, "the end {b} is at the weapon's range from the muzzle");
    // No miss lifetime in the header: lost means ended at once.
    let cut = run(ShotCfg::default(), Some(10));
    assert!(cut.iter().map(|r| r.0).fold(0.0, f32::max) < 8.0, "no timer, the beam ran on");
    // Lost after the timer has run: ended at once (the target goes at tick 30, the timer ran out at 5 + 15).
    let late = run(miss, Some(30));
    let late_last = late.iter().map(|r| r.0).fold(0.0, f32::max);
    assert!(late_last > 18.0 && late_last < 27.0, "lost after the timer, last drawn at {late_last}");
}

#[test]
fn a_target_the_shot_killed_on_the_fire_tick_is_held_for_the_beams_life() {
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    setup.target_until = Some(4);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let mut longest = 0.0f32;
    for _ in 0..50 {
        app.update();
        for s in shots(&mut app) {
            longest = longest.max(age(&app, &s));
        }
    }
    assert!(longest > 25.0, "the beam ended at age {longest}");
}

#[test]
fn a_beam_ends_when_its_owner_is_gone_and_when_the_target_is_beyond_twice_the_range() {
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    setup.shooter_until = Some(10);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let mut ended = None;
    for i in 0..40 {
        app.update();
        if ended.is_none() && i > 6 && shots(&mut app).is_empty() {
            ended = Some(i);
        }
    }
    assert!(ended.is_some_and(|i| i <= 12), "ended at frame {ended:?}, the owner went at tick 10");
    // Authored range 1000: the cut is at twice the range, so a target at 1900 is in and one at 2100 is out.
    let beams = |z: i32| {
        let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
        setup.target_z = z;
        let mut app = setup.app(Some(RenderProfile::Stock));
        let mut n = 0;
        for _ in 0..12 {
            app.update();
            n = n.max(shots(&mut app).len());
        }
        n
    };
    assert_eq!(beams(1900), 1);
    assert_eq!(beams(2100), 0);
}

/// The frame (one per tick) a lone phaser beam fired at tick 4 is last drawn on, over `frames` frames.
fn last_drawn(setup: &Setup, frames: usize) -> Option<usize> {
    let mut app = setup.app(Some(RenderProfile::Stock));
    let mut last = None;
    for i in 0..frames {
        app.update();
        if !shots(&mut app).is_empty() {
            last = Some(i);
        }
    }
    last
}

#[test]
fn under_beam_damage_at_end_the_beam_is_drawn_while_the_sim_holds_it_and_not_after_it_loses_it() {
    // The sim holds the beam through its thirty ticks: drawn to the end of its life, as ever.
    let held = Setup::new(ShotCfg::default()).at_end(ShotCfg::default()).fire(4, "w_phaser", 0).held(4, 34);
    let full = last_drawn(&held, 60).expect("drawn");
    assert!((32..=36).contains(&full), "a held beam is drawn for its whole life: last frame {full}");
    // The sim loses it at tick 12 (its target out of range, say): the drawing ends within a few ticks, and never comes back.
    let lost = Setup::new(ShotCfg::default()).at_end(ShotCfg::default()).fire(4, "w_phaser", 0).held(4, 12);
    let cut = last_drawn(&lost, 60).expect("drawn");
    assert!(cut <= 16, "a beam the sim lost is still drawn at frame {cut}");
    assert!(cut >= 8, "and it was drawn while the sim held it (frame {cut})");
    // Every other profile reads the beam as before: the same view rows (or none) change nothing.
    let off = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0).held(4, 12);
    assert_eq!(last_drawn(&off, 60), last_drawn(&Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0), 60), "the switch off: the rows are not read");
    assert!(last_drawn(&off, 60).is_some_and(|f| f >= 28), "a beam without the switch lives its life");
}

#[test]
fn under_beam_damage_at_end_a_beam_the_sim_lost_is_drawn_on_to_its_range_while_the_miss_timer_runs_then_ends() {
    let miss = ShotCfg { beam_miss_lifetime: Some(Fixed::from_f64(0.5)), normal_weapon_team_color: None };
    // The last frame drawn and its end, for a beam the sim loses at tick 12.
    let run = |cfg: ShotCfg| {
        let setup = Setup::new(cfg).at_end(cfg).fire(4, "w_phaser", 0).held(4, 12);
        let mut app = setup.app(Some(RenderProfile::Stock));
        let (mut last, mut end) = (None, None);
        for i in 0..60 {
            app.update();
            if let Some(s) = shots(&mut app).first() {
                last = Some(i);
                end = Some(s.geom);
            }
        }
        (last.expect("drawn"), end)
    };
    // With a timer of fifteen ticks from the shot the beam runs on to the weapon's range, then ends: INTERIM, along the last aim line.
    let (last, end) = run(miss);
    assert!((17..=22).contains(&last), "drawn on to the end of the timer: last frame {last}");
    let Some(Geom::Segment { b, .. }) = end else { panic!("{end:?}") };
    assert!((b.length() - 1000.0).abs() < 25.0, "the end {b} is at the weapon's range from the muzzle");
    // No timer: it ends within a few ticks of the sim's loss, as before.
    assert!(run(ShotCfg::default()).0 <= 16);
    // The switch off: the rows are not read and the beam lives its life (the other profiles are unchanged).
    let off = Setup::new(miss).fire(4, "w_phaser", 0).held(4, 12);
    assert!(last_drawn(&off, 60).is_some_and(|f| f >= 28));
}

#[test]
fn under_beam_damage_at_end_a_dying_shooter_or_a_target_beyond_twice_the_range_ends_the_beam_at_once_and_a_lost_target_does_not() {
    let miss = ShotCfg { beam_miss_lifetime: Some(Fixed::from_f64(0.5)), normal_weapon_team_color: None };
    let base = || Setup::new(miss).at_end(miss).fire(4, "w_phaser", 0).held(4, 12);
    // The target is lost (phased, say) with the timer running: drawn on past the loss.
    assert!(last_drawn(&base(), 60).is_some_and(|f| f >= 17));
    // The shooter starts to die when the sim loses the beam: it ends within a few ticks, timer or not.
    let mut dying = base();
    dying.shooter_dying = Some(12);
    assert!(last_drawn(&dying, 60).is_some_and(|f| f <= 16), "a dying shooter's beam ran on");
    // The target is beyond twice the range (the authored range is 1000): it ends at once as without the switch.
    let mut far = base();
    far.target_z = 2100;
    assert!(last_drawn(&far, 60).is_some_and(|f| f <= 16), "a beam to a target beyond the break distance ran on");
}

#[test]
fn a_beam_the_sim_lost_stays_lost_to_the_end_of_its_life_with_a_timer_longer_than_the_life() {
    // A timer of two seconds outlasts the beam's life: the beam, lost at tick 12 with its target still in the view, is
    // drawn on to its range to the end and does not snap back to the target when the judging window ends.
    let long = ShotCfg { beam_miss_lifetime: Some(Fixed::from_f64(2.0)), normal_weapon_team_color: None };
    let setup = Setup::new(long).at_end(long).fire(4, "w_phaser", 0).held(4, 12);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let mut late = Vec::new();
    for i in 0..60 {
        app.update();
        if let Some(s) = shots(&mut app).first()
            && i >= 14
            && let Geom::Segment { b, .. } = s.geom
        {
            late.push(b.length());
        }
    }
    assert!(late.len() >= 10, "drawn on to the end of the life: {} frames", late.len());
    assert!(late.iter().all(|l| (l - 1000.0).abs() < 25.0), "{late:?}");
}

#[test]
fn the_drawing_breaks_a_beam_at_the_distance_the_sim_does() {
    // One source: the sim's factor, public, squared. An authored range of 1000: the sim holds a target at its limit and loses one beyond.
    let limit = (sta_sim::combat::weapons::BEAM_BREAK_RANGE_FACTOR * 1000) as f32;
    assert_eq!(break_distance_squared(1000.0), limit * limit);
    assert!(1900.0f32.powi(2) <= break_distance_squared(1000.0) && 2100.0f32.powi(2) > break_distance_squared(1000.0));
}

#[test]
fn a_torpedo_in_flight_keeps_one_line_to_its_node_when_the_targets_shields_drop() {
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_torp", 2);
    setup.shields = true;
    setup.shields_until = Some(30);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let node = Vec3::new(5.0, 0.0, 0.0);
    let mut points = Vec::new();
    for i in 0..58 {
        app.update();
        if i == 0 {
            model(&mut app, 2, &[("hull_a", node), ("hull_b", node)]);
        }
        for s in shots(&mut app) {
            if let Geom::Billboard { c } = s.geom {
                points.push((i, c, s.muzzle));
            }
        }
    }
    assert!(points.iter().any(|p| p.0 < 28) && points.iter().any(|p| p.0 > 34), "drawn before and after the shields dropped");
    // One straight line from the muzzle: every drawn place is the same share of the way along it, to a fixed end.
    let (_, last, muzzle) = *points.last().unwrap();
    let dir = (last - muzzle).normalize();
    let worst = points.iter().map(|(_, c, m)| (*c - *m).cross(dir).length()).fold(0.0, f32::max);
    assert!(worst < 0.05, "a sideways step of {worst} across the flight");
    // The line goes to the node (five units off the target's centre), not the centre.
    let centre_dir = (Vec3::new(0.0, 0.0, -600.0) - muzzle).normalize();
    assert!(dir.distance(centre_dir) > 1e-3, "the flight aims at the centre");
}

#[test]
fn under_beam_damage_at_end_the_sims_break_distance_decides_not_the_drawings_own_reading() {
    // The drawing's own cut ends a beam whose target is beyond twice the range (the authored range is 1000), measured from
    // the muzzle: with the switch off a target at 2100 gets no beam. With the switch on the sim decides: it holds the beam
    // (its distance is the sim's own, centre to centre), so the beam is drawn.
    let mut setup = Setup::new(ShotCfg::default()).at_end(ShotCfg::default()).fire(4, "w_phaser", 0).held(4, 34);
    setup.target_z = 2100;
    assert!(last_drawn(&setup, 60).is_some_and(|f| f >= 30), "the sim holds it: drawn");
    let mut off = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    off.target_z = 2100;
    assert_eq!(last_drawn(&off, 60), None, "the switch off: the drawing's own cut, as before");
}

#[test]
fn a_shot_in_flight_flies_on_when_its_shooter_is_gone_and_a_beam_does_not() {
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_pulse", 1).fire(4, "w_torp", 2).fire(4, "w_phaser", 0);
    setup.shooter_until = Some(10);
    let mut app = setup.app(Some(RenderProfile::Stock));
    for i in 0..40 {
        app.update();
        let paths: Vec<ShotPath> = shots(&mut app).iter().map(|s| s.path).collect();
        if i > 14 {
            // The owner went at tick 10: the beam is gone, the streak and the sprite still fly (flight is 60 ticks).
            assert!(!paths.contains(&ShotPath::Ribbon), "frame {i}: {paths:?}");
            assert!(paths.contains(&ShotPath::Streak) && paths.contains(&ShotPath::Sprite), "frame {i}: {paths:?}");
            assert!(shots(&mut app).iter().all(|s| s.geom != Geom::None));
        }
    }
}

#[test]
fn a_pulse_streak_and_a_torpedo_sprite_fly_to_their_target_and_end_on_the_hit() {
    let setup = Setup::new(ShotCfg::default()).fire(4, "w_pulse", 1).fire(4, "w_torp", 2);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let (mut streaks, mut sprites) = (0, 0);
    for _ in 0..30 {
        app.update();
        for s in shots(&mut app) {
            let a = age(&app, &s);
            // The flight takes a whole number of ticks (its distance over 10 a tick, rounded up): the shot's own speed.
            let flown = s.muzzle.distance(s.to) / s.life * a;
            match s.geom {
                Geom::Segment { a: head, b: tail } => {
                    streaks += 1;
                    assert_eq!(s.path, ShotPath::Streak);
                    assert!((head.distance(s.muzzle) - (flown + 10.0)).abs() < 0.2, "head {head} after {a} ticks");
                    // min(Length = 30, distance flown) back along the path.
                    assert!((head.distance(tail) - 30.0f32.min(flown + 10.0)).abs() < 0.2, "{} long after {a} ticks", head.distance(tail));
                    assert_eq!(s.radius, 4.0);
                }
                Geom::Billboard { c } => {
                    sprites += 1;
                    assert_eq!(s.path, ShotPath::Sprite);
                    assert!((c.distance(s.muzzle) - flown).abs() < 0.2, "sprite at {c} after {a} ticks");
                }
                Geom::None => {}
            }
        }
    }
    assert!(streaks > 10 && sprites > 10);
    // A Hit for the torpedo ends its sprite and not the pulse's streak.
    let mut hit = Setup::new(ShotCfg::default()).fire(4, "w_pulse", 1).fire(4, "w_torp", 2);
    let k = hit.db.key("w_torp").unwrap().0;
    let src = format!(
        "{{ units: [
            {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
            {{ id: 2, def: \"t_target\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}] }},
        ], events: [
            {{ at: 4, event: {{ Fired: {{ shooter: 1, weapon: {w}, hardpoint: 1, target: {{ Entity: 2 }} }} }} }},
            {{ at: 4, event: {{ Fired: {{ shooter: 1, weapon: {k}, hardpoint: 2, target: {{ Entity: 2 }} }} }} }},
            {{ at: 14, event: {{ Hit: {{ target: 2, by: 1, weapon: {k}, facing: 0, shield: 0, hull: 60, at: {{ x: 0, y: 0, z: 590 }} }} }} }},
        ] }}",
        w = hit.db.key("w_pulse").unwrap().0
    );
    hit.events.clear();
    let inner = ScriptedSource::from_json5(&src, &hit.db).unwrap();
    let mut app = hit.app(Some(RenderProfile::Stock));
    *app.world_mut().resource_mut::<SimLink>() = SimLink::new(Box::new(Shielded { inner, shields: false, shields_until: None, held: None, dying_from: None, flown: Vec::new() }), Box::new(LocalRouter::new()), PlayerId(0));
    let mut after = Vec::new();
    for _ in 0..24 {
        app.update();
        after.push(shots(&mut app).iter().map(|s| s.path).collect::<Vec<_>>());
    }
    assert!(after.iter().any(|v| v.contains(&ShotPath::Sprite)));
    assert_eq!(after.last().unwrap(), &vec![ShotPath::Streak], "the torpedo's sprite ended at the hit, the pulse flies on");
}

#[test]
fn white_unless_the_team_colour_flag_is_set_and_then_the_owners_for_the_listed_classes() {
    let palette = || crate::viewer::TeamColours(vec![("empty".into(), Color::srgb(0.5, 0.5, 0.5)), ("red".into(), Color::srgb(1.0, 0.0, 0.0))]);
    let colours = |flag: Option<bool>| {
        let cfg = ShotCfg { beam_miss_lifetime: None, normal_weapon_team_color: flag };
        let setup = Setup::new(cfg).fire(4, "w_phaser", 0).fire(4, "w_torp", 2).fire(4, "w_art", 4);
        let mut app = setup.app(Some(RenderProfile::Stock));
        app.insert_resource(palette());
        for _ in 0..8 {
            app.update();
        }
        shots(&mut app).into_iter().map(|s| (s.path, s.colour)).collect::<Vec<_>>()
    };
    for flag in [None, Some(false)] {
        assert!(colours(flag).iter().all(|c| c.1 == Vec3::ONE), "white without the flag");
    }
    let on = colours(Some(true));
    assert_eq!(on.len(), 3);
    // Seat 0 takes the first team entry; a class outside the list (artillery) stays white even with the flag.
    let red = Vec3::new(1.0, 0.0, 0.0);
    assert_eq!(on.iter().filter(|c| c.1 == red).count(), 2, "{on:?}");
    assert_eq!(on.iter().filter(|c| c.1 == Vec3::ONE).count(), 1, "{on:?}");
}

#[test]
fn sprites_resolve_with_the_ordnance_duration_of_their_content() {
    let mut fast = torpedo();
    fast.odf = "w_torp_fast".into();
    fast.look.sprite_duration = Some(0.5);
    let db = ContentBuilder::default().weapon(phaser()).weapon(torpedo()).weapon(fast).build().unwrap();
    let sprites = ShotSprites::build(&table(), &db);
    assert!(sprites.get("BEAMSPR").is_some() && sprites.get("torpspr").is_some() && sprites.get("pulsespr").is_none(), "only the sprites the content names");
    // The torpedo sprite is shared and the one override wins for both.
    let at = |t: f32| sprites.get("torpspr").unwrap().rect_at(t)[0];
    assert!((at(0.3) - 0.5).abs() < 1e-6, "{}", at(0.3));
}

// ---------------------------------------------------------------------------------------------
// Flares
// ---------------------------------------------------------------------------------------------

fn flares(app: &mut App) -> Vec<Flare> {
    let mut v: Vec<Flare> = app.world_mut().query::<&Flare>().iter(app.world()).cloned().collect();
    v.sort_by(|a, b| (a.object.0, a.born).partial_cmp(&(b.object.0, b.born)).unwrap());
    v
}

fn of_kind(v: &[Flare], kind: FlareKind) -> Vec<Flare> {
    v.iter().filter(|f| f.kind == kind).cloned().collect()
}

/// Every flare of `kind` the app shows over `frames` frames, once each (by object and birth).
fn seen(app: &mut App, frames: usize, kind: FlareKind) -> Vec<Flare> {
    let mut all: Vec<Flare> = Vec::new();
    for _ in 0..frames {
        app.update();
        for f in of_kind(&flares(app), kind) {
            if !all.iter().any(|g| g.object == f.object && g.born == f.born) {
                all.push(f);
            }
        }
    }
    all
}

/// The model of unit `id`: a root and named nodes at given offsets (render space).
fn model(app: &mut App, id: u32, nodes: &[(&str, Vec3)]) -> Entity {
    let root = app.world_mut().spawn((GlobalTransform::default(), InheritedVisibility::VISIBLE)).id();
    for (name, at) in nodes {
        app.world_mut().spawn((Name::new(name.to_string()), GlobalTransform::from_translation(*at), ChildOf(root)));
    }
    app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(id), root);
    root
}

#[test]
fn the_muzzle_flare_lasts_the_part_of_the_life_after_the_grow_up_to_the_cap() {
    assert!((stock_flares::muzzle_life(1.0, 0.2) - 0.8).abs() < 1e-6);
    assert_eq!(stock_flares::muzzle_life(20.0, 0.1), stock_flares::MUZZLE_LIFE_CAP, "capped");
    assert_eq!(stock_flares::muzzle_life(1.0, 0.0), 1.0);
    assert_eq!(stock_flares::muzzle_life(1.0, 1.0), 0.0);
}

#[test]
fn only_the_ordnance_class_that_has_a_flare_raises_a_muzzle_flare_sized_and_timed_from_the_data() {
    let setup = Setup::new(ShotCfg::default())
        .fire(4, "w_phaser", 0)
        .fire(4, "w_pulse", 1)
        .fire(4, "w_torp", 2)
        .fire(4, "w_tractor", 3)
        .fire(4, "w_art", 4)
        .fire(4, "w_phaser_plain", 0);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let muzzle = seen(&mut app, 12, FlareKind::Muzzle);
    assert_eq!(muzzle.len(), 1, "{muzzle:?}");
    let f = &muzzle[0];
    assert_eq!(f.object, EntityId(1));
    assert_eq!(f.sprite, "flarespr");
    // Radius 4 times the multiplier 2; (1 - 0.2) of a 1 s life span, 24 ticks; the shot's colour (white).
    assert_eq!(f.half, 8.0);
    assert_eq!(f.life, 0.8 * TICK_HZ as f32);
    assert_eq!(f.colour, Vec3::ONE);
}

#[test]
fn the_impact_flare_appears_on_hull_hits_only() {
    // (shields up, target, weapon) and whether a hull hit.
    let cases = [
        (false, "t_target", "w_phaser", 1),
        (true, "t_target", "w_phaser", 0),
        (true, "t_target", "w_phaser_ign", 1),
        (true, "t_planet", "w_phaser", 1),
        (false, "t_planet", "w_phaser", 1),
    ];
    for (shields, target, weapon, want) in cases {
        let mut setup = Setup::new(ShotCfg::default()).fire(4, weapon, 0);
        setup.shields = shields;
        setup.target_def = target;
        let mut app = setup.app(Some(RenderProfile::Stock));
        let mut impacts: Vec<Flare> = Vec::new();
        let mut fired = None;
        for _ in 0..45 {
            app.update();
            fired = fired.or_else(|| shots(&mut app).first().map(|s| s.fired));
            for f in of_kind(&flares(&mut app), FlareKind::Impact) {
                if !impacts.iter().any(|g| g.born == f.born) {
                    impacts.push(f);
                }
            }
        }
        assert_eq!(impacts.len(), want, "shields up {shields}, {target}, {weapon}");
        if let Some(f) = impacts.first() {
            // Radius 4 times 2, one second, white, on the target, born when the beam reaches it (a fifth of 30 ticks after the shot).
            assert_eq!((f.object, f.half, f.life, f.colour, f.sprite.as_str()), (EntityId(2), 8.0, TICK_HZ as f32, Vec3::ONE, stock_flares::IMPACT_SPRITE));
            let after = f.born - fired.unwrap() as f64;
            assert!((5.99..7.5).contains(&after), "born {after} ticks after the shot");
        }
    }
}

#[test]
fn a_beam_at_a_point_and_the_other_classes_raise_no_impact_flare() {
    let setup = Setup::new(ShotCfg::default()).fire(4, "w_pulse", 1).fire(4, "w_torp", 2).fire(4, "w_art", 4).fire(4, "w_tractor", 3).fire_at_point(4, "w_phaser", 0);
    let mut app = setup.app(Some(RenderProfile::Stock));
    // The phaser at a place is drawn (a stock beam), muzzle flare and all, but has no object to land on.
    let mut beam = false;
    let mut impacts = Vec::new();
    for _ in 0..90 {
        app.update();
        beam |= shots(&mut app).iter().any(|s| s.path == ShotPath::Ribbon);
        impacts.extend(of_kind(&flares(&mut app), FlareKind::Impact));
    }
    assert!(beam, "the beam at a point was not fired");
    assert!(impacts.is_empty());
}

#[test]
fn flares_follow_their_objects() {
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    setup.target_x = 300;
    let mut app = setup.app(Some(RenderProfile::Stock));
    // The muzzle's node 20 ahead of the moving ship; the target's hull hardpoint 7 right and 2 ahead of its centre.
    model(&mut app, 1, &[("HP_Phaser", Vec3::new(0.0, 0.0, -20.0))]);
    model(&mut app, 2, &[("hull_a", Vec3::new(7.0, 0.0, -2.0)), ("hull_b", Vec3::new(7.0, 0.0, -2.0))]);
    let (mut muzzle_x, mut impact_x) = (Vec::new(), Vec::new());
    for _ in 0..40 {
        app.update();
        let centre = {
            let w = app.world();
            point(w.resource::<SimLink>(), EntityId(2), w.resource::<AnimClock>().alpha()).unwrap()
        };
        for f in flares(&mut app) {
            match f.kind {
                FlareKind::Muzzle => {
                    assert!((f.at.z - -20.0).abs() < 1e-3, "{}", f.at);
                    muzzle_x.push(f.at.x);
                }
                FlareKind::Impact => {
                    // In the target's frame: the same offset from its centre whatever it has moved.
                    assert!((f.at - centre).distance(Vec3::new(7.0, 0.0, -2.0)) < 1e-2, "{} from the target at {centre}", f.at);
                    impact_x.push(f.at.x);
                }
                FlareKind::HullHit | FlareKind::DeathSpark => unreachable!("no sparks in this scene"),
            }
        }
    }
    assert!(muzzle_x.len() > 10 && impact_x.len() > 10, "{} / {}", muzzle_x.len(), impact_x.len());
    assert!(muzzle_x.last().unwrap() > muzzle_x.first().unwrap(), "the muzzle flare stood still while the ship moved");
    assert!(impact_x.last().unwrap() - impact_x.first().unwrap() > 5.0, "the impact flare stood still while the target moved");
}

#[test]
fn a_muzzle_flare_ends_before_its_beam_and_with_its_owner() {
    let mut app = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0).app(Some(RenderProfile::Stock));
    let mut muzzle_last = 0;
    let mut beam_last = 0;
    for i in 0..45 {
        app.update();
        if !of_kind(&flares(&mut app), FlareKind::Muzzle).is_empty() {
            muzzle_last = i;
        }
        if !shots(&mut app).is_empty() {
            beam_last = i;
        }
    }
    assert!(muzzle_last + 4 <= beam_last, "the flare lasts 24 ticks of the beam's 30 (flare {muzzle_last}, beam {beam_last})");
    assert!(app.world().resource::<NodeParticles>().total() == 0, "its slots are free again");
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0);
    setup.shooter_until = Some(10);
    let mut app = setup.app(Some(RenderProfile::Stock));
    for _ in 0..20 {
        app.update();
    }
    assert!(of_kind(&flares(&mut app), FlareKind::Muzzle).is_empty(), "the owner is gone");
}

#[test]
fn at_most_four_flares_live_per_object() {
    let mut setup = Setup::new(ShotCfg::default());
    for t in 4..10 {
        setup = setup.fire(t, "w_phaser", 0);
    }
    let mut app = setup.app(Some(RenderProfile::Stock));
    let (mut most_muzzle, mut most_impact) = (0, 0);
    let mut fired = Vec::new();
    for _ in 0..50 {
        app.update();
        let now = flares(&mut app);
        most_muzzle = most_muzzle.max(of_kind(&now, FlareKind::Muzzle).len());
        most_impact = most_impact.max(of_kind(&now, FlareKind::Impact).len());
        let store = app.world().resource::<NodeParticles>();
        assert!(store.live(EntityId(1)) <= 4 && store.live(EntityId(2)) <= 4);
        assert_eq!(store.total(), now.len(), "the store knows exactly the live flares");
        for s in shots(&mut app) {
            if !fired.contains(&s.fired) {
                fired.push(s.fired);
            }
        }
    }
    assert_eq!((most_muzzle, most_impact), (4, 4), "six beams, four flares of each kind");
    assert_eq!(fired.len(), 6);
    // The newest lose (INTERIM): the muzzle flares that were drawn are those of the first four beams.
    let mut app = {
        let mut s = Setup::new(ShotCfg::default());
        for t in 4..10 {
            s = s.fire(t, "w_phaser", 0);
        }
        s.app(Some(RenderProfile::Stock))
    };
    let muzzle = seen(&mut app, 12, FlareKind::Muzzle);
    let mut born: Vec<f64> = muzzle.iter().map(|f| f.born).collect();
    born.sort_by(f64::total_cmp);
    fired.sort_unstable();
    assert_eq!(born, fired[..4].iter().map(|t| *t as f64).collect::<Vec<_>>());
}

#[test]
fn the_cap_is_four_per_object_and_two_hundred_in_all_for_any_kind_of_particle() {
    let mut world = World::new();
    let mut store = NodeParticles::default();
    // Four on one object, the fifth is refused; another object has its own four.
    for _ in 0..4 {
        let e = world.spawn_empty().id();
        assert!(store.admit(EntityId(1), e));
    }
    let e = world.spawn_empty().id();
    assert!(!store.admit(EntityId(1), e) && !store.has_room(EntityId(1)));
    assert!(store.admit(EntityId(2), e));
    // Fifty objects of four fill it to two hundred; no object has room after that.
    for obj in 10..60 {
        for _ in 0..4 {
            let e = world.spawn_empty().id();
            if store.total() < stock_flares::MAX_TOTAL {
                assert!(store.admit(EntityId(obj), e), "object {obj}");
            }
        }
    }
    assert_eq!(store.total(), stock_flares::MAX_TOTAL);
    assert!(!store.has_room(EntityId(999)) && !store.admit(EntityId(999), e));
    // A slot given back is free again.
    let first = store.slots_for_test()[0];
    store.release(first);
    assert!(store.has_room(EntityId(999)));
}

/// #1145 (N2): the Stock path follows the same rule: a beam of a shooter the client does not draw is laid out only
/// where its origin is over ground in sight; its impact flare follows its target.
#[test]
fn a_stock_beam_of_a_hidden_shooter_is_not_laid_out_and_its_flare_follows_the_target() {
    let run = |shooter_drawn: bool, target_drawn: bool, seen_origin: bool| {
        let mut app = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0).app(Some(RenderProfile::Stock));
        let mut grid = crate::render::fog::FogGrid::new([-1000.0, -1000.0], [1000.0, 1000.0]);
        if seen_origin {
            let c = grid.cell_of(0.0, 0.0).unwrap();
            grid.set_cell_for_test(c % grid.w, c / grid.w, true, true);
        }
        app.insert_resource(crate::render::fog::FogState { grid, updates: 1 });
        for (id, on) in [(1, shooter_drawn), (2, target_drawn)] {
            if on {
                let e = app.world_mut().spawn(GlobalTransform::default()).id();
                app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(id), e);
            }
        }
        let (mut laid, mut flare_shown, mut flare_seen) = (false, false, false);
        for _ in 0..45 {
            app.update();
            laid |= shots(&mut app).iter().any(|s| !matches!(s.geom, Geom::None));
            // The flare's `shown` is set by the visibility system after it is made: read it at the end of its life.
            let now = of_kind(&flares(&mut app), FlareKind::Impact);
            if !now.is_empty() {
                flare_seen = true;
                flare_shown = now.iter().any(|f| f.shown);
            }
        }
        (laid, flare_seen, flare_shown)
    };
    // Hidden shooter, hidden target, nothing in sight: no beam, no shown flare.
    let (laid, seen, shown) = run(false, false, false);
    assert!(!laid && !shown, "nothing of unseen units over unseen ground: laid {laid}, flare exists {seen}, shown {shown}");
    // Hidden shooter, drawn target: the flare on the target shows, the beam (the shooter's place) does not.
    let (laid, seen, shown) = run(false, true, false);
    assert!(!laid, "the hidden shooter's place is not shown");
    assert!(seen && shown, "the flare on a unit the client draws shows");
    // The origin over ground in sight: the beam is laid out.
    assert!(run(false, false, true).0, "origin in sight: drawn");
    // A drawn shooter: always.
    assert!(run(true, true, false).0, "a drawn shooter's beam is laid out whatever the ground");
}

#[test]
fn flares_are_drawn_only_while_their_object_is_visible() {
    let mut app = Setup::new(ShotCfg::default()).fire(4, "w_phaser", 0).app(Some(RenderProfile::Stock));
    let root = model(&mut app, 1, &[("HP_Phaser", Vec3::new(0.0, 0.0, -20.0))]);
    app.world_mut().entity_mut(root).insert(InheritedVisibility::HIDDEN);
    for _ in 0..12 {
        app.update();
    }
    let hidden = of_kind(&flares(&mut app), FlareKind::Muzzle);
    assert_eq!(hidden.len(), 1, "a hidden object's flare still exists and holds its slot");
    assert!(!hidden[0].shown);
    let sprites = ShotSprites::build(&table(), app.world().resource::<ContentRes>().0.as_ref());
    let quads = |v: &[Flare]| lay_out(std::iter::empty(), v.iter(), &sprites, Vec3::Z * 100.0, Vec3::X, Vec3::Y).values().map(|g| g.idx.len() / 6).sum::<usize>();
    assert_eq!(quads(&hidden), 0);
    app.world_mut().entity_mut(root).insert(InheritedVisibility::VISIBLE);
    app.update();
    let shown = of_kind(&flares(&mut app), FlareKind::Muzzle);
    assert!(shown[0].shown);
    assert_eq!(quads(&shown), 1);
    // The quad is a screen-aligned square of twice the half size about the flare.
    let g = &lay_out(std::iter::empty(), shown.iter(), &sprites, Vec3::Z * 100.0, Vec3::X, Vec3::Y)["flarespr"];
    let (min, max) = g.pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(Vec3::from(*p)), b.max(Vec3::from(*p))));
    assert!((max - min - Vec3::new(16.0, 16.0, 0.0)).length() < 1e-3);
}

#[test]
fn the_remaster_look_raises_no_flares_and_the_flare_sprites_are_resolved_for_the_stock_one() {
    let setup = mixed_setup(ShotCfg::default());
    for profile in [None, Some(RenderProfile::Refit)] {
        let mut app = setup.app(profile);
        for _ in 0..120 {
            app.update();
            assert!(flares(&mut app).is_empty());
        }
        assert_eq!(app.world().resource::<NodeParticles>().total(), 0);
    }
    // Under Stock the same scene raises them; the sprites table of the content holds the flares' sprites.
    let mut app = setup.app(Some(RenderProfile::Stock));
    assert!(!seen(&mut app, 120, FlareKind::Muzzle).is_empty());
    let sprites = ShotSprites::build(&table(), &setup.db);
    assert!(sprites.get("flarespr").is_some() && sprites.get(stock_flares::IMPACT_SPRITE).is_some());
}

// ---------------------------------------------------------------------------------------------
// Against the mounted files
// ---------------------------------------------------------------------------------------------

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): every shot definition with a stock path names a
/// sprite of the mounted sprite table whose texture exists in the mounted files. Skips without an install.
#[test]
fn install_every_stock_shot_resolves_its_sprite_and_texture() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let table = sta_assets::load_sprite_table(&vfs);
    let (mut with_path, mut drawn, mut bad) = (0, 0, Vec::new());
    let mut fallbacks: BTreeMap<&'static str, usize> = BTreeMap::new();
    for w in db.weapons().filter(|w| class_path(&w.class.0).is_some()) {
        with_path += 1;
        if let Some(name) = w.look.sprite.as_deref()
            && let Some(s) = table.sprite(name)
            && sta_assets::find_texture(&vfs, &s.texture).is_none()
        {
            bad.push(format!("{}: texture of sprite {name} is not mounted", w.odf));
        }
        match shot_spec_or_reason(w, &table) {
            Ok(_) => drawn += 1,
            Err(why) => {
                *fallbacks.entry(why).or_default() += 1;
                eprintln!("fallback: {} ({}): {why}", w.odf, w.class.0);
                if why == "its Sprite is in no sprite table" {
                    bad.push(format!("{}: sprite is in no sprite table", w.odf));
                }
            }
        }
    }
    eprintln!("{with_path} shot definitions with a stock path, {drawn} drawn the stock way, fallbacks by reason: {fallbacks:?}");
    assert!(with_path > 0 && drawn > 0, "no shot definition took the stock path ({with_path} with a class path, {drawn} drawn)");
    assert!(bad.is_empty(), "{bad:#?}");
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): every shot definition with a muzzle flare resolves its
/// flare sprite in the mounted sprite table and finds its texture, and so does the impact flare's.
#[test]
fn install_every_flare_resolves_its_sprite_and_texture() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let table = sta_assets::load_sprite_table(&vfs);
    let usable = |name: &str| -> Result<(), String> {
        let s = table.sprite(name).ok_or(format!("sprite {name} is in no sprite table"))?;
        if spr_anim::sprite_blend(&s.material) != SpriteBlend::Add {
            return Err(format!("sprite {name} is not additive"));
        }
        sta_assets::find_texture(&vfs, &s.texture).map(|_| ()).ok_or(format!("texture of sprite {name} is not mounted"))
    };
    let (mut with_flare, mut bad) = (0, Vec::new());
    for w in db.weapons() {
        let Some(f) = w.look.flare.as_ref() else { continue };
        with_flare += 1;
        if let Err(e) = usable(&f.sprite) {
            bad.push(format!("{}: {e}", w.odf));
        }
    }
    if let Err(e) = usable(stock_flares::IMPACT_SPRITE) {
        bad.push(format!("the impact flare: {e}"));
    }
    eprintln!("{with_flare} shot definitions with a muzzle flare");
    assert!(with_flare > 0, "no definition has a flare");
    assert!(bad.is_empty(), "{bad:#?}");
}

// ---------------------------------------------------------------------------------------------
// On the GPU
// ---------------------------------------------------------------------------------------------

/// With `STA64_GPU_TESTS=1` and a hardware adapter: a shot drawn with the material over a bright
/// background adds to it, the texture's alpha darkens nothing, and an opaque surface in front of it
/// hides it (depth test on). Prints SKIP and passes otherwise.
#[test]
fn gpu_a_shot_adds_to_the_frame_and_the_texture_alpha_does_not_darken_it() {
    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::window::ExitCondition;
    use std::sync::Mutex;

    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to draw a stock shot");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
            // The global logger is set once per process; the tests that read shader errors from it need it for themselves.
            .disable::<bevy::log::LogPlugin>()
            .disable::<bevy::winit::WinitPlugin>(),
    );
    if let Some(info) = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        && crate::bench::frames::is_software_adapter(info.device_type, &info.name)
    {
        eprintln!("SKIP: only a software adapter is available ({})", info.name);
        return;
    }
    app.add_plugins(StockShotsMaterialPlugin).insert_resource(ClearColor(Color::BLACK));
    // A 2 x 1 texture, every texel the same grey; the left one opaque, the right one with alpha 0.
    let grey = 64u8;
    let texture = Image::new(
        Extent3d { width: 2, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![grey, grey, grey, 255, grey, grey, grey, 0],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    let texture = app.world_mut().resource_mut::<Assets<Image>>().add(texture);
    let mut target = Image::new_target_texture(64, 64, TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    // The shot: a ribbon 2 wide from x = -1 to 1 at z = 1 (nearer the camera than the background at z = 0).
    let mut g = Geo::default();
    g.quad([Vec3::new(-2.0, 2.0, 1.0), Vec3::new(2.0, 2.0, 1.0), Vec3::new(2.0, -2.0, 1.0), Vec3::new(-2.0, -2.0, 1.0)], 1.0, Vec3::ONE);
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, g.pos.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, g.uv.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, g.col.clone())
        .with_inserted_indices(Indices::U32(g.idx.clone()));
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let shot = app.world_mut().resource_mut::<Assets<StockShotMaterial>>().add(StockShotMaterial {
        params: ShotParams { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), flags: Vec4::ZERO, tint: Vec4::ONE },
        texture,
    });
    app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(shot), Transform::IDENTITY));
    // A background of linear 0.2 grey, and an opaque green surface in front of the shot over the upper half.
    let flat = |c: Color| StandardMaterial { base_color: c, unlit: true, ..default() };
    let bg = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(flat(Color::linear_rgb(0.2, 0.2, 0.2)));
    let green = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(flat(Color::linear_rgb(0.0, 0.5, 0.0)));
    let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(40.0, 40.0));
    let upper = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(40.0, 20.0));
    app.world_mut().spawn((Mesh3d(plane), MeshMaterial3d(bg), Transform::IDENTITY));
    app.world_mut().spawn((Mesh3d(upper), MeshMaterial3d(green), Transform::from_xyz(0.0, 10.0, 2.0)));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.finish();
    app.cleanup();
    // The shader is an asset loaded in the background: give it frames before the capture.
    for _ in 0..90 {
        app.update();
    }
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let out = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| out.lock().unwrap().push(shot.image.clone()));
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let frame = frames.lock().unwrap().first().cloned().expect("no capture");
    let rgba = frame.try_into_dynamic().unwrap().to_rgba8();
    let px = |x: u32, y: u32| rgba.get_pixel(x, y).0;
    let srgb = |lin: f32| (Color::from(LinearRgba::new(lin, lin, lin, 1.0)).to_srgba().red * 255.0).round() as i32;
    let close = |got: u8, want: i32| (i32::from(got) - want).abs() <= 4;
    // The camera is 10 away, the shot 9: about 8.6 pixels a unit, so the shot covers pixels 15 to 49. Left and right halves of the shot, below the middle.
    let sum = 0.2 + f32::from(grey) / 255.0;
    let (left, right, outside) = (px(24, 41), px(40, 41), px(5, 41));
    eprintln!("left {left:?}, right {right:?}, outside {outside:?}, hidden {:?}", px(24, 23));
    assert!(close(outside[0], srgb(0.2)), "the background {outside:?}");
    assert!(close(left[0], srgb(sum)), "the opaque texel adds: {left:?} against {}", srgb(sum));
    assert!(close(right[0], srgb(sum)), "a texel with alpha 0 adds as much, it does not darken: {right:?}");
    // Behind the green surface the shot is not seen at all (red stays 0 where the shot would add grey).
    let hidden = px(24, 23);
    assert!(hidden[0] <= 3 && hidden[2] <= 3, "the shot shows through an opaque surface in front: {hidden:?}");
}

/// A stock shot as the update system leaves it, for the layout and the frame tests.
fn authored_shot(path: ShotPath, geom: Geom, radius: f32, sprite: &str) -> StockShot {
    StockShot {
        path,
        shooter: EntityId(1),
        weapon: ContentKey(0),
        target: Target::Entity(EntityId(2)),
        fired: 0,
        life: 100.0,
        sprite: sprite.to_owned(),
        radius,
        length: 0.0,
        colour: Vec3::ONE,
        travel: 0.0,
        range: 1000.0,
        ignore_shield: false,
        miss: None,
        node: None,
        target_node: None,
        muzzle: Vec3::ZERO,
        per_tick: 0.0,
        to: Vec3::ZERO,
        last_body: None,
        last_end: Vec3::ZERO,
        shield_end: Vec3::ZERO,
        pinned: false,
        hidden_shooter: false,
        muzzle_flare: None,
        impact_flare: None,
        arrived: false,
        flown: false,
        bound: None,
        start_distance: 0.0,
        progress: 0.0,
        muzzle_offset: Vec3::ZERO,
        aim_offset: Vec3::ZERO,
        unbound_fallback: false,
        end_height: None,
        sim_lost: false,
        geom,
    }
}

/// With `STA64_GPU_TESTS=1` and a hardware adapter: an AUTHORED atlas (one flat colour per flipbook frame, and a
/// white marker in the top left corner of every cell) drawn through the shot layout and material that
/// `draw_shots` uses, for a ribbon, a streak and a billboard, read back from the frame:
/// - the cell: only the frame's colour (and the marker) shows, none of its neighbours;
/// - the texture's start: at the beam's muzzle end for the ribbon, at the head for the streak;
/// - no flipped axis: the marker is at the texture's top left, which a ribbon puts on its + side (the side
///   the eye-relative cross product gives), the billboard at the screen's top left;
/// - the frame order at a given game time: the colour at four times a quarter of the flipbook apart.
///
/// Prints SKIP and passes otherwise.
#[test]
fn gpu_the_atlas_is_drawn_in_its_cell_the_right_way_round_and_in_frame_order() {
    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::image::ImageSampler;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::window::ExitCondition;
    use std::sync::Mutex;

    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to draw the atlas test");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
            // The global logger is set once per process; the tests that read shader errors from it need it for themselves.
            .disable::<bevy::log::LogPlugin>()
            .disable::<bevy::winit::WinitPlugin>(),
    );
    if let Some(info) = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>() {
        eprintln!("adapter: {} ({:?})", info.name, info.device_type);
        if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
            eprintln!("SKIP: only a software adapter is available ({})", info.name);
            return;
        }
    }
    app.add_plugins(StockShotsMaterialPlugin).insert_resource(ClearColor(Color::BLACK));

    // The atlas: a 2 x 2 grid of 32 x 32 cells, frame k at column k % 2, row k / 2 (frames 2 and 3 are in the second
    // row). Cell k is one flat colour, its top left 8 x 8 texels are white.
    let frame_colour = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
    let mut data = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            let c = if x % 32 < 8 && y % 32 < 8 { [255, 255, 255] } else { frame_colour[x / 32 + 2 * (y / 32)] };
            data.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut atlas = Image::new(Extent3d { width: 64, height: 64, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    atlas.sampler = ImageSampler::nearest();
    let texture = app.world_mut().resource_mut::<Assets<Image>>().add(atlas);

    // Its sprite table and content, resolved the way the game resolves them: one sprite, four frames in a second, row by row.
    let table = SpriteTable::parse(b"sprite_table\n@referenceWidth=64\n@referenceHeight=64\n@tmaterial=additive\natlas tex_atlas 0 0 32 32 @anim=flip4\n@animation flip4\noffset 4 1.0 step\n@auto=square\n@keyframes\n");
    let db = ContentBuilder::default().weapon(weapon("w_atlas", "phaser", WeaponKind::Beam, ShotLook { life_span: Some(1.0), ..look("atlas") })).build().unwrap();
    let sprites = ShotSprites::build(&table, &db);
    let sprite = sprites.get("atlas").expect("the authored sprite resolves").clone();

    // The camera 1 pixel per world unit at z = 0: a 128 x 128 target, a 45 degree view, the eye at 64 / tan(22.5 degrees).
    let eye = Vec3::new(0.0, 0.0, 64.0 / (std::f32::consts::FRAC_PI_8).tan());
    // Ribbon: from the muzzle (-60, -20) to (0, -20), 16 wide. Streak: head (55, 40), tail (15, 40), 24 wide. Billboard: centre (-40, 30), half 12.
    let ribbon = authored_shot(ShotPath::Ribbon, Geom::Segment { a: Vec3::new(-60.0, -20.0, 0.0), b: Vec3::new(0.0, -20.0, 0.0) }, 16.0, "atlas");
    let streak = authored_shot(ShotPath::Streak, Geom::Segment { a: Vec3::new(55.0, 40.0, 0.0), b: Vec3::new(15.0, 40.0, 0.0) }, 24.0, "atlas");
    let billboard = authored_shot(ShotPath::Sprite, Geom::Billboard { c: Vec3::new(-40.0, 30.0, 0.0) }, 12.0, "atlas");
    let shots = [ribbon, streak, billboard];
    let g = &lay_out(shots.iter(), std::iter::empty(), &sprites, eye, Vec3::X, Vec3::Y)["atlas"];
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, g.pos.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; g.pos.len()])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, g.uv.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, g.col.clone())
        .with_inserted_indices(Indices::U32(g.idx.clone()));
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let params_at = |t: f32| ShotParams { rect: Vec4::from(sprite.rect_at(t)), flags: Vec4::new(f32::from(sprite.tile_u), 0.0, 0.0, 0.0), tint: Vec4::ONE };
    let material = app.world_mut().resource_mut::<Assets<StockShotMaterial>>().add(StockShotMaterial { params: params_at(0.0), texture });
    app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(material.clone()), Transform::IDENTITY));
    let mut target = Image::new_target_texture(128, 128, TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.finish();
    app.cleanup();
    for _ in 0..90 {
        app.update();
    }
    let capture = |app: &mut App| {
        let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
        let out = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| out.lock().unwrap().push(shot.image.clone()));
        for _ in 0..60 {
            app.update();
            if !frames.lock().unwrap().is_empty() {
                break;
            }
        }
        let frame = frames.lock().unwrap().first().cloned().expect("no capture");
        frame.try_into_dynamic().unwrap().to_rgba8()
    };
    // Game time (seconds) at the middle of each frame of the flipbook, whose first channel is a second long.
    for (k, t) in [0.125f32, 0.375, 0.625, 0.875].into_iter().enumerate() {
        app.world_mut().resource_mut::<Assets<StockShotMaterial>>().get_mut(&material).unwrap().params = params_at(t);
        for _ in 0..3 {
            app.update();
        }
        let rgba = capture(&mut app);
        let px = |x: u32, y: u32| {
            let p = rgba.get_pixel(x, y).0;
            [p[0], p[1], p[2]]
        };
        let (want, white, black) = (frame_colour[k], [255u8, 255, 255], [0u8, 0, 0]);
        let near = |got: [u8; 3], want: [u8; 3]| got.iter().zip(want).all(|(g, w)| (i32::from(*g) - i32::from(w)).abs() <= 12);
        let check = |what: &str, x: u32, y: u32, want: [u8; 3]| {
            let got = px(x, y);
            assert!(near(got, want), "frame {k} (game time {t}): {what} at ({x}, {y}) is {got:?}, expected {want:?}");
        };
        // Billboard, a 24 px square at x 12..36, y 22..46: the marker at its top left (a quarter of a side, 6 px), nothing else white.
        check("billboard marker", 14, 24, white);
        check("billboard, marker's right", 20, 24, want);
        check("billboard, marker's below", 14, 30, want);
        check("billboard top right", 33, 24, want);
        check("billboard bottom left", 14, 43, want);
        check("billboard bottom right", 33, 43, want);
        check("billboard middle", 24, 34, want);
        for (x, y) in [(8, 34), (40, 34), (24, 18), (24, 50)] {
            check("outside the billboard", x, y, black);
        }
        // Ribbon, x 4..64 (the texture stretched once from the muzzle at the left to the end), rows 76..92, its + side below
        // (the cross product of the muzzle's and the end's directions from the eye points down): the marker at the muzzle's bottom.
        check("ribbon marker (muzzle end, + side)", 8, 90, white);
        check("ribbon, muzzle end, - side", 8, 79, want);
        check("ribbon, beside the marker", 24, 90, want);
        check("ribbon, far end, - side", 58, 79, want);
        check("ribbon, far end, + side", 58, 90, want);
        check("ribbon middle", 30, 84, want);
        check("beyond the ribbon's muzzle end", 1, 84, black);
        check("beyond the ribbon's far end", 68, 84, black);
        // Streak, x 79..119 with its head at the right, rows 12..36, its + side above: the marker at the head's top.
        check("streak marker (head, + side)", 115, 14, white);
        check("streak, head, - side", 115, 30, want);
        check("streak, tail, + side", 85, 14, want);
        check("streak, tail, - side", 85, 30, want);
        check("streak middle", 100, 24, want);
        check("beyond the streak's head", 123, 24, black);
        check("beyond the streak's tail", 75, 24, black);
    }
}

// ---------------------------------------------------------------------------------------------
// Flown torpedoes (#1068): drawn where the view's shot row says
// ---------------------------------------------------------------------------------------------

/// A path that leaves the shooter sideways and comes back to the target: not the straight line.
fn bent(k: i64) -> (f64, f64) {
    let k = k as f64;
    (100.0 * (60.0 - k) / 60.0 * (k.min(10.0) / 10.0), 10.0 * k)
}

fn drawn(app: &mut App) -> Vec<Vec3> {
    shots(app).iter().filter_map(|s| if let Geom::Billboard { c } = s.geom { Some(c) } else { None }).collect()
}

#[test]
fn a_flown_torpedo_is_drawn_where_the_views_row_puts_it_not_on_the_line_to_its_target() {
    let setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 4, 64, bent).fire(4, "w_torp", 2);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let (mut off_line, mut frames) = (0.0f32, 0);
    for _ in 0..60 {
        app.update();
        for s in shots(&mut app) {
            if let Geom::Billboard { c } = s.geom {
                frames += 1;
                // the straight line from the muzzle to the target's centre
                let dir = (Vec3::new(0.0, 0.0, -600.0) - s.muzzle).normalize();
                off_line = off_line.max((c - s.muzzle).cross(dir).length());
            }
        }
    }
    assert!(frames > 30, "drawn through the flight: {frames} frames");
    assert!(off_line > 40.0, "the path bends away from the straight line by {off_line}");
}

#[test]
fn a_flown_torpedo_the_sim_no_longer_holds_vanishes_with_no_hit_effect() {
    // The rows stop at tick 40 and no hit comes: the shot expired.
    let setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 4, 40, bent).fire(4, "w_torp", 2);
    let mut app = setup.app(Some(RenderProfile::Stock));
    for i in 0..60 {
        app.update();
        if i >= 42 {
            assert!(shots(&mut app).is_empty(), "frame {i}: gone");
        }
    }
    assert!(app.world().resource::<super::super::stock_hull_hits::HullHitPoints>().0.is_empty(), "no hull hit spark for an expired shot");
}

#[test]
fn a_flown_torpedo_whose_target_is_gone_flies_on_where_the_row_goes() {
    let mut setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 4, 64, bent).fire(4, "w_torp", 2);
    setup.target_until = Some(20);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let mut after = Vec::new();
    for i in 0..60 {
        app.update();
        if i > 25 {
            after.extend(drawn(&mut app));
        }
    }
    assert!(after.len() > 20, "still drawn after the target went: {}", after.len());
    assert!(after.windows(2).all(|w| w[0].distance(w[1]) < 40.0), "a continuous flight");
}

#[test]
fn a_flown_torpedos_last_place_is_within_one_step_of_the_hull_node_its_spark_sits_on() {
    let setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 4, 64, bent).fire(4, "w_torp", 2);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let node = Vec3::new(5.0, 0.0, 0.0);
    let mut last = None;
    for i in 0..80 {
        app.update();
        if i == 0 {
            model(&mut app, 2, &[("hull_a", node), ("hull_b", node)]);
        }
        if let Some(c) = drawn(&mut app).last() {
            last = Some(*c);
        }
    }
    // The target stands at (0, 0, -600) in render space; the shot's last row is one step (10) short of its centre.
    let aim = Vec3::new(0.0, 0.0, -600.0) + node;
    let last = last.expect("drawn");
    assert!(last.distance(aim) <= 12.0, "last drawn place {last} against the node {aim}");
}

#[test]
fn a_flown_torpedo_without_a_row_in_the_view_is_drawn_as_without_the_switch() {
    // The content has the switch on but the view has no rows: after the slack the old line is drawn.
    let mut setup = Setup::new(ShotCfg::default()).fire(4, "w_torp", 2);
    setup.db = content_with(ShotCfg::default(), false, true);
    let mut app = setup.app(Some(RenderProfile::Stock));
    for _ in 0..20 {
        app.update();
    }
    let s = shots(&mut app);
    assert_eq!(s.len(), 1);
    assert!(matches!(s[0].geom, Geom::Billboard { .. }), "drawn along the line");
}

#[test]
fn a_hit_takes_the_torpedo_whose_row_is_gone_not_the_earliest_fired() {
    // Two torpedoes at one target: the first (shot 1) is slow, the second (shot 2) arrives first and hits at tick 50.
    fn slow(k: i64) -> (f64, f64) {
        (0.0, 5.0 * k as f64)
    }
    fn fast(k: i64) -> (f64, f64) {
        (0.0, 20.0 * k as f64)
    }
    let setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 4, 120, slow).also_flown(6, 30, 2, fast).fire(4, "w_torp", 2).fire(6, "w_torp", 2).hit(31, "w_torp");
    let mut app = setup.app(Some(RenderProfile::Stock));
    for _ in 0..24 {
        app.update();
    }
    assert_eq!(shots(&mut app).len(), 2, "both in flight");
    for _ in 0..30 {
        app.update();
    }
    let left = shots(&mut app);
    assert_eq!(left.len(), 1, "the one that hit is gone");
    assert_eq!(left[0].fired, 4, "the slow, earlier torpedo flies on; the second one hit");
}

#[test]
fn a_flown_torpedo_ends_on_the_node_in_height_too_when_shooter_and_target_differ() {
    let mut setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 4, 64, bent).fire(4, "w_torp", 2);
    setup.target_y = 100;
    let mut app = setup.app(Some(RenderProfile::Stock));
    let node = Vec3::new(5.0, 0.0, 0.0);
    let mut last = None;
    for i in 0..80 {
        app.update();
        if i == 0 {
            model(&mut app, 2, &[("hull_a", node), ("hull_b", node)]);
        }
        if let Some(c) = drawn(&mut app).last() {
            last = Some(*c);
        }
    }
    let aim = Vec3::new(0.0, 100.0, -600.0) + node;
    let last = last.expect("drawn");
    assert!(last.distance(aim) <= 14.0, "last drawn place {last} against the node {aim}");
    assert!((last.y - 100.0).abs() < 4.0, "the height follows the target's: {}", last.y);
}

#[test]
fn a_flown_torpedo_waits_for_its_row_hidden_and_then_draws_from_the_row() {
    // The rows begin three ticks after the shot: until then it is not drawn (no jump from the old line).
    let setup = Setup::new(ShotCfg::default()).flight(ShotCfg::default(), 6, 64, bent).fire(4, "w_torp", 2);
    let mut app = setup.app(Some(RenderProfile::Stock));
    let mut seen = Vec::new();
    for _ in 0..12 {
        app.update();
        seen.push(shots(&mut app).iter().any(|s| s.geom != Geom::None));
    }
    let first = seen.iter().position(|d| *d).expect("drawn once bound");
    assert!(seen[..first].iter().all(|d| !d), "hidden before it binds");
    assert!(first >= 1);
}
