//! The sprite nodes' materials: the shared path ([`super::animate`])
//! against the per-node path it replaced ([`animate_reference`], kept here for the tests only).
use super::*;
use crate::clock::{AnimClock, ClockMode};
use sta_assets::a2fmt::spr::SpriteTable;

/// An invented table: flipbook and colour-cycling sprites, a blinking node, a still one.
pub(super) const TABLE: &str = "sprite_table\n@referenceWidth=128\n@referenceHeight=64\n@tmaterial=additive\n\
glow tex_a 0 0 32 32 @anim=flip4\n\
dot tex_b 0 32 32 32\n\
@sprite_node puffA glow pulse (2,2) (0.5,0.5,0.5) billboard\n\
@sprite_node puffB glow pulse (2,2) (1,0.5,0.25) billboard\n\
@sprite_node still dot const (3,3) (0.2,0.4,0.8)\n\
@sprite_node blink dot strobe (1,1) (1,1,1)\n\
@animation pulse\ncolour 2 1.0 linear\n@keyframes\n0.0 (1,0.25,0.5)\n0.5 (0.25,0.5,1)\n\
@animation strobe\ndraw 2 2.0 step\n@keyframes\n0.0 1\n0.3 0\n\
@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n";

pub(super) fn table() -> SpriteTables {
    SpriteTables(Arc::new(SpriteTable::parse(TABLE.as_bytes())))
}

/// The node ids of [`TABLE`], as `spawn_sprite_node` reads them.
pub(super) const NODES: [&str; 4] = ["puffA", "puffB", "still", "blink"];

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

/// The system before the materials were shared: visibility and the material of
/// every node written every frame. Kept to compare against.
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
        app.add_systems(Update, (share_materials, animate).chain());
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
fn the_shared_path_gives_every_drawn_node_the_colour_and_uv_of_the_per_node_path_at_every_frame() {
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
            let handle = &new.world().get::<MeshMaterial3d<StandardMaterial>>(pn[i]).unwrap().0;
            let mn = new.world().resource::<Assets<StandardMaterial>>().get(handle).unwrap();
            assert_eq!(bits(mo.base_color), bits(mn.base_color), "colour of {i} at {n}");
            assert_eq!(uv_bits(mo.uv_transform), uv_bits(mn.uv_transform), "uv of {i} at {n}");
            compared += 1;
        }
    }
    assert!(compared > 500, "{compared} comparisons");
}

#[test]
fn nodes_of_one_look_share_a_material_until_their_fade_is_not_one() {
    let mut app = base_app(false);
    let puffs = spawn_puffs(&mut app);
    app.world_mut().resource_mut::<AnimClock>().advance_idle(0.0);
    for &e in &puffs {
        app.world_mut().entity_mut(e).insert(SpriteFade(1.0));
    }
    app.update();
    let handles: std::collections::HashSet<_> = puffs.iter().map(|&e| app.world().get::<MeshMaterial3d<StandardMaterial>>(e).unwrap().0.id()).collect();
    assert_eq!(handles.len(), NODES.len(), "one material per look");
    // One puff fades: it gets its own; when the fade is 1 again it goes back to the shared one.
    let shared = app.world().get::<MeshMaterial3d<StandardMaterial>>(puffs[0]).unwrap().0.clone();
    app.world_mut().entity_mut(puffs[0]).insert(SpriteFade(0.5));
    app.update();
    let own = app.world().get::<MeshMaterial3d<StandardMaterial>>(puffs[0]).unwrap().0.clone();
    assert_ne!(own, shared);
    app.world_mut().entity_mut(puffs[0]).insert(SpriteFade(1.0));
    app.update();
    assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(puffs[0]).unwrap().0, shared);
    // The others kept the shared material throughout.
    assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(puffs[NODES.len()]).unwrap().0, shared);
}
