//! Tests of the Stock look's shield hit. The definitions, sprite tables, models and textures are authored here
//! (no install); time is stepped by hand (one sim tick per frame), never read from the clock of the machine.
use super::*;
use crate::app::configure_sets;
use crate::render::fx::impacts::{ImpactFx, ImpactFxPlugin, ImpactKind};
use crate::render::profile::RenderProfile;
use crate::sim_link::SimLinkPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_assets::a2fmt::Matrix34;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_assets::model::{Group, LightingMaterial, Maps, Mesh as ModelMesh, Node, SourceFormat};
use sta_content::{ContentBuilder, ContentDb, ShotLook, UnitDef, WeaponKind};
use sta_sim::command::LocalRouter;
use sta_sim::fake::ScriptedSource;
use sta_sim::{Fixed, PlayerId, TickOutput, WorldSource, WorldView};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------------------------

#[test]
fn the_colour_follows_the_shield_fraction_with_the_gain() {
    // Full shields are green, empty ones red, in between the two; the gain multiplies all of it.
    assert_eq!(cap_colour(1.0), Vec3::new(0.0, GAIN, 0.0));
    assert_eq!(cap_colour(0.0), Vec3::new(GAIN, 0.0, 0.0));
    assert_eq!(cap_colour(0.5), Vec3::new(0.5 * GAIN, 0.5 * GAIN, 0.0));
    let c = cap_colour(0.1);
    assert!((c - GAIN * Vec3::new(0.9, 0.1, 0.0)).length() < 1e-6);
    // Out of range is clamped, never negative.
    assert_eq!(cap_colour(1.5), cap_colour(1.0));
    assert_eq!(cap_colour(-1.0), cap_colour(0.0));
}

#[test]
fn a_beams_level_is_what_the_view_shows_plus_what_its_hit_took() {
    // After the tick the pool shows 30 of 100; the beam's hit took 20 from it: the level before the beam is 0.5.
    assert_eq!(beam_fraction_before(30.0, 20.0, 100.0), 0.5);
    // A projectile's is the pool after its damage.
    assert_eq!(shield_fraction(30.0, 100.0), 0.3);
    // Never above full, never with a pool of no size.
    assert_eq!(beam_fraction_before(90.0, 40.0, 100.0), 1.0);
    assert_eq!(shield_fraction(5.0, 0.0), 0.0);
    // A hit that took nothing leaves the level as it is.
    assert_eq!(beam_fraction_before(30.0, 0.0, 100.0), 0.3);
}

#[test]
fn a_caps_life_is_the_ordnances_shield_duration_else_the_default_in_ticks() {
    let mut def = WeaponDef::new("w", "phaser", WeaponKind::Beam);
    assert_eq!(cap_life_ticks(&def), DEFAULT_DURATION * TICK_HZ as f32);
    def.base.shield_duration = Some(Fixed::from_int(3));
    assert_eq!(cap_life_ticks(&def), 3.0 * TICK_HZ as f32);
    def.base.shield_duration = Some(Fixed::from_int(1));
    assert_eq!(cap_life_ticks(&def), TICK_HZ as f32);
}

#[test]
fn a_cap_is_cut_at_the_exact_tick_with_no_fade() {
    // A three second cap: live up to the last instant before its end, gone from the tick it ends.
    let life = 3.0 * TICK_HZ as f32;
    let born = 17.25;
    assert!(cap_live(born, born, life));
    assert!(cap_live(born + f64::from(life) - 0.001, born, life));
    assert!(!cap_live(born + f64::from(life), born, life));
    assert!(!cap_live(born + f64::from(life) + 40.0, born, life));
}

fn flipbook_table() -> SpriteTable {
    // Sixteen frames in half a second, a four by four sheet read row by row.
    SpriteTable::parse(b"sprite_table\n@referenceWidth=64\n@referenceHeight=64\n@animation cells\noffset 16 0.5 step\n@auto=square\n@keyframes\n")
}

#[test]
fn the_flipbook_cell_comes_from_the_game_time_the_same_for_every_cap() {
    let t = flipbook_table();
    let cell = |game: f32| flipbook_rect(&t, Some(("cells", 0.0)), game);
    // Sixteen frames in half a second, by the helper's rule: the slot is floor(time * 16 / (length + 0.05)), so a slot is
    // 0.55 / 16 long and the last cell is not reached within the half second.
    let frame = 0.55 / 16.0;
    // Frame k at the middle of its slot: column k % 4, row k / 4, a quarter of the sheet each.
    for k in 0..15u32 {
        let r = cell((k as f32 + 0.5) * frame);
        assert_eq!((r.x, r.y, r.z, r.w), ((k % 4) as f32 * 0.25, (k / 4) as f32 * 0.25, 1.0, 1.0), "frame {k}");
    }
    // Just before the loop ends it still shows the cell before the last one.
    assert_eq!(cell(0.4999).x, 0.5);
    assert_eq!(cell(0.4999).y, 0.75);
    // It loops: half a second on is the first cell again; a time long after a hit's birth is whatever the clock says.
    assert_eq!(cell(0.5 + 0.5 * frame), cell(0.5 * frame));
    assert_eq!(cell(0.5 * 240.0 + 5.5 * frame), cell(5.5 * frame));
    // The model's own time offset shifts it; no animation, or an unknown one, leaves the model's coordinates.
    assert_eq!(flipbook_rect(&t, Some(("cells", 3.0 * frame)), 0.5 * frame), cell(3.5 * frame));
    assert_eq!(flipbook_rect(&t, None, 1.0), Vec4::new(0.0, 0.0, 1.0, 1.0));
    assert_eq!(flipbook_rect(&t, Some(("nosuch", 0.0)), 1.0), Vec4::new(0.0, 0.0, 1.0, 1.0));
}

#[test]
fn the_flipbook_stands_while_the_game_is_paused() {
    // The cell is a function of the game clock, which does not run while paused; the wall clock does not enter.
    let t = flipbook_table();
    let mut clock = AnimClock::new(crate::clock::ClockMode::Wall, 0);
    let frame = |clock: &mut AnimClock, dt: f64| {
        for _ in 0..clock.begin_frame(dt) {
            clock.ticked();
        }
        clock.end_frame();
    };
    for _ in 0..7 {
        frame(&mut clock, 0.05);
    }
    let wall = clock.wall();
    clock.set_paused(true);
    let before = flipbook_rect(&t, Some(("cells", 0.0)), clock.now() as f32);
    for _ in 0..40 {
        frame(&mut clock, 0.05);
        assert_eq!(flipbook_rect(&t, Some(("cells", 0.0)), clock.now() as f32), before, "the cell moved while paused");
    }
    assert!(clock.wall() > wall, "the wall clock ran on");
}

// ---------------------------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------------------------

/// The cap's frame as one matrix: the ellipsoid's, then the turn of the pole.
fn world(frame: &Transform, turn: &Transform) -> bevy::math::Affine3A {
    frame.compute_affine() * turn.compute_affine()
}

#[test]
fn the_pole_faces_the_hit_on_the_ellipsoid_and_stands_a_database_radius_scale_outside_it() {
    let centre = Vec3::new(10.0, -4.0, 250.0);
    let (rot, axes) = crate::render::fx::impacts::shield_axes(Vec3::new(0.6, 0.0, -0.8), 20.0);
    let r_db = 4.5;
    for dir in [Vec3::new(1.0, 0.0, 0.0), Vec3::new(-0.3, 0.8, 0.5).normalize(), Vec3::Y, Vec3::NEG_Z, Vec3::Z] {
        let (frame, turn) = cap_frame(centre, rot, axes, r_db, dir);
        let pole = world(&frame, &turn).transform_point3(POLE * 5.0);
        // The point of the ellipsoid in that direction of its unit sphere, and the pole at 5 / r_db times it.
        let hit = centre + rot * (axes * dir);
        assert!((pole - centre - (hit - centre) * (5.0 / r_db)).length() < 1e-3, "{dir}: {pole} vs {hit}");
        // Seen from the centre the pole is where the hit is.
        assert!(dir_toward(rot, axes, pole - centre).distance(dir) < 1e-4, "{dir}");
    }
}

#[test]
fn the_cap_stretches_with_an_anisotropic_ellipsoid_by_its_axes_over_the_database_radius() {
    let (rot, axes) = (Quat::IDENTITY, Vec3::new(30.0, 10.0, 60.0));
    let (frame, turn) = cap_frame(Vec3::ZERO, rot, axes, 4.0, POLE);
    let m = world(&frame, &turn);
    // With the pole already at the pole, a model point one unit along each axis is stretched by that axis over the radius.
    assert!((m.transform_point3(Vec3::X) - Vec3::new(7.5, 0.0, 0.0)).length() < 1e-4);
    assert!((m.transform_point3(Vec3::Y) - Vec3::new(0.0, 2.5, 0.0)).length() < 1e-4);
    assert!((m.transform_point3(Vec3::NEG_Z) - Vec3::new(0.0, 0.0, -15.0)).length() < 1e-4);
    // The stretch is in the target's frame, after the turn of the pole: turned to the +x pole the cap's own z reaches
    // along the ellipsoid's x axis, its length is the x axis's.
    let (frame, turn) = cap_frame(Vec3::ZERO, rot, axes, 4.0, Vec3::X);
    let m = world(&frame, &turn);
    assert!((m.transform_point3(POLE) - Vec3::new(7.5, 0.0, 0.0)).length() < 1e-4);
    // The ellipsoid follows the target's heading.
    let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let (frame, turn) = cap_frame(Vec3::ZERO, turned, axes, 4.0, POLE);
    assert!((world(&frame, &turn).transform_point3(Vec3::X) - turned * Vec3::new(7.5, 0.0, 0.0)).length() < 1e-4);
}

// ---------------------------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------------------------

const RIM: usize = 8;

/// An authored spherical cap: radius 5, from z 2.5 to the pole at z 5, `RIM` vertices at the rim.
fn cap_mesh(rim: usize) -> ModelMesh {
    let rim_r = 5.0 * (std::f32::consts::FRAC_PI_3).sin();
    let mut positions = vec![[0.0, 0.0, 5.0]];
    for k in 0..rim {
        let a = k as f32 / rim as f32 * std::f32::consts::TAU;
        positions.push([rim_r * a.cos(), rim_r * a.sin(), 2.5]);
    }
    let uvs = vec![[0.0, 0.0], [0.25, 0.0], [0.25, 0.25], [0.0, 0.25]];
    let faces = (0..rim).map(|k| [(0, 0), (1 + k as u32, 1), (1 + ((k + 1) % rim) as u32, 2)]).collect();
    ModelMesh { positions, uvs, groups: vec![Group { material: "m".into(), blend: "additive".into(), maps: Maps { diffuse: Some("authored_cap".into()), ..Default::default() }, faces }], two_sided: true }
}

fn node(name: &str, parent: Option<usize>, children: Vec<usize>, kind: NodeKind) -> Node {
    Node { name: name.into(), parent, children, transform: Matrix34::IDENTITY, kind }
}

/// Three levels of detail under a `geometry` node: the finest has `RIM` rim vertices, the coarser ones fewer.
fn authored_model() -> Model {
    Model {
        format: SourceFormat::Sod,
        version: 1.7,
        materials: vec![LightingMaterial { name: "m".into(), ambient: [1.0; 3], diffuse: [1.0; 3], specular: [0.0; 3], power: 1.0, model: 0, self_illuminating: false }],
        nodes: vec![
            node("geometry", None, vec![1, 2, 3], NodeKind::Null),
            node("lod0_cap", Some(0), vec![], NodeKind::Mesh(cap_mesh(RIM))),
            node("lod1_cap", Some(0), vec![], NodeKind::Mesh(cap_mesh(4))),
            node("lod2_cap", Some(0), vec![], NodeKind::Mesh(cap_mesh(3))),
        ],
        channels: Vec::new(),
        flipbooks: vec![("lod0_cap".into(), "cells".into(), 0.0)],
        warnings: Vec::new(),
        pbr: None,
    }
}

#[test]
fn the_database_radius_is_the_farthest_vertex_from_the_centre_of_the_bounding_box() {
    let m = authored_model();
    // The box is x, y within the rim radius and z 2.5 to 5: its centre is at z 3.75; the rim is the farthest.
    let rim = 5.0 * (std::f32::consts::FRAC_PI_3).sin();
    let want = (rim * rim + 1.25 * 1.25).sqrt();
    let got = database_radius(&m).unwrap();
    assert!((got - want).abs() < 1e-3, "{got} vs {want}");
    // Every vertex of every mesh is within it of the box centre.
    let (min, max) = m.bounds().unwrap();
    let c = (Vec3::from(min) + Vec3::from(max)) * 0.5;
    for (i, n) in m.nodes.iter().enumerate() {
        if let Some(mesh) = n.mesh() {
            for p in &mesh.positions {
                assert!((Vec3::from(m.model_transform(i).transform_point(*p)) - c).length() <= got + 1e-4);
            }
        }
    }
    // No mesh, no radius.
    assert!(database_radius(&Model { nodes: vec![node("n", None, vec![], NodeKind::Null)], ..authored_model() }).is_none());
}

#[test]
fn the_cap_is_drawn_from_the_finest_level_of_detail_only_in_render_space_with_its_texture_and_flipbook() {
    let geo = cap_geometry(&authored_model()).unwrap();
    // One level: the pole and the rim, one triangle per rim edge.
    let positions = geo.mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap().to_vec();
    assert_eq!(geo.mesh.indices().unwrap().len(), 3 * RIM);
    // A coarser level would add its own vertices: this one has the pole and, per rim vertex, one corner for each of its two triangles.
    assert_eq!(positions.len(), 1 + 2 * RIM);
    // The model's z is mirrored with the rest of the render space: the pole is at -z, which is the pole's axis.
    assert_eq!(Vec3::from(positions[0]), POLE * 5.0);
    assert!(positions[1..].iter().all(|p| p[2] == -2.5));
    assert_eq!(geo.texture.as_deref(), Some("authored_cap"));
    assert_eq!(geo.flipbook, Some(("cells".to_owned(), 0.0)));
    // The radius is the whole model's, the coarser levels included (they are meshes of the model).
    assert!((geo.r_db - database_radius(&authored_model()).unwrap()).abs() < 1e-6);
    // The vertex colour is white, so the tint alone colours the cap.
    let col = geo.mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap();
    assert_eq!(col.len(), positions.len());
}

// ---------------------------------------------------------------------------------------------
// In a world
// ---------------------------------------------------------------------------------------------

const TABLE: &str = "sprite_table\n@referenceWidth=128\n@referenceHeight=128\n@tmaterial=additive\nbeamspr tex_beam 0 0 32 32\ntorpspr tex_torp 0 32 32 32\nexplosion1 tex_impact 0 64 32 32\n";

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

/// A phaser with a one-second life, a fifth of it to grow, and the given `shieldDuration` seconds.
fn phaser(duration: i32) -> WeaponDef {
    let mut w = weapon("w_phaser", "phaser", WeaponKind::Beam, ShotLook { life_span: Some(1.0), travel_time_fraction: Some(0.2), ..look("beamspr") });
    w.base.shield_duration = Some(Fixed::from_int(duration));
    w
}

fn content(duration: i32) -> ContentDb {
    let mut ign = weapon("w_torp_ign", "photontorpedo", WeaponKind::Projectile, look("torpspr"));
    ign.base.ignore_shield = Some(true);
    let mut ign_beam = phaser(duration);
    ign_beam.odf = "w_phaser_ign".into();
    ign_beam.base.ignore_shield = Some(true);
    ContentBuilder::default()
        .weapon(phaser(duration))
        .weapon(ign_beam)
        .weapon(weapon("w_torp", "photontorpedo", WeaponKind::Projectile, look("torpspr")))
        .weapon(ign)
        // A beam of a class with no stock drawing, and a projectile of one.
        .weapon(weapon("w_tractor", "tractor_bm", WeaponKind::Beam, ShotLook { life_span: Some(1.0), ..look("beamspr") }))
        .weapon(weapon("w_odd", "somethingnew", WeaponKind::Projectile, look("torpspr")))
        .unit(UnitDef::new("t_ship", "cruiser"))
        .unit(UnitDef::new("t_target", "cruiser"))
        .unit(UnitDef::new("t_planet", "planet"))
        .build()
        .unwrap()
}

/// A scripted source whose units 2 and above show the given pool on every facing.
struct Shielded {
    inner: ScriptedSource,
    pool: [i32; 2],
}

impl Shielded {
    fn patch(&self, view: Arc<WorldView>) -> Arc<WorldView> {
        let mut v = (*view).clone();
        for u in v.units.iter_mut().filter(|u| u.id.0 >= 2) {
            u.shields = [[Fixed::from_int(self.pool[0]), Fixed::from_int(self.pool[1])]; 4];
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

struct Scene {
    db: ContentDb,
    /// The pool unit 2 shows: `[now, max]`.
    pool: [i32; 2],
    target_def: &'static str,
    /// The tick unit 2 is removed from the view.
    target_until: Option<u64>,
    /// `(tick, what)`: a shot at unit 2 or the hit it makes.
    events: Vec<(u64, String)>,
}

impl Scene {
    fn new(duration: i32) -> Scene {
        Scene { db: content(duration), pool: [30, 100], target_def: "t_target", target_until: None, events: Vec::new() }
    }
    fn fire(mut self, at: u64, weapon: &str) -> Self {
        let k = self.db.key(weapon).unwrap().0;
        self.events.push((at, format!("Fired: {{ shooter: 1, weapon: {k}, hardpoint: 0, target: {{ Entity: 2 }} }}")));
        self
    }
    /// The hit of a shot: what the shield and the hull took.
    fn hit(mut self, at: u64, weapon: &str, shield: i32, hull: i32) -> Self {
        let k = self.db.key(weapon).unwrap().0;
        self.events.push((at, format!("Hit: {{ target: 2, by: 1, weapon: {k}, facing: 0, shield: {shield}, hull: {hull}, at: {{ x: 0, y: 0, z: 600 }} }}")));
        self
    }
    fn app(&self, profile: Option<RenderProfile>) -> App {
        let ev: String = self.events.iter().map(|(at, e)| format!("{{ at: {at}, event: {{ {e} }} }},")).collect();
        let text = format!(
            "{{ units: [
                {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
                {{ id: 2, def: \"{}\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}]{} }},
            ], events: [{ev}] }}",
            self.target_def,
            self.target_until.map_or(String::new(), |t| format!(", until: {t}"))
        );
        let inner = ScriptedSource::from_json5(&text, &self.db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<crate::render::fx::quad::FxQuadMaterial>>()
            .init_resource::<Assets<crate::render::fx::shield::ShieldMaterial>>()
            .init_resource::<Assets<crate::render::fx::fire::FireMaterial>>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / TICK_HZ as f64)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(Arc::new(self.db.clone())))
            .insert_resource(SpriteTables(Arc::new(SpriteTable::parse(TABLE.as_bytes()))))
            .insert_resource(SimLink::new(Box::new(Shielded { inner, pool: self.pool }), Box::new(LocalRouter::new()), PlayerId(0)));
        if let Some(p) = profile {
            app.insert_resource(p);
        }
        app.add_plugins((crate::render::fx::weapons::WeaponFxPlugin, ImpactFxPlugin));
        configure_sets(&mut app);
        app
    }
}

fn caps(app: &mut App) -> Vec<ShieldCap> {
    let mut v: Vec<ShieldCap> = app.world_mut().query::<&ShieldCap>().iter(app.world()).cloned().collect();
    v.sort_by(|a, b| a.born.total_cmp(&b.born));
    v
}

fn shells(app: &mut App) -> usize {
    app.world_mut().query::<&ImpactFx>().iter(app.world()).filter(|f| f.kind == ImpactKind::Shield).count()
}

fn sparks(app: &mut App) -> usize {
    app.world_mut().query::<&ImpactFx>().iter(app.world()).filter(|f| f.kind == ImpactKind::Spark).count()
}

/// Every cap the app shows over `frames` frames, once each, with the frame counts it was alive for.
fn run(app: &mut App, frames: usize) -> Vec<(ShieldCap, usize)> {
    let mut all: Vec<(ShieldCap, usize)> = Vec::new();
    for _ in 0..frames {
        app.update();
        for c in caps(app) {
            match all.iter_mut().find(|(g, _)| g.target == c.target && g.weapon == c.weapon && g.born == c.born && g.shooter == c.shooter) {
                Some((_, n)) => *n += 1,
                None => all.push((c, 1)),
            }
        }
    }
    all
}

#[test]
fn the_effect_is_registered_as_stock_and_the_shell_is_a_refit_thing() {
    let mut app = App::new();
    app.add_plugins(StockShieldHitsPlugin);
    assert!(app.world().resource::<ProfileEffects>().has_stock(Effect::ShieldHits));
    // `Impacts` stays Refit's: hull sparks, explosions and smoke are drawn in both looks.
    assert!(!app.world().resource::<ProfileEffects>().has_stock(Effect::Impacts));
    let effects = app.world().resource::<ProfileEffects>();
    assert!(crate::render::fx::impacts::refit_shield_shell(Some(&RenderProfile::Refit), Some(effects)));
    assert!(!crate::render::fx::impacts::refit_shield_shell(Some(&RenderProfile::Stock), Some(effects)));
    // Without the profile resources (the windowless tests) the Refit shell is what is drawn.
    assert!(crate::render::fx::impacts::refit_shield_shell(None, None));
    assert!(crate::render::fx::impacts::refit_shield_shell(Some(&RenderProfile::Stock), None));
}

#[test]
fn a_stock_beam_makes_one_cap_when_it_arrives_with_the_level_before_its_damage_and_the_pole_at_its_end() {
    // The sim applies the beam's damage on the fire tick: 20 taken, the pool shows 30 of 100, so 50 of 100 before it.
    let scene = Scene::new(1).fire(4, "w_phaser").hit(4, "w_phaser", 20, 0);
    let mut app = scene.app(Some(RenderProfile::Stock));
    let seen = run(&mut app, 70);
    assert_eq!(seen.len(), 1, "one cap per beam");
    let (c, frames) = &seen[0];
    assert_eq!((c.target, c.shooter), (EntityId(2), EntityId(1)));
    // Born when the beam reaches the target: a fifth of its 30 ticks after it was fired.
    assert!((9.9..11.5).contains(&c.born), "born at tick {}", c.born);
    assert_eq!(c.life, TICK_HZ as f32, "the ordnance's shieldDuration");
    assert_eq!(c.fraction, 0.5);
    assert_eq!(c.colour, cap_colour(0.5));
    assert!(c.follows.is_some());
    assert!((c.dir.length() - 1.0).abs() < 1e-4);
    // The beam comes from the shooter at the origin: the pole is on the side of the target facing it.
    let (rot, axes) = crate::render::fx::impacts::shield_axes(Vec3::NEG_Z, crate::render::fx::impacts::DEFAULT_RADIUS);
    let toward = crate::render::units::to_bevy([0.0; 3]) - crate::render::units::to_bevy([0.0, 0.0, 600.0]);
    assert!(c.dir.distance(dir_toward(rot, axes, toward)) < 1e-3, "pole {}", c.dir);
    // A hard cut after its second: alive for as many frames as ticks it lives (give or take the frame it is made in).
    assert!((TICK_HZ as usize - 1..=TICK_HZ as usize + 1).contains(frames), "{frames} frames");
    assert!(caps(&mut app).is_empty(), "the cap is gone");
}

#[test]
fn a_cap_shorter_than_its_beam_ends_early_and_never_comes_back() {
    // The beam runs 30 ticks; a 0.3 second cap born at its arrival ends well before the beam does.
    let mut scene = Scene::new(1);
    let mut w = phaser(1);
    w.base.shield_duration = Some(Fixed::from_f64(0.3));
    scene.db = ContentBuilder::default().weapon(w).unit(UnitDef::new("t_ship", "cruiser")).unit(UnitDef::new("t_target", "cruiser")).build().unwrap();
    let scene = scene.fire(4, "w_phaser").hit(4, "w_phaser", 20, 0);
    let mut app = scene.app(Some(RenderProfile::Stock));
    let (mut born, mut gone_at, mut beam_gone) = (None, None, None);
    for f in 0..60u64 {
        app.update();
        let live = !caps(&mut app).is_empty();
        let beam = app.world_mut().query::<&StockShot>().iter(app.world()).any(|s| s.path == ShotPath::Ribbon);
        if live && born.is_none() {
            born = Some(f);
        }
        if born.is_some() && !live && gone_at.is_none() {
            gone_at = Some(f);
            assert!(beam, "the beam had already ended when its cap did");
        }
        if gone_at.is_some() {
            assert!(!live, "the cap came back at frame {f}");
        }
        if born.is_some() && !beam && beam_gone.is_none() {
            beam_gone = Some(f);
        }
    }
    let (born, gone_at, beam_gone) = (born.expect("a cap"), gone_at.expect("it ended"), beam_gone.expect("the beam ended"));
    // 0.3 s is 9 ticks.
    assert!((8..=10).contains(&(gone_at - born)), "lived {} frames", gone_at - born);
    assert!(beam_gone > gone_at + 5, "the beam ended at {beam_gone}, the cap at {gone_at}");
}

#[test]
fn a_projectile_hit_makes_a_cap_at_the_event_with_the_level_after_its_damage_pole_toward_the_shooter() {
    let scene = Scene::new(1).fire(4, "w_torp").hit(20, "w_torp", 20, 10);
    let mut app = scene.app(Some(RenderProfile::Stock));
    let seen = run(&mut app, 70);
    assert_eq!(seen.len(), 1);
    let (c, _) = &seen[0];
    assert!((19.0..22.0).contains(&c.born), "born at tick {}", c.born);
    // The pool after the tick: 30 of 100. No beam to follow.
    assert_eq!((c.fraction, c.follows), (0.3, None));
    assert_eq!(c.colour, cap_colour(0.3));
    // The default duration: the torpedo's ordnance carries no shieldDuration.
    assert_eq!(c.life, DEFAULT_DURATION * TICK_HZ as f32);
    let (rot, axes) = crate::render::fx::impacts::shield_axes(Vec3::NEG_Z, crate::render::fx::impacts::DEFAULT_RADIUS);
    // The shooter at the origin, the target at z 600 (render space mirrors z): the pole is the side facing the shooter.
    let toward = crate::render::units::to_bevy([0.0; 3]) - crate::render::units::to_bevy([0.0, 0.0, 600.0]);
    assert!(c.dir.distance(dir_toward(rot, axes, toward)) < 1e-3, "pole {}", c.dir);
}

#[test]
fn a_shot_of_a_class_with_no_stock_drawing_makes_its_cap_at_the_event_toward_the_shooter() {
    // A projectile and a beam, neither drawn the stock way: at the event, as every other shield-absorbing hit.
    let scene = Scene::new(1).fire(4, "w_odd").hit(10, "w_odd", 20, 0).fire(40, "w_tractor").hit(40, "w_tractor", 20, 0);
    let mut app = scene.app(Some(RenderProfile::Stock));
    let seen = run(&mut app, 100);
    assert_eq!(seen.len(), 2, "{:?}", seen.iter().map(|s| s.0.born).collect::<Vec<_>>());
    assert!((9.0..12.0).contains(&seen[0].0.born) && (39.0..42.0).contains(&seen[1].0.born));
    assert!(seen.iter().all(|(c, _)| c.follows.is_none() && c.fraction == 0.3));
    let (rot, axes) = crate::render::fx::impacts::shield_axes(Vec3::NEG_Z, crate::render::fx::impacts::DEFAULT_RADIUS);
    let toward = crate::render::units::to_bevy([0.0; 3]) - crate::render::units::to_bevy([0.0, 0.0, 600.0]);
    assert!(seen.iter().all(|(c, _)| c.dir.distance(dir_toward(rot, axes, toward)) < 1e-3));
}

#[test]
fn caps_are_not_limited_merged_or_pruned() {
    // Twelve hits in a row on one target: twelve caps, all alive together at the end of the burst.
    let mut scene = Scene::new(1).fire(4, "w_torp");
    for k in 0..12 {
        scene = scene.hit(10 + k, "w_torp", 5, 0);
    }
    let mut app = scene.app(Some(RenderProfile::Stock));
    let mut most = 0;
    for _ in 0..40 {
        app.update();
        most = most.max(caps(&mut app).len());
    }
    assert_eq!(most, 12);
}

#[test]
fn no_cap_for_ignore_shield_a_planet_shields_down_or_a_hit_the_shield_did_not_take() {
    let cases: Vec<(&str, Scene)> = vec![
        ("ignoreShield projectile", Scene::new(1).fire(4, "w_torp_ign").hit(10, "w_torp_ign", 20, 0)),
        ("ignoreShield beam", Scene::new(1).fire(4, "w_phaser_ign").hit(4, "w_phaser_ign", 20, 0)),
        ("a planet, projectile", {
            let mut s = Scene::new(1).fire(4, "w_torp").hit(10, "w_torp", 20, 0);
            s.target_def = "t_planet";
            s
        }),
        ("a planet, beam", {
            let mut s = Scene::new(1).fire(4, "w_phaser").hit(4, "w_phaser", 20, 0);
            s.target_def = "t_planet";
            s
        }),
        // Shields down: nothing absorbed, the pool shows zero. The hull hit draws sparks.
        ("shields down, projectile", {
            let mut s = Scene::new(1).fire(4, "w_torp").hit(10, "w_torp", 0, 30);
            s.pool = [0, 100];
            s
        }),
        ("shields down, beam", {
            let mut s = Scene::new(1).fire(4, "w_phaser").hit(4, "w_phaser", 0, 30);
            s.pool = [0, 100];
            s
        }),
    ];
    for (what, scene) in cases {
        let mut app = scene.app(Some(RenderProfile::Stock));
        let seen = run(&mut app, 70);
        assert!(seen.is_empty(), "{what}: a cap was made");
    }
}

fn impact_flares(app: &mut App) -> usize {
    app.world_mut().query::<&crate::render::fx::stock_flares::Flare>().iter(app.world()).filter(|f| f.kind == crate::render::fx::stock_flares::FlareKind::Impact).count()
}

/// Runs `scene` and gives the most caps and the most impact flares seen at once, and the caps' fractions.
fn caps_and_flares(scene: &Scene) -> (usize, usize, Vec<f32>) {
    let mut app = scene.app(Some(RenderProfile::Stock));
    let (mut c, mut f, mut fr) = (0, 0, Vec::new());
    for _ in 0..60 {
        app.update();
        let now = caps(&mut app);
        c = c.max(now.len());
        if now.len() > fr.len() {
            fr = now.iter().map(|x| x.fraction).collect();
        }
        f = f.max(impact_flares(&mut app));
    }
    (c, f, fr)
}

#[test]
fn a_beam_the_shield_absorbed_makes_a_cap_even_if_it_emptied_the_shield_and_one_it_did_not_the_flare() {
    // The beam emptied the pool (the view shows 0 of 100 after its damage, 20 taken): a cap at the level before (0.2), no flare.
    let mut empties = Scene::new(1).fire(4, "w_phaser").hit(4, "w_phaser", 20, 0);
    empties.pool = [0, 100];
    let (c, f, fr) = caps_and_flares(&empties);
    assert_eq!((c, f), (1, 0), "an absorbed beam: a cap and no impact flare");
    assert_eq!(fr, vec![0.2]);
    // The shield did not take it (shields were already down): the impact flare, no cap.
    let mut down = Scene::new(1).fire(4, "w_phaser").hit(4, "w_phaser", 0, 30);
    down.pool = [0, 100];
    assert_eq!(caps_and_flares(&down).0, 0);
    assert_eq!(caps_and_flares(&down).1, 1);
    // Shields up and absorbing, not emptied: a cap, no flare, as before.
    let up = Scene::new(1).fire(4, "w_phaser").hit(4, "w_phaser", 20, 0);
    let (c, f, _) = caps_and_flares(&up);
    assert_eq!((c, f), (1, 0));
}

#[test]
fn beams_on_one_tick_and_pool_all_see_the_level_before_all_of_that_ticks_damage() {
    // Two beams in one tick took 20 and 10 from a pool that shows 30 of 100 afterwards: both were fired at a pool of 60.
    let scene = Scene::new(1).fire(4, "w_phaser").fire(4, "w_phaser").hit(4, "w_phaser", 20, 0).hit(4, "w_phaser", 10, 0);
    let (c, _, fr) = caps_and_flares(&scene);
    assert_eq!(c, 2);
    assert!(fr.iter().all(|f| (f - 0.6).abs() < 1e-6), "{fr:?}");
}

#[test]
fn the_cap_model_is_dropped_and_loaded_again_when_the_mounted_files_change_and_a_failed_load_is_retried() {
    use crate::render::textures::TextureCache;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(crate::viewer::Mounted(sta_vfs::Vfs::default()))
        .init_resource::<TextureCache>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<CapLoadState>();
    app.add_systems(Update, reset_on_remount.run_if(resource_exists_and_changed::<crate::viewer::Mounted>));
    app.update();
    let assets = ShieldCapAssets { mesh: default(), texture: default(), flipbook: None, r_db: 4.0 };
    app.insert_resource(assets);
    app.world_mut().resource_mut::<CapLoadState>().tried = true;
    app.update();
    assert!(app.world().get_resource::<ShieldCapAssets>().is_some() && app.world().resource::<CapLoadState>().tried, "nothing changed: kept");
    // Another game or mod is mounted: the old cap goes and the load may be tried again.
    app.world_mut().resource_mut::<crate::viewer::Mounted>().0 = sta_vfs::Vfs::default();
    app.update();
    assert!(app.world().get_resource::<ShieldCapAssets>().is_none());
    assert!(!app.world().resource::<CapLoadState>().tried, "a failed load is tried again after a change");
}

/// A hit with shield and hull both taking damage, in a world under `profile`: the caps and the Refit shells it makes.
fn one_hit(profile: Option<RenderProfile>) -> (usize, usize, usize) {
    let scene = Scene::new(1).fire(4, "w_torp").hit(10, "w_torp", 20, 30);
    let mut app = scene.app(profile);
    let (mut caps_seen, mut shells_seen, mut sparks_seen) = (0, 0, 0);
    for _ in 0..40 {
        app.update();
        caps_seen = caps_seen.max(caps(&mut app).len());
        shells_seen = shells_seen.max(shells(&mut app));
        sparks_seen = sparks_seen.max(sparks(&mut app));
    }
    (caps_seen, shells_seen, sparks_seen)
}

#[test]
fn the_remaster_look_still_spawns_its_shield_shell_and_no_stock_cap_and_stock_the_reverse() {
    // Remaster (Refit) look: the shell, the hull sparks, no cap.
    let (cap, shell, spark) = one_hit(Some(RenderProfile::Refit));
    assert_eq!((cap, shell), (0, 1), "under Refit one shell and no stock cap");
    assert!(spark > 0, "the hull sparks stay");
    // Without profile resources (today's windowless path) it is Refit's, too.
    assert_eq!(one_hit(None).0, 0);
    assert_eq!(one_hit(None).1, 1);
    // Stock look: the cap, no Refit shell; and none of Refit's hull streaks either (the stock spark is
    // `stock_hull_hits.rs`', #1153).
    let (cap, shell, stock_spark) = one_hit(Some(RenderProfile::Stock));
    assert_eq!((cap, shell), (1, 0), "under Stock one cap and no Refit shell");
    assert_eq!(stock_spark, 0, "no Refit hull streak under Stock");
}

#[test]
fn shields_going_down_draws_nothing_under_stock_yet_and_the_shell_under_refit() {
    let down = |profile| {
        let mut scene = Scene::new(1);
        scene.events.push((10, "ShieldsDown: { id: 2, facing: 1 }".into()));
        let mut app = scene.app(Some(profile));
        let mut most = 0;
        for _ in 0..30 {
            app.update();
            most = most.max(shells(&mut app));
            assert!(caps(&mut app).is_empty());
        }
        most
    };
    assert_eq!(down(RenderProfile::Stock), 0);
    assert_eq!(down(RenderProfile::Refit), 1);
}

#[test]
fn a_cap_ends_with_its_target() {
    let mut scene = Scene::new(3).fire(4, "w_torp").hit(10, "w_torp", 20, 0);
    scene.target_until = Some(20);
    let mut app = scene.app(Some(RenderProfile::Stock));
    let mut alive = 0;
    for _ in 0..60 {
        app.update();
        alive += usize::from(!caps(&mut app).is_empty());
    }
    // A three second cap would run for 90 ticks; its target left the view at tick 20.
    assert!((5..=15).contains(&alive), "{alive} frames");
    assert!(caps(&mut app).is_empty());
}

// ---------------------------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------------------------

/// With `STA64_GPU_TESTS=1` and a hardware adapter: an AUTHORED cap model and texture (no install) drawn through the
/// production material, visual spawn and flipbook systems and the production frame and life functions, read back
/// from the frame:
/// - additive: over a grey clear colour the cap's colour is added to it, and two caps on one spot add up;
/// - the colour follows the shield fraction: green at 1, red at 0, the two halves between, times the gain;
/// - the flipbook cell follows the game clock: two cells of the authored sheet, two game times;
/// - the pole faces the hit: a cap whose pole is toward the camera covers the centre, one whose pole is toward +x
///   covers the right of it and not the centre;
/// - a hard cut at the life's end: lit one tick before it, gone at it.
///
/// Prints SKIP and passes otherwise.
#[test]
fn gpu_the_cap_adds_in_its_level_colour_in_its_cell_with_the_pole_at_the_hit_and_is_cut_at_the_end() {
    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::image::ImageSampler;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::window::ExitCondition;
    use std::sync::Mutex;

    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to draw the shield cap test");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
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
    app.add_plugins(crate::render::fx::stock_shots::StockShotsMaterialPlugin).insert_resource(ClearColor(Color::BLACK));

    // The sheet: 4 x 4 cells of 16 x 16 texels. Cell 0 is a quarter, cell 5 (column 1, row 1) an eighth, all else full.
    let mut data = Vec::new();
    for y in 0..64usize {
        for x in 0..64usize {
            let v = match (x / 16, y / 16) {
                (0, 0) => 64u8,
                (1, 1) => 32,
                _ => 255,
            };
            data.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let mut sheet = Image::new(Extent3d { width: 64, height: 64, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    sheet.sampler = ImageSampler::nearest();
    let texture = app.world_mut().resource_mut::<Assets<Image>>().add(sheet);
    let geo = cap_geometry(&authored_model()).unwrap();
    let r_db = geo.r_db;
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(geo.mesh);
    app.insert_resource(ShieldCapAssets { mesh, texture, flipbook: geo.flipbook, r_db });
    app.insert_resource(SpriteTables(Arc::new(flipbook_table())));
    app.insert_resource(AnimClock::new(crate::clock::ClockMode::Wall, 0));
    // The production visual spawn and flipbook, and the production frame and cut (`cap_frame`, `cap_live`).
    fn place(mut cmds: Commands, clock: Res<AnimClock>, mut caps: Query<(Entity, &ShieldCap, &mut Transform, Option<&CapVisual>), Without<CapMesh>>, mut turns: Query<&mut Transform, (With<CapMesh>, Without<ShieldCap>)>) {
        let now = clock.now() * TICK_HZ as f64;
        for (e, cap, mut tf, visual) in &mut caps {
            if !cap_live(now, cap.born, cap.life) {
                cmds.entity(e).despawn();
                continue;
            }
            let (frame, turn) = cap_frame(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(20.0), 4.507_f32, cap.dir);
            *tf = frame;
            if let Some(CapVisual(c)) = visual
                && let Ok(mut t) = turns.get_mut(*c)
            {
                *t = turn;
            }
        }
    }
    app.add_systems(Update, (attach_visuals, animate_caps, place).chain());

    let eye = Vec3::new(0.0, 0.0, 64.0 / (std::f32::consts::FRAC_PI_8).tan());
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
    for _ in 0..60 {
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
    let srgb = |v: f32| (255.0 * if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }).round() as i32;
    let near = |got: [u8; 3], want: [i32; 3]| got.iter().zip(want).all(|(g, w)| (i32::from(*g) - w).abs() <= 12);
    let life = 24.0;
    let cap = |dir: Vec3, f: f32| ShieldCap {
        target: EntityId(2),
        shooter: EntityId(1),
        weapon: ContentKey(1),
        follows: None,
        born: 0.0,
        life,
        fraction: f,
        colour: cap_colour(f),
        dir,
        shown: true,
    };
    let set_tick = |app: &mut App, tick: u64| {
        let mut clock = AnimClock::new(crate::clock::ClockMode::Wall, 0);
        for _ in 0..tick {
            clock.ticked();
        }
        app.insert_resource(clock);
    };
    let clear = |app: &mut App| {
        let caps: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<ShieldCap>>().iter(app.world()).collect();
        for e in caps {
            app.world_mut().entity_mut(e).despawn();
        }
    };
    let show = |app: &mut App, caps: Vec<ShieldCap>, tick: u64, bg: f32| {
        clear(app);
        app.world_mut().resource_mut::<ClearColor>().0 = Color::linear_rgb(bg, bg, bg);
        set_tick(app, tick);
        for c in caps {
            app.world_mut().spawn((c, Transform::IDENTITY, Visibility::Inherited));
        }
        for _ in 0..4 {
            app.update();
        }
    };
    let px = |img: &image::RgbaImage, x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    };

    // Face on, cell 0 (a quarter of full), at the three levels: the colour times the gain, the centre only.
    let quarter = 64.0 / 255.0;
    for (f, want) in [(1.0, [0, srgb(GAIN * quarter), 0]), (0.0, [srgb(GAIN * quarter), 0, 0]), (0.5, [srgb(GAIN * quarter * 0.5); 3])] {
        show(&mut app, vec![cap(Vec3::Z, f)], 0, 0.0);
        let img = capture(&mut app);
        let want = if f == 0.5 { [want[0], want[0], 0] } else { want };
        assert!(near(px(&img, 64, 64), want), "f {f}: centre {:?}, wanted {want:?}", px(&img, 64, 64));
        assert!(near(px(&img, 64 + 34, 64), [0, 0, 0]) && near(px(&img, 6, 6), [0, 0, 0]) && near(px(&img, 64, 64 - 34), [0, 0, 0]), "f {f}: outside the cap is black");
    }
    // Added to what is there, the texture's alpha darkens nothing: a grey of 0.2 plus the cap's green.
    show(&mut app, vec![cap(Vec3::Z, 1.0)], 0, 0.2);
    let img = capture(&mut app);
    assert!(near(px(&img, 64, 64), [srgb(0.2), srgb(0.2 + GAIN * quarter), srgb(0.2)]), "additive over grey: {:?}", px(&img, 64, 64));
    assert!(near(px(&img, 6, 6), [srgb(0.2); 3]), "the grey outside is untouched");
    // Two caps on one spot add up (the sum of two halves of full green is full green).
    show(&mut app, vec![cap(Vec3::Z, 1.0), cap(Vec3::Z, 1.0)], 0, 0.0);
    let img = capture(&mut app);
    assert!(near(px(&img, 64, 64), [0, srgb(2.0 * GAIN * quarter), 0]), "two caps: {:?}", px(&img, 64, 64));

    // The flipbook cell follows the game clock: at tick 6 of the game (a fifth of a second) the sheet shows cell 5, an eighth of full.
    show(&mut app, vec![cap(Vec3::Z, 1.0)], 6, 0.0);
    let img = capture(&mut app);
    assert!(near(px(&img, 64, 64), [0, srgb(GAIN * 32.0 / 255.0), 0]), "cell 5: {:?}", px(&img, 64, 64));

    // The pole faces the hit: toward +x the cap lies right of the centre and leaves the centre bare.
    show(&mut app, vec![cap(Vec3::X, 1.0)], 0, 0.0);
    let img = capture(&mut app);
    // Seen from the side the ray crosses the dome twice, front and back, and both add (the dome has no depth write).
    assert!(near(px(&img, 64 + 17, 64), [0, 255, 0]), "pole toward +x, right of the centre: {:?}", px(&img, 64 + 17, 64));
    assert!(near(px(&img, 64 - 17, 64), [0, 0, 0]) && near(px(&img, 64, 64), [0, 0, 0]), "pole toward +x leaves the centre and the left bare");

    // A hard cut at the end of its life: lit one tick before, gone at it.
    show(&mut app, vec![cap(Vec3::Z, 1.0)], 23, 0.0);
    let img = capture(&mut app);
    assert!(px(&img, 64, 64)[1] > 100, "lit one tick before its end: {:?}", px(&img, 64, 64));
    show(&mut app, vec![cap(Vec3::Z, 1.0)], 24, 0.0);
    let img = capture(&mut app);
    assert!(near(px(&img, 64, 64), [0, 0, 0]), "gone at its end: {:?}", px(&img, 64, 64));
}

// ---------------------------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------------------------

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): the default shield-hit model resolves and loads, the radius of its
/// database is positive and every vertex lies within it of the box centre, its texture is mounted and its
/// flipbook is an animation of the sprite table. Skips without an install.
#[test]
fn install_the_default_model_resolves_loads_and_every_vertex_lies_within_its_database_radius() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
    let Some((path, model)) = crate::render::fx::mining_beam::load_named_model(&vfs, DEFAULT_MODEL, "test") else {
        panic!("the default shield-hit model {DEFAULT_MODEL} is not in the install at {root}");
    };
    eprintln!("the default shield-hit model resolved to {path}");
    let r_db = database_radius(&model).expect("the model has meshes");
    assert!(r_db > 0.0, "{r_db}");
    let (min, max) = model.bounds().unwrap();
    let c = (Vec3::from(min) + Vec3::from(max)) * 0.5;
    let mut vertices = 0;
    for (i, n) in model.nodes.iter().enumerate() {
        if let Some(mesh) = n.mesh() {
            for p in &mesh.positions {
                vertices += 1;
                assert!((Vec3::from(model.model_transform(i).transform_point(*p)) - c).length() <= r_db + 1e-3);
            }
        }
    }
    assert!(vertices > 0);
    let geo = cap_geometry(&model).expect("a cap geometry");
    assert!((geo.r_db - r_db).abs() < 1e-6);
    let texture = geo.texture.as_deref().expect("the cap names a texture");
    assert!(sta_assets::find_texture(&vfs, texture).is_some(), "the texture {texture} is not mounted");
    let table = sta_assets::load_sprite_table(&vfs);
    let (anim, _) = geo.flipbook.as_ref().expect("the cap names a flipbook");
    assert!(table.animation(anim).is_some(), "the flipbook {anim} is in no sprite table");
}
