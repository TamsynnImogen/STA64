//! The sprite nodes' animation: [`super::animate`] against [`animate_reference`], a verbatim copy of
//! `main`'s, which any later change of how a node's colour reaches the screen is held to; and the
//! adopted nodes of ADR 0009 (slots, looks, demotion for the cloak and build fade and the ghost)
//! against the same reference, without rendering.
use super::material::{self, SpriteMaterial, SpriteSlots, SpriteState};
use super::*;
use crate::clock::{AnimClock, ClockMode};
use crate::render::fx::specials::{UnitFade, apply_fade};
use crate::ui::hud::ghost::{Ghost, tint_ghost};
use bevy::mesh::MeshTag;
use bevy::render::storage::ShaderBuffer;
use sta_assets::a2fmt::spr::SpriteTable;

/// An invented table: flipbook and colour-cycling sprites, a blinking node, a still one.
pub(super) const TABLE: &str = "sprite_table\n@referenceWidth=128\n@referenceHeight=64\n@tmaterial=additive\n\
glow tex_a 0 0 32 32 @anim=flip4\n\
dot tex_b 0 32 32 32\n\
@tmaterial=alpha\n\
bl_a tex_a 0 0 32 32\n\
bl_b tex_b 0 32 32 32\n\
@tmaterial=additive\n\
ad_a tex_a 0 0 32 32\n\
ad_b tex_b 0 32 32 32\n\
@tmaterial=alphathreshold\n\
cut_a tex_a 0 0 32 32\n\
cut_b tex_b 0 32 32 32\n\
@sprite_node puffA glow pulse (2,2) (0.5,0.5,0.5) billboard\n\
@sprite_node puffB glow pulse (2,2) (1,0.5,0.25) billboard\n\
@sprite_node still dot const (3,3) (0.2,0.4,0.8)\n\
@sprite_node blink dot strobe (1,1) (1,1,1)\n\
@sprite_node blA bl_a pulse (2,2) (1,0.3,0.3) billboard\n\
@sprite_node blB bl_b const (2,2) (0.3,1,0.4) billboard\n\
@sprite_node blC bl_b pulse (2,2) (0.3,1,0.4) billboard\n\
@sprite_node adC ad_b pulse (2,2) (1,0.8,0.2) billboard\n\
@sprite_node adA ad_a pulse (2,2) (0.6,0.4,1) billboard\n\
@sprite_node adB ad_b const (2,2) (1,0.8,0.2) billboard\n\
@sprite_node cutA cut_a const (3,3) (0.9,0.9,0.9)\n\
@sprite_node cutB cut_b pulse (3,3) (0.5,1,1)\n\
@sprite_node blinkGlow glow strobe (1,1) (1,1,1) billboard\n\
@sprite_node stillGlow glow const (2,2) (0.5,0.5,0.5) billboard\n\
@animation pulse\ncolour 2 1.0 linear\n@keyframes\n0.0 (1,0.25,0.5)\n0.5 (0.25,0.5,1)\n\
@animation strobe\ndraw 2 2.0 step\n@keyframes\n0.0 1\n0.3 0\n\
@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n";

pub(super) fn table() -> SpriteTables {
    SpriteTables(Arc::new(SpriteTable::parse(TABLE.as_bytes())))
}

/// The node ids of [`TABLE`], as `spawn_sprite_node` reads them.
pub(super) const NODES: [&str; 4] = ["puffA", "puffB", "still", "blink"];

/// Nodes of the other blend modes: alpha blend, additive and cut-out, two looks each.
pub(super) const BLENDS: [&str; 6] = ["blA", "blB", "adA", "adB", "cutA", "cutB"];

/// What `model_scene::spawn_sprite_node` makes of a node, without a texture (the tests have none).
pub(super) fn material_of(t: &SpriteTables, node: &str) -> (StandardMaterial, SpriteFx, Handle<StandardMaterial>) {
    let def = t.0.sprite_node(node).unwrap();
    let sprite = t.0.sprite(&def.sprite).unwrap();
    let m = StandardMaterial {
        base_color: Color::srgb(def.colour[0], def.colour[1], def.colour[2]),
        alpha_mode: alpha_mode(spr_anim::sprite_blend(&sprite.material)),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        uv_transform: uv_affine(spr_anim::sprite_uv(&t.0, sprite, 0.0)),
        ..default()
    };
    (m, SpriteFx { node_anim: def.animation.clone(), sprite: def.sprite.clone(), colour: def.colour, material: Handle::default() }, Handle::default())
}

/// `super::animate` as it is on `main` (visibility, and colour and uv of every node written into
/// its own material every frame). Kept verbatim to compare against.
pub(super) fn animate_reference(
    clock: Res<AnimClock>,
    tables: Res<SpriteTables>,
    mut fx: Query<(&SpriteFx, Option<&SpriteFade>, Option<&crate::render::rock::MoonNimbus>, &mut Visibility)>,
    flip: Query<&UvFlipbook>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = clock.wall() as f32;
    let table = &tables.0;
    for (f, fade, nimbus, mut vis) in &mut fx {
        let mut col = f.colour;
        let mut shown = nimbus.is_none_or(|n| n.on);
        if let Some(a) = f.node_anim.as_deref().and_then(|a| table.animation(a)) {
            match spr_anim::value(a, t, spr_anim::LOOP) {
                Some(Value::Draw(false)) => shown = false,
                Some(Value::Colour(c)) => col = c,
                _ => {}
            }
        }
        if let Some(k) = fade {
            col = col.map(|c| c * k.0);
        }
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if !shown {
            continue;
        }
        if let Some(sprite) = table.sprite(&f.sprite) {
            let (c, uv) = (colour(col), uv_affine(spr_anim::sprite_uv(table, sprite, t)));
            if materials.get(&f.material).is_some_and(|m| m.base_color != c || m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.base_color = c;
                m.uv_transform = uv;
            }
        }
    }
    for f in &flip {
        if let Some(a) = table.animation(&f.animation) {
            let o = spr_anim::offset_of(spr_anim::value(a, t + f.offset, spr_anim::LOOP));
            let uv = Affine2::from_translation(Vec2::new(o[0], o[1]));
            if materials.get(&f.material).is_some_and(|m| m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.uv_transform = uv;
            }
        }
    }
}

/// The fade of puff `i` at frame `n`: not 1 for some, exactly 1 for others, 0 sometimes, none for a quarter.
pub(super) fn fade_of(i: usize, n: usize) -> Option<f32> {
    match i % 4 {
        0 => Some(1.0),
        1 => Some(0.3),
        2 => Some([1.0, 0.3, 0.7, 1.0, 0.0][(n + i) % 5]),
        _ => None,
    }
}

fn bits(c: Color) -> [u32; 4] {
    let c = c.to_srgba();
    [c.red.to_bits(), c.green.to_bits(), c.blue.to_bits(), c.alpha.to_bits()]
}

fn uv_bits(a: Affine2) -> Vec<u32> {
    a.to_cols_array().iter().map(|v| v.to_bits()).collect()
}

const PUFFS: usize = 64;

fn base_app(reference: bool) -> App {
    let mut app = App::new();
    app.init_resource::<Assets<StandardMaterial>>().insert_resource(table()).insert_resource(AnimClock::new(ClockMode::Bench { step: 0.173 }, 0));
    if reference {
        app.add_systems(Update, animate_reference);
    } else {
        app.add_systems(Update, animate);
    }
    app
}

fn spawn_puffs(app: &mut App) -> Vec<Entity> {
    let t = table();
    (0..PUFFS)
        .map(|i| {
            let (m, mut fx, _) = material_of(&t, NODES[i % NODES.len()]);
            let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
            fx.material = h.clone();
            app.world_mut().spawn((fx, MeshMaterial3d(h), Visibility::default())).id()
        })
        .collect()
}

fn set_frame(app: &mut App, puffs: &[Entity], n: usize) {
    app.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
    for (i, &e) in puffs.iter().enumerate() {
        let mut en = app.world_mut().entity_mut(e);
        match fade_of(i, n) {
            Some(k) => {
                en.insert(SpriteFade(k));
            }
            None => {
                en.remove::<SpriteFade>();
            }
        }
    }
}

#[test]
fn animate_gives_every_shown_node_the_colour_and_uv_of_the_reference_at_every_frame() {
    let (mut old, mut new) = (base_app(true), base_app(false));
    let (po, pn) = (spawn_puffs(&mut old), spawn_puffs(&mut new));
    let mut compared = 0;
    for n in 0..60 {
        set_frame(&mut old, &po, n);
        set_frame(&mut new, &pn, n);
        old.update();
        new.update();
        for i in 0..PUFFS {
            // What the nodes' visibility asks for is the same for every node, drawn or not.
            assert_eq!(old.world().get::<Visibility>(po[i]), new.world().get::<Visibility>(pn[i]), "visibility of {i} at {n}");
            if old.world().get::<Visibility>(po[i]) == Some(&Visibility::Hidden) {
                continue;
            }
            let mo = old.world().resource::<Assets<StandardMaterial>>().get(&old.world().get::<SpriteFx>(po[i]).unwrap().material).unwrap();
            let mn = new.world().resource::<Assets<StandardMaterial>>().get(&new.world().get::<SpriteFx>(pn[i]).unwrap().material).unwrap();
            assert_eq!(bits(mo.base_color), bits(mn.base_color), "colour of {i} at {n}");
            assert_eq!(uv_bits(mo.uv_transform), uv_bits(mn.uv_transform), "uv of {i} at {n}");
            compared += 1;
        }
    }
    assert!(compared > 500, "{compared} comparisons");
}

#[test]
fn the_invented_table_has_nodes_of_every_blend_mode() {
    let t = table();
    let mode = |n: &str| material_of(&t, n).0.alpha_mode;
    assert_eq!((mode("blA"), mode("blB")), (AlphaMode::Blend, AlphaMode::Blend));
    assert_eq!((mode("adA"), mode("adB")), (AlphaMode::Add, AlphaMode::Add));
    assert_eq!((mode("cutA"), mode("cutB")), (AlphaMode::Mask(0.5), AlphaMode::Mask(0.5)));
}

/// The `Update` schedule of the map, specials and HUD plugins built together, as the game does.
/// `contradiction` adds the opposite of one of the explicit orders (`animate` before `apply_fade`, `animate`
/// before `tint_ghost`), which must make the build fail.
fn build_game_update_schedule(contradiction: Option<&str>) -> Result<(), String> {
    use bevy::state::app::StatesPlugin;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::asset::AssetPlugin::default()));
    crate::app::configure_sets(&mut app);
    app.insert_resource(AnimClock::new(ClockMode::Bench { step: 0.1 }, 0))
        .add_plugins((crate::sim_link::SimLinkPlugin, crate::map::MapPlugin, crate::render::fx::specials::SpecialsFxPlugin, crate::ui::hud::HudPlugin));
    match contradiction {
        Some("apply_fade") => app.add_systems(Update, crate::render::fx::specials::apply_fade.before(animate)),
        Some("tint_ghost") => app.add_systems(Update, crate::ui::hud::ghost::tint_ghost.before(animate)),
        _ => &mut app,
    };
    match app.world_mut().try_schedule_scope(Update, |world, schedule| schedule.initialize(world).map(|_| ())) {
        Ok(Ok(())) => Ok(()),
        other => Err(format!("{other:?}")),
    }
}

/// The map's animation chain is ordered before `apply_fade` and `tint_ghost`, so every system ahead of
/// `animate` in that chain now runs before the sets after `Interpolate`. Bevy refuses an ordering cycle
/// when the schedule is built, which would be a startup panic for every player.
#[test]
fn the_map_specials_and_hud_plugins_build_one_update_schedule_without_an_ordering_cycle() {
    build_game_update_schedule(None).expect("the Update schedule builds");
}

/// Control for the test above: the opposite of each explicit order is a cycle and the build reports it, so
/// the test above can fail, and both orders really are in the schedule.
#[test]
fn the_schedule_check_sees_an_ordering_cycle_against_each_explicit_order() {
    assert!(build_game_update_schedule(Some("apply_fade")).is_err());
    assert!(build_game_update_schedule(Some("tint_ghost")).is_err());
}

/// An app with the game's animation, cloak and build fade and ghost tint, without rendering. With
/// `adopting`, sprite nodes are adopted as in the game (the assets, the slots, the observers) and
/// animated by [`animate`]; without, they keep their own materials, animated by [`animate_reference`].
fn game_app(adopting: bool) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<StandardMaterial>()
        .init_asset::<SpriteMaterial>()
        .init_asset::<ShaderBuffer>()
        .insert_resource(table())
        .insert_resource(AnimClock::new(ClockMode::Bench { step: 0.173 }, 0));
    if adopting {
        // Small, so the nodes below grow it.
        let slots = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), 4);
        app.insert_resource(slots);
        material::adoption(&mut app);
        see_unhidden(&mut app);
        app.add_systems(Update, (animate, apply_fade, tint_ghost).chain());
    } else {
        app.add_systems(Update, (animate_reference, apply_fade, tint_ghost).chain());
    }
    app
}

/// A stand-in for the visibility systems in an app without rendering, so the parity mark
/// ([`material::mark_seen_changes`]) has views to ask: one view sees every node that is not hidden and
/// has no hidden ancestor. Runs before the mark.
fn see_unhidden(app: &mut App) {
    fn system(mut nodes: Query<(Entity, &mut ViewVisibility)>, vis: Query<&Visibility>, parents: Query<&ChildOf>) {
        for (node, mut seen) in &mut nodes {
            let (mut at, mut hidden) = (node, false);
            loop {
                hidden |= vis.get(at) == Ok(&Visibility::Hidden);
                match parents.get(at) {
                    Ok(p) => at = p.parent(),
                    Err(_) => break,
                }
            }
            let want = if hidden { ViewVisibility::HIDDEN } else { ViewVisibility::VISIBLE };
            if *seen != want {
                *seen = want;
            }
        }
    }
    app.add_systems(PostUpdate, system.before(material::mark_seen_changes));
}

/// A sprite node as `model_scene::spawn_sprite_node` makes it (no texture), under `parent`.
fn spawn_node(app: &mut App, node: &str, parent: Option<Entity>) -> Entity {
    let (m, mut fx, _) = material_of(&table(), node);
    let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
    fx.material = h.clone();
    let mut e = app.world_mut().spawn((fx, MeshMaterial3d(h), Visibility::default()));
    if let Some(p) = parent {
        e.insert(ChildOf(p));
    }
    e.id()
}

/// A material field by field. `Debug` prints every field, floats in their shortest exact form, so two
/// equal strings are bit-equal materials (no texture handles here: the nodes have none).
fn fields(m: &StandardMaterial) -> String {
    format!("{m:?}")
}

/// The reference node's own material.
fn own_material(app: &App, e: Entity) -> StandardMaterial {
    let h = &app.world().get::<MeshMaterial3d<StandardMaterial>>(e).expect("the reference node's material").0;
    app.world().resource::<Assets<StandardMaterial>>().get(h).unwrap().clone()
}

fn slot_of(app: &App, e: Entity) -> [u32; 4] {
    let tag = app.world().get::<MeshTag>(e).expect("an adopted node's tag");
    let colour = app.world().resource::<SpriteSlots>().read(app.world().resource::<Assets<ShaderBuffer>>(), tag).unwrap();
    colour.map(f32::to_bits)
}

fn linear_bits(c: Color) -> [u32; 4] {
    material::linear(c).map(f32::to_bits)
}

/// What node `en` of the adopting app draws equals what node `eo` of the reference app draws: through
/// a private material, the same material field by field; through a look, the reference's colour in
/// its slot (bit-equal to Bevy's conversion of it) and the look equal to the reference's material in
/// everything else.
fn assert_drawn_equal(old: &App, eo: Entity, new: &App, en: Entity, what: &str) {
    let mo = own_material(old, eo);
    if let Some(private) = new.world().get::<MeshMaterial3d<StandardMaterial>>(en) {
        let mn = new.world().resource::<Assets<StandardMaterial>>().get(&private.0).unwrap();
        assert_eq!(fields(mn), fields(&mo), "{what}: private material");
        return;
    }
    let look = &new.world().get::<MeshMaterial3d<SpriteMaterial>>(en).expect("on a look or demoted").0;
    let look = new.world().resource::<Assets<SpriteMaterial>>().get(look).unwrap();
    assert_eq!(slot_of(new, en), linear_bits(mo.base_color), "{what}: slot");
    assert_eq!(fields(&StandardMaterial { base_color: mo.base_color, ..look.base.clone() }), fields(&mo), "{what}: look");
}

/// ADR 0009 step 2 gate (CPU parity): at every frame, an adopted node's slot is bit-equal to Bevy's
/// conversion of the colour the reference writes into the node's own material, a shown node draws
/// through its look whose uv is the reference's, and the material a demotion would build equals the
/// reference's own material field by field, shown or not.
#[test]
fn an_adopted_nodes_slot_look_and_demoted_material_equal_the_reference_at_every_frame() {
    let (mut old, mut new) = (game_app(false), game_app(true));
    let (po, pn) = (spawn_puffs(&mut old), spawn_puffs(&mut new));
    assert_eq!(new.world().resource::<SpriteSlots>().live(), PUFFS as u32, "every node holds a slot from its spawn on");
    let mut shown = 0;
    for n in 0..60 {
        set_frame(&mut old, &po, n);
        set_frame(&mut new, &pn, n);
        old.update();
        new.update();
        for i in 0..PUFFS {
            let what = format!("node {i} at frame {n}");
            let vis = old.world().get::<Visibility>(po[i]);
            assert_eq!(vis, new.world().get::<Visibility>(pn[i]), "{what}: visibility");
            let mo = own_material(&old, po[i]);
            let st = new.world().get::<SpriteState>(pn[i]).expect("adopted");
            assert_eq!(slot_of(&new, pn[i]), linear_bits(mo.base_color), "{what}: slot");
            let demoted = material::demoted(st, new.world().resource::<Assets<SpriteMaterial>>()).unwrap();
            assert_eq!(fields(&demoted), fields(&mo), "{what}: demoted material");
            if vis != Some(&Visibility::Hidden) {
                assert_eq!(new.world().get::<MeshMaterial3d<SpriteMaterial>>(pn[i]).map(|m| m.0.id()), Some(st.look.id()), "{what}: drawn through its look");
                assert_drawn_equal(&old, po[i], &new, pn[i], &what);
                shown += 1;
            }
        }
    }
    assert!(shown > 500, "{shown} comparisons");
    // The nodes share their sprites' looks: two sprites, two materials.
    assert_eq!(new.world().resource::<Assets<SpriteMaterial>>().len(), 2);
}

/// Modified events per frame, by asset type.
#[derive(Resource, Default, Debug, Clone, Copy)]
struct Modified {
    standard: usize,
    sprite: usize,
    buffer: usize,
}

fn count_modified(
    mut counts: ResMut<Modified>,
    mut standard: MessageReader<AssetEvent<StandardMaterial>>,
    mut sprite: MessageReader<AssetEvent<SpriteMaterial>>,
    mut buffer: MessageReader<AssetEvent<ShaderBuffer>>,
) {
    *counts = Modified {
        standard: standard.read().filter(|e| matches!(e, AssetEvent::Modified { .. })).count(),
        sprite: sprite.read().filter(|e| matches!(e, AssetEvent::Modified { .. })).count(),
        buffer: buffer.read().filter(|e| matches!(e, AssetEvent::Modified { .. })).count(),
    };
}

/// ADR 0009 step 2 gate (CPU guard): once adopted, cycling colours, fades and a flipbook modify no
/// `StandardMaterial`, and `SpriteMaterial`s at most once per look per frame; the colours go into the
/// slot buffer. A write that went through to a material shows here as a `StandardMaterial` event or
/// one `SpriteMaterial` event too many.
#[test]
fn in_steady_state_sprite_nodes_modify_no_standard_material_and_each_look_at_most_once_a_frame() {
    let mut app = game_app(true);
    app.init_resource::<Modified>().add_systems(Last, count_modified);
    let puffs = spawn_puffs(&mut app);
    for n in 0..3 {
        set_frame(&mut app, &puffs, n);
        app.update();
    }
    let looks = app.world().resource::<Assets<SpriteMaterial>>().len();
    let mut total = Modified::default();
    for n in 3..63 {
        set_frame(&mut app, &puffs, n);
        app.update();
        let m = *app.world().resource::<Modified>();
        assert_eq!(m.standard, 0, "frame {n}: StandardMaterial modified");
        assert!(m.sprite <= looks, "frame {n}: {} SpriteMaterial events for {looks} looks", m.sprite);
        total.standard += m.standard;
        total.sprite += m.sprite;
        total.buffer += m.buffer;
    }
    // The guard watched something: the flipbook's look moved and the colours went into the slots.
    assert!(total.sprite > 0 && total.buffer > 0, "{total:?}");
}

/// ADR 0009 step 2 gate: the cloak and build fade and the placement ghost give adopted nodes copies
/// equal to `main`'s field by field, frame by frame: also for a node blinking off (so holding its
/// last shown colour) when the cloak begins, and after the fade ends, when the nodes are back on
/// their looks with their slots rewritten. A node's tag is the same through demotion and restore.
#[test]
fn fade_and_ghost_copies_of_adopted_nodes_equal_mains_field_by_field() {
    let (mut old, mut new) = (game_app(false), game_app(true));
    let family = |app: &mut App| {
        let unit = app.world_mut().spawn((UnitFade { cloak: 1.0, build: 1.0 }, Visibility::default())).id();
        let unit_nodes: Vec<Entity> = NODES.iter().map(|n| spawn_node(app, n, Some(unit))).collect();
        let ghost = app.world_mut().spawn((Ghost::pinned(sta_sim::ContentKey::NONE), Visibility::default())).id();
        let ghost_nodes: Vec<Entity> = NODES.iter().map(|n| spawn_node(app, n, Some(ghost))).collect();
        (unit, unit_nodes, ghost_nodes)
    };
    let (uo, uno, gno) = family(&mut old);
    let (un, unn, gnn) = family(&mut new);
    let blink = NODES.iter().position(|&n| n == "blink").unwrap();
    let tags: Vec<MeshTag> = unn.iter().map(|&e| new.world().get::<MeshTag>(e).unwrap().clone()).collect();
    // Cloak from the frame after the blinking node is first seen off (it stays off for several
    // frames), fade on, decloak, then cloak once more.
    let after_start = [0.5, 0.4, 0.3, 0.3, 0.2, 1.0, 1.0, 1.0, 0.6, 0.7, 1.0];
    let mut began = None;
    let mut seen_off = false;
    let mut demoted_frames = 0;
    for n in 0..80 {
        let cloak = match began {
            None if seen_off => {
                began = Some(n);
                after_start[0]
            }
            None => 1.0,
            Some(b) => after_start.get(n - b).copied().unwrap_or(1.0),
        };
        if old.world().get::<UnitFade>(uo).unwrap().cloak != cloak {
            old.world_mut().get_mut::<UnitFade>(uo).unwrap().cloak = cloak;
            new.world_mut().get_mut::<UnitFade>(un).unwrap().cloak = cloak;
        }
        old.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        new.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        old.update();
        new.update();
        if began == Some(n) {
            assert_eq!(old.world().get::<Visibility>(uno[blink]), Some(&Visibility::Hidden), "the blinking node is off when the cloak begins");
        }
        seen_off |= old.world().get::<Visibility>(uno[blink]) == Some(&Visibility::Hidden);
        for (i, (&eo, &en)) in uno.iter().zip(&unn).enumerate() {
            let what = format!("unit node {i} at frame {n}");
            assert_eq!(old.world().get::<Visibility>(eo), new.world().get::<Visibility>(en), "{what}: visibility");
            assert_eq!(new.world().get::<MeshTag>(en), Some(&tags[i]), "{what}: tag");
            if cloak < 1.0 {
                assert!(new.world().get::<MeshMaterial3d<StandardMaterial>>(en).is_some(), "{what}: demoted");
                assert_drawn_equal(&old, eo, &new, en, &what);
                demoted_frames += 1;
            } else {
                assert!(new.world().get::<MeshMaterial3d<SpriteMaterial>>(en).is_some(), "{what}: on its look");
                if old.world().get::<Visibility>(eo) != Some(&Visibility::Hidden) {
                    assert_drawn_equal(&old, eo, &new, en, &what);
                }
            }
        }
        for (i, (&eo, &en)) in gno.iter().zip(&gnn).enumerate() {
            assert!(new.world().get::<MeshMaterial3d<StandardMaterial>>(en).is_some(), "ghost node {i} at frame {n}: demoted");
            assert_drawn_equal(&old, eo, &new, en, &format!("ghost node {i} at frame {n}"));
        }
    }
    assert!(began.is_some() && demoted_frames >= 7 * NODES.len(), "the cloak ran ({began:?}, {demoted_frames})");
    // The ghost's and the unit's nodes hold their slots; nothing leaked.
    assert_eq!(new.world().resource::<SpriteSlots>().live(), 2 * NODES.len() as u32);
}

/// Marks the node [`spawn_late`] spawned.
#[derive(Component)]
struct Late;

/// At which update [`spawn_late`] spawns.
#[derive(Resource)]
struct LateAt(usize);

/// Spawn a flipbook node after the animation, as a unit's model can be spawned after `animate` has
/// run in the game.
fn spawn_late(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>, at: Res<LateAt>, mut frame: Local<usize>) {
    *frame += 1;
    if *frame == at.0 {
        let (m, mut fx, _) = material_of(&table(), "puffA");
        let h = materials.add(m);
        fx.material = h.clone();
        commands.spawn((fx, MeshMaterial3d(h), Visibility::default(), Late));
    }
}

/// A node spawned after `animate` is drawn that frame with the uv it was spawned with, as its own
/// material is on `main`; on the adopted path that is a private copy of its look while the look's
/// flipbook has moved on, and the node joins the look at its next shown frame.
#[test]
fn a_node_spawned_after_the_animation_draws_its_spawn_uv_then_joins_its_look() {
    let (mut old, mut new) = (game_app(false), game_app(true));
    let (po, pn) = (spawn_puffs(&mut old), spawn_puffs(&mut new));
    // An update at which the glow's flipbook is away from its first cell.
    let at = 3;
    for app in [&mut old, &mut new] {
        app.insert_resource(LateAt(at)).add_systems(Update, spawn_late.after(tint_ghost));
    }
    let late = |app: &mut App| app.world_mut().query_filtered::<Entity, With<Late>>().iter(app.world()).next();
    for n in 0..8 {
        set_frame(&mut old, &po, n);
        set_frame(&mut new, &pn, n);
        old.update();
        new.update();
        let (Some(eo), Some(en)) = (late(&mut old), late(&mut new)) else { continue };
        let st = new.world().get::<SpriteState>(en).unwrap();
        let drawn = new.world().get::<MeshMaterial3d<SpriteMaterial>>(en).unwrap().0.id();
        let shown = old.world().get::<Visibility>(eo) != Some(&Visibility::Hidden);
        if n + 1 == at {
            assert_ne!(drawn, st.look.id(), "spawned after the animation, on a private copy");
        } else if shown {
            assert_eq!(drawn, st.look.id(), "frame {n}: on its look");
        }
        if shown {
            assert_drawn_equal(&old, eo, &new, en, &format!("late node at frame {n}"));
        }
    }
    assert!(late(&mut new).is_some());
    // The private copy is gone once the node joined its look.
    assert_eq!(new.world().resource::<Assets<SpriteMaterial>>().len(), 2);
}

/// The sprite nodes whose material Bevy re-specialises and re-queues this frame: those whose material
/// component changed, or whose material asset was modified (Bevy's `AssetChanged` on
/// `MeshMaterial3d`).
#[derive(Resource, Default)]
struct Requeued(std::collections::HashSet<Entity>);

fn collect_requeued(
    mut out: ResMut<Requeued>,
    mut standard: MessageReader<AssetEvent<StandardMaterial>>,
    mut sprite: MessageReader<AssetEvent<SpriteMaterial>>,
    own: Query<(Entity, Ref<MeshMaterial3d<StandardMaterial>>), With<SpriteFx>>,
    looks: Query<(Entity, Ref<MeshMaterial3d<SpriteMaterial>>), With<SpriteFx>>,
) {
    let standard: std::collections::HashSet<_> = standard.read().filter_map(|e| if let AssetEvent::Modified { id } = e { Some(*id) } else { None }).collect();
    let sprite: std::collections::HashSet<_> = sprite.read().filter_map(|e| if let AssetEvent::Modified { id } = e { Some(*id) } else { None }).collect();
    out.0 = own
        .iter()
        .filter(|(_, m)| m.is_changed() || standard.contains(&m.0.id()))
        .map(|(e, _)| e)
        .chain(looks.iter().filter(|(_, m)| m.is_changed() || sprite.contains(&m.0.id())).map(|(e, _)| e))
        .collect();
}

/// Orders the nodes [`spawn_late_each`] spawned.
#[derive(Component)]
struct LateIdx(usize);

/// Spawn a still node on the flipbook sprite after the animation at each of the first updates, so
/// some are spawned while the look shows another cell than the spawn uv, and some of those see the
/// spawn uv again at their next frame.
fn spawn_late_each(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>, mut frame: Local<usize>) {
    *frame += 1;
    if *frame <= 16 {
        let (m, mut fx, _) = material_of(&table(), "stillGlow");
        let h = materials.add(m);
        fx.material = h.clone();
        commands.spawn((fx, MeshMaterial3d(h), Visibility::default(), LateIdx(*frame)));
    }
}

/// ADR 0009 S5 (parity mode): an adopted node is re-queued in exactly the frames its own material
/// would be modified on `main`, for every node that was drawn in the frame before and is still drawn:
/// blinking and still nodes on a flipbook sprite, colour-cycling ones, and nodes spawned after the
/// animation (on a private copy while their look shows another cell).
///
/// A node coming back from hidden is left out of the comparison: Bevy queues newly visible entities
/// ahead of changed ones and yields each entity once (`iter_to_queue`, read from the Bevy 0.20.1
/// source), so such a node takes the same place in the queue whether or not its material also changed.
/// That is where the shared look's uv write can touch a node whose own material `main` would leave
/// alone (it was hidden while the look moved on).
#[test]
fn adopted_nodes_are_re_queued_in_the_frames_mains_own_materials_change() {
    let (mut old, mut new) = (game_app(false), game_app(true));
    const FIXED: [&str; 6] = ["blinkGlow", "blinkGlow", "stillGlow", "puffA", "still", "blink"];
    let fo: Vec<Entity> = FIXED.iter().map(|n| spawn_node(&mut old, n, None)).collect();
    let fnew: Vec<Entity> = FIXED.iter().map(|n| spawn_node(&mut new, n, None)).collect();
    for app in [&mut old, &mut new] {
        app.init_resource::<Requeued>().add_systems(Update, spawn_late_each.after(tint_ghost)).add_systems(Last, collect_requeued);
    }
    let late = |app: &mut App| {
        let mut l: Vec<(usize, Entity)> = app.world_mut().query::<(Entity, &LateIdx)>().iter(app.world()).map(|(e, i)| (i.0, e)).collect();
        l.sort_unstable();
        l.into_iter().map(|(_, e)| e).collect::<Vec<_>>()
    };
    let mut drawn_before = std::collections::HashMap::new();
    let (mut compared, mut requeued) = (0, 0);
    for n in 0..48 {
        old.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        new.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        old.update();
        new.update();
        let (oall, nall): (Vec<Entity>, Vec<Entity>) = (fo.iter().copied().chain(late(&mut old)).collect(), fnew.iter().copied().chain(late(&mut new)).collect());
        assert_eq!(oall.len(), nall.len());
        for (i, (&eo, &en)) in oall.iter().zip(&nall).enumerate() {
            let vis = old.world().get::<Visibility>(eo);
            assert_eq!(vis, new.world().get::<Visibility>(en), "node {i} at frame {n}: visibility");
            let drawn = vis != Some(&Visibility::Hidden);
            if drawn && drawn_before.get(&i) == Some(&true) {
                let (ro, rn) = (old.world().resource::<Requeued>().0.contains(&eo), new.world().resource::<Requeued>().0.contains(&en));
                assert_eq!(rn, ro, "node {i} at frame {n}: re-queued on the adopted path, against main");
                compared += 1;
                requeued += usize::from(ro);
            }
            drawn_before.insert(i, drawn);
        }
    }
    assert!(compared > 400 && requeued > 50 && requeued < compared, "{requeued} re-queued of {compared} compared");
}

// --- #1236: `animate` against the `animate` it replaced, which ran every node every frame ---

/// `super::animate` as it was before #1236 (on `main` at e95f16d): every node, every frame, each
/// animation and sprite looked up and played per node. Kept verbatim to compare against.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn animate_before_1236(
    clock: Res<crate::clock::AnimClock>,
    tables: Res<SpriteTables>,
    mut fx: Query<(
        &SpriteFx,
        Option<&SpriteFade>,
        Option<&crate::render::rock::MoonNimbus>,
        &mut Visibility,
        Option<&mut material::SpriteState>,
        Option<&mut MeshMaterial3d<material::SpriteMaterial>>,
    )>,
    flip: Query<&UvFlipbook>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut looks: Option<ResMut<Assets<material::SpriteMaterial>>>,
    slots: Option<Res<material::SpriteSlots>>,
    buffers: Option<ResMut<Assets<bevy::render::storage::ShaderBuffer>>>,
    mut writes: Local<Vec<(bevy::mesh::MeshTag, [f32; 4])>>,
) {
    let t = clock.wall() as f32;
    let table = &tables.0;
    let parity = material::parity();
    writes.clear();
    for (f, fade, nimbus, mut vis, state, mut drawn) in &mut fx {
        let mut col = f.colour;
        // A moon's nimbus is drawn under the Stock look only.
        let mut shown = nimbus.is_none_or(|n| n.on);
        if let Some(a) = f.node_anim.as_deref().and_then(|a| table.animation(a)) {
            match spr_anim::value(a, t, spr_anim::LOOP) {
                Some(Value::Draw(false)) => shown = false,
                Some(Value::Colour(c)) => col = c,
                _ => {}
            }
        }
        if let Some(k) = fade {
            col = col.map(|c| c * k.0);
        }
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if !shown {
            continue;
        }
        // Write only on change: a write re-uploads the material or the slot buffer, and a
        // map has thousands of these, most of them static.
        let Some(sprite) = table.sprite(&f.sprite) else { continue };
        let (c, uv) = (colour(col), uv_affine(spr_anim::sprite_uv(table, sprite, t)));
        let Some(mut st) = state else {
            if materials.get(&f.material).is_some_and(|m| m.base_color != c || m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.base_color = c;
                m.uv_transform = uv;
            }
            continue;
        };
        let Some(looks) = looks.as_deref_mut() else { continue };
        // The look shows the uv of its shown nodes; the first of them this frame writes it.
        if looks.get(&st.look).is_some_and(|l| l.base.uv_transform != uv)
            && let Some(mut l) = looks.get_mut(&st.look)
        {
            l.base.uv_transform = uv;
        }
        if c != st.colour || uv != st.uv {
            if c != st.colour {
                writes.push((st.tag.clone(), material::linear(c)));
            }
            st.colour = c;
            st.uv = uv;
            if let Some(d) = drawn.as_mut() {
                if d.0 != st.look {
                    // A node spawned on a private copy (another uv than its look's) joins its look
                    // only in a frame where its own material would change on `main`, so the swap
                    // re-queues it when `main` does. Until then the copy keeps showing the node's
                    // uv, which has not changed; from now on the look shows it (written above).
                    d.0 = st.look.clone();
                } else if parity {
                    d.set_changed();
                }
            }
        }
    }
    if let (Some(slots), Some(mut buffers)) = (slots, buffers) {
        slots.write(&mut buffers, writes.iter().map(|(tag, c)| (tag, *c)));
    }
    for f in &flip {
        if let Some(a) = table.animation(&f.animation) {
            let o = spr_anim::offset_of(spr_anim::value(a, t + f.offset, spr_anim::LOOP));
            let uv = Affine2::from_translation(Vec2::new(o[0], o[1]));
            if materials.get(&f.material).is_some_and(|m| m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.uv_transform = uv;
            }
        }
    }
}

/// [`TABLE`] with what the #1236 scene adds: a flipbook sprite drawn only by blinking nodes (so its
/// look goes stale while they are all off), a sprite whose uv animation holds an overflowing key
/// (its uv is NaN or infinite), and a node whose colour overflows (NaN once a fade of 0 multiplies
/// it). Malformed data, but a mod can ship it.
fn table_1236_text() -> String {
    TABLE
        .replace(
            "cut_b tex_b 0 32 32 32\n",
            "cut_b tex_b 0 32 32 32\n@tmaterial=additive\nflipdot tex_b 0 0 32 32 @anim=flip4\nnanspr tex_a 0 0 32 32 @anim=nanoff\n",
        )
        .replace(
            "@animation pulse\n",
            "@sprite_node blinkFlip flipdot strobe (1,1) (1,1,1) billboard\n\
             @sprite_node nanNode nanspr const (1,1) (1,1,1) billboard\n\
             @sprite_node hotNode dot const (1,1) (1e39,0.5,0.5) billboard\n\
             @animation nanoff\nu_offset 2 1.0 linear\n@keyframes\n0.0 1e39\n0.5 0\n@animation pulse\n",
        )
}

fn table_1236() -> SpriteTables {
    SpriteTables(Arc::new(SpriteTable::parse(table_1236_text().as_bytes())))
}

/// The table the #1236 scene swaps in mid-run: the flipbook sprite's rectangle and a colour key moved,
/// and one sprite gone (its nodes then find none).
fn table_1236_swapped() -> SpriteTables {
    let text = table_1236_text()
        .replace("glow tex_a 0 0 32 32 @anim=flip4", "glow tex_a 0 32 32 32 @anim=flip4")
        .replace("0.5 (0.25,0.5,1)", "0.5 (0.75,0.5,0.125)")
        .replace("ad_b tex_b 0 32 32 32\n", "");
    SpriteTables(Arc::new(SpriteTable::parse(text.as_bytes())))
}

/// A sprite node of [`table_1236`] as `model_scene::spawn_sprite_node` makes it, under `parent`.
fn spawn_node_1236(app: &mut App, node: &str, parent: Option<Entity>) -> Entity {
    let (m, mut fx, _) = material_of(&table_1236(), node);
    let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
    fx.material = h.clone();
    let mut e = app.world_mut().spawn((fx, MeshMaterial3d(h), Visibility::default()));
    if let Some(p) = parent {
        e.insert(ChildOf(p));
    }
    e.id()
}

/// An app with the game's adoption, cloak and build fade and ghost tint, animated by the `animate`
/// before #1236 (`before`) or by today's.
fn adopted_app(before: bool) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<StandardMaterial>()
        .init_asset::<SpriteMaterial>()
        .init_asset::<ShaderBuffer>()
        .insert_resource(table_1236())
        .insert_resource(AnimClock::new(ClockMode::Bench { step: 0.173 }, 0));
    let slots = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), 4);
    app.insert_resource(slots);
    material::adoption(&mut app);
    if before {
        app.add_systems(Update, (animate_before_1236, apply_fade, tint_ghost).chain());
    } else {
        see_unhidden(&mut app);
        app.add_systems(Update, (animate, apply_fade, tint_ghost).chain());
    }
    app.init_resource::<Requeued>().init_resource::<Modified>().add_systems(Last, (collect_requeued, count_modified));
    app.add_systems(Update, spawn_late_mixed.after(tint_ghost));
    app
}

/// Orders the nodes [`spawn_late_mixed`] spawned.
#[derive(Component)]
struct LateMixed(usize);

/// The parent [`spawn_late_mixed`] puts every third node under.
#[derive(Resource)]
struct LateParent(Entity);

/// After the animation, at each of the first updates, a node of another kind (every third under
/// the parent that is hidden and shown again).
fn spawn_late_mixed(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>, parent: Res<LateParent>, mut frame: Local<usize>) {
    const KINDS: [&str; 7] = ["stillGlow", "puffA", "blink", "blinkGlow", "still", "blA", "blinkFlip"];
    *frame += 1;
    if *frame <= 18 {
        let (m, mut fx, _) = material_of(&table_1236(), KINDS[*frame % KINDS.len()]);
        let h = materials.add(m);
        fx.material = h.clone();
        let mut e = commands.spawn((fx, MeshMaterial3d(h), Visibility::default(), LateMixed(*frame)));
        if *frame % 3 == 0 {
            e.insert(ChildOf(parent.0));
        }
    }
}

/// The nodes of the #1236 scene, in the same order in both apps.
struct Scene1236 {
    /// A parent hidden and shown again mid-animation, with nodes of every kind under it.
    parent: Entity,
    unit: Entity,
    /// Nodes whose fade the frames change (index into [`Scene1236::fixed`]).
    faded: Vec<usize>,
    /// A moon nimbus node.
    nimbus: usize,
    /// A still node whose visibility another system writes, as the construction reveal does.
    poked: usize,
    /// A still node whose `SpriteFx` is edited mid-run (it starts to blink, in another colour).
    edited: usize,
    fixed: Vec<Entity>,
}

fn scene_1236(app: &mut App) -> Scene1236 {
    let parent = app.world_mut().spawn(Visibility::default()).id();
    app.insert_resource(LateParent(parent));
    let unit = app.world_mut().spawn((UnitFade { cloak: 1.0, build: 1.0 }, Visibility::default())).id();
    let ghost = app.world_mut().spawn((Ghost::pinned(sta_sim::ContentKey::NONE), Visibility::default())).id();
    const KINDS: [&str; 12] = ["still", "puffA", "puffB", "blink", "blinkGlow", "stillGlow", "blA", "blB", "adA", "adB", "cutA", "cutB"];
    let mut fixed = Vec::new();
    for round in 0..3 {
        for k in KINDS {
            let p = match round {
                0 => None,
                1 => Some(parent),
                _ => Some(unit),
            };
            fixed.push(spawn_node_1236(app, k, p));
        }
    }
    for k in ["puffA", "blink", "still", "stillGlow"] {
        fixed.push(spawn_node_1236(app, k, Some(ghost)));
    }
    // The flipbook look drawn only by blinking nodes, one of them under the parent.
    fixed.push(spawn_node_1236(app, "blinkFlip", None));
    fixed.push(spawn_node_1236(app, "blinkFlip", None));
    fixed.push(spawn_node_1236(app, "blinkFlip", Some(parent)));
    // Non-finite values: a NaN or infinite uv, and an overflowing colour with and without a fade of 0.
    fixed.push(spawn_node_1236(app, "nanNode", None));
    fixed.push(spawn_node_1236(app, "hotNode", None));
    let hot = spawn_node_1236(app, "hotNode", None);
    app.world_mut().entity_mut(hot).insert(SpriteFade(0.0));
    fixed.push(hot);
    let nimbus = fixed.len();
    let e = spawn_node_1236(app, "blinkGlow", None);
    app.world_mut().entity_mut(e).insert(crate::render::rock::MoonNimbus { on: true });
    fixed.push(e);
    let poked = 0;
    // Puffs of the first two rounds (free and under the parent) get fades.
    let faded = (0..2 * KINDS.len()).filter(|i| i % 2 == 1).collect();
    Scene1236 { parent, unit, faded, nimbus, poked, edited: 2 * KINDS.len(), fixed }
}

/// The scene's inputs at frame `n`, the same in both apps: whether the clock moves, the parent's
/// and nimbus's visibility, the fades (changed only every few frames, sometimes removed), the cloak,
/// an outside write of a node's visibility, an edit of a node's `SpriteFx`, and a table swapped in
/// and back.
fn drive_1236(app: &mut App, s: &Scene1236, n: usize) {
    // Held frames (the clock does not move), so unchanged frames come up.
    if !matches!(n % 9, 4 | 5 | 6) {
        app.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
    }
    let hidden = (12..20).contains(&n) || (33..47).contains(&n) || n == 60;
    let want = if hidden { Visibility::Hidden } else { Visibility::Inherited };
    if *app.world().get::<Visibility>(s.parent).unwrap() != want {
        *app.world_mut().get_mut::<Visibility>(s.parent).unwrap() = want;
    }
    let on = !(50..58).contains(&n);
    if app.world().get::<crate::render::rock::MoonNimbus>(s.fixed[s.nimbus]).unwrap().on != on {
        app.world_mut().get_mut::<crate::render::rock::MoonNimbus>(s.fixed[s.nimbus]).unwrap().on = on;
    }
    for (j, &i) in s.faded.iter().enumerate() {
        let e = s.fixed[i];
        let k = match (j + n / 4) % 6 {
            0 => None,
            1 => Some(1.0),
            2 => Some(0.3),
            3 => Some(0.3 + 0.01 * (n / 4) as f32),
            4 => Some(0.0),
            _ => Some(0.7),
        };
        let now = app.world().get::<SpriteFade>(e).map(|f| f.0);
        match k {
            Some(k) if now.map(f32::to_bits) != Some(k.to_bits()) => {
                app.world_mut().entity_mut(e).insert(SpriteFade(k));
            }
            None if now.is_some() => {
                app.world_mut().entity_mut(e).remove::<SpriteFade>();
            }
            _ => {}
        }
    }
    let cloak = [0.6, 0.4, 0.3, 0.3, 0.3, 0.5, 1.0, 1.0, 0.8].get(n.wrapping_sub(24)).copied().unwrap_or(1.0);
    if app.world().get::<UnitFade>(s.unit).unwrap().cloak != cloak {
        app.world_mut().get_mut::<UnitFade>(s.unit).unwrap().cloak = cloak;
    }
    if n == 21 || n == 66 {
        *app.world_mut().get_mut::<Visibility>(s.fixed[s.poked]).unwrap() = Visibility::Hidden;
    }
    if n == 40 {
        let mut fx = app.world_mut().get_mut::<SpriteFx>(s.fixed[s.edited]).unwrap();
        fx.node_anim = Some("strobe".into());
        fx.colour = [0.9, 0.1, 0.2];
    }
    if n == 50 {
        app.insert_resource(table_1236_swapped());
    }
    if n == 75 {
        app.insert_resource(table_1236());
    }
}

fn nodes_1236(app: &mut App, s: &Scene1236) -> Vec<Entity> {
    let mut late: Vec<(usize, Entity)> = app.world_mut().query::<(Entity, &LateMixed)>().iter(app.world()).map(|(e, i)| (i.0, e)).collect();
    late.sort_unstable();
    s.fixed.iter().copied().chain(late.into_iter().map(|(_, e)| e)).collect()
}

/// Everything of node `en` that `animate` decides or that is drawn from it, against node `eo` of the
/// other app: visibility, state, slot, which material it draws through and that material's fields,
/// and whether Bevy re-queues it this frame.
fn same_node(old: &App, eo: Entity, new: &App, en: Entity) -> Result<(), String> {
    let (wo, wn) = (old.world(), new.world());
    if wo.get::<Visibility>(eo) != wn.get::<Visibility>(en) {
        return Err(format!("visibility {:?} against {:?}", wo.get::<Visibility>(eo), wn.get::<Visibility>(en)));
    }
    let (so, sn) = (wo.get::<SpriteState>(eo).unwrap(), wn.get::<SpriteState>(en).unwrap());
    if (bits(so.colour), uv_bits(so.uv)) != (bits(sn.colour), uv_bits(sn.uv)) {
        return Err(format!("state {:?} {:?} against {:?} {:?}", so.colour, so.uv, sn.colour, sn.uv));
    }
    if slot_of(old, eo) != slot_of(new, en) {
        return Err(format!("slot {:?} against {:?}", slot_of(old, eo), slot_of(new, en)));
    }
    match (wo.get::<MeshMaterial3d<StandardMaterial>>(eo), wn.get::<MeshMaterial3d<StandardMaterial>>(en)) {
        (Some(a), Some(b)) => {
            let (a, b) = (wo.resource::<Assets<StandardMaterial>>().get(&a.0).unwrap(), wn.resource::<Assets<StandardMaterial>>().get(&b.0).unwrap());
            if fields(a) != fields(b) {
                return Err(format!("demoted material {} against {}", fields(a), fields(b)));
            }
        }
        (None, None) => {
            let (a, b) = (&wo.get::<MeshMaterial3d<SpriteMaterial>>(eo).unwrap().0, &wn.get::<MeshMaterial3d<SpriteMaterial>>(en).unwrap().0);
            if (a.id() == so.look.id()) != (b.id() == sn.look.id()) {
                return Err("one draws through its look, the other through a private copy".into());
            }
            let (a, b) = (wo.resource::<Assets<SpriteMaterial>>().get(a).unwrap(), wn.resource::<Assets<SpriteMaterial>>().get(b).unwrap());
            if fields(&a.base) != fields(&b.base) {
                return Err(format!("look {} against {}", fields(&a.base), fields(&b.base)));
            }
        }
        _ => return Err("demoted on one side only".into()),
    }
    // Parity marks are for nodes some view sees (S5, as amended): a node nothing sees is in no phase,
    // so whether it is re-queued is not compared. Seen nodes must match.
    let (ro, rn) = (wo.resource::<Requeued>().0.contains(&eo), wn.resource::<Requeued>().0.contains(&en));
    if wn.get::<ViewVisibility>(en).is_some_and(|v| v.get()) && ro != rn {
        return Err(format!("re-queued {ro} against {rn}"));
    }
    Ok(())
}

/// Runs the #1236 scene in both apps for `frames` frames, calling `meddle` on both apps before
/// each update, and returns the first difference (frame, node, what), or the number of node frames
/// compared and the new app.
fn run_1236(frames: usize, mut meddle: impl FnMut(&mut App, &Scene1236, usize)) -> Result<(usize, App, Scene1236), String> {
    let (mut old, mut new) = (adopted_app(true), adopted_app(false));
    let (so, sn) = (scene_1236(&mut old), scene_1236(&mut new));
    let mut compared = 0;
    for n in 0..frames {
        drive_1236(&mut old, &so, n);
        drive_1236(&mut new, &sn, n);
        meddle(&mut old, &so, n);
        meddle(&mut new, &sn, n);
        old.update();
        new.update();
        let (no, nn) = (nodes_1236(&mut old, &so), nodes_1236(&mut new, &sn));
        if no.len() != nn.len() {
            return Err(format!("frame {n}: {} nodes against {}", no.len(), nn.len()));
        }
        for (i, (&eo, &en)) in no.iter().zip(&nn).enumerate() {
            same_node(&old, eo, &new, en).map_err(|e| format!("frame {n}, node {i}: {e}"))?;
            compared += 1;
        }
        let (mo, mn) = (*old.world().resource::<Modified>(), *new.world().resource::<Modified>());
        if (mo.standard, mo.sprite, mo.buffer) != (mn.standard, mn.sprite, mn.buffer) {
            return Err(format!("frame {n}: modified events {mo:?} against {mn:?}"));
        }
    }
    Ok((compared, new, sn))
}

/// #1236 gate: today's `animate`, which passes over nodes whose inputs did not change and plays each
/// animation and sprite once per frame, decides exactly what the `animate` before it decided, frame
/// by frame, for every node, drawn or not: visibility, state, slot, the material it draws through
/// (look, private copy or demoted) field by field, the re-queue set, and the modified events of
/// every asset type. The scene has still, colour-cycling, blinking and flipbook nodes of every blend
/// mode, a parent hidden and shown again mid-animation with nodes of every kind under it, nodes
/// spawned after the animation (some under that parent), a cloaking unit, a ghost, fades that
/// change, go and come back, a nimbus switched off and on, an outside write of a node's visibility,
/// and frames where the clock holds.
#[test]
fn animate_decides_what_the_animate_before_1236_decided_for_every_node_at_every_frame() {
    let (compared, mut new, s) = run_1236(90, |_, _, _| {}).unwrap_or_else(|e| panic!("{e}"));
    let nodes = nodes_1236(&mut new, &s);
    // The non-finite cases were reached: a NaN uv and an overflowing colour times a fade of 0.
    let state = |i: usize| format!("{:?}", new.world().get::<SpriteState>(nodes[i]).unwrap());
    let (nan_uv, hot_faded) = (s.nimbus - 3, s.nimbus - 1);
    assert!(state(nan_uv).contains("NaN") && state(hot_faded).contains("NaN"), "{} / {}", state(nan_uv), state(hot_faded));
    assert!(compared > 90 * 50, "{compared} node frames compared");
    // Not vacuous: nodes were passed over (and others were not).
    let passes: Vec<u32> = nodes.iter().map(|&e| new.world().get::<SpriteKeys>(e).unwrap().passes).collect();
    let total: u32 = passes.iter().sum();
    assert!(passes[s.poked] < 20 && passes.iter().any(|&p| p >= 55) && (total as usize) < compared * 2 / 3, "passes per node: {passes:?}");
}

/// Control for the gate above: a fade changed in both apps behind Bevy's change detection, on a still
/// node in a frame where the clock holds, is read by the `animate` before #1236 (which reads every
/// node every frame) and passed over by today's, and the comparison reports it.
#[test]
fn the_1236_comparison_sees_a_node_passed_over_when_it_should_not_be() {
    let r = run_1236(40, |app, s, n| {
        // `blB` (still), whose fade the driver leaves alone from frame 28 to 31; frame 31 holds the clock.
        let e = s.fixed[s.faded[3]];
        if n == 31 {
            app.world_mut().get_mut::<SpriteFade>(e).expect("a fade at frame 31").bypass_change_detection().0 = 0.55;
        }
    });
    let e = r.err().expect("the comparison reports the difference");
    assert!(e.starts_with("frame 31, node 7: state"), "{e}");
}

// --- #1237: the parity mark only for nodes a view sees ---

/// A colour-cycling node under a parent that is hidden for a while: in each frame it is re-queued
/// exactly when its state changed and a view sees it. While hidden (state changing, nothing seeing it)
/// it is not marked; in the frame it is seen again it is marked, with its slot already holding the
/// frame's colour (written in `Update`, as the state is), and the list of waiting marks is empty
/// after every frame.
#[test]
fn a_node_is_marked_in_the_frame_a_view_sees_it_and_not_while_none_does() {
    let mut app = game_app(true);
    app.init_resource::<Requeued>().add_systems(Last, collect_requeued);
    let parent = app.world_mut().spawn(Visibility::default()).id();
    // A look whose uv does not animate (a flipbook look's asset change re-queues its nodes by itself).
    let node = spawn_node(&mut app, "blA", Some(parent));
    let (mut seen_changed, mut unseen_changed, mut entered) = (0, 0, 0);
    let mut was_seen = false;
    for n in 0..40 {
        app.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        *app.world_mut().get_mut::<Visibility>(parent).unwrap() = if (10..22).contains(&n) { Visibility::Hidden } else { Visibility::Inherited };
        let before = bits(app.world().get::<SpriteState>(node).unwrap().colour);
        app.update();
        let st = app.world().get::<SpriteState>(node).unwrap();
        let changed = bits(st.colour) != before;
        let seen = app.world().get::<ViewVisibility>(node).unwrap().get();
        let marked = app.world().resource::<Requeued>().0.contains(&node);
        assert_eq!(marked, changed && seen, "frame {n}: changed {changed}, seen {seen}");
        assert!(app.world().resource::<material::ParityMarks>().is_empty(), "frame {n}: marks left waiting");
        assert_eq!(slot_of(&app, node), linear_bits(st.colour), "frame {n}: slot");
        (seen_changed, unseen_changed) = (seen_changed + usize::from(changed && seen), unseen_changed + usize::from(changed && !seen));
        entered += usize::from(seen && !was_seen && n > 0);
        was_seen = seen;
    }
    assert!(seen_changed > 15 && unseen_changed > 8 && entered == 1, "seen {seen_changed}, unseen {unseen_changed}, entered {entered}");
}

/// #1146 with #1269: a sprite node under a fog gate that hides it (the object never seen) is not marked, whatever its
/// colour does meanwhile, and is marked in the frame it is seen again when its colour changed that frame: the gate's
/// own `Visibility` toggle through `apply_gates` is what the parity mark sees.
#[test]
fn a_node_under_a_fog_gate_is_marked_only_in_frames_a_view_sees_it() {
    use crate::render::fog::{FogGrid, FogState};
    use crate::render::fog_objects::{RegionSights, SightGate, apply_gates};
    let mut app = game_app(true);
    app.init_resource::<Requeued>()
        .init_resource::<RegionSights>()
        .insert_resource(crate::render::units::LocalPlayer(Some(sta_sim::PlayerId(1))))
        .add_systems(Last, collect_requeued)
        .add_systems(Update, apply_gates.before(animate));
    let gate = app.world_mut().spawn((SightGate { pos: Vec2::new(0.0, 0.0), radius: 50.0, region: None, ghost: 1.0 }, Visibility::Inherited)).id();
    let node = spawn_node(&mut app, "blA", Some(gate));
    let never = || FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]);
    let (mut shown_marked, mut hidden_changed) = (0, 0);
    for n in 0..40u32 {
        // Never seen until frame 12, in sight from then on.
        let grid = if n < 12 { never() } else { never().lifted(true, true) };
        app.insert_resource(FogState { grid, updates: n + 1 });
        app.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        let before = bits(app.world().get::<SpriteState>(node).unwrap().colour);
        app.update();
        let changed = bits(app.world().get::<SpriteState>(node).unwrap().colour) != before;
        let seen = app.world().get::<ViewVisibility>(node).unwrap().get();
        assert_eq!(seen, n >= 12, "frame {n}: the gate's visibility reaches the node's view visibility");
        let marked = app.world().resource::<Requeued>().0.contains(&node);
        // Frame 0: the spawn itself counts as a change of the node's material.
        if n > 0 {
            assert_eq!(marked, changed && seen, "frame {n}: changed {changed}, seen {seen}");
        }
        assert!(app.world().resource::<material::ParityMarks>().is_empty(), "frame {n}: marks left waiting");
        shown_marked += usize::from(marked);
        hidden_changed += usize::from(changed && !seen);
    }
    assert!(hidden_changed > 5, "the colour changed while the gate hid the node: {hidden_changed}");
    assert!(shown_marked > 15, "and it is marked in the frames it is seen: {shown_marked}");
}

/// Runs the nodes listed in alternate frames only through Bevy's own visibility, mesh and material
/// systems (no GPU). The mark must reach `EntitiesNeedingSpecialization` and `Mesh3d`'s change flag in
/// exactly the frames the node was listed in, for a node a view sees, and never for one nothing sees. With
/// `late`, the mark system is ordered after the specialisation check instead of before it, as a negative
/// control: the mark then lands in the frame after the listing, and the check reports it.
fn the_mark_lands_in_the_listed_frame(late: bool) -> Result<(), String> {
    use bevy::camera::visibility::{VisibilityPlugin, VisibilitySystems};
    use bevy::pbr::{EntitiesNeedingSpecialization, MaterialPlugin};
    #[derive(Resource)]
    struct Nodes(Entity, Entity);
    #[derive(Resource, Default)]
    struct Saw {
        specialise: Vec<Entity>,
        mesh_changed: Vec<Entity>,
    }
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::mesh::MeshPlugin, VisibilityPlugin))
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<bevy::shader::Shader>()
        .add_plugins(MaterialPlugin::<SpriteMaterial>::default());
    if late {
        app.init_resource::<material::ParityMarks>().add_systems(
            PostUpdate,
            material::mark_seen_changes
                .after(VisibilitySystems::MarkNewlyHiddenEntitiesInvisible)
                .after(bevy::pbr::check_entities_needing_specialization::<SpriteMaterial>),
        );
    } else {
        material::adoption(&mut app);
    }
    // The view: sees every node that is not hidden, in the set the cameras' check is in.
    fn view(mut nodes: Query<(&Visibility, &mut ViewVisibility)>) {
        use bevy::camera::visibility::SetViewVisibility;
        for (vis, mut seen) in &mut nodes {
            if *vis != Visibility::Hidden {
                seen.set_visible();
            }
        }
    }
    fn list(nodes: Res<Nodes>, mut marks: ResMut<material::ParityMarks>, mut frame: Local<u32>) {
        // Alternate frames only, so a mark that lands a frame late shows in the frames between.
        if *frame % 2 == 0 {
            marks.push(nodes.0);
            marks.push(nodes.1);
        }
        *frame += 1;
    }
    fn look(
        mut saw: ResMut<Saw>,
        spec: Res<EntitiesNeedingSpecialization<SpriteMaterial>>,
        meshes: Query<(Entity, Ref<Mesh3d>), With<MeshMaterial3d<SpriteMaterial>>>,
    ) {
        saw.specialise = spec.changed.clone();
        saw.mesh_changed = meshes.iter().filter(|(_, m)| m.is_changed()).map(|(e, _)| e).collect();
    }
    app.init_resource::<Saw>()
        .add_systems(Update, list)
        .add_systems(PostUpdate, view.in_set(VisibilitySystems::CheckVisibility))
        .add_systems(Last, look);
    let spawn = |app: &mut App, v: Visibility| app.world_mut().spawn((Mesh3d(Handle::default()), MeshMaterial3d::<SpriteMaterial>(Handle::default()), v)).id();
    let (seen, unseen) = (spawn(&mut app, Visibility::Inherited), spawn(&mut app, Visibility::Hidden));
    app.insert_resource(Nodes(seen, unseen));
    for n in 0..12u32 {
        app.update();
        let saw = app.world().resource::<Saw>();
        if n < 4 {
            // Spawn and the first frames settle (added components count as changed).
            continue;
        }
        // The node is listed in frames 0, 2, 4, ...
        let listed = n % 2 == 0;
        // `Mesh3d`'s change flag is the mark's trace in exactly the frame it lands (Bevy's private system
        // turns a changed material into a changed mesh in the same frame); the specialisation set also
        // holds nodes for other reasons (asset events of the default handles), so it is only required
        // to hold the node in the frames it was listed in.
        let (s, m) = (saw.specialise.contains(&seen), saw.mesh_changed.contains(&seen));
        if (listed && !s) || m != listed {
            return Err(format!("frame {n}: listed {listed}, in the specialisation set {s}, mesh changed {m}"));
        }
        if saw.mesh_changed.contains(&unseen) {
            return Err(format!("frame {n}: the node nothing sees was marked"));
        }
    }
    Ok(())
}

/// The mark reaches Bevy's specialisation and mesh re-extraction in the frame `animate` listed the node,
/// not a frame later (see [`the_mark_lands_in_the_listed_frame`]). The PostUpdate schedule builds with
/// the mark's two orderings (no cycle).
#[test]
fn the_mark_lands_in_the_frame_it_is_listed_in_through_bevys_own_systems() {
    the_mark_lands_in_the_listed_frame(false).unwrap_or_else(|e| panic!("{e}"));
}

/// Negative control of the test above: a mark ordered after the specialisation check lands a frame late,
/// and the check says so.
#[test]
fn the_same_check_sees_a_mark_that_lands_a_frame_late() {
    let e = the_mark_lands_in_the_listed_frame(true).expect_err("the late mark must be seen");
    assert!(e.contains("listed"), "{e}");
}

/// #1146: a node with `SpriteStill` draws its animation at the time it stands at, the other nodes run on; when the
/// component goes the node takes the clock's time again, and nothing is written for it while it stands.
#[test]
fn a_still_node_draws_its_animation_at_the_time_it_stands_at_and_resumes_with_the_clock() {
    let mut app = game_app(true);
    // A node whose sprite does not flip (a flipbook's uv is its look's, shared and live: declared).
    let live = spawn_node(&mut app, "blA", None);
    let still = spawn_node(&mut app, "blA", None);
    let at = 0.25f32;
    let t = table();
    let frozen = match spr_anim::value(t.0.animation("pulse").unwrap(), at, spr_anim::LOOP) {
        Some(Value::Colour(c)) => linear_bits(colour(c)),
        other => panic!("the invented pulse gives a colour, not {other:?}"),
    };
    let frame = |app: &mut App| {
        app.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
        app.update();
    };
    frame(&mut app);
    app.world_mut().entity_mut(still).insert(SpriteStill(at));
    let mut live_seen = std::collections::BTreeSet::new();
    for _ in 0..30 {
        frame(&mut app);
        assert_eq!(slot_of(&app, still), frozen, "the still node stands at its time");
        live_seen.insert(slot_of(&app, live));
    }
    assert!(live_seen.len() > 3, "the live node animates meanwhile: {} colours", live_seen.len());
    // Standing still costs nothing: no pass over the node, no write (`passes` counts full passes).
    let passes = app.world().get::<SpriteKeys>(still).unwrap().passes;
    for _ in 0..10 {
        frame(&mut app);
    }
    assert_eq!(app.world().get::<SpriteKeys>(still).unwrap().passes, passes, "a still node is passed over");
    // In sight again: the clock's time, the same as the node that never stopped.
    app.world_mut().entity_mut(still).remove::<SpriteStill>();
    frame(&mut app);
    assert_eq!(slot_of(&app, still), slot_of(&app, live));
}
