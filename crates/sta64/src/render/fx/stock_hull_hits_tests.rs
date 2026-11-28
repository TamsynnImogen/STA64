//! Tests of the Stock look at a weapon hit on a bare hull and on a dying hull, and of the damage smoke (#1153,
//! D-02). The definitions, sprite tables and content are authored here; time is stepped by hand.
use super::*;
use crate::app::configure_sets;
use crate::render::fx::impacts::{self, ImpactFx, ImpactFxPlugin, SmokeEmitter, HIT_SPARKS};
use crate::render::fx::stock_shots;
use crate::render::sprites::SpriteTables;
use crate::render::hulls::HullPart;
use crate::render::units::HullShare;
use crate::sim_link::{ContentRes, SimEntities, SimEntity, SimLink, SimLinkPlugin};
use bevy::time::TimeUpdateStrategy;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_content::{ContentBuilder, ShotLook, UnitDef, WeaponDef, WeaponKind};
use sta_sim::PlayerId;
use sta_sim::command::LocalRouter;
use sta_sim::fake::ScriptedSource;
use std::sync::Arc;
use std::time::Duration;

/// An invented table: one additive sprite for each member of the family (or none), with a flipbook.
fn table(family: bool) -> SpriteTable {
    let mut text = String::from("sprite_table\n@referenceWidth=128\n@referenceHeight=128\n@tmaterial=additive\n");
    if family {
        for name in stock_flares::SPARK_FAMILY {
            text += &format!("{name} tex_{name} 0 0 32 32 @anim=flip4\n");
        }
    }
    text += "beamspr tex_beam 0 64 32 32\ntorpspr tex_torp 0 96 32 32\n";
    text += "other tex_other 0 32 32 32\n@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n";
    SpriteTable::parse(text.as_bytes())
}

fn content() -> sta_content::ContentDb {
    let mut beam = WeaponDef::new("t_beam", "phaser", WeaponKind::Beam);
    beam.range = sta_sim::Fixed::from_int(1000);
    beam.look = ShotLook { sprite: Some("beamspr".into()), radius: Some(4.0), life_span: Some(1.0), travel_time_fraction: Some(0.2), ..Default::default() };
    let mut torpedo = WeaponDef::new("t_torp", "photontorpedo", WeaponKind::Projectile);
    torpedo.range = sta_sim::Fixed::from_int(1000);
    torpedo.shot_speed = sta_sim::Fixed::from_int(3000);
    torpedo.look = ShotLook { sprite: Some("torpspr".into()), radius: Some(4.0), ..Default::default() };
    let mut ship = UnitDef::new("t_ship", "cruiser");
    ship.stats.hull_target_hardpoints = vec!["hull_a".into()];
    ContentBuilder::default()
        .weapon(torpedo)
        .weapon(beam)
        .unit(ship)
        .build()
        .unwrap()
}

/// Unit 1 shoots; unit 4 is hit and stays; unit 5 is hit, starts to die at tick 10 and is gone at tick 70.
fn run_scene(profile: Option<RenderProfile>, family: bool, events: &str, frames: u32, mut each: impl FnMut(&mut App, u32)) -> App {
    let db = content();
    let torp = db.key("t_torp").unwrap().0;
    let beam = db.key("t_beam").unwrap().0;
    let events = events.replace("TORP", &torp.to_string()).replace("BEAM", &beam.to_string());
    let text = format!(
        "{{ units: [
            {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
            {{ id: 4, def: \"t_ship\", owner: 1, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}] }},
            {{ id: 5, def: \"t_ship\", owner: 1, hull: 800, rails: [{{ at: 0, pos: {{ x: 500, y: 0, z: 600 }} }}], until: 70 }},
        ], events: [{events}] }}"
    );
    let src = ScriptedSource::from_json5(&text, &db).unwrap();
    let tables = Arc::new(table(family));
    let sprites = stock_shots::ShotSprites::build(&tables, &db);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<crate::render::fx::quad::FxQuadMaterial>>()
        .init_resource::<Assets<crate::render::fx::shield::ShieldMaterial>>()
        .init_resource::<Assets<crate::render::fx::fire::FireMaterial>>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / TICK_HZ as f64)))
        .add_plugins(SimLinkPlugin)
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(SpriteTables(tables))
        .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
        .add_plugins((crate::render::fx::weapons::WeaponFxPlugin, ImpactFxPlugin));
    if let Some(p) = profile {
        app.insert_resource(p);
    }
    app.insert_resource(sprites);
    configure_sets(&mut app);
    for f in 0..frames {
        app.update();
        each(&mut app, f);
    }
    app
}

fn flares(app: &mut App, kind: FlareKind) -> Vec<Flare> {
    app.world_mut().query::<&Flare>().iter(app.world()).filter(|f| f.kind == kind).cloned().collect()
}

fn hit(at: u32, target: u32, shield: u32, hull: u32) -> String {
    format!("{{ at: {at}, event: {{ Hit: {{ target: {target}, by: 1, weapon: TORP, facing: 0, shield: {shield}, hull: {hull}, at: {{ x: 0, y: 0, z: 590 }} }} }} }},")
}

/// A beam of unit 1 at unit 4 and its hit on the hull, on one tick.
fn beam_hit(at: u32) -> String {
    format!(
        "{{ at: {at}, event: {{ Fired: {{ shooter: 1, weapon: BEAM, hardpoint: 0, target: {{ Entity: 4 }} }} }} }},
         {{ at: {at}, event: {{ Hit: {{ target: 4, by: 1, weapon: BEAM, facing: 0, shield: 0, hull: 40, at: {{ x: 0, y: 0, z: 590 }} }} }} }},"
    )
}

/// A model for unit `id` (a root at the origin) with named nodes and hull parts of the given vertices at `at`.
fn model(app: &mut App, id: u32, nodes: &[(&str, Vec3)], hull: Option<(Vec3, Vec<[f32; 3]>)>) {
    let root = app.world_mut().spawn((GlobalTransform::default(), InheritedVisibility::VISIBLE)).id();
    for (name, at) in nodes {
        app.world_mut().spawn((Name::new(name.to_string()), GlobalTransform::from_translation(*at), ChildOf(root)));
    }
    if let Some((at, vertices)) = hull {
        // The client keeps its own sample of a hull part's vertices (the part's mesh is not readable once drawn).
        app.world_mut().spawn((crate::render::hulls::hull_sample(&vertices), HullPart::default(), GlobalTransform::from_translation(at), ChildOf(root)));
    }
    app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(id), root);
}

fn dying(at: u32, id: u32) -> String {
    format!("{{ at: {at}, event: {{ Dying: {{ id: {id}, by: 1 }} }} }},")
}

fn streaks(app: &mut App, unit: u32) -> usize {
    app.world_mut().query::<&ImpactFx>().iter(app.world()).filter(|i| i.unit.0 == unit && i.kind == impacts::ImpactKind::Spark).count()
}

#[test]
fn the_rules_pick_the_size_and_the_family_member() {
    // Half size: a share of the radius, never under the floor.
    assert_eq!(stock_flares::spark_half(100.0), 100.0 * stock_flares::SPARK_SIZE_FACTOR);
    assert_eq!(stock_flares::spark_half(1.0), stock_flares::SPARK_MIN_HALF);
    let db = content();
    let sprites = stock_shots::ShotSprites::build(&table(true), &db);
    let seen: std::collections::BTreeSet<&str> = (0..100).filter_map(|i| pick(&sprites, i as f32 / 100.0)).collect();
    assert_eq!(seen.len(), stock_flares::SPARK_FAMILY.len(), "every member is reachable: {seen:?}");
    assert!(pick(&sprites, 1.0).is_some() && pick(&sprites, 0.0).is_some());
    // No member in the table: none.
    assert_eq!(pick(&stock_shots::ShotSprites::build(&table(false), &db), 0.5), None);
}

#[test]
fn under_stock_a_hull_hit_raises_one_spark_of_the_family_and_none_of_refits_streaks() {
    let events = hit(20, 4, 0, 40);
    let mut app = run_scene(Some(RenderProfile::Stock), true, &events, 25, |_, _| {});
    let sparks = flares(&mut app, FlareKind::HullHit);
    assert_eq!(sparks.len(), 1, "one spark per hull hit");
    let s = &sparks[0];
    assert_eq!(s.object, EntityId(4));
    assert!(stock_flares::SPARK_FAMILY.contains(&s.sprite.as_str()), "{}", s.sprite);
    assert_eq!(s.half, stock_flares::spark_half(impacts::DEFAULT_RADIUS));
    assert_eq!(s.life, stock_flares::SPARK_LIFE * TICK_HZ as f32);
    assert!(matches!(s.anchor, Anchor::Frame(_)), "it follows the unit");
    assert_eq!(streaks(&mut app, 4), 0, "none of Refit's streaks");
    // A hit the shields took draws no spark of this kind (the cap is the shield's).
    let mut app = run_scene(Some(RenderProfile::Stock), true, &hit(20, 4, 40, 0), 25, |_, _| {});
    assert!(flares(&mut app, FlareKind::HullHit).is_empty());
}

#[test]
fn under_the_remaster_look_a_hull_hit_draws_todays_streaks_and_no_spark() {
    let mut app = run_scene(Some(RenderProfile::Refit), true, &hit(20, 4, 0, 40), 25, |_, _| {});
    assert_eq!(streaks(&mut app, 4), HIT_SPARKS as usize, "today's streaks");
    assert!(flares(&mut app, FlareKind::HullHit).is_empty() && flares(&mut app, FlareKind::DeathSpark).is_empty());
    // No profile resources at all (the other effects' windowless tests): today's streaks as well.
    let mut app = run_scene(None, true, &hit(20, 4, 0, 40), 25, |_, _| {});
    assert_eq!(streaks(&mut app, 4), HIT_SPARKS as usize);
}

#[test]
fn a_hit_on_a_unit_already_dying_draws_none_and_the_lethal_hit_does() {
    // The lethal hit comes with the unit's `Dying` of the same tick; the next one lands on a dying unit.
    let events = format!("{}{}{}", hit(15, 5, 0, 40), dying(15, 5), hit(22, 5, 0, 40));
    let mut app = run_scene(Some(RenderProfile::Stock), true, &events, 25, |_, _| {});
    let sparks = flares(&mut app, FlareKind::HullHit);
    assert_eq!(sparks.len(), 1, "the lethal hit's spark only");
    assert!((sparks[0].born - 15.0).abs() < 2.5, "born at the lethal hit: {}", sparks[0].born);
}

#[test]
fn the_cap_of_four_per_object_is_shared_with_every_node_particle() {
    let events: String = (0..6).map(|_| hit(20, 4, 0, 10)).collect();
    let mut app = run_scene(Some(RenderProfile::Stock), true, &events, 25, |_, _| {});
    assert_eq!(flares(&mut app, FlareKind::HullHit).len(), stock_flares::MAX_PER_OBJECT, "six hits, four sparks");
    assert_eq!(app.world().resource::<NodeParticles>().live(EntityId(4)), stock_flares::MAX_PER_OBJECT);
}

#[test]
fn a_dying_hull_emits_sparks_on_the_interval_within_the_cap_until_it_is_gone() {
    let events = dying(10, 5);
    let (mut max_live, mut born) = (0, std::collections::BTreeSet::new());
    let mut app = run_scene(Some(RenderProfile::Stock), true, &events, 90, |app, _| {
        let live = flares(app, FlareKind::DeathSpark);
        max_live = max_live.max(live.len());
        born.extend(live.iter().map(|f| f.born.to_bits()));
    });
    // A spark is due every 0.05 s, far faster than four can live their second: the cap is full during the
    // phase and refills as each one ends, so over the 60 ticks of the phase more than one cap's worth is made
    // (distinct birth times are counted, so a burst born in one frame counts once).
    assert_eq!(max_live, stock_flares::MAX_PER_OBJECT, "the cap is reached and never passed");
    assert!(born.len() > stock_flares::MAX_PER_OBJECT, "{} birth times over the phase", born.len());
    assert!(flares(&mut app, FlareKind::DeathSpark).is_empty(), "they end with the unit's removal");
    // A unit that does not die draws none; under the Remaster look neither does a dying one.
    let mut quiet = run_scene(Some(RenderProfile::Refit), true, &events, 40, |_, _| {});
    assert!(flares(&mut quiet, FlareKind::DeathSpark).is_empty());
}

#[test]
fn a_table_without_the_family_draws_nothing_under_stock_not_refits_streaks() {
    let events = format!("{}{}", hit(20, 4, 0, 40), dying(10, 5));
    let mut app = run_scene(Some(RenderProfile::Stock), false, &events, 60, |_, _| {});
    assert!(flares(&mut app, FlareKind::HullHit).is_empty() && flares(&mut app, FlareKind::DeathSpark).is_empty());
    assert_eq!(streaks(&mut app, 4), 0);
}

#[test]
fn stock_draws_no_damage_smoke_at_any_hull_level_and_the_remaster_look_keeps_it() {
    for (look, want) in [(RenderProfile::Stock, false), (RenderProfile::Refit, true)] {
        for share in [0.9, 0.4, 0.1] {
            let mut app = run_scene(Some(look), true, "", 0, |_, _| {});
            app.world_mut().spawn((SimEntity(EntityId(4)), HullShare(share)));
            for _ in 0..3 {
                app.update();
            }
            let made = app.world_mut().query::<&SmokeEmitter>().iter(app.world()).count();
            let level = share < 0.5;
            assert_eq!(made > 0, want && level, "{look:?} at hull share {share}");
        }
    }
    // Without the profile resources it is today's rule.
    assert!(damage_smoke(None, None));
}

#[test]
fn a_beams_and_a_projectiles_hull_hit_spark_is_drawn_on_the_hull_node_and_else_at_the_centre() {
    let spark_at = |events: &str, with_node: bool| {
        let mut app = run_scene(Some(RenderProfile::Stock), true, events, 30, |app, f| {
            if f == 0 && with_node {
                model(app, 4, &[("hull_a", Vec3::new(5.0, 0.0, 0.0))], None);
            }
        });
        let sparks = flares(&mut app, FlareKind::HullHit);
        assert_eq!(sparks.len(), 1, "one spark for {events}");
        match sparks[0].anchor {
            Anchor::Frame(local) => local,
            _ => panic!("a spark follows its unit"),
        }
    };
    // A beam: the target hardpoint the shot chose (a node five units off the centre) against the centre.
    let beam = beam_hit(14);
    let (on_node, on_centre) = (spark_at(&beam, true), spark_at(&beam, false));
    assert!(((on_node - on_centre).length() - 5.0).abs() < 0.01, "{on_node} against {on_centre}");
    // A projectile drawn the Stock way, in flight when it hits: aimed at the same node, so the spark is there.
    let fired = "{ at: 10, event: { Fired: { shooter: 1, weapon: TORP, hardpoint: 0, target: { Entity: 4 } } } },";
    let torp = format!("{fired}{}", hit(14, 4, 0, 40));
    let (on_node, on_centre) = (spark_at(&torp, true), spark_at(&torp, false));
    assert!(((on_node - on_centre).length() - 5.0).abs() < 0.01, "{on_node} against {on_centre}");
    // A projectile with no stock shot in flight (none was fired in the view): where the event puts it, the centre.
    let bare = hit(14, 4, 0, 40);
    assert_eq!(spark_at(&bare, true), spark_at(&bare, false));
}

#[test]
fn death_sparks_are_vertices_of_the_drawn_hull_and_else_inside_the_shield_ellipsoid() {
    let events = dying(10, 5);
    let sparks_of = |hull: Option<(Vec3, Vec<[f32; 3]>)>| {
        let mut seen = Vec::new();
        let _ = run_scene(Some(RenderProfile::Stock), true, &events, 40, |app, f| {
            if f == 0 {
                model(app, 5, &[], hull.clone());
            }
            for s in flares(app, FlareKind::DeathSpark) {
                if let Anchor::Frame(local) = s.anchor {
                    seen.push(local);
                }
            }
        });
        seen
    };
    // A hull part whose vertices are a hundred units off the centre: every spark lands on one of them.
    let on_hull = sparks_of(Some((Vec3::new(100.0, 0.0, 0.0), vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])));
    assert!(!on_hull.is_empty());
    assert!(on_hull.iter().all(|l| (99.0..101.5).contains(&l.length())), "{on_hull:?}");
    // No drawn hull: STA64's own stand-in, inside the shield ellipsoid of the radius.
    let bare = sparks_of(None);
    assert!(!bare.is_empty());
    let reach = impacts::DEFAULT_RADIUS * 1.5 + 0.01;
    assert!(bare.iter().all(|l| l.length() <= reach), "{bare:?}");
}

#[test]
fn the_dying_set_and_the_emitters_are_forgotten_at_a_matchs_end() {
    use bevy::ecs::system::RunSystemOnce;
    let mut app = run_scene(Some(RenderProfile::Stock), true, &dying(10, 5), 30, |_, _| {});
    assert!(app.world().resource::<DyingUnits>().0.contains(&EntityId(5)));
    assert!(!app.world().resource::<Emitters>().0.is_empty());
    app.world_mut().run_system_once(forget).unwrap();
    assert!(app.world().resource::<DyingUnits>().0.is_empty() && app.world().resource::<Emitters>().0.is_empty());
}

#[test]
fn a_beam_lost_with_its_target_raises_no_hull_hit_spark_since_it_does_no_damage() {
    // Fired at unit 5 at tick 60, the unit is gone at tick 70, mid-life: the sim delivers no hit, so there is none to draw.
    let fired = "{ at: 60, event: { Fired: { shooter: 1, weapon: BEAM, hardpoint: 0, target: { Entity: 5 } } } },";
    let mut seen = 0;
    let mut app = run_scene(Some(RenderProfile::Stock), true, fired, 90, |app, _| {
        seen += app.world_mut().query::<&stock_shots::StockShot>().iter(app.world()).count();
    });
    assert!(seen > 0, "the beam was drawn");
    assert!(flares(&mut app, FlareKind::HullHit).is_empty() && app.world().resource::<HullHitPoints>().0.is_empty());
}

/// The torpedo of unit 1 at unit 4 fired at tick 10 and landing at tick 16 (its flight is six ticks): its last drawn
/// place and the hull-hit spark's, in the world.
fn torpedo_end_and_spark(with_node: bool) -> (Vec3, Vec3) {
    let fired = "{ at: 10, event: { Fired: { shooter: 1, weapon: TORP, hardpoint: 0, target: { Entity: 4 } } } },";
    let events = format!("{fired}{}", hit(15, 4, 0, 40));
    let mut last = None;
    let mut app = run_scene(Some(RenderProfile::Stock), true, &events, 30, |app, f| {
        if f == 0 && with_node {
            model(app, 4, &[("hull_a", Vec3::new(5.0, 0.0, 0.0))], None);
        }
        for s in app.world_mut().query::<&stock_shots::StockShot>().iter(app.world()) {
            if let stock_shots::Geom::Billboard { c } = s.geom {
                last = Some(c);
            }
        }
    });
    let sparks = flares(&mut app, FlareKind::HullHit);
    assert_eq!(sparks.len(), 1);
    (last.expect("the torpedo was drawn"), sparks[0].at)
}

#[test]
fn the_drawn_projectile_ends_where_its_hull_hit_spark_is_at_the_node_and_at_the_centre_without_one() {
    // The torpedo flies from the shooter (at the origin) along a straight line, a share of the way to its aim point
    // when last drawn: that share of the spark's place, to within a tenth of a unit.
    let on_line = |(end, spark): (Vec3, Vec3)| {
        let share = end.z / spark.z;
        assert!(share > 0.5 && share <= 1.0, "a share {share} of the way");
        end.distance(spark * share) < 0.1
    };
    let with = torpedo_end_and_spark(true);
    assert!(on_line(with), "the torpedo {} and the spark {} are not on one line from the shooter", with.0, with.1);
    let bare = torpedo_end_and_spark(false);
    assert!(on_line(bare), "without nodes: {} and {}", bare.0, bare.1);
    // The node is five units off the centre: the sparks differ by that.
    assert!((with.1.distance(bare.1) - 5.0).abs() < 0.5, "{} against {}", with.1, bare.1);
}

#[test]
fn two_hits_of_one_weapon_on_one_target_in_one_frame_take_two_shots() {
    let fired = "{ at: 10, event: { Fired: { shooter: 1, weapon: TORP, hardpoint: 0, target: { Entity: 4 } } } },";
    let events = format!("{fired}{fired}{}{}", hit(14, 4, 0, 10), hit(14, 4, 0, 10));
    let mut counts = Vec::new();
    let _ = run_scene(Some(RenderProfile::Stock), true, &events, 22, |app, _| {
        counts.push(app.world_mut().query::<&stock_shots::StockShot>().iter(app.world()).count());
    });
    assert!(counts.contains(&2), "both torpedoes were drawn: {counts:?}");
    let two = counts.iter().rposition(|c| *c == 2).unwrap();
    assert_eq!(counts[two + 1], 0, "both shots ended on their hits: {counts:?}");
}

/// #1426: the death sparks never read a hull mesh the renderer has taken. A unit's hull part whose mesh is in the extracted
/// state (put there by the engine's own call) and that has no sample of its own: the sparks go on, inside the shield
/// ellipsoid, and nothing panics.
#[test]
fn death_sparks_do_not_read_a_hull_mesh_the_renderer_has_taken() {
    let mut app = run_scene(Some(RenderProfile::Stock), true, &dying(10, 5), 40, |app, f| {
        if f == 0 {
            model(app, 5, &[], None);
            let root = app.world().resource::<SimEntities>().0[&EntityId(5)];
            let mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD)
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[100.0, 0.0, 0.0]; 3]);
            let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
            app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(&handle).unwrap().take_gpu_data().expect("the engine's extraction");
            app.world_mut().spawn((Mesh3d(handle), HullPart::default(), GlobalTransform::default(), ChildOf(root)));
        }
    });
    assert!(!flares(&mut app, FlareKind::DeathSpark).is_empty(), "the sparks went on");
}

/// A sample is an even stride through the vertices, all of them when there are few (the memory cost: a few KB a part).
#[test]
fn a_hull_sample_is_all_the_vertices_when_few_and_an_even_stride_when_many() {
    use crate::render::hulls::{HULL_SAMPLE_MAX, hull_sample};
    let few: Vec<[f32; 3]> = (0..10).map(|i| [i as f32, 0.0, 0.0]).collect();
    assert_eq!((&*hull_sample(&few).points, hull_sample(&few).full), (&few[..], 10));
    let many: Vec<[f32; 3]> = (0..HULL_SAMPLE_MAX * 4).map(|i| [i as f32, 0.0, 0.0]).collect();
    let sample = hull_sample(&many);
    assert_eq!(sample.points.len(), HULL_SAMPLE_MAX);
    assert_eq!(sample.full, many.len(), "the part's whole vertex count is kept beside the sample");
    assert_eq!(sample.points[0], many[0]);
    assert_eq!(sample.points[1][0], 4.0, "every fourth vertex");
}

/// A unit's parts are weighted by their whole vertex counts, not by the sizes of their samples: a part of a hundred
/// vertices beside one of three thousand gets about a thirtieth of the picks, as when every vertex of the model was
/// equally likely (the sample cap must not make small parts over-represented).
#[test]
fn parts_are_picked_by_their_whole_vertex_counts_not_their_sample_sizes() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    let root = world.spawn(GlobalTransform::default()).id();
    let part = |world: &mut World, x: f32, count: usize| {
        let vertices: Vec<[f32; 3]> = (0..count).map(|_| [x, 0.0, 0.0]).collect();
        world.spawn((crate::render::hulls::hull_sample(&vertices), HullPart::default(), GlobalTransform::default(), ChildOf(root)));
    };
    part(&mut world, 1000.0, 3000);
    part(&mut world, -1000.0, 100);
    let picks = 3100;
    let small = world
        .run_system_once(move |h: HullVertices| (0..picks).filter(|i| h.pick(root, (*i as f32 + 0.5) / picks as f32).unwrap().x < 0.0).count())
        .unwrap();
    assert_eq!(small, 100, "one pick in thirty-one, not the 100 of 356 a sample-weighted choice gives");
}

