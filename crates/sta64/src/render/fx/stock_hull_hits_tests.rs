//! Tests of the Stock look at a weapon hit on a bare hull and on a dying hull, and of the damage smoke (#1153,
//! D-02). The definitions, sprite tables and content are authored here; time is stepped by hand.
use super::*;
use crate::app::configure_sets;
use crate::render::fx::impacts::{self, ImpactFx, ImpactFxPlugin, SmokeEmitter, HIT_SPARKS};
use crate::render::fx::stock_shots;
use crate::render::sprites::SpriteTables;
use crate::render::units::HullShare;
use crate::sim_link::{ContentRes, SimEntity, SimLink, SimLinkPlugin};
use bevy::time::TimeUpdateStrategy;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_content::{ContentBuilder, UnitDef, WeaponDef, WeaponKind};
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
    text += "other tex_other 0 32 32 32\n@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n";
    SpriteTable::parse(text.as_bytes())
}

fn content() -> sta_content::ContentDb {
    ContentBuilder::default()
        .weapon(WeaponDef::new("t_torp", "photontorpedo", WeaponKind::Projectile))
        .unit(UnitDef::new("t_ship", "cruiser"))
        .build()
        .unwrap()
}

/// Unit 1 shoots; unit 4 is hit and stays; unit 5 is hit, starts to die at tick 10 and is gone at tick 70.
fn run_scene(profile: Option<RenderProfile>, family: bool, events: &str, frames: u32, mut each: impl FnMut(&mut App, u32)) -> App {
    let db = content();
    let torp = db.key("t_torp").unwrap().0;
    let events = events.replace("TORP", &torp.to_string());
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
