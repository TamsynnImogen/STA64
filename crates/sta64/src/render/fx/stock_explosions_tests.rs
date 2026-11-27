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
    let d = explosion("a", Some(3.0), Some("ring"), Some(0.6), Some(40.0));
    assert!(!shockwave_due(&d, true, 0.5, 60.0), "before the delay");
    assert!(shockwave_due(&d, true, 0.7, 60.0));
    assert!(!shockwave_due(&d, true, 0.7, 40.0), "the radius must be above the minimum");
    assert!(!shockwave_due(&d, false, 0.7, 60.0), "no shockwave named");
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
    assert!(once_uv(&table, fixed, 99.0).is_some(), "a sprite with no flipbook stays for the explosion's life");
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
        [explosion("boom_ship", Some(1.0), Some("ring"), Some(0.2), Some(10.0)), explosion("boom_base", Some(3.0), Some("ring"), Some(0.2), Some(40.0))],
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
fn under_stock_a_unit_a_building_and_a_hull_hit_ask_for_the_stock_effect() {
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
