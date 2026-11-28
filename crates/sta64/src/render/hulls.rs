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
//! colour, the others scaled by their illuminance over the Key's, as `rock.rs` does), written
//! into the hull materials only when they change. The eye is not a material value: the shader reads
//! the world position of the view being drawn from Bevy's view uniform, so a camera move rewrites
//! no material (#1251).

use std::collections::HashMap;

mod emissive_bloom;
mod emissive_blur;
mod emissive_state;

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

/// The most points kept per hull part for the death sparks (see [`HullSample`]).
pub const HULL_SAMPLE_MAX: usize = 256;

/// A few of a hull part's vertices, in the part's own space, kept by the client in a form of its own: the part's mesh
/// is handed to the renderer (its vertex data lives in the render world only, #1426) and cannot be read at run time.
/// An even stride through the vertices, at most [`HULL_SAMPLE_MAX`]: a few KB per distinct part, shared by every copy
/// through the part cache, where keeping the meshes' main-world data would cost every hull its full vertex and index
/// buffers for the whole session.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct HullSample {
    /// The sampled vertices.
    pub points: std::sync::Arc<[[f32; 3]]>,
    /// How many vertices the part has in all (at least `points.len()`): a unit's parts are weighted by these, so that
    /// every vertex of the whole model is as likely as any other, however small the part.
    pub full: usize,
}

/// The sample of `positions`: all of them when there are few, else an even stride through them.
pub fn hull_sample(positions: &[[f32; 3]]) -> HullSample {
    let points = if positions.len() <= HULL_SAMPLE_MAX {
        positions.into()
    } else {
        (0..HULL_SAMPLE_MAX).map(|i| positions[i * positions.len() / HULL_SAMPLE_MAX]).collect()
    };
    HullSample { points, full: positions.len() }
}

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
    /// The group is a team face the original draws with the team colour in place of its material's
    /// colours (a `team*` material, the Borg glow family left out). Only the Armada I Stock look reads it
    /// (`render/stock/hulls.rs`).
    pub team_face: bool,
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
    pub cut: Vec4,
    pub uv: Vec4,
    pub dvec: [Vec4; DIRS],
    pub dcol: [Vec4; DIRS],
    /// The Key's shadow map (`shadows.rs`): world to u, v, depth along the Key.
    pub sh: [Vec4; 3],
    /// x 1 with a map; y one texel (u, v); z the depth bias; w how much of the Key a shadow takes.
    pub shp: Vec4,
    /// Stock profile (RND-24): the SOD lighting material (`mamb.w` 1 draws the group the
    /// stock way, 2 the Armada I stock way (scene ambient added to the material's, the sum
    /// clamped to 1), 0 as Refit's look does).
    pub mamb: Vec4,
    pub mdif: Vec4,
    pub mspc: Vec4,
    /// Specular and emissive maps (RND-38, `mapped.rs`): x 1 with a specular map, y 1 with an
    /// emissive map.
    pub map: Vec4,
    /// The highlight of a part with a specular map: colour and exponent (`mapped::highlight_params`).
    pub mhl: Vec4,
    /// Live weights for warp, impulse, shields, life support, sensors and weapons.
    pub emission_a: Vec4,
    /// Sensors/weapons in xy, additional warp/impulse bloom gain in zw.
    pub emission_b: Vec4,
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
    /// The specular map (RND-38): its RGB masks the highlight, read when `params.map.x` says there is one.
    #[texture(6)]
    #[sampler(7)]
    pub spec_map: Option<Handle<Image>>,
    /// The emissive map (RND-38), added after lighting when `params.map.y` says there is one. Its own
    /// sampler: the map's repeat sampler, whatever the specular binding holds.
    #[texture(8, dimension = "2d_array")]
    #[sampler(9)]
    pub emissive_map: Option<Handle<Image>>,
    pub double_sided: bool,
}

impl HullMaterial {
    /// Carry a part's specular and emissive maps (RND-38) into the material and its flags.
    pub fn set_maps(&mut self, maps: Option<&crate::render::mapped::HullMaps>, switches: crate::render::mapped::MapSwitches) {
        self.spec_map = maps.and_then(|m| m.specular.clone()).filter(|_| switches.specular);
        self.emissive_map = maps.and_then(|m| m.emissive.clone()).filter(|_| switches.emissive);
        self.params.map.x = u8::from(self.spec_map.is_some()) as f32;
        self.params.map.y = u8::from(self.emissive_map.is_some()) as f32;
    }

    fn emission_weights(gains: emissive_state::EmissiveGains) -> (Vec4, Vec4) {
        let [warp, impulse, shields, life, sensors, weapons] = gains.surface;
        let [warp_bloom, impulse_bloom] = gains.halo_boost;
        (Vec4::new(warp, impulse, shields, life), Vec4::new(sensors, weapons, warp_bloom, impulse_bloom))
    }
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

    /// A cut-out part is a mask material, and its depth prepass runs a fragment shader with the same alpha
    /// test as the colour pass, so a texel the test rejects writes no depth. Bevy 0.20.1 gives a prepass
    /// pipeline a fragment stage only for a mask material that names a prepass fragment shader
    /// (`MAY_DISCARD` plus `get_shader(PrepassFragmentShader)`); an opaque part keeps the depth-only
    /// pipeline, so non-cut parts are drawn as before. That holds only while no camera has a normal,
    /// motion-vector or deferred prepass and no unclipped-depth emulation runs: otherwise Bevy uses this
    /// output-less shader for EVERY hull part and every hull's prepass pipeline breaks (none does today).
    /// The same pipeline serves Bevy's shadow depth pass, which no light enables; the ship shadows
    /// (`shadows.rs`) use their own caster material and are not cut-out.
    fn alpha_mode(&self) -> AlphaMode {
        if self.params.cut.x > 0.5 { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque }
    }

    fn prepass_fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/hulls_prepass.wesl").into_owned())
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

/// The scene's lights, as the shader takes them.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct HullView {
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
    /// An Armada I install (`Sta1Classic`): the Stock look follows the Armada I lighting rules:
    /// scene ambient added to the material's (`mamb.w` 2), and no specular highlight from the
    /// SOD lighting material (its values are read and kept). Armada II and Fleet Operations
    /// are not changed by this.
    pub(crate) armada_one: bool,
}

impl HullView {
    /// The view for the Stock look (tests).
    #[cfg(test)]
    pub(crate) fn set_stock(&mut self, on: bool) {
        self.stock = on;
    }

    /// The view of an Armada I install (tests).
    #[cfg(test)]
    pub(crate) fn set_armada_one(&mut self, on: bool) {
        self.armada_one = on;
    }

    pub(crate) fn write(&self, p: &mut HullParams) {
        if p.mamb.w > 0.5 {
            p.amb = self.ambient.extend(p.amb.w);
        }
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
        embedded_asset!(app, "hulls_prepass.wesl");
        app.add_plugins(MaterialPlugin::<HullMaterial>::default())
            .init_resource::<HullLighting>()
            .init_resource::<HullView>()
            .init_resource::<HullMaterials>()
            .insert_resource(crate::render::mapped::MapSwitches::startup())
            .add_plugins(crate::render::stock::hulls::StockHullsPlugin)
            .add_systems(Update, (
                toggle, update_view, apply.in_set(HullApply),
                update_emissive.after(crate::app::GameSet::SimLink).after(crate::clock::advance_idle_clock),
                follow_source,
            ).chain());
        emissive_bloom::register(app);
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

/// Each frame: the lights, written into the materials when they changed.
#[allow(clippy::too_many_arguments)]
fn update_view(
    sun: Query<(&GlobalTransform, &DirectionalLight, Option<&crate::map::MapLightSource>)>,
    ambient: Option<Res<GlobalAmbientLight>>,
    profile: Option<Res<crate::render::profile::RenderProfile>>,
    effects: Option<Res<crate::render::profile::ProfileEffects>>,
    content: Option<Res<crate::sim_link::ContentRes>>,
    standard: Res<Assets<StandardMaterial>>,
    mut view: ResMut<HullView>,
    sources: Query<(&HullSource, &HullPart)>,
    mut materials: ResMut<Assets<HullMaterial>>,
    switches: Option<Res<crate::render::quality::PerfSwitches>>,
) {
    if switches.is_some_and(|s| s.freeze_hull_materials) {
        return;
    }
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
    let armada_one = content.is_some_and(|c| c.0.profile() == sta_content::Profile::Sta1Classic);
    // A scene with no map (the front-end menu, the viewer, a bench scene) sets the ambient light as a
    // knob for Bevy's renderer (plain white), which is no scene ambient. Under the Armada I rule the
    // ambient is added to the material's, so such a scene takes none (stated default: zero, the
    // material's own ambient and the scene's lights alone). A map view's lights carry `MapLightSource`.
    let has_map = sun.iter().any(|(_, _, s)| s.is_some());
    let amb = ambient.filter(|_| !armada_one || has_map).map_or(Vec3::ZERO, |a| {
        let c = a.color.to_srgba();
        Vec3::new(c.red, c.green, c.blue)
    });
    let now = HullView { dvec, dcol, sdvec, sdcol, stock, ambient: amb, armada_one, ..view.clone() };
    if now.dvec == view.dvec && now.dcol == view.dcol && now.sdvec == view.sdvec && now.sdcol == view.sdcol && now.stock == view.stock && now.armada_one == view.armada_one && now.ambient == view.ambient {
        return;
    }
    let switched = now.stock != view.stock || now.armada_one != view.armada_one;
    *view = now;
    if switched {
        // The profile changed: every hull material is made again in the other look.
        let mut updated = std::collections::HashSet::new();
        for (source, part) in &sources {
            if !updated.insert(source.hull.id()) {
                continue;
            }
            if let (Some(std), Some(mut m)) = (standard.get(&source.source), materials.get_mut(&source.hull)) {
                let (shadow, spec, emissive) = (m.shadow.clone(), m.spec_map.clone(), m.emissive_map.clone());
                let (emission_a, emission_b) = (m.params.emission_a, m.params.emission_b);
                *m = hull_material(std, part, &view);
                m.shadow = shadow;
                m.params.map.x = u8::from(spec.is_some()) as f32;
                m.params.map.y = u8::from(emissive.is_some()) as f32;
                m.spec_map = spec;
                m.emissive_map = emissive;
                m.params.emission_a = emission_a;
                m.params.emission_b = emission_b;
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
        emission_a: Vec4::ONE,
        emission_b: Vec4::new(1.0, 1.0, 0.0, 0.0),
        ..default()
    };
    if view.stock {
        crate::render::stock::hulls::apply_material(&mut params, m, part, view.armada_one);
    }
    let mapped = crate::render::mapped::MapSwitches::startup();
    params.mhl = crate::render::mapped::highlight_params(part.stock.as_ref(), mapped.specular_material_colours);
    set_source(&mut params, m);
    view.write(&mut params);
    HullMaterial {
        params,
        texture: m.base_color_texture.clone(),
        normal: m.normal_map_texture.clone(),
        shadow: view.shadow.clone(),
        spec_map: None,
        emissive_map: None,
        double_sided: m.double_sided || m.cull_mode.is_none(),
    }
}

/// Each live hull gets its own channel weights; no image is rebuilt on a damage or movement update.
#[allow(clippy::too_many_arguments)]
fn update_emissive(
    link: Option<Res<crate::sim_link::SimLink>>,
    content: Option<Res<crate::sim_link::ContentRes>>,
    clock: Option<Res<crate::clock::AnimClock>>,
    parts: Query<(&ChildOf, &HullSource), With<crate::render::mapped::HullMaps>>,
    nodes: Query<&crate::render::appearance::ModelNode>,
    hierarchy: Query<&ChildOf>,
    units: Query<&crate::sim_link::SimEntity>,
    mut materials: ResMut<Assets<HullMaterial>>,
    mut states: Local<HashMap<sta_sim::EntityId, emissive_state::EmissiveState>>,
) {
    let Some(link) = link else {
        states.clear();
        return;
    };
    if link.is_added() {
        states.clear();
    }
    let view = link.real_curr();
    let elapsed = std::time::Duration::from_secs_f64(clock.as_deref().map_or(0.0, crate::clock::AnimClock::wall));
    states.retain(|id, _| view.unit(*id).is_some());
    let mut gains_this_frame = HashMap::new();
    for (parent, source) in &parts {
        if materials.get(&source.hull).is_none_or(|m| m.emissive_map.is_none()) {
            continue;
        }
        let Ok(node) = nodes.get(parent.parent()) else { continue };
        let mut root = node.root;
        let sim = loop {
            if let Ok(sim) = units.get(root) {
                break Some(sim.0);
            }
            let Ok(parent) = hierarchy.get(root) else { break None };
            root = parent.parent();
        };
        let Some(id) = sim else { continue };
        let Some(unit) = view.unit(id) else { continue };
        let gains = *gains_this_frame.entry(id).or_insert_with(|| {
            let cap = link.impulse_cap(id)
                .or_else(|| view.attrs.impulse.iter().find(|(owner, _)| *owner == id).map(|(_, speed)| *speed))
                .or_else(|| content.as_ref()?.0.unit(unit.def).map(|def| def.movement.max_speed))
                .unwrap_or_default();
            let phase = emissive_state::FlickerPhase::new(elapsed, id);
            states.entry(id).or_default().gains(view.appearance.systems(id), unit.speed, cap, link.warp_phase(id), phase)
        });
        let (a, b) = HullMaterial::emission_weights(gains);
        if materials.get(&source.hull).is_some_and(|m| m.params.emission_a != a || m.params.emission_b != b)
            && let Some(mut material) = materials.get_mut(&source.hull)
        {
            material.params.emission_a = a;
            material.params.emission_b = b;
        }
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
    fresh: Query<(Entity, &HullPart, &MeshMaterial3d<StandardMaterial>, Option<&crate::render::mapped::HullMaps>), Without<HullSource>>,
    mut shown: Query<(Entity, &mut HullSource)>,
    switches: Option<Res<crate::render::mapped::MapSwitches>>,
) {
    let switches = switches.map_or_else(crate::render::mapped::MapSwitches::default, |s| *s);
    if !fresh.is_empty() {
        made.0.retain(|id, _| standard.contains(*id));
    }
    for (e, part, mat, maps) in &fresh {
        let Some(std) = standard.get(&mat.0) else { continue };
        if !takes_hull_look(std, part) {
            continue;
        }
        let hull = if switches.emissive && maps.is_some_and(|maps| maps.emissive.is_some()) {
            // The images are shared, but two ships can have different damage and motion.
            let mut material = hull_material(std, part, &view);
            material.set_maps(maps, switches);
            hulls.add(material)
        } else if let Some((handle, cached_part)) = made.0.get_mut(&mat.0.id()) {
            // Appearance can rebind the same private source after capture.
            // Refresh its cached conversion before showing it again.
            if let Some(mut material) = hulls.get_mut(&*handle) {
                *material = hull_material(std, part, &view);
                material.set_maps(maps, switches);
            }
            *cached_part = *part;
            handle.clone()
        } else {
            let mut material = hull_material(std, part, &view);
            material.set_maps(maps, switches);
            let handle = hulls.add(material);
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
        let Some(found) = crate::testsupport::install_with_mod(sta_testkit::Kind::Any) else { return };
        let root = found.root_string();
        let m = found.mod_string();
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
        let Some(found) = crate::testsupport::install_with_mod(sta_testkit::Kind::Any) else { return };
        let root = found.root_string();
        let m = found.mod_string();
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
        let src = include_str!("hulls.wesl")
            .replace("constants::MATERIAL_BIND_GROUP", "3")
            .replace("@if(EMISSIVE_MASK)", "")
            .replace("import bevy_pbr::render::mesh_functions::get_tag;", "fn get_tag(i: u32) -> u32 { return 0u; }")
            // The view comes from Bevy's own shader library: a stub stands in for it.
            .replace("import bevy_pbr::render::mesh_view_bindings::view;", "struct ViewStub { world_position: vec3<f32> }\nvar<private> view: ViewStub;");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    /// The cut-out prepass shader parses and validates, and tests the same alpha as the colour pass.
    #[test]
    fn prepass_shader_validates_and_cuts_like_the_colour_pass() {
        let raw = include_str!("hulls_prepass.wesl");
        let src = raw.replace("constants::MATERIAL_BIND_GROUP", "3");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(raw.contains("k.cut.x > 0.5 && t.a < 0.5"));
        assert!(include_str!("hulls.wesl").contains("k.cut.x > 0.5 && t.a < 0.5"));
    }

    /// Only a cut-out part is a mask material (the one with a prepass fragment stage); the rest stay opaque.
    #[test]
    fn only_cut_parts_are_mask_materials() {
        let view = HullView::default();
        let cut = hull_material(&standard(), &HullPart { cut_alpha: true, ..default() }, &view);
        assert!(matches!(cut.alpha_mode(), AlphaMode::Mask(_)));
        let plain = hull_material(&standard(), &HullPart::default(), &view);
        assert!(matches!(plain.alpha_mode(), AlphaMode::Opaque));
    }

    /// The path `embedded_asset!` registers must be the one the material asks for.
    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "hulls_prepass.wesl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/render/hulls_prepass.wesl");
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

    use super::{HullLighting, HullMaterial, HullPart, HullPlugin};

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

    const SIZE: u32 = 256;

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
        let _serial = crate::bench::frames::gpu_serial();
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .set(LogPlugin { level: bevy::log::Level::WARN, custom_layer: error_layer, ..default() })
                .disable::<bevy::winit::WinitPlugin>(),
        );
        app.add_plugins((HullPlugin, crate::render::fx::quad::FxQuadPlugin));
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
        let named = |n: &str| &frames.iter().find(|f| f.0 == n).unwrap_or_else(|| panic!("no frame {n}")).1;
        let (pbr, hull) = (named("pbr"), named("hull"));
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
        // RND-38: a specular map masks the highlight and an emissive map adds to the lit colour,
        // on the pipeline Bevy made from the shader (maps set straight on the hull materials).
        {
            use crate::render::mapped::{HullMaps, MapSwitches};
            let flat = |rgb: [u8; 3], srgb: bool| {
                let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
                Image::new(Extent3d { width: 2, height: 2, depth_or_array_layers: 1 }, TextureDimension::D2, [rgb[0], rgb[1], rgb[2], 255].repeat(4), format, RenderAssetUsages::RENDER_WORLD)
            };
            let emit = |texture: sta_assets::Rgba| {
                let mut channels: [Option<sta_assets::Rgba>; 6] = Default::default();
                channels[1] = Some(texture);
                crate::render::textures::upload_emissive(&channels).unwrap()
            };
            let (white, black, red) = {
                let mut images = app.world_mut().resource_mut::<Assets<Image>>();
                (images.add(flat([255, 255, 255], false)), images.add(flat([0, 0, 0], false)), images.add(emit(sta_assets::Rgba { width: 2, height: 2, has_alpha: false, pixels: [255, 0, 0, 255].repeat(4) })))
            };
            let set = |app: &mut App, spec: Option<Handle<Image>>, emissive: Option<Handle<Image>>| {
                let maps = HullMaps { specular: spec, emissive };
                let ids: Vec<_> = app.world().resource::<Assets<HullMaterial>>().iter().map(|(id, _)| id).collect();
                let mut assets = app.world_mut().resource_mut::<Assets<HullMaterial>>();
                for id in ids {
                    if let Some(mut m) = assets.get_mut(id) {
                        m.set_maps(Some(&maps), MapSwitches::default());
                    }
                }
            };
            let sum = |d: &[u8], c: usize| d.chunks_exact(4).map(|p| p[c] as u64).sum::<u64>();
            let take = |app: &mut App, name: &str| {
                for _ in 0..4 {
                    app.update();
                }
                capture(app, &target, name);
                let frame = FRAMES.lock().unwrap().iter().rev().find(|f| f.0 == name).unwrap().1.clone();
                save(name, &frame);
                frame
            };
            // Upright again: the reflected part above shades inside out, which hides every highlight.
            app.world_mut().entity_mut(subject).insert(Transform::IDENTITY);
            set(&mut app, None, None);
            let base = take(&mut app, "mapped-none");
            set(&mut app, Some(white.clone()), None);
            let lit = take(&mut app, "mapped-white");
            set(&mut app, Some(black.clone()), None);
            let masked = take(&mut app, "mapped-black");
            set(&mut app, None, Some(red));
            let glow = take(&mut app, "mapped-emissive");
            eprintln!("RND-38 sums (G): none {} white {} black {}; (R): none {} emissive {}; (G): emissive {}", sum(&base, 1), sum(&lit, 1), sum(&masked, 1), sum(&base, 0), sum(&glow, 0), sum(&glow, 1));
            assert!(sum(&lit, 1) > sum(&base, 1), "a white specular map adds the default highlight");
            assert!(sum(&masked, 1) <= sum(&base, 1), "a black map adds none");
            assert!(sum(&glow, 0) > sum(&base, 0) + 10_000, "a red emissive map adds red");
            assert!(sum(&glow, 1) <= sum(&base, 1) + 1_000, "and only red");

            // The Stock look draws the same masks: no highlight of its own here (the part states no
            // lighting material), so a white map adds the defaults' highlight and a black one nothing.
            app.insert_resource(crate::render::profile::RenderProfile::Stock);
            for _ in 0..4 {
                app.update();
            }
            set(&mut app, None, None);
            let s_base = take(&mut app, "mapped-stock-none");
            set(&mut app, Some(white.clone()), None);
            let s_lit = take(&mut app, "mapped-stock-white");
            set(&mut app, Some(black.clone()), None);
            let s_masked = take(&mut app, "mapped-stock-black");
            eprintln!("RND-38 stock sums (G): none {} white {} black {}", sum(&s_base, 1), sum(&s_lit, 1), sum(&s_masked, 1));
            assert!(sum(&s_lit, 1) > sum(&s_base, 1) + 1_000, "Stock look: a white specular map adds the default highlight");
            assert!(sum(&s_masked, 1) <= sum(&s_base, 1) + 1_000, "Stock look: a black map adds none");

            // An emissive map alone, on UVs well outside 0..1: its own repeat sampler tiles it, so the
            // glow keeps its pattern (a clamped sampler would smear the edge texel over the sphere).
            let strip = app.world_mut().resource_mut::<Assets<Image>>().add(emit(sta_assets::Rgba {
                width: 2, height: 1, has_alpha: false, pixels: vec![255, 0, 0, 255, 0, 0, 0, 255],
            }));
            set(&mut app, None, None);
            let plain1 = take(&mut app, "mapped-uv1-none");
            set(&mut app, None, Some(strip.clone()));
            let tiled1 = take(&mut app, "mapped-uv1-strip");
            let mesh = app.world().entity(subject).get::<Mesh3d>().unwrap().0.clone();
            if let Some(mut m) = app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(&mesh)
                && let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = m.attribute_mut(Mesh::ATTRIBUTE_UV_0)
            {
                for u in uv.iter_mut() {
                    *u = [u[0] * 4.0, u[1] * 4.0];
                }
            }
            set(&mut app, None, None);
            let plain4 = take(&mut app, "mapped-uv4-none");
            set(&mut app, None, Some(strip));
            let tiled4 = take(&mut app, "mapped-uv4-strip");
            let (d1, d4) = (sum(&tiled1, 0) as i64 - sum(&plain1, 0) as i64, sum(&tiled4, 0) as i64 - sum(&plain4, 0) as i64);
            eprintln!("RND-38 emissive-only red gain: UV 0..1 {d1}, UV 0..4 {d4}");
            assert!(d1 > 10_000, "the strip glows on UVs inside 0..1");
            assert!(d4 * 10 > d1 * 6, "and on UVs outside 0..1 as much (repeat), not smeared from the edge ({d4} vs {d1})");

            // Compare the same scene with and without screen-space bloom.
            use crate::render::fx::quad::{self, Look};
            use crate::render::post::PostFx;
            use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
            app.world_mut().entity_mut(camera).insert((Tonemapping::None, DebandDither::Disabled));
            app.insert_resource(PostFx::default());
            let texels = sta_assets::Rgba {
                width: 64, height: 64, has_alpha: false,
                pixels: (0..4096).flat_map(|n| {
                    if n % 16 < 4 { [255, 0, 0, 255] } else { [0, 0, 0, 255] }
                }).collect(),
            };
            let image = emit(texels);
            let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
            set(&mut app, None, Some(image.clone()));
            let bare = take(&mut app, "emissive-bloom-off");
            app.world_mut().resource_mut::<PostFx>().bloom = true;
            let bloomed = take(&mut app, "emissive-bloom-on");
            let mut affected = 0;
            let mut spill = 0;
            let mut peak_gain = 0;
            let background = &bare[..3];
            for (before, after) in bare.chunks_exact(4).zip(bloomed.chunks_exact(4)) {
                let gain = after[0].saturating_sub(before[0]);
                assert_eq!(before[3], after[3], "the halo preserves the target alpha");
                affected += usize::from(gain > 2);
                if &before[..3] == background {
                    assert!(after[1].saturating_sub(before[1]) <= gain && after[2].saturating_sub(before[2]) <= gain,
                        "the soft halo from a red map stays red-dominant in the sky");
                    spill += usize::from(gain > 2);
                }
                peak_gain = peak_gain.max(gain);
            }
            eprintln!("Emissive halo: {affected} lit neighbours, {spill} background pixels, peak gain {peak_gain}/255");
            assert!(affected > 20 && spill > 20 && peak_gain < 118, "a soft halo extends past the silhouette");

            // Drive the real material from a simulation view, including the repair latch.
            use super::emissive_state::FlickerPhase;
            use crate::clock::AnimClock;
            use crate::render::appearance::ModelNode;
            use crate::sim_link::{SimEntity, SimLink};
            use sta_sim::appearance::SystemAppearance;
            use sta_sim::command::LocalRouter;
            use sta_sim::setup::{GameOptions, GameSetup};
            use sta_sim::{Fixed, PlayerId};
            use std::time::Duration;
            app.init_resource::<AnimClock>();
            let db = std::sync::Arc::new(sta_content::fake::db());
            let mut sim = sta_sim::Sim::from_setup(&GameSetup {
                seed: 1, players: Vec::new(), map: std::sync::Arc::new(sta_content::MapDef::empty()),
                content: db.clone(), options: GameOptions::default(),
            });
            let id = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), sta_sim::Vec3::ZERO,
                (Fixed::ZERO, Fixed::ONE)).unwrap();
            let link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
            let mut view = (**link.real_curr()).clone();
            app.insert_resource(link);
            let root = app.world_mut().spawn((SimEntity(id), Transform::IDENTITY, Visibility::Inherited)).id();
            let node = app.world_mut().spawn((ModelNode { root, index: 0 }, Transform::IDENTITY,
                Visibility::Inherited, ChildOf(root))).id();
            app.world_mut().entity_mut(subject).insert((ChildOf(node), HullMaps { specular: None, emissive: Some(image) }));
            let feed = |app: &mut App, view: &mut sta_sim::WorldView, enabled, hp| {
                view.appearance.systems = vec![(id, [Some(SystemAppearance {
                    enabled, health: [Fixed::from_int(hp), Fixed::from_int(40)],
                }), None, None, None, None])];
                app.world_mut().resource_mut::<SimLink>().feed(std::sync::Arc::new(view.clone()));
            };
            // Advance only between captures so the chosen phase survives GPU readback.
            let advance_phase = |app: &mut App, visible| {
                let mut clock = app.world_mut().resource_mut::<AnimClock>();
                let elapsed = Duration::from_secs_f64(clock.wall());
                let offset = (1..=64)
                    .map(|steps| Duration::from_millis(90 * steps))
                    .find(|offset| FlickerPhase::new(elapsed + *offset, id).visible(0) == visible)
                    .expect("both engine flicker phases occur");
                clock.advance_idle(offset.as_secs_f64());
            };
            feed(&mut app, &mut view, false, 40);
            advance_phase(&mut app, false);
            let dark = take(&mut app, "emissive-engines-disabled-off");
            assert!(sum(&dark, 0) + 10_000 < sum(&bloomed, 0));
            advance_phase(&mut app, true);
            assert!(take(&mut app, "emissive-engines-disabled-on") == bloomed, "disabled lights and glow return in the on phase");
            feed(&mut app, &mut view, true, 0);
            assert!(take(&mut app, "emissive-engines-destroyed") == dark, "destroyed engines remain dark in the on phase");
            feed(&mut app, &mut view, true, 10);
            advance_phase(&mut app, false);
            assert!(take(&mut app, "emissive-engines-partial-repair-off") == dark);
            advance_phase(&mut app, true);
            assert!(take(&mut app, "emissive-engines-partial-repair-on") == dark);
            feed(&mut app, &mut view, true, 40);
            assert!(take(&mut app, "emissive-engines-repaired") == bloomed, "full repair restores the original light");

            // A dark, opaque occluder exposes any leaked light from the hidden emitter.
            let occluder_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(1.4));
            let occluder_material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
                base_color: Color::srgb(0.16, 0.18, 0.2), unlit: true, ..default()
            });
            let occluder = app.world_mut().spawn((Mesh3d(occluder_mesh), MeshMaterial3d(occluder_material), Transform::from_xyz(0.0, 0.0, 1.5))).id();
            app.world_mut().resource_mut::<PostFx>().bloom = false;
            let hidden = take(&mut app, "emissive-occluded-off");
            app.world_mut().resource_mut::<PostFx>().bloom = true;
            assert!(take(&mut app, "emissive-occluded-on") == hidden, "occluded maps add no bloom");
            app.world_mut().despawn(occluder);

            // Even a white hull and a bright additive shot contribute nothing to the halo.
            app.world_mut().entity_mut(subject).insert(Visibility::Hidden);
            let white = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
                base_color: Color::WHITE, unlit: true, ..default()
            });
            let ball = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.35));
            let white = app.world_mut().spawn((Mesh3d(ball), MeshMaterial3d(white), Transform::from_xyz(-0.7, 0.0, 0.0))).id();
            let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(quad::rectangle());
            let look = Look::new(quad::K_HOT, Vec3::new(0.0, 1.0, 0.0));
            let material = app.world_mut().resource_mut::<Assets<quad::FxQuadMaterial>>()
                .add(quad::FxQuadMaterial { params: look.params(4.0, 0.0) });
            let flare = app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(material),
                Transform::from_xyz(0.7, 0.0, 0.0).with_scale(Vec3::splat(0.55)))).id();
            app.world_mut().resource_mut::<PostFx>().bloom = false;
            let bright = take(&mut app, "non-emissive-bloom-off");
            assert!(bright.chunks_exact(4).any(|p| p[0] == 255 && p[1] == 255 && p[2] == 255));
            assert!(bright.chunks_exact(4).any(|p| p[0] < 10 && p[1] == 255 && p[2] < 10));
            app.world_mut().resource_mut::<PostFx>().bloom = true;
            assert_eq!(take(&mut app, "non-emissive-bloom-on"), bright, "only mapped emissions can bloom");

            // The installed test mod adds emissive channels to the Classic FBattle model.
            if let Ok(mod_dir) = std::env::var("STA64_EMISSIVE_INSTALL") {
                use bevy::ecs::system::RunSystemOnce;
                use crate::render::model_scene::{self, SpawnCtx};
                use crate::render::textures::{Loader, TextureCache};
                let mod_dir = std::path::Path::new(&mod_dir);
                let install = mod_dir.ancestors().nth(3).expect("install/Data/Mods/mod");
                let active = mod_dir.file_name().and_then(|name| name.to_str()).expect("mod folder");
                let vfs = sta_vfs::Vfs::mount_install(install, Some(active)).unwrap();
                let (path, format) = sta_assets::find_model(&vfs, "fbattle").expect("installed FBattle model");
                let model = sta_assets::load_model(&vfs, &path, format).unwrap();
                let (lo, hi) = model.bounds().unwrap();
                let radius = Vec3::from_array(hi).distance(Vec3::from_array(lo)) * 0.5;
                let scale = 1.0 / radius;
                let center = (Vec3::from_array(lo) + Vec3::from_array(hi)) * (0.5 * scale) * Vec3::new(1.0, 1.0, -1.0);
                app.insert_resource(ClearColor(Color::srgb(0.045, 0.04, 0.19)));
                app.world_mut().entity_mut(camera).insert(
                    Transform::from_translation(center + Vec3::new(0.66, 1.4, 1.48))
                        .looking_at(center, Vec3::Y),
                );
                for entity in [white, flare] {
                    app.world_mut().entity_mut(entity).insert(Visibility::Hidden);
                }
                let stats = app.world_mut().run_system_once(
                    move |mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>,
                          mut materials: ResMut<Assets<StandardMaterial>>, mut images: ResMut<Assets<Image>>| {
                        let mut cache = TextureCache::default();
                        let sprites = sta_assets::a2fmt::spr::SpriteTable::default();
                        let root = commands.spawn((SimEntity(id), Transform::IDENTITY, Visibility::Inherited)).id();
                        model_scene::spawn(&mut SpawnCtx {
                            commands: &mut commands, meshes: &mut meshes, materials: &mut materials,
                            textures: Loader { vfs: &vfs, cache: &mut cache, images: &mut images,
                                formats: bevy::image::CompressedImageFormats::NONE },
                            team: Color::WHITE, borg: false, sprites: &sprites, damaged: false,
                            normal_maps: true, parts: None, model_key: path.clone(), unlit: false,
                            scale, diffuse_tint: false,
                        }, &model, root)
                    },
                ).unwrap();
                assert!(stats.meshes > 0, "the installed SOD should produce meshes");
                for _ in 0..20 { app.update(); }
                app.world_mut().resource_mut::<PostFx>().bloom = false;
                let bare = take(&mut app, "fbattle-bloom-off");
                app.world_mut().resource_mut::<PostFx>().bloom = true;
                let lit = take(&mut app, "fbattle-bloom-on");
                let changed_pixels = bare.chunks_exact(4).zip(lit.chunks_exact(4)).filter(|(a, b)| a != b).count();
                let outside = |before: &[u8], after: &[u8]| {
                    before.chunks_exact(4).zip(after.chunks_exact(4))
                        .filter(|(a, b)| a[..3] == before[..3] && (0..3).any(|i| b[i].saturating_sub(a[i]) > 3)).count()
                };
                let spill = outside(&bare, &lit);
                eprintln!("Installed FBattle: {} meshes, {changed_pixels} changed pixels, {spill} coloured background pixels", stats.meshes);
                assert!(spill > 20, "installed FBattle glow should extend into space");

                // Count the extra light outside the ship, where a surface-only gain
                // cannot change the picture. The installed model has both engine maps.
                let sky_gain = |frame: &[u8]| {
                    bare.chunks_exact(4).zip(frame.chunks_exact(4))
                        .filter(|(before, _)| before[..3] == bare[..3])
                        .map(|(before, after)| (0..3).map(|i| u64::from(after[i].saturating_sub(before[i]))).sum::<u64>())
                        .sum::<u64>()
                };
                let idle_gain = sky_gain(&lit);
                view.attrs.impulse = vec![(id, Fixed::from_int(100))];
                for (name, speed) in [("fbattle-impulse-halo", 100), ("fbattle-warp-halo", 150)] {
                    view.units.iter_mut().find(|unit| unit.id == id).unwrap().speed = Fixed::from_int(speed);
                    feed(&mut app, &mut view, true, 40);
                    let moving = take(&mut app, name);
                    let gain = sky_gain(&moving);
                    eprintln!("{name}: total sky RGB gain {gain} vs idle {idle_gain}");
                    assert!(gain > idle_gain + idle_gain / 20, "{name} should visibly strengthen the halo");
                }
                view.units.iter_mut().find(|unit| unit.id == id).unwrap().speed = Fixed::ZERO;
                feed(&mut app, &mut view, true, 40);
                assert_eq!(take(&mut app, "fbattle-halo-stopped"), lit, "idle halo returns when the ship stops");

                feed(&mut app, &mut view, false, 40);
                advance_phase(&mut app, false);
                let dark = take(&mut app, "fbattle-engines-disabled-off");
                assert!(dark != lit, "FBattle engine textures go dark during the off phase");
                advance_phase(&mut app, true);
                assert!(take(&mut app, "fbattle-engines-disabled-on") == lit, "FBattle engine textures light up during the on phase");
                feed(&mut app, &mut view, false, 0);
                assert!(take(&mut app, "fbattle-engines-destroyed") == dark, "destroyed FBattle engines remain dark");
                feed(&mut app, &mut view, true, 40);
                app.world_mut().entity_mut(camera).insert(Msaa::Sample4);
                app.world_mut().resource_mut::<PostFx>().bloom = false;
                let msaa_off = take(&mut app, "fbattle-msaa-bloom-off");
                app.world_mut().resource_mut::<PostFx>().bloom = true;
                assert!(outside(&msaa_off, &take(&mut app, "fbattle-msaa-bloom-on")) > 20, "multisample mask resolves the halo");
            }
        }
        let errors = ERRORS.lock().unwrap().clone();
        assert!(errors.is_empty(), "shader/pipeline ERROR events:\n{}", errors.join("\n"));
    }

    /// #1251: a lit hull's eye-dependent terms (highlight, rim) follow the camera of the view being drawn,
    /// not the map's main camera. Two cameras draw one hull into two targets; the second one is an
    /// action-view style camera (not the main camera). Its frame must equal the frame the main camera draws
    /// from the same pose, and differ from the main camera's frame from its own place.
    #[test]
    fn a_lit_hull_follows_the_camera_of_the_view_being_drawn() {
        if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
            eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU shader test");
            return;
        }
        let _serial = crate::bench::frames::gpu_serial();
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<LogPlugin>()
                .disable::<bevy::winit::WinitPlugin>(),
        );
        app.add_plugins((HullPlugin, crate::render::fx::quad::FxQuadPlugin));
        app.insert_resource(ClearColor(Color::srgb(0.01, 0.01, 0.025)));
        app.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 60.0, ..default() });
        let target = |app: &mut App| {
            let mut img = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
            img.asset_usage = RenderAssetUsages::RENDER_WORLD;
            app.world_mut().resource_mut::<Assets<Image>>().add(img)
        };
        let (main_target, view_target) = (target(&mut app), target(&mut app));
        let texture = app.world_mut().resource_mut::<Assets<Image>>().add(checker());
        let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(1.0).mesh().uv(32, 18));
        app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, ..default() }, Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y)));
        app.world_mut().spawn((DirectionalLight { illuminance: 1500.0, ..default() }, Transform::from_xyz(-4.0, -2.0, -3.0).looking_at(Vec3::ZERO, Vec3::Y)));
        let main_pose = Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y);
        let view_pose = Transform::from_xyz(3.0, 2.0, -3.0).looking_at(Vec3::ZERO, Vec3::Y);
        let main_cam = app.world_mut().spawn((Camera3d::default(), RenderTarget::from(main_target.clone()), Msaa::Off, main_pose)).id();
        let view_cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                crate::render::action_camera::ActionCamera,
                Camera { order: -9, ..default() },
                RenderTarget::from(view_target.clone()),
                Msaa::Off,
                view_pose,
            ))
            .id();
        let std = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture),
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        app.world_mut().spawn((Mesh3d(sphere), MeshMaterial3d(std), HullPart::default()));
        app.finish();
        app.cleanup();
        for _ in 0..10 {
            app.update();
        }
        let adapter = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
        });
        match adapter {
            Some(info) => {
                eprintln!("GPU adapter: {} ({:?}, {:?})", info.name, info.device_type, info.backend);
                if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
                    eprintln!("SKIP: only a software adapter is available ({})", info.name);
                    return;
                }
            }
            None => {
                eprintln!("SKIP: no render adapter");
                return;
            }
        }
        for _ in 0..4 {
            app.update();
        }
        capture(&mut app, &main_target, "eye-main-own");
        capture(&mut app, &view_target, "eye-view-own");
        // The main camera takes the other view's pose, and that view stops drawing.
        app.world_mut().entity_mut(view_cam).insert(Camera { is_active: false, order: -9, ..default() });
        app.world_mut().entity_mut(main_cam).insert(view_pose);
        for _ in 0..4 {
            app.update();
        }
        capture(&mut app, &main_target, "eye-main-at-view-pose");
        let frames = FRAMES.lock().unwrap().clone();
        let named = |n: &str| &frames.iter().rev().find(|f| f.0 == n).unwrap_or_else(|| panic!("no frame {n}")).1;
        let (own, view, moved) = (named("eye-main-own"), named("eye-view-own"), named("eye-main-at-view-pose"));
        let (diff_views, _, _, lit, _, _) = compare(own, view);
        let (diff_same_pose, _, _, _, _, _) = compare(view, moved);
        eprintln!("hull seen from two poses: mean abs {diff_views:.2}/255, lit pixels {lit}; same pose, other camera: mean abs {diff_same_pose:.4}/255");
        assert!(lit > 500, "the hull drew {lit} pixels");
        assert!(diff_views > 0.5, "the two poses show the same frame (diff {diff_views})");
        assert_eq!(view, moved, "the second view's frame is not the frame its own pose gives the main camera");
    }
}

#[cfg(test)]
#[path = "hulls/cutout_gpu.rs"]
mod cutout_gpu;

#[cfg(test)]
#[path = "hulls/appearance_tests.rs"]
mod appearance_tests;
