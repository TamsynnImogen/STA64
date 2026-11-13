//! Hull lighting: ships, stations and the like drawn with Armada II Refit's hull look
//! (`hulls.wesl`, ported from `render3d/mods/hulls/shaders/hulls.hlsl`) in place of
//! Bevy's PBR (docs/WORKPLAN.md, RND-06).
//!
//! `model_scene` builds every mesh part with a `StandardMaterial` and marks it
//! [`HullPart`]. [`apply`] gives each marked part a [`HullMaterial`] built from that
//! material (the texture, the normal map, the team colour, the cut-out) and keeps the
//! `StandardMaterial` handle, so [`HullLighting`] can switch back: **H** toggles it in
//! the viewer and the map, for the A/B comparison. A part stays on Bevy's PBR when it
//! is not a hull's lit surface: additive and blended groups, unlit ones (skies, glows),
//! and M3D parts that carry a glow or specular map, which Refit's look has no use for.
//!
//! Settings are Refit's `[Hulls]` defaults (`look.rs`): ambient (0.05, 0.05, 0.07),
//! specular 0.7 with exponent 24 (times the texture's brightness), rim (0.12, 0.14, 0.20)
//! with exponent 3, the Key x1.1, highlight knee 0.8, self-illumination 1.8; bump-mapped
//! hulls take the Borg profile (ambient (0.015, 0.018, 0.015), specular 0.15 with
//! exponent 64, rim (0.06, 0.13, 0.03), the Key x0.8), as stock goes by whether a mesh is
//! bump-mapped.
//!
//! The lights are the scene's directional lights, brightest first (the Key at full
//! colour, the others scaled by their illuminance over the Key's, as `rock.rs` does), the
//! eye the main camera; both are written into the hull materials only when they change.

use std::collections::HashMap;

use bevy::asset::{embedded_asset, AssetPath};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

const AMBIENT: [f32; 3] = [0.05, 0.05, 0.07];
const SPEC: f32 = 0.7;
const SPEC_POW: f32 = 24.0;
const RIM: [f32; 3] = [0.12, 0.14, 0.20];
const RIM_POW: f32 = 3.0;
const SUN: f32 = 1.1;
const BORG_AMBIENT: [f32; 3] = [0.015, 0.018, 0.015];
const BORG_SPEC: f32 = 0.15;
const BORG_SPEC_POW: f32 = 64.0;
const BORG_RIM: [f32; 3] = [0.06, 0.13, 0.03];
const BORG_SUN: f32 = 0.8;
const SELF_ILLUM: f32 = 1.8;
const KNEE: f32 = 0.8;
const BUMP: f32 = 1.0;
/// Directional lights the shader takes (Refit's `DIRS`).
const DIRS: usize = 4;

/// Marks a mesh part of a hull (set by `model_scene`), with what the material alone
/// does not say about it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct HullPart {
    /// The group's lighting material is self-illuminating: the texture's alpha marks
    /// the lit-from-within texels.
    pub self_illum: bool,
    /// The texture's alpha is a cut-out (a `default` mesh whose group is not
    /// self-illuminating: open girders).
    pub cut_alpha: bool,
    /// The group's SOD lighting material, for the Stock profile (`render/stock/hulls.rs`).
    pub stock: Option<crate::render::stock::hulls::StockLight>,
}

/// Hull lighting on (Refit's look) or off (Bevy's PBR, as before): the A/B switch.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct HullLighting {
    pub on: bool,
}

impl Default for HullLighting {
    fn default() -> Self {
        HullLighting { on: true }
    }
}

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct HullParams {
    pub amb: Vec4,
    pub tint: Vec4,
    pub misc: Vec4,
    pub rim: Vec4,
    pub eye: Vec4,
    pub cut: Vec4,
    pub uv: Vec4,
    pub dvec: [Vec4; DIRS],
    pub dcol: [Vec4; DIRS],
    /// The Key's shadow map (`shadows.rs`): world to u, v, depth along the Key.
    pub sh: [Vec4; 3],
    /// x 1 with a map; y one texel (u, v); z the depth bias; w how much of the Key a shadow takes.
    pub shp: Vec4,
    /// Stock profile (RND-24): the SOD lighting material (`mamb.w` 1 draws the group the
    /// stock way, 0 as Refit's look does).
    pub mamb: Vec4,
    pub mdif: Vec4,
    pub mspc: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(HullKey)]
pub struct HullMaterial {
    #[uniform(0)]
    pub params: HullParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub normal: Option<Handle<Image>>,
    /// The Key's shadow map, read by `hulls.wesl` when `params.shp.x` says there is one.
    #[texture(5)]
    pub shadow: Option<Handle<Image>>,
    pub double_sided: bool,
}

/// What a hull pipeline differs by.
#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct HullKey {
    double_sided: bool,
}

impl From<&HullMaterial> for HullKey {
    fn from(m: &HullMaterial) -> Self {
        HullKey { double_sided: m.double_sided }
    }
}

impl Material for HullMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/hulls.wesl").into_owned())
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Forward defaults to Back, but Bevy's depth prepass defaults to None.
        // Match both passes: reflected hulls otherwise write occluding depth
        // for faces that their color pass culls.
        descriptor.primitive.cull_mode = if key.bind_group_data.double_sided { None } else { Some(Face::Back) };
        Ok(())
    }
}

/// The scene's lights and eye, as the shader takes them.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct HullView {
    eye: Vec4,
    dvec: [Vec4; DIRS],
    dcol: [Vec4; DIRS],
    /// The Stock look's lights: towards each, and the colour as the light's own
    /// (gamma-space) numbers, every light the map's own at full weight.
    sdvec: [Vec4; DIRS],
    sdcol: [Vec4; DIRS],
    sh: [Vec4; 3],
    shp: Vec4,
    shadow: Option<Handle<Image>>,
    /// The Stock profile draws hulls (RND-24) and the scene's ambient light colour.
    pub(crate) stock: bool,
    pub(crate) ambient: Vec3,
}

impl HullView {
    /// The view for the Stock look (tests).
    #[cfg(test)]
    pub(crate) fn set_stock(&mut self, on: bool) {
        self.stock = on;
    }

    pub(crate) fn write(&self, p: &mut HullParams) {
        if p.mamb.w > 0.5 {
            p.amb = self.ambient.extend(p.amb.w);
        }
        p.eye = self.eye;
        let stock = p.mamb.w > 0.5;
        p.dvec = if stock { self.sdvec } else { self.dvec };
        p.dcol = if stock { self.sdcol } else { self.dcol };
        p.sh = self.sh;
        p.shp = self.shp;
    }

    /// The Key's shadow map as `shadows.rs` fitted it this frame (RND-15).
    pub fn set_shadow(&mut self, key: &crate::render::shadows::KeyShadow) {
        self.sh = key.rows;
        // A texel, the depth bias of the hulls (half the gas's), and the whole Key taken.
        self.shp = Vec4::new(key.params.x, key.params.w, key.params.y * 0.5, 1.0);
        self.shadow = Some(key.image.clone());
    }
}

/// A part's hull material and the `StandardMaterial` it was built from.
#[derive(Component)]
pub struct HullSource {
    pub source: Handle<StandardMaterial>,
    hull: Handle<HullMaterial>,
    shown: bool,
}

#[cfg(test)]
impl HullSource {
    /// A converted hull part for ghost material integration tests without a GPU.
    pub(crate) fn test_source(source: Handle<StandardMaterial>) -> Self {
        Self { source, hull: Handle::default(), shown: true }
    }
}

/// The hull material made for each `StandardMaterial` (parts share materials).
#[derive(Resource, Default)]
struct HullMaterials(HashMap<AssetId<StandardMaterial>, (Handle<HullMaterial>, HullPart)>);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HullApply;

pub struct HullPlugin;

impl Plugin for HullPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "hulls.wesl");
        app.add_plugins(MaterialPlugin::<HullMaterial>::default())
            .init_resource::<HullLighting>()
            .init_resource::<HullView>()
            .init_resource::<HullMaterials>()
            .add_plugins(crate::render::stock::hulls::StockHullsPlugin)
            .add_systems(Update, (toggle, update_view, apply.in_set(HullApply), follow_source).chain());
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut lighting: ResMut<HullLighting>) {
    if keys.just_pressed(KeyCode::KeyH) {
        lighting.on = !lighting.on;
        info!("hull lighting: {}", if lighting.on { "Refit's look" } else { "Bevy PBR" });
    }
}

/// The directional lights, brightest first: the Key at full colour, the rest scaled by
/// their illuminance over the Key's. (Towards-the-light direction, colour, key flag.)
///
/// The third value is the same colours as the light's own (gamma-space) numbers, which the
/// Stock look multiplies as the original engine does.
fn lights(lights: impl Iterator<Item = (Vec3, Vec3, f32, Vec3)>) -> ([Vec4; DIRS], [Vec4; DIRS], [Vec4; DIRS]) {
    let mut ls: Vec<(Vec3, Vec3, f32, Vec3)> = lights.collect();
    ls.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut dvec = [Vec4::ZERO; DIRS];
    let mut dcol = [Vec4::ZERO; DIRS];
    let mut raw = [Vec4::ZERO; DIRS];
    let key = ls.first().map_or(1.0, |k| k.2.max(1e-3));
    for (i, (to, col, lux, rcol)) in ls.into_iter().take(DIRS).enumerate() {
        dvec[i] = to.extend(0.0);
        let k = if i == 0 { 1.0 } else { lux / key };
        let flag = if i == 0 { 1.0 } else { 0.0 };
        dcol[i] = (col * k).extend(flag);
        raw[i] = (rcol * k).extend(flag);
    }
    (dvec, dcol, raw)
}

/// The directional lights a Stock-look surface (hull or planet) is lit by: when the map aimed its own
/// lights (`MapLightSource`), only those, else every directional light. Hulls and planets share this, so
/// a light that is not the map's (a viewer or menu light) cannot light one and not the other while both are Stock.
pub fn stock_light_set<'a>(lights: impl Iterator<Item = (&'a GlobalTransform, &'a DirectionalLight, Option<&'a crate::map::MapLightSource>)> + Clone) -> Vec<(Vec3, &'a DirectionalLight)> {
    let own = lights.clone().any(|(_, _, s)| s.is_some_and(|s| s.0));
    lights.filter(|(_, _, s)| !own || s.is_some_and(|s| s.0)).map(|(t, l, _)| (t.back().as_vec3(), l)).collect()
}

/// Each frame: the eye and the lights, written into the materials when they changed.
#[allow(clippy::too_many_arguments)]
fn update_view(
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    sun: Query<(&GlobalTransform, &DirectionalLight, Option<&crate::map::MapLightSource>)>,
    ambient: Option<Res<GlobalAmbientLight>>,
    profile: Option<Res<crate::render::profile::RenderProfile>>,
    effects: Option<Res<crate::render::profile::ProfileEffects>>,
    standard: Res<Assets<StandardMaterial>>,
    mut view: ResMut<HullView>,
    made: Res<HullMaterials>,
    mut materials: ResMut<Assets<HullMaterial>>,
    switches: Option<Res<crate::render::quality::PerfSwitches>>,
) {
    if switches.is_some_and(|s| s.freeze_hull_materials) {
        return;
    }
    let Ok(cam) = camera.single() else { return };
    let (dvec, dcol, _) = lights(sun.iter().map(|(t, l, _)| {
        let c = l.color.to_linear();
        (t.back().as_vec3(), Vec3::new(c.red, c.green, c.blue), l.illuminance, Vec3::ZERO)
    }));
    // Stock: the map's own lights, each at its colour (the original sets one D3D light per
    // `dlight`); a scene that has none (the viewer) takes every light as above.
    let own = sun.iter().any(|(_, _, s)| s.is_some_and(|s| s.0));
    let (sdvec, _, sdcol) = lights(stock_light_set(sun.iter()).into_iter().map(|(to, l)| {
        let r = l.color.to_srgba();
        (to, Vec3::ZERO, if own { 1.0 } else { l.illuminance }, Vec3::new(r.red, r.green, r.blue))
    }));
    let stock = matches!((profile, effects), (Some(p), Some(e)) if e.implementation(*p, crate::render::profile::Effect::Hulls) == crate::render::profile::RenderProfile::Stock);
    let amb = ambient.map_or(Vec3::ZERO, |a| {
        let c = a.color.to_srgba();
        Vec3::new(c.red, c.green, c.blue)
    });
    let now = HullView { eye: cam.translation().extend(1.0), dvec, dcol, sdvec, sdcol, stock, ambient: amb, ..view.clone() };
    let moved = (now.eye - view.eye).length_squared() > 1e-4;
    if !moved && now.dvec == view.dvec && now.dcol == view.dcol && now.sdvec == view.sdvec && now.sdcol == view.sdcol && now.stock == view.stock && now.ambient == view.ambient {
        return;
    }
    let switched = now.stock != view.stock;
    *view = now;
    if switched {
        // The profile changed: every hull material is made again in the other look.
        for (id, (h, part)) in &made.0 {
            if let (Some(std), Some(mut m)) = (standard.get(*id), materials.get_mut(h)) {
                let shadow = m.shadow.clone();
                *m = hull_material(std, part, &view);
                m.shadow = shadow;
            }
        }
    }
    for (_, m) in materials.iter_mut() {
        view.write(&mut m.params);
    }
}

/// The part is a hull's lit surface, which this look draws.
fn takes_hull_look(m: &StandardMaterial, part: &HullPart) -> bool {
    matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_))
        && (!m.unlit || part.self_illum)
        && m.emissive_texture.is_none()
        && m.metallic_roughness_texture.is_none()
}

/// The hull material for a standard one: Refit's `[Hulls]` values, the texture, the
/// normal map and the team colour (the base colour) carried over.
pub fn hull_material(m: &StandardMaterial, part: &HullPart, view: &HullView) -> HullMaterial {
    let bump = m.normal_map_texture.is_some();
    let (amb, spec, pow, rim, sun) = if bump {
        (BORG_AMBIENT, BORG_SPEC, BORG_SPEC_POW, BORG_RIM, BORG_SUN)
    } else {
        (AMBIENT, SPEC, SPEC_POW, RIM, SUN)
    };
    let cut = matches!(m.alpha_mode, AlphaMode::Mask(_)) || part.cut_alpha;
    let mut params = HullParams {
        amb: Vec3::from(amb).extend(if part.self_illum { SELF_ILLUM } else { 0.0 }),
        misc: Vec4::new(1.0, spec, pow, sun),
        rim: Vec3::from(rim).extend(RIM_POW),
        cut: Vec4::new(if cut { 1.0 } else { 0.0 }, if bump { BUMP } else { 0.0 }, KNEE, 0.0),
        ..default()
    };
    if view.stock {
        crate::render::stock::hulls::apply_material(&mut params, m, part);
    }
    set_source(&mut params, m);
    view.write(&mut params);
    HullMaterial {
        params,
        texture: m.base_color_texture.clone(),
        normal: m.normal_map_texture.clone(),
        shadow: view.shadow.clone(),
        double_sided: m.double_sided || m.cull_mode.is_none(),
    }
}

/// What follows the `StandardMaterial` while it is drawn as a hull: the team colour
/// (the viewer's T) and the flipbook's UV offset.
fn set_source(p: &mut HullParams, m: &StandardMaterial) {
    let c = m.base_color.to_linear();
    p.tint = Vec4::new(c.red, c.green, c.blue, 0.0);
    p.uv = Vec4::new(m.uv_transform.translation.x, m.uv_transform.translation.y, 0.0, 0.0);
}

/// Give new hull parts their material, and show each part the look the switch says.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn apply(
    mut commands: Commands,
    lighting: Res<HullLighting>,
    view: Res<HullView>,
    standard: Res<Assets<StandardMaterial>>,
    mut hulls: ResMut<Assets<HullMaterial>>,
    mut made: ResMut<HullMaterials>,
    fresh: Query<(Entity, &HullPart, &MeshMaterial3d<StandardMaterial>), Without<HullSource>>,
    mut shown: Query<(Entity, &mut HullSource)>,
) {
    if !fresh.is_empty() {
        made.0.retain(|id, _| standard.contains(*id));
    }
    for (e, part, mat) in &fresh {
        let Some(std) = standard.get(&mat.0) else { continue };
        if !takes_hull_look(std, part) {
            continue;
        }
        let hull = if let Some((handle, cached_part)) = made.0.get_mut(&mat.0.id()) {
            // Appearance can rebind the same private source after capture.
            // Refresh its cached conversion before showing it again.
            if let Some(mut material) = hulls.get_mut(&*handle) {
                *material = hull_material(std, part, &view);
            }
            *cached_part = *part;
            handle.clone()
        } else {
            let handle = hulls.add(hull_material(std, part, &view));
            made.0.insert(mat.0.id(), (handle.clone(), *part));
            handle
        };
        let mut ec = commands.entity(e);
        ec.try_insert(HullSource { source: mat.0.clone(), hull: hull.clone(), shown: lighting.on });
        if lighting.on {
            ec.try_insert(MeshMaterial3d(hull)).try_remove::<MeshMaterial3d<StandardMaterial>>();
        }
    }
    if !lighting.is_changed() {
        return;
    }
    for (e, mut s) in &mut shown {
        if s.shown == lighting.on {
            continue;
        }
        s.shown = lighting.on;
        let mut ec = commands.entity(e);
        if lighting.on {
            ec.insert(MeshMaterial3d(s.hull.clone())).remove::<MeshMaterial3d<StandardMaterial>>();
        } else {
            ec.insert(MeshMaterial3d(s.source.clone())).remove::<MeshMaterial3d<HullMaterial>>();
        }
    }
}

/// Carry a changed `StandardMaterial`'s team colour and UV offset over to its hull
/// material (the viewer's T key and the flipbooks write the standard one).
fn follow_source(
    mut changed: MessageReader<AssetEvent<StandardMaterial>>,
    standard: Res<Assets<StandardMaterial>>,
    mut made: ResMut<HullMaterials>,
    mut hulls: ResMut<Assets<HullMaterial>>,
) {
    for ev in changed.read() {
        if let AssetEvent::Removed { id } = ev {
            if let Some((handle, _)) = made.0.remove(id) {
                hulls.remove(handle.id());
            }
            continue;
        }
        let AssetEvent::Modified { id } = ev else { continue };
        let (Some((h, _)), Some(std)) = (made.0.get(id), standard.get(*id)) else { continue };
        let Some(mut m) = hulls.get_mut(h) else { continue };
        set_source(&mut m.params, std);
        m.texture = std.base_color_texture.clone();
        m.normal = std.normal_map_texture.clone();
    }
}

/// Whether a face group takes the owner's team colour (RND-17). The one rule the
/// engine and Refit use: the group's lighting material name starts with `team`
/// (`team_basic_shaderbump`); every other group is drawn white. The texture's alpha is
/// not a team mask: it is the self-illumination (`hulls.wesl` leaves those texels
/// untinted).
pub fn is_team_material(material: &str) -> bool {
    material.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("team"))
}

/// (team-coloured faces, all faces) of a model, by [`is_team_material`].
#[cfg(test)]
fn team_share(model: &sta_assets::model::Model) -> (usize, usize) {
    let groups = model.nodes.iter().filter_map(|n| n.mesh()).flat_map(|m| m.groups.iter());
    groups.fold((0, 0), |(t, all), g| (t + if is_team_material(&g.material) { g.faces.len() } else { 0 }, all + g.faces.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_team_materials_take_the_team_colour() {
        assert!(is_team_material("team_basic_shaderbump"));
        assert!(is_team_material("Team"));
        assert!(!is_team_material("basic_shaderbump"));
        assert!(!is_team_material("tea"));
        assert!(!is_team_material("glow_white"));
        assert!(!is_team_material("é"));
    }

    /// Install-gated (`STA64_TEST_INSTALL`, optional `STA64_TEST_MOD`; skips when
    /// absent): per race, the team-coloured share of each start unit's faces. The
    /// share is exactly the faces of `team*` groups, and a start unit is fully tinted
    /// only when every group of its model is a team group. The Federation construction
    /// ship keeps untinted faces beside its team ones.
    #[test]
    fn install_start_units_team_share() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            eprintln!("SKIP: STA64_TEST_INSTALL not set");
            return;
        };
        let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
        let (races, _) = sta_content::race::Races::load(&vfs, &index);
        let mut seen = 0;
        for r in races.playable() {
            for k in races.loadout(&db, &r.name).units {
                let def = db.unit(k).unwrap();
                let key = def.model.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
                let Some((path, fmt)) = sta_assets::find_model(&vfs, &key) else { continue };
                let model = sta_assets::load_model(&vfs, &path, fmt).unwrap();
                let (team, all) = team_share(&model);
                let materials: Vec<&str> = model.nodes.iter().filter_map(|n| n.mesh()).flat_map(|m| m.groups.iter()).map(|g| g.material.as_str()).collect();
                println!("{:<18} {:<22} {:>5}/{:<5} faces team = {:>3}%", r.name, def.odf, team, all, team * 100 / all.max(1));
                assert!(all > 0, "{} has no faces", def.odf);
                // Fully tinted only when the data marks every group team.
                assert_eq!(team == all, materials.iter().all(|m| is_team_material(m)), "{}", def.odf);
                // A construction ship has grey parts beside the team ones.
                if def.class.0.contains("construct") || def.odf.ends_with("const") {
                    assert!(team > 0 && team < all, "{} is {team}/{all} team", def.odf);
                }
                seen += 1;
            }
        }
        assert!(seen > 0, "no start unit had a model");
    }

    /// RND-20 data check, install-gated (`STA64_TEST_INSTALL`, optional
    /// `STA64_TEST_MOD`): the engine multiplies the lit texel by the team colour (the
    /// shader does the same; self-illuminated texels, alpha, stay untinted). For the
    /// Federation construction ship, the mean lit team-group texel times each palette
    /// colour: with the engine's own white/federation/gray colours the hull stays near
    /// grey (saturation low); saturated player colours tint it as in game.
    #[test]
    fn install_fconst_team_tint_stays_grey_for_pale_colours() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            eprintln!("SKIP: STA64_TEST_INSTALL not set");
            return;
        };
        let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
        let (races, _) = sta_content::race::Races::load(&vfs, &index);
        let Some(model) = races
            .playable().into_iter()
            .filter(|r| r.name == "unitedfederation")
            .flat_map(|r| races.loadout(&db, &r.name).units)
            .filter_map(|k| db.unit(k))
            .filter(|d| d.odf.ends_with("const"))
            .find_map(|d| {
                let key = d.model.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
                let (path, fmt) = sta_assets::find_model(&vfs, &key)?;
                sta_assets::load_model(&vfs, &path, fmt).ok()
            })
        else {
            eprintln!("SKIP: no Federation construction ship");
            return;
        };
        let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
        for g in model.nodes.iter().filter_map(|n| n.mesh()).flat_map(|m| m.groups.iter()).filter(|g| is_team_material(&g.material)) {
            let Some(tex) = g.maps.diffuse.as_deref() else { continue };
            let Some((p, f)) = sta_assets::find_texture(&vfs, tex) else { continue };
            let Ok(img) = sta_assets::decode_rgba(&vfs, &p, f) else { continue };
            // Weight by the group's faces; lit texels only (alpha low).
            let w = g.faces.len() as f64;
            let (mut s, mut c) = ([0.0f64; 3], 0.0f64);
            for px in img.pixels.chunks_exact(4).filter(|px| !img.has_alpha || px[3] < 128) {
                for k in 0..3 {
                    s[k] += px[k] as f64 / 255.0;
                }
                c += 1.0;
            }
            if c > 0.0 {
                for k in 0..3 {
                    sum[k] += s[k] / c * w;
                }
                n += w;
            }
        }
        if n == 0.0 {
            eprintln!("SKIP: fconst has no readable team textures");
            return;
        }
        let mean = sum.map(|v| v / n);
        let sat = |c: [f64; 3]| {
            let mx = c.iter().cloned().fold(0.0, f64::max);
            let mn = c.iter().cloned().fold(1.0, f64::min);
            if mx > 0.0 { (mx - mn) / mx } else { 0.0 }
        };
        println!("fconst mean lit team texel {mean:.3?} saturation {:.2}", sat(mean));
        for (name, col) in crate::viewer::TeamColours::parse(crate::viewer::STOCK_TEAMCOLOR) {
            let c = col.to_srgba();
            let tinted = [mean[0] * c.red as f64, mean[1] * c.green as f64, mean[2] * c.blue as f64];
            println!("  {name:<12} -> {tinted:.3?} saturation {:.2}", sat(tinted));
            if matches!(name.as_str(), "white" | "federation" | "gray" | "empty") {
                assert!(sat(tinted) < 0.35, "{name}: {tinted:?} is not near-grey");
            }
        }
    }

    /// The shader must parse and validate (naga, as wgpu runs it), with Bevy's
    /// material bind group filled in.
    #[test]
    fn hull_shader_validates() {
        let src = include_str!("hulls.wesl").replace("constants::MATERIAL_BIND_GROUP", "3");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    /// The path `embedded_asset!` registers must be the one the material asks for.
    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "hulls.wesl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/render/hulls.wesl");
    }

    fn standard() -> StandardMaterial {
        StandardMaterial { base_color: Color::WHITE, ..default() }
    }

    #[test]
    fn keeps_what_the_hull_look_does_not_draw() {
        let part = HullPart::default();
        assert!(takes_hull_look(&standard(), &part));
        assert!(takes_hull_look(&StandardMaterial { alpha_mode: AlphaMode::Mask(0.5), ..standard() }, &part));
        // Glows and weapon parts, blended groups and skies keep Bevy's materials.
        assert!(!takes_hull_look(&StandardMaterial { alpha_mode: AlphaMode::Add, unlit: true, ..standard() }, &part));
        assert!(!takes_hull_look(&StandardMaterial { alpha_mode: AlphaMode::Blend, ..standard() }, &part));
        assert!(!takes_hull_look(&StandardMaterial { unlit: true, ..standard() }, &part));
        // A self-illuminating group is lit by the hull look (the texture's alpha is its glow).
        assert!(takes_hull_look(&StandardMaterial { unlit: true, ..standard() }, &HullPart { self_illum: true, ..part }));
        // M3D parts with glow or specular maps stay on PBR.
        assert!(!takes_hull_look(&StandardMaterial { emissive_texture: Some(Handle::default()), ..standard() }, &part));
    }

    #[test]
    fn profile_follows_the_normal_map() {
        let view = HullView::default();
        let plain = hull_material(&standard(), &HullPart::default(), &view);
        assert_eq!(plain.params.misc, Vec4::new(1.0, SPEC, SPEC_POW, SUN));
        assert_eq!(plain.params.cut.y, 0.0);
        let bumped = hull_material(&StandardMaterial { normal_map_texture: Some(Handle::default()), ..standard() }, &HullPart::default(), &view);
        assert_eq!(bumped.params.misc, Vec4::new(1.0, BORG_SPEC, BORG_SPEC_POW, BORG_SUN));
        assert_eq!(bumped.params.cut.y, BUMP);
        let glow = hull_material(&standard(), &HullPart { self_illum: true, cut_alpha: false, ..default() }, &view);
        assert_eq!(glow.params.amb.w, SELF_ILLUM);
        assert_eq!(glow.params.cut.x, 0.0);
        let open = hull_material(&standard(), &HullPart { self_illum: false, cut_alpha: true, ..default() }, &view);
        assert_eq!(open.params.cut.x, 1.0);
    }

    #[test]
    fn team_colour_and_two_sidedness_carry_over() {
        let std = StandardMaterial { base_color: Color::linear_rgb(0.2, 0.4, 0.6), cull_mode: None, double_sided: true, ..default() };
        let m = hull_material(&std, &HullPart::default(), &HullView::default());
        assert_eq!(m.params.tint, Vec4::new(0.2, 0.4, 0.6, 0.0));
        assert!(m.double_sided);
        assert!(!hull_material(&StandardMaterial { cull_mode: Some(bevy::render::render_resource::Face::Back), ..standard() }, &HullPart::default(), &HullView::default()).double_sided);
    }

    #[test]
    fn lights_are_brightest_first_with_the_key_flagged() {
        let (dvec, dcol, _) = lights([
            (Vec3::X, Vec3::ONE, 1500.0, Vec3::ONE),
            (Vec3::Y, Vec3::new(1.0, 0.5, 0.0), 6000.0, Vec3::new(1.0, 0.7, 0.0)),
        ].into_iter());
        assert_eq!(dvec[0].truncate(), Vec3::Y);
        assert_eq!(dcol[0], Vec4::new(1.0, 0.5, 0.0, 1.0));
        assert_eq!(dvec[1].truncate(), Vec3::X);
        assert_eq!(dcol[1], Vec4::new(0.25, 0.25, 0.25, 0.0));
        assert_eq!(dcol[2], Vec4::ZERO);
    }
}

/// Opt-in GPU test: the hull material must build a real pipeline and draw, and its
/// frame must stay near Bevy's PBR for the same hull and lights.
///
/// Bevy loads `.wgsl` as plain WGSL, so a shader that used naga_oil's
/// `#{MATERIAL_BIND_GROUP}` or `#ifdef` fails to compile; the failure is only logged
/// (`ERROR`), the process exits 0 and nothing draws. The naga test above checks the
/// text; only a pipeline made on a GPU, through Bevy's own shader loader, shows what
/// the game does. This one draws a sphere (a textured, Borg-profile one too) under both
/// looks and fails on any `ERROR` event.
///
/// Needs a hardware adapter: prints `SKIP` and passes unless `STA64_GPU_TESTS=1`, and
/// skips when the only adapter is a software one. `STA64_GPU_OUT=<dir>` keeps the
/// frames as PNG.
#[cfg(test)]
mod gpu {
    use std::sync::Mutex;

    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::log::tracing::{self, Event, Subscriber};
    use bevy::log::tracing_subscriber::{layer::Context, Layer};
    use bevy::log::{BoxedLayer, LogPlugin};
    use bevy::prelude::*;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::render::RenderPlugin;
    use bevy::window::ExitCondition;

    use super::{HullLighting, HullPart, HullPlugin};

    static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static FRAMES: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());

    /// Keeps every ERROR event's message.
    struct ErrorLog;

    impl<S: Subscriber> Layer<S> for ErrorLog {
        fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
            if *event.metadata().level() != tracing::Level::ERROR {
                return;
            }
            struct Msg(String);
            impl tracing::field::Visit for Msg {
                fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                    self.0.push_str(&format!("{}={v:?} ", f.name()));
                }
            }
            let mut m = Msg(String::new());
            event.record(&mut m);
            if let Ok(mut e) = ERRORS.lock() {
                e.push(format!("{}: {}", event.metadata().target(), m.0));
            }
        }
    }

    fn error_layer(_: &mut App) -> Option<BoxedLayer> {
        Some(Box::new(ErrorLog))
    }

    const SIZE: u32 = 96;

    /// A checker texture with a varying alpha channel, so the textured hull has detail.
    fn checker() -> Image {
        let mut px = Vec::new();
        for y in 0..16u32 {
            for x in 0..16u32 {
                let on = (x / 4 + y / 4) % 2 == 0;
                px.extend_from_slice(&if on { [200, 190, 170, 255] } else { [90, 100, 120, 255] });
            }
        }
        Image::new(Extent3d { width: 16, height: 16, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)
    }

    fn capture(app: &mut App, target: &Handle<Image>, name: &str) {
        let before = FRAMES.lock().unwrap().len();
        let owned = name.to_owned();
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |c: On<ScreenshotCaptured>| {
            FRAMES.lock().unwrap().push((owned.clone(), c.image.data.clone().unwrap_or_default()));
        });
        for _ in 0..40 {
            app.update();
            if FRAMES.lock().unwrap().len() > before {
                return;
            }
        }
        panic!("no frame came back for {name}");
    }

    /// (mean absolute difference per channel, mean luminance a, mean luminance b, pixels
    /// lit in a, in b, pixels whose lit-ness differs), over 8-bit sRGB.
    fn compare(a: &[u8], b: &[u8]) -> (f32, f32, f32, usize, usize, usize) {
        let bg = |p: &[u8]| p[0] < 8 && p[1] < 8 && p[2] < 12;
        let lum = |p: &[u8]| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
        let (mut diff, mut la, mut lb, mut na, mut nb, mut mism) = (0.0, 0.0, 0.0, 0, 0, 0);
        for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            diff += (0..3).map(|c| (pa[c] as f32 - pb[c] as f32).abs()).sum::<f32>() / 3.0;
            let (ia, ib) = (!bg(pa), !bg(pb));
            if ia {
                na += 1;
                la += lum(pa);
            }
            if ib {
                nb += 1;
                lb += lum(pb);
            }
            mism += usize::from(ia != ib);
        }
        let n = (a.len() / 4) as f32;
        (diff / n, la / na.max(1) as f32, lb / nb.max(1) as f32, na, nb, mism)
    }

    fn save(name: &str, data: &[u8]) {
        let Ok(dir) = std::env::var("STA64_GPU_OUT") else { return };
        let _ = std::fs::create_dir_all(&dir);
        let img = Image::new(Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 }, TextureDimension::D2, data.to_vec(), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
        if let Ok(d) = img.try_into_dynamic() {
            let _ = d.to_rgb8().save(format!("{dir}/{name}.png"));
        }
    }

    #[test]
    fn hull_material_builds_draws_and_stays_near_pbr() {
        if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
            eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU shader test");
            return;
        }
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .set(LogPlugin { level: bevy::log::Level::WARN, custom_layer: error_layer, ..default() })
                .disable::<bevy::winit::WinitPlugin>(),
        );
        app.add_plugins(HullPlugin);
        app.insert_resource(ClearColor(Color::srgb(0.01, 0.01, 0.025)));
        app.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 60.0, ..default() });

        let mut img = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
        img.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(img);
        let texture = app.world_mut().resource_mut::<Assets<Image>>().add(checker());
        let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(1.0).mesh().uv(32, 18));
        // The viewer's lights: a Key and a fill.
        app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, ..default() }, Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y)));
        app.world_mut().spawn((DirectionalLight { illuminance: 1500.0, ..default() }, Transform::from_xyz(-4.0, -2.0, -3.0).looking_at(Vec3::ZERO, Vec3::Y)));
        let camera = app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::from(target.clone()),
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        )).id();
        // Two-sided, so the shader's back-face branch and the specialised pipeline run too.
        let std = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture),
            perceptual_roughness: 0.6,
            reflectance: 0.3,
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        let subject = app.world_mut().spawn((Mesh3d(sphere), MeshMaterial3d(std.clone()), HullPart::default())).id();

        app.finish();
        app.cleanup();
        for _ in 0..10 {
            app.update();
        }
        // The adapter lives in the render world, which has it once the first frames have run.
        let info = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
            app.get_sub_app(bevy::render::RenderApp)
                .and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
        });
        if let Some(info) = info {
            eprintln!("GPU adapter: {} ({:?}, {:?})", info.name, info.device_type, info.backend);
            if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
                eprintln!("SKIP: only a software adapter is available ({})", info.name);
                return;
            }
        } else {
            eprintln!("SKIP: no render adapter");
            return;
        }
        app.world_mut().resource_mut::<HullLighting>().on = false;
        for _ in 0..4 {
            app.update();
        }
        capture(&mut app, &target, "pbr");
        app.world_mut().resource_mut::<HullLighting>().on = true;
        for _ in 0..4 {
            app.update();
        }
        capture(&mut app, &target, "hull");

        let frames = FRAMES.lock().unwrap().clone();
        let (pbr, hull) = (&frames[0].1, &frames[1].1);
        save("pbr", pbr);
        save("hull", hull);
        let (diff, la, lb, na, nb, mism) = compare(pbr, hull);
        eprintln!("frame diff, pbr vs hull: mean abs {diff:.2}/255, mean lit luminance {la:.1} vs {lb:.1}, lit pixels {na} vs {nb}, silhouette mismatch {mism}");
        // It drew (the hull is on screen) and it is a different look.
        assert!(nb > 500, "the hull drew {nb} pixels");
        assert!(diff > 0.5, "the A/B switch changed nothing (diff {diff})");
        // Same silhouette, and a frame within tolerance of Bevy's PBR: Refit's look is
        // not PBR, so this is a bound on how far it may drift, not a match.
        assert!(mism <= na / 50, "silhouette differs by {mism} of {na} pixels");
        assert!(diff < 45.0, "mean frame difference {diff}/255 against PBR");
        assert!((la - lb).abs() < 60.0, "mean luminance {la:.1} against {lb:.1}");

        // A reflected, single-sided hull must keep the same silhouette when
        // the map's depth prepass is enabled. This needs no installed assets.
        app.world_mut().resource_mut::<HullLighting>().on = false;
        for _ in 0..4 { app.update(); }
        let single_sided = {
            let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            let mut source = materials.get(&std).unwrap().clone();
            source.cull_mode = Some(bevy::render::render_resource::Face::Back);
            source.double_sided = false;
            materials.add(source)
        };
        app.world_mut().entity_mut(subject).insert((
            MeshMaterial3d(single_sided),
            Transform::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
        ));
        app.world_mut().resource_mut::<HullLighting>().on = true;
        for _ in 0..6 { app.update(); }
        capture(&mut app, &target, "mirrored-bare");
        app.world_mut().entity_mut(camera).insert(bevy::core_pipeline::prepass::DepthPrepass);
        for _ in 0..6 { app.update(); }
        capture(&mut app, &target, "mirrored-prepass");
        let frames = FRAMES.lock().unwrap().clone();
        let bare = &frames.iter().find(|f| f.0 == "mirrored-bare").unwrap().1;
        let depth = &frames.iter().find(|f| f.0 == "mirrored-prepass").unwrap().1;
        save("mirrored-bare", bare);
        save("mirrored-prepass", depth);
        let (_, _, _, before, after, mismatch) = compare(bare, depth);
        eprintln!("reflected hull: bare {before} lit pixels; prepass {after}; silhouette mismatch {mismatch}");
        assert!(before > 500 && after * 100 >= before * 95, "depth prepass hides reflected hull: {before} -> {after}");
        assert!(mismatch <= before / 50, "depth prepass changes reflected silhouette by {mismatch} pixels");
        let errors = ERRORS.lock().unwrap().clone();
        assert!(errors.is_empty(), "shader/pipeline ERROR events:\n{}", errors.join("\n"));
    }
}

#[cfg(test)]
#[path = "hulls/appearance_tests.rs"]
mod appearance_tests;
