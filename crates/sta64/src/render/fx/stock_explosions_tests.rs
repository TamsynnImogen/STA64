//! Tests of the Stock look's explosions (#1153). The definitions, sprite tables and content are authored here; time
//! is stepped by hand, never read from the clock of the machine.
use super::*;
use crate::app::configure_sets;
use crate::render::fx::impacts::{ImpactFx, ImpactFxPlugin, BLAST_SPARKS, HIT_SPARKS};
use crate::sim_link::{ContentRes, SimLink, SimLinkPlugin};
use bevy::time::TimeUpdateStrategy;
use sta_content::{ChunkCfg, ContentBuilder, ExplosionTable, Fixed, UnitDef, WeaponDef, WeaponKind};
use sta_sim::PlayerId;
use sta_sim::command::LocalRouter;
use sta_sim::fake::ScriptedSource;
use std::collections::BTreeSet;
use std::time::Duration;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn explosion(name: &str, length: Option<f64>, wave: Option<&str>, delay: Option<f64>, minimum: Option<f64>) -> ExplosionDef {
    ExplosionDef {
        name: name.into(),
        class: "fireballexplode".into(),
        sound: None,
        model: Some("cluster".into()),
        length: length.map(fx),
        shockwave: wave.map(Into::into),
        shockwave_delay: delay.map(fx),
        shockwave_minimum: minimum.map(fx),
    }
}

fn wave(name: &str) -> ShockwaveDef {
    ShockwaveDef {
        name: name.into(),
        class: "shockwavexplode".into(),
        rays: Some(6),
        color: Some([fx(1.0), fx(0.5), fx(0.25)]),
        texture: Some("ring_tex".into()),
        radius_factor: Some(fx(4.0)),
        fade_factor: Some(fx(0.5)),
        rate: Some(fx(10.0)),
    }
}

// ---------------------------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------------------------

#[test]
fn the_fireball_lives_its_length_and_without_the_key_as_long_as_its_sprites() {
    assert_eq!(fireball_life(&explosion("a", Some(2.5), None, None, None), 1.5), 2.5);
    assert_eq!(fireball_life(&explosion("a", None, None, None, None), 1.5), 1.5, "no key: the sprites' end (INTERIM)");
}

#[test]
fn the_shockwave_is_due_after_its_delay_above_its_minimum_and_only_when_one_is_named() {
    let d = explosion("a", Some(3.0), Some("ring"), Some(0.45), Some(35.0));
    assert!(!shockwave_due(&d, true, 0.4, 60.0), "before the delay");
    assert!(shockwave_due(&d, true, 0.5, 60.0));
    assert!(!shockwave_due(&d, true, 0.5, 35.0), "the radius must be above the minimum");
    assert!(!shockwave_due(&d, false, 0.5, 60.0), "no shockwave named");
    // Absent keys: zero (INTERIM).
    let bare = explosion("b", None, Some("ring"), None, None);
    assert!(shockwave_due(&bare, true, 0.01, 1.0));
}

#[test]
fn the_ring_grows_by_rate_and_its_outer_edge_twice_that_and_fades_to_nothing_at_its_end() {
    let w = wave("ring");
    // Radius 30, factor 4: ends where the outer edge passes 120, fades from 60 (fade factor 0.5); rate 10.
    let (scale, k) = ring_state(&w, 30.0, 2.0).unwrap();
    assert_eq!((scale, k), (20.0, 1.0), "outer edge at 40: inside the flat part");
    let (_, k) = ring_state(&w, 30.0, 4.5).unwrap();
    assert!((k - (120.0 - 90.0) / 60.0).abs() < 1e-5, "outer edge at 90: linear, got {k}");
    assert!(ring_state(&w, 30.0, 6.0).is_some_and(|(_, k)| k.abs() < 1e-5), "the edge reaches the end: nothing is left");
    assert!(ring_state(&w, 30.0, 6.1).is_none(), "past the end: removed");
    // Without a rate, factor or fade there is no ring (INTERIM).
    for strip in [|w: &mut ShockwaveDef| w.rate = None, |w: &mut ShockwaveDef| w.radius_factor = None, |w: &mut ShockwaveDef| w.fade_factor = None] {
        let mut w = wave("ring");
        strip(&mut w);
        assert!(ring_state(&w, 30.0, 1.0).is_none());
    }
}

#[test]
fn the_ring_mesh_has_three_bands_a_ray_and_mirrors_u_from_ray_to_ray() {
    let mesh = ring_mesh(6);
    let positions: Vec<[f32; 3]> = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap().to_vec();
    assert_eq!(positions.len(), 6 * 6, "six vertices a ray");
    assert_eq!(mesh.indices().unwrap().len(), 6 * 3 * 6, "three bands of two triangles a ray");
    let radii: BTreeSet<u32> = positions.iter().map(|p| ((p[0] * p[0] + p[2] * p[2]).sqrt() * 100.0).round() as u32).collect();
    assert_eq!(radii, BTreeSet::from([100, 200]), "from one to two ring radii");
    let heights: BTreeSet<i32> = positions.iter().map(|p| p[1].round() as i32).collect();
    assert_eq!(heights, BTreeSet::from([0, CROSSED_HEIGHT as i32, -CROSSED_HEIGHT as i32]), "one flat band and two crossed");
    let uvs: Vec<[f32; 2]> = match mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() {
        bevy::mesh::VertexAttributeValues::Float32x2(v) => v.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!((uvs[0][0], uvs[6][0], uvs[12][0]), (0.0, 1.0, 0.0), "u alternates with the ray");
    assert_eq!((uvs[0][1], uvs[1][1]), (0.0, 1.0), "v from the inner to the outer edge");
}

#[test]
fn a_flipbook_plays_once_and_its_sprite_is_hidden_afterwards() {
    let table = SpriteTable::parse(b"sprite_table\n@referenceWidth=64\n@referenceHeight=64\n@tmaterial=additive\natlas tex_atlas 0 0 32 32 @anim=flip4\nfixed tex_fixed 32 0 32 32\n@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n");
    let atlas = table.sprite("atlas").unwrap();
    assert_eq!(flipbook_end(&table, atlas), Some(1.0));
    let cell = |age: f32| once_uv(&table, atlas, age).map(|r| r.map(|v| (v * 64.0).round() as i32));
    let cells: Vec<_> = [0.0, 0.3, 0.55, 0.9].map(cell).into_iter().collect();
    assert!(cells.iter().all(Option::is_some), "it shows until its end: {cells:?}");
    assert_eq!(cells.iter().collect::<BTreeSet<_>>().len(), 4, "four cells over a second: {cells:?}");
    assert_eq!(cell(1.0), None, "it has played once");
    assert_eq!(cell(5.0), None, "and does not come back");
    let fixed = table.sprite("fixed").unwrap();
    assert_eq!(flipbook_end(&table, fixed), None);
    assert!(once_uv(&table, fixed, 99.0).is_some(), "a sprite with no flipbook stays for the explosion's life (INTERIM)");
}

// ---------------------------------------------------------------------------------------------
// Which effect each look asks for
// ---------------------------------------------------------------------------------------------

fn content() -> sta_content::ContentDb {
    let mut ship = UnitDef::new("t_ship", "cruiser");
    ship.fx.explosion = Some("boom_ship".into());
    let mut base = UnitDef::new("t_base", "starbase");
    base.fx.explosion = Some("boom_base".into());
    // A unit whose definition does not resolve keeps today's effect.
    let mut lost = UnitDef::new("t_lost", "cruiser");
    lost.fx.explosion = Some("nowhere".into());
    let table = ExplosionTable::from_parts(
        [explosion("boom_ship", Some(1.0), Some("ring"), Some(0.2), Some(10.0)), explosion("boom_base", Some(3.0), Some("ring"), Some(0.2), Some(35.0))],
        [wave("ring")],
        ChunkCfg::default(),
    );
    ContentBuilder::default()
        .weapon(WeaponDef::new("t_torp", "photontorpedo", WeaponKind::Projectile))
        .unit(ship)
        .unit(base)
        .unit(lost)
        .build()
        .unwrap()
        .with_explosions(table)
}

fn timeline(db: &sta_content::ContentDb) -> String {
    let torp = db.key("t_torp").unwrap().0;
    // A ship, a building and a unit without a definition die; a fourth ship is hit on its hull.
    format!(
        "{{ units: [
            {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}], until: 100 }},
            {{ id: 2, def: \"t_base\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 500, y: 0, z: 0 }} }}], until: 100 }},
            {{ id: 3, def: \"t_lost\", owner: 1, hull: 800, rails: [{{ at: 0, pos: {{ x: 900, y: 0, z: 0 }} }}], until: 100 }},
            {{ id: 4, def: \"t_ship\", owner: 1, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}] }},
        ], events: [{{ at: 20, event: {{ Hit: {{ target: 4, by: 1, weapon: {torp}, facing: 0, shield: 0, hull: 40, at: {{ x: 0, y: 0, z: 590 }} }} }} }}] }}"
    )
}

/// What a run drew: the fireballs and rings (with the frame they were first seen in and the last), the impact
/// effects by kind.
#[derive(Default, Debug)]
struct Drawn {
    fireballs: BTreeSet<(u32, u32)>,
    fireball_frames: std::collections::BTreeMap<u32, (u32, u32)>,
    rings: BTreeSet<u32>,
    impacts: std::collections::BTreeMap<String, BTreeSet<(u32, u64, u8)>>,
}

fn run(profile: RenderProfile) -> Drawn {
    let db = content();
    let src = ScriptedSource::from_json5(&timeline(&db), &db).unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 45.0)))
        .insert_resource(profile)
        .add_plugins(SimLinkPlugin)
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
        .add_plugins(ImpactFxPlugin);
    configure_sets(&mut app);
    let mut drawn = Drawn::default();
    for f in 0..700u32 {
        app.update();
        let mut q = app.world_mut().query::<&StockExplosion>();
        for x in q.iter(app.world()) {
            drawn.fireballs.insert((x.unit.0, x.born as u32));
            let e = drawn.fireball_frames.entry(x.unit.0).or_insert((f, f));
            e.1 = f;
        }
        let mut q = app.world_mut().query::<&StockRing>();
        drawn.rings.extend(q.iter(app.world()).map(|r| r.unit.0));
        let mut q = app.world_mut().query::<&ImpactFx>();
        for i in q.iter(app.world()) {
            drawn.impacts.entry(format!("{:?}", i.kind)).or_default().insert((i.unit.0, i.fired, i.idx));
        }
    }
    drawn
}

use crate::render::profile::RenderProfile;
use sta_assets::a2fmt::spr::SpriteTable;

#[test]
fn under_stock_a_unit_and_a_building_ask_for_the_stock_effect_and_a_hull_hit_keeps_refits_sparks() {
    let d = run(RenderProfile::Stock);
    let units: BTreeSet<u32> = d.fireballs.iter().map(|f| f.0).collect();
    assert_eq!(units, BTreeSet::from([1, 2]), "the ship and the building each request the stock explosion: {d:?}");
    // The unit whose definition does not resolve keeps today's effect: its fireball, ring and blast sparks.
    let of = |k: &str| d.impacts.get(k).map_or(0, |s| s.iter().filter(|i| i.0 == 3).count());
    assert_eq!((of("Fire"), of("Ring"), of("Spark")), (1, 1, BLAST_SPARKS as usize), "{d:?}");
    // Nothing of today's explosion for the two with a definition; the hull hit's sparks stay Refit's (INTERIM).
    assert!(d.impacts.values().flatten().all(|i| i.0 == 3 || (i.0 == 4 && i.2 < HIT_SPARKS)), "only the unit without a definition draws Refit's explosion: {:?}", d.impacts);
    assert_eq!(d.impacts["Spark"].iter().filter(|i| i.0 == 4).count(), HIT_SPARKS as usize);
}

#[test]
fn under_the_remaster_look_they_ask_for_todays_effect() {
    let d = run(RenderProfile::Refit);
    assert!(d.fireballs.is_empty() && d.rings.is_empty(), "no stock explosion: {d:?}");
    let of = |k: &str, unit: u32| d.impacts.get(k).map_or(0, |s| s.iter().filter(|i| i.0 == unit).count());
    for unit in [1, 2, 3] {
        assert_eq!((of("Fire", unit), of("Ring", unit), of("Spark", unit)), (1, 1, BLAST_SPARKS as usize), "unit {unit}");
    }
    assert_eq!(of("Spark", 4), HIT_SPARKS as usize, "the hull hit's sparks");
}

#[test]
fn the_ring_follows_the_delay_the_minimum_and_the_length() {
    let d = run(RenderProfile::Stock);
    // The ship's radius is above its definition's minimum, the building's (a higher one) is not: one ring.
    assert_eq!(d.rings, BTreeSet::from([1]), "{d:?}");
    // The building's fireball (a longer length) outlives the ship's.
    let (ship, base) = (d.fireball_frames[&1], d.fireball_frames[&2]);
    assert!(base.1 - base.0 > ship.1 - ship.0, "length 3 outlives length 1: {ship:?} {base:?}");
    assert!(ship.1 - ship.0 > 10, "and the ship's lives some frames: {ship:?}");
}

// ---------------------------------------------------------------------------------------------
// The order of the checks, the unit's frame, and the drawing path
// ---------------------------------------------------------------------------------------------

use crate::clock::AnimClock;
use bevy::ecs::system::RunSystemOnce;

/// Advance the clock by one sim tick, as a frame at the sim's rate does.
fn step(app: &mut App) {
    let mut c = app.world_mut().resource_mut::<AnimClock>();
    for _ in 0..c.begin_frame(1.0 / TICK_HZ as f64) {
        c.ticked();
    }
    c.end_frame();
    app.update();
}

fn request_for(unit: u32, born_age: f32, heading: Vec3, explosion: ExplosionDef, wave: Option<ShockwaveDef>) -> ExplosionRequest {
    ExplosionRequest { unit: EntityId(unit), at: Vec3::new(5.0, 6.0, 7.0), heading, radius: 30.0, born: -(born_age as f64) * TICK_HZ as f64, explosion, wave }
}

#[test]
fn the_shockwave_is_checked_before_the_end_of_life_and_the_cluster_stands_in_the_units_frame() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).insert_resource(AnimClock::default()).init_resource::<ExplosionQueue>();
    app.add_systems(Update, (spawn_explosions, update_explosions).chain());
    // A long frame: the first update finds the fireball already past its length (1.0) and its ring's delay (0.9).
    let heading = Vec3::X;
    app.world_mut().resource_mut::<ExplosionQueue>().0.push(request_for(1, 1.1, heading, explosion("a", Some(1.0), Some("ring"), Some(0.9), Some(10.0)), Some(wave("ring"))));
    app.update();
    app.update();
    let rings = app.world_mut().query::<&StockRing>().iter(app.world()).count();
    let fireballs = app.world_mut().query::<&StockExplosion>().iter(app.world()).count();
    assert_eq!((rings, fireballs), (1, 0), "the ring is made, then the fireball ends");
    // A fireball still alive stands in the unit's frame: its forward axis is the heading (less one frame's turn).
    app.world_mut().resource_mut::<ExplosionQueue>().0.push(request_for(2, 0.0, heading, explosion("b", Some(5.0), None, None, None), None));
    app.update();
    let tf = app.world_mut().query::<(&StockExplosion, &Transform)>().iter(app.world()).next().map(|(_, t)| *t).unwrap();
    assert!(tf.forward().as_vec3().dot(heading) > 0.999, "forward {:?} for heading {heading:?}", tf.forward());
    assert_eq!(tf.translation, Vec3::new(5.0, 6.0, 7.0));
}

fn drawing_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<StockShotMaterial>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .insert_resource(AnimClock::default())
        .insert_resource(SpriteTables(Arc::new(SpriteTable::parse(
            b"sprite_table\n@referenceWidth=64\n@referenceHeight=64\n@tmaterial=additive\natlas tex_atlas 0 0 32 32 @anim=flip4\n@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n",
        ))));
    app.add_systems(Update, (update_explosions, animate_sprites, update_rings).chain());
    app
}

/// The drawing path as far as it runs without the mounted files: the cluster's quads (offset, size, billboard), the
/// flipbook cell written to each, the hiding after the flipbook, and the ring's scale, tint and end. What does not run
/// here is `attach_visuals`' loading of the model and textures from the mounted install (it needs the player's
/// files, the texture cache and a render device); its spawn part is `spawn_cluster`, run below with authored nodes.
#[test]
fn the_cluster_is_spawned_animated_and_hidden_and_the_ring_is_tinted_and_ended() {
    let mut app = drawing_app();
    let image = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
    let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(crate::render::sprites::unit_quad());
    let nodes = vec![
        ClusterNode { sprite: "atlas".into(), offset: Vec3::new(1.0, 2.0, 3.0), size: Vec2::new(4.0, 6.0), colour: Vec3::new(1.0, 0.5, 0.25), billboard: true, texture: Some(image.clone()) },
        ClusterNode { sprite: "atlas".into(), offset: Vec3::new(-1.0, 0.0, 0.0), size: Vec2::new(4.0, 6.0), colour: Vec3::ONE, billboard: false, texture: Some(image.clone()) },
        ClusterNode { sprite: "atlas".into(), offset: Vec3::ZERO, size: Vec2::ONE, colour: Vec3::ONE, billboard: true, texture: None },
    ];
    let root = app
        .world_mut()
        .spawn((
            StockExplosion {
                unit: EntityId(1),
                born: 0.0,
                radius: 30.0,
                explosion: explosion("a", Some(2.0), Some("ring"), Some(0.2), Some(10.0)),
                wave: Some(wave("ring")),
                sprites_end: 0.0,
                ring_made: false,
                axis: Vec3::Y,
                visuals: true,
            },
            Transform::IDENTITY,
        ))
        .id();
    let table = app.world().resource::<SpriteTables>().0.clone();
    let end = app.world_mut().run_system_once(move |mut cmds: Commands, mut mats: ResMut<Assets<StockShotMaterial>>| spawn_cluster(&mut cmds, root, &nodes, &quad, &table, &mut mats)).unwrap();
    app.update();
    assert_eq!(end, 1.0, "the flipbook's length");
    let mut q = app.world_mut().query::<(&ExplosionSprite, &Transform, Has<Billboard>)>();
    let mut quads: Vec<(Vec3, Vec3, bool)> = q.iter(app.world()).map(|(_, t, b)| (t.translation, t.scale, b)).collect();
    quads.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    assert_eq!(quads, vec![(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(4.0, 6.0, 1.0), false), (Vec3::new(1.0, 2.0, 3.0), Vec3::new(8.0, 12.0, 1.0), true)], "a node without a texture is skipped; a billboard is sized by half extents");

    // The cell written to the first sprite's material follows the age; at the end of the flipbook the sprite is hidden.
    let rect_of = |app: &mut App| {
        let mut q = app.world_mut().query::<(&Transform, &MeshMaterial3d<StockShotMaterial>, &Visibility)>();
        let all: Vec<_> = q.iter(app.world()).map(|(t, _, v)| (t.translation, *v)).collect();
        let now = app.world().resource::<AnimClock>().now();
        let (m, v) = q.iter(app.world()).find(|(t, _, _)| t.translation.x > 0.5).map(|(_, m, v)| (m.clone(), *v)).unwrap_or_else(|| panic!("no sprite at clock {now}: {all:?}"));
        (app.world().resource::<Assets<StockShotMaterial>>().get(&m.0).unwrap().params, v)
    };
    step(&mut app);
    let (first, vis) = rect_of(&mut app);
    assert_eq!(vis, Visibility::Inherited);
    assert_eq!(first.tint, Vec4::new(1.0, 0.5, 0.25, 1.0), "the node's colour");
    for _ in 0..9 {
        step(&mut app);
    }
    let (later, _) = rect_of(&mut app);
    assert_ne!(first.rect, later.rect, "the flipbook advanced");
    for _ in 0..25 {
        step(&mut app);
    }
    assert_eq!(rect_of(&mut app).1, Visibility::Hidden, "hidden once the flipbook has played");

    // The ring: tint and scale follow `ring_state`; it ends where the rules end it.
    let w = wave("ring");
    let ring_image = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
    let material = app.world_mut().resource_mut::<Assets<StockShotMaterial>>().add(StockShotMaterial { params: ShotParams { rect: Vec4::ZERO, flags: Vec4::ZERO, tint: Vec4::ZERO }, texture: ring_image });
    let now = app.world().resource::<AnimClock>().now() * TICK_HZ as f64;
    let ring = app.world_mut().spawn((StockRing { unit: EntityId(1), born: now, radius: 30.0, wave: w.clone(), visuals: true }, Transform::IDENTITY, MeshMaterial3d(material.clone()))).id();
    for _ in 0..90 {
        step(&mut app);
        let t = (app.world().resource::<AnimClock>().now() * TICK_HZ as f64 - now) as f32 / TICK_HZ as f32;
        let Some((scale, k)) = ring_state(&w, 30.0, t.max(0.0)) else { continue };
        let tint = app.world().resource::<Assets<StockShotMaterial>>().get(&material).unwrap().params.tint;
        assert!((tint - ring_tint(&w, k).unwrap()).abs().max_element() < 1e-3, "at {t} s: {tint}");
        assert!((app.world().get::<Transform>(ring).unwrap().scale.x - scale.max(1e-4)).abs() < 1e-3);
    }
    assert!(app.world().get::<Transform>(ring).unwrap().scale.x > 1.0);
    for _ in 0..200 {
        step(&mut app);
    }
    assert!(app.world().get_entity(ring).is_err(), "the ring ended");
}
