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
    SpriteTable::parse(TABLE.as_bytes())
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
    weapon("w_phaser", "phaser", WeaponKind::Beam, ShotLook { life_span: Some(1.0), travel_time_fraction: Some(0.2), ..look("beamspr") })
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
fn the_shader_validates_and_adds_no_alpha_or_brightness() {
    let src = include_str!("stock_shots.wesl");
    super::super::validate_wgsl(src);
    assert!(src.contains("vec4<f32>(texel.rgb * in.color.rgb, 0.0)"), "the output is texture times vertex colour, alpha 0");
}

#[test]
fn stock_post_fx_draws_no_bloom_by_default() {
    assert!(!crate::render::post::PostFx::default().bloom, "the shots would be bloomed with it");
}

// ---------------------------------------------------------------------------------------------
// In a world
// ---------------------------------------------------------------------------------------------

fn content(rules: ShotCfg) -> ContentDb {
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
    ContentBuilder::default()
        .weapon(phaser())
        .weapon(pulse())
        .weapon(torpedo())
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
}

impl Shielded {
    fn patch(&self, view: Arc<WorldView>) -> Arc<WorldView> {
        if !self.shields {
            return view;
        }
        let mut v = (*view).clone();
        for u in v.units.iter_mut().filter(|u| u.id.0 >= 2) {
            u.shields = [[Fixed::from_int(50), Fixed::from_int(50)]; 4];
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
    /// Unit 2's def and a rail point (the target); `until` ends it.
    target_def: &'static str,
    target_z: i32,
    target_until: Option<u64>,
    shooter_until: Option<u64>,
    events: Vec<(u64, &'static str, u8, u64)>,
}

impl Setup {
    fn new(rules: ShotCfg) -> Setup {
        Setup { db: content(rules), shields: false, target_def: "t_target", target_z: 600, target_until: None, shooter_until: None, events: Vec::new() }
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
            ev += &format!("{{ at: {at}, event: {{ Fired: {{ shooter: 1, weapon: {k}, hardpoint: {hp}, target: {{ Entity: {target} }} }} }} }},");
        }
        let text = format!(
            "{{ units: [
                {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}, {{ at: 300, pos: {{ x: 60, y: 0, z: 0 }} }}]{} }},
                {{ id: 2, def: \"{}\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: {} }} }}]{} }},
            ], events: [{ev}] }}",
            until(self.shooter_until),
            self.target_def,
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
            .insert_resource(SimLink::new(Box::new(Shielded { inner, shields: self.shields }), Box::new(LocalRouter::new()), PlayerId(0)));
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
fn a_beam_ends_when_its_owner_is_gone_and_when_the_target_is_beyond_root_two_times_the_range() {
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
    // Range 1000: a target at 1500 is in (1500^2 < 2 * 1000^2), one at 1500 for a 1000-range weapon... a target at 1800 is out.
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
    assert_eq!(beams(1300), 1);
    assert_eq!(beams(1500), 0);
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
    *app.world_mut().resource_mut::<SimLink>() = SimLink::new(Box::new(Shielded { inner, shields: false }), Box::new(LocalRouter::new()), PlayerId(0));
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
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
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
        params: ShotParams { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), flags: Vec4::ZERO },
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
