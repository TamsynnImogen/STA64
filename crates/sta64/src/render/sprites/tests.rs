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
@sprite_node adA ad_a pulse (2,2) (0.6,0.4,1) billboard\n\
@sprite_node adB ad_b const (2,2) (1,0.8,0.2) billboard\n\
@sprite_node cutA cut_a const (3,3) (0.9,0.9,0.9)\n\
@sprite_node cutB cut_b pulse (3,3) (0.5,1,1)\n\
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
        .insert_resource(table())
        .insert_resource(AnimClock::new(ClockMode::Bench { step: 0.173 }, 0));
    if adopting {
        app.init_asset::<SpriteMaterial>().init_asset::<ShaderBuffer>();
        // Small, so the nodes below grow it.
        let slots = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), 4);
        app.insert_resource(slots);
        material::adoption(&mut app);
        app.add_systems(Update, (animate, apply_fade, tint_ghost).chain());
    } else {
        app.add_systems(Update, (animate_reference, apply_fade, tint_ghost).chain());
    }
    app
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
