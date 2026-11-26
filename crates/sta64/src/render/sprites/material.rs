//! The sprite nodes' material and colour slots (ADR 0009, S1, S2, S4 and S5).
//!
//! - [`SpriteMaterial`]: one shared material per look (texture, blend mode, uv), Bevy's
//!   `StandardMaterial` extended with [`SpriteTint`], which binds the colour buffer in Bevy's bindless
//!   mode where the device supports it and as a plain storage buffer where not (S2 as amended
//!   2026-11-23). Its fragment shader is `sprite.wesl`, Bevy's forward fragment shader with one added
//!   multiply (S3); the prepass keeps Bevy's own.
//! - [`SpriteSlots`]: one `[f32; 4]` linear colour per sprite node in one `ShaderBuffer`, at the slot
//!   its `MeshTag` names. Slots come from a free list and never move, so a live node's tag never
//!   changes; the buffer grows by doubling.
//! - Adoption: an observer on adding [`SpriteFx`] gives the node a slot, its tag, its look's shared
//!   material and a [`SpriteState`], in the same command flush that spawns it, so no frame draws an
//!   unadopted node. The slot goes back when the [`SpriteState`] (our own component, not the generic
//!   `MeshTag`) is removed, which a despawn does. An app without rendering has no slots and adopts
//!   nothing: its nodes keep their own materials (`main`'s per-node path).
//! - [`SpriteDemotion`]: the cloak and build fade and the placement ghost give a node a private
//!   `StandardMaterial` built from its look and its [`SpriteState`] (S4); the fade gives it back.
//!
//! Rules every writer of a slot keeps (each one guards against drawing a stale colour):
//! - a slot is written in the same frame it is handed out: a reused slot still holds the colour of
//!   the node released from it, a new one is zero, and `animate` writes only on change;
//! - the slot always holds [`linear`] of the node's [`SpriteState::colour`]: `animate` writes both
//!   together, also while the node is demoted, and a node coming back to its look has its slot
//!   written again;
//! - a cut-out look's slot holds alpha exactly 1: the prepass discards on the look's own alpha,
//!   which knows nothing of the slot, so a cut-out node that fades (alpha below 1) is demoted to a
//!   private material instead of fading through its slot. Sprite colours carry no alpha of their
//!   own (`Color::srgb`), so only the fade lowers it;
//! - sprite nodes are spawned, and slots written, before `PostUpdate`'s asset events
//!   (`AssetEventSystems`): only then is the write extracted in the same frame. Every spawner of
//!   sprite nodes runs in `Update` today (unit, construction and map models, the viewer) and
//!   `animate`, `apply_fade` and `tint_ghost` run there too. A node spawned later in the frame would
//!   be drawn once with whatever its slot held before (a released node's colour, or zero), where on
//!   `main` it is not drawn at all until its new material is prepared. There is no cheap assertion
//!   for this (an observer cannot tell which schedule applied its commands), so a new spawner keeps
//!   to `Update`.
//!
//! [`SpriteFx`]: super::SpriteFx

use super::SpriteFx;
use bevy::asset::{embedded_asset, AssetPath, RenderAssetUsages};
use bevy::ecs::system::SystemParam;
use bevy::math::Affine2;
use bevy::mesh::MeshTag;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, BufferUsages};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use std::collections::HashMap;

/// A look's shared material: per-look state in the base, the node's colour from its slot.
pub type SpriteMaterial = ExtendedMaterial<StandardMaterial, SpriteTint>;

/// Where the material asks for its fragment shader (registered by [`SpriteMaterialPlugin`], so a mounted
/// file can shadow it by path, ADR 0004 D12).
pub const SHADER: &str = "embedded://sta64/render/sprites/sprite.wesl";

/// The extension: the colour buffer every look's material binds.
///
/// Bindless where the device supports it (S2 as amended): the extension's own index table at binding
/// 100 holds one entry (bindless index 50) and the buffer is a storage binding array at binding 101,
/// so every look sits in one material slab with one bind group, as main's per-node standard
/// materials do. Bevy's bindless mode for a buffer binding array needs buffer binding arrays as well
/// as texture binding arrays; without them Bevy falls back to the non-bindless form, the buffer bound
/// plainly at binding 50 ([`bindless`] says which form a device gets).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct SpriteTint {
    /// [`SpriteSlots::buffer`]; `sprite_colours` in `sprite.wesl`.
    #[storage(50, read_only, binding_array(101))]
    pub colours: Handle<ShaderBuffer>,
}

impl MaterialExtension for SpriteTint {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse(SHADER).into_owned())
    }
}

/// Whether the sprite material runs in its bindless form on this device (`false`: the non-bindless
/// fallback), by Bevy's own rule for a material.
#[cfg(test)]
pub fn bindless(device: &bevy::render::renderer::RenderDevice) -> bool {
    bevy::render::material_bind_groups::material_uses_bindless_resources::<SpriteMaterial>(device)
}

/// The form [`bindless`] gives, as a GPU test's output names it.
#[cfg(test)]
pub fn mode_name(device: &bevy::render::renderer::RenderDevice) -> &'static str {
    if bindless(device) { "bindless" } else { "fallback (non-bindless)" }
}

/// A look's shared material from the material a node would have on its own: the same texture,
/// blend mode and uv, with the material colour exactly 1 (linear white, whose conversion is the
/// identity), so the shader's multiply by the slot gives the per-node material's product (S3).
///
/// Byte equality with the per-node material (S6) rests on these, which every sprite node meets
/// (`model_scene::spawn_sprite_node`) and any new kind of sprite node must keep:
/// - the mesh has no vertex colours (Bevy would multiply them in before the slot's colour, which is
///   another rounding order than the per-node path's);
/// - the look is unlit (lighting would act on the product in another rounding order);
/// - the alpha mode is not `AlphaToCoverage` (coverage would come from another alpha path);
/// - a cut-out (`Mask`) look's slots hold alpha exactly 1 (the prepass discards on the look's own
///   alpha; see the module's slot rules).
pub fn look_material(node: &StandardMaterial, slots: &SpriteSlots) -> SpriteMaterial {
    ExtendedMaterial {
        base: StandardMaterial { base_color: Color::LinearRgba(LinearRgba::WHITE), ..node.clone() },
        extension: SpriteTint { colours: slots.buffer() },
    }
}

/// The colour a slot holds for a node whose own material would have `base_color` `c`: the conversion
/// Bevy applies to `StandardMaterial::base_color` before upload (`LinearRgba::from(..).to_vec4()`), done
/// on the CPU so the value is bit-equal to the per-node material's.
pub fn linear(c: Color) -> [f32; 4] {
    LinearRgba::from(c).to_vec4().to_array()
}

/// One colour slot per sprite node (S1).
#[derive(Resource, Debug)]
pub struct SpriteSlots {
    buffer: Handle<ShaderBuffer>,
    /// Slots the buffer holds.
    capacity: u32,
    /// Slots handed out at least once; every slot below this is live or in `free`.
    used: u32,
    /// Released slots, reused before a new one is taken.
    free: Vec<u32>,
}

impl SpriteSlots {
    /// The first buffer's slots (16 bytes each); it doubles when full.
    pub const INITIAL_CAPACITY: u32 = 256;

    pub fn new(buffers: &mut Assets<ShaderBuffer>) -> Self {
        Self::with_capacity(buffers, Self::INITIAL_CAPACITY)
    }

    /// A buffer of `capacity` slots (at least one), all zero.
    pub fn with_capacity(buffers: &mut Assets<ShaderBuffer>, capacity: u32) -> Self {
        let capacity = capacity.max(1);
        // Kept in the main world too: growth re-uploads the whole buffer from it, and writes go
        // into it in `Update`, so they are extracted in the same frame.
        let mut buffer = ShaderBuffer::new(vec![[0.0f32; 4]; capacity as usize], RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
        buffer.label = "sprite colours".into();
        buffer.buffer_usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
        Self { buffer: buffers.add(buffer), capacity, used: 0, free: Vec::new() }
    }

    /// The buffer every look's [`SpriteTint`] binds. Growth keeps the handle.
    pub fn buffer(&self) -> Handle<ShaderBuffer> {
        self.buffer.clone()
    }

    #[cfg(test)]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Slots held by live nodes.
    #[cfg(test)]
    pub fn live(&self) -> u32 {
        self.used - self.free.len() as u32
    }

    /// A slot for a new node, as the tag to put on it: a released slot if there is one, else the next
    /// unused one, doubling the buffer when it is full. No live slot moves.
    pub fn allocate(&mut self, buffers: &mut Assets<ShaderBuffer>) -> MeshTag {
        if let Some(slot) = self.free.pop() {
            return MeshTag::new(slot);
        }
        if self.used == self.capacity {
            self.capacity *= 2;
            if let Some(mut b) = buffers.get_mut(&self.buffer) {
                // Zero-extends the main-world data; the render world makes a new buffer of the new
                // size from it and rebuilds the bind groups that held the old one.
                b.resize(u64::from(self.capacity) * SLOT_BYTES);
            }
        }
        let slot = self.used;
        self.used += 1;
        MeshTag::new(slot)
    }

    /// Give a removed node's slot back. Its contents stay until the slot is handed out and written again.
    pub fn release(&mut self, tag: &MeshTag) {
        debug_assert!(tag.value < self.used && !self.free.contains(&tag.value), "slot {} released twice or never handed out", tag.value);
        self.free.push(tag.value);
    }

    /// Write slots' colours, `(tag, linear colour)`, touching the buffer asset once.
    pub fn write<'a>(&self, buffers: &mut Assets<ShaderBuffer>, writes: impl IntoIterator<Item = (&'a MeshTag, [f32; 4])>) {
        let mut writes = writes.into_iter().peekable();
        if writes.peek().is_none() {
            return;
        }
        let Some(mut b) = buffers.get_mut(&self.buffer) else { return };
        let Some(slots) = b.cast_slice_mut::<[f32; 4]>() else { return };
        for (tag, colour) in writes {
            slots[tag.value as usize] = colour;
        }
    }

    /// The colour a slot holds.
    #[cfg(test)]
    pub fn read(&self, buffers: &Assets<ShaderBuffer>, tag: &MeshTag) -> Option<[f32; 4]> {
        buffers.get(&self.buffer)?.cast_slice::<[f32; 4]>()?.get(tag.value as usize).copied()
    }
}

/// Bytes per slot: one `vec4<f32>`.
const SLOT_BYTES: u64 = 16;

/// What makes nodes share a look's material (S2): texture, blend mode (with a cut-out's threshold)
/// and sprite name. Everything else a sprite node's own material holds is the same for every node of
/// a sprite (unlit, two-sided, no culling, the sprite's uv at a given time); the colour is the
/// slot's. A node with a uv of its own (a flipbook offset) could not share a look; sprite nodes have
/// none.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LookKey {
    texture: Option<AssetId<Image>>,
    alpha: (std::mem::Discriminant<AlphaMode>, u32),
    sprite: String,
}

impl LookKey {
    pub fn of(node: &StandardMaterial, sprite: &str) -> Self {
        let threshold = match node.alpha_mode {
            AlphaMode::Mask(t) => t.to_bits(),
            _ => 0,
        };
        Self { texture: node.base_color_texture.as_ref().map(Handle::id), alpha: (std::mem::discriminant(&node.alpha_mode), threshold), sprite: sprite.to_owned() }
    }
}

/// The looks' shared materials by key, each with the number of adopted nodes of that key. The table
/// holds a strong handle per look and hands out clones of it; the entry goes when the key's last node
/// is released, so a look (and its texture) lives exactly as long as a node uses it.
#[derive(Resource, Debug)]
pub struct SpriteLooks {
    by_key: HashMap<LookKey, (Handle<SpriteMaterial>, u32)>,
    /// Whether new sprite nodes are adopted. Test-only switch: the game never sets it to `false`.
    /// The GPU test turns it off for its reference path (`main`'s per-node materials), so both of its
    /// apps hold the same observers and resources and so the same entity ids; the headless
    /// install-content bench turns it off under `STA64_SPRITE_REFERENCE=1` for the frame-identity
    /// test on real content (S6, `bench::pair_view`).
    #[doc(hidden)]
    pub(crate) adopt: bool,
}

impl Default for SpriteLooks {
    fn default() -> Self {
        Self { by_key: HashMap::new(), adopt: true }
    }
}

/// An adopted sprite node's state: what `main`'s per-node material would hold now (the colour and uv
/// `animate` last wrote while the node was shown), its slot and its look.
#[derive(Component, Debug)]
pub struct SpriteState {
    /// The node's colour as its own material's `base_color` would be; its slot holds [`linear`] of it.
    pub colour: Color,
    pub uv: Affine2,
    /// The node's slot, as on its `MeshTag`. Unchanged across demotion and restore.
    pub tag: MeshTag,
    /// The look's shared material. The node draws through it unless it is demoted, or it was spawned
    /// showing another uv than its look's (then through a private copy until `animate` changes its
    /// state, the frame its own material would next change on `main`).
    pub look: Handle<SpriteMaterial>,
    /// The look's key, to count the node off its look when it is released.
    key: LookKey,
}

/// Parity mode (S5): when a node's state changes under `main`'s rule, its material component is
/// marked changed, so the nodes re-queued are `main`'s. Always on in the game; tests can turn it off
/// with `STA64_SPRITE_NO_PARITY=1` to see what dropping it would change (ADR 0009, open question 1).
pub(crate) fn parity() -> bool {
    #[cfg(test)]
    {
        std::env::var("STA64_SPRITE_NO_PARITY").as_deref() != Ok("1")
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// Nodes whose parity mark (S5) waits for visibility: `animate` lists a node in `Update` in each frame
/// its state changed under `main`'s rule, and [`mark_seen_changes`] in `PostUpdate` marks the ones some
/// view sees and empties the list.
#[derive(Resource, Default, Debug)]
pub(crate) struct ParityMarks(Vec<Entity>);

impl ParityMarks {
    pub(crate) fn push(&mut self, node: Entity) {
        self.0.push(node);
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Parity mode (S5, as amended: the mark is for nodes a view sees). Marks the material component of
/// each listed node that is visible in at least one view this frame, as `main` marks every node whose
/// state changed. A node that no view sees is in no phase, so whether it is marked changes no view's
/// phase; a node that comes into view is queued as newly visible (Bevy's `iter_to_queue`, bevy_render
/// 0.20.1 `camera.rs`), first and once, with or without the mark.
///
/// Order (bevy_camera, bevy_light, bevy_mesh and bevy_pbr 0.20.1): after
/// `MarkNewlyHiddenEntitiesInvisible`, which follows every writer of [`ViewVisibility`] (the cameras'
/// `CheckVisibility` and the lights' shadow-view checks), so a node seen by any view has its final
/// flag; and before `mark_3d_meshes_as_changed_if_their_assets_changed`, which both
/// `check_entities_needing_specialization` and Bevy's private system that turns a changed material
/// into a changed `Mesh3d` run after. The mark then reaches specialisation, queueing and mesh
/// re-extraction in the frame `animate` wrote the state, as it does on `main`, not a frame later.
pub(crate) fn mark_seen_changes(
    mut marks: ResMut<ParityMarks>,
    seen: Query<&ViewVisibility>,
    mut looks: Query<&mut MeshMaterial3d<SpriteMaterial>>,
) {
    for node in marks.0.drain(..) {
        if seen.get(node).is_ok_and(|v| v.get())
            && let Ok(mut m) = looks.get_mut(node)
        {
            m.set_changed();
        }
    }
}

/// The adoption and release observers, the looks' table and the parity mark (in
/// [`SpriteMaterialPlugin`]; tests without rendering add them on their own).
pub(crate) fn adoption(app: &mut App) {
    use bevy::camera::visibility::VisibilitySystems;
    app.init_resource::<SpriteLooks>()
        .init_resource::<ParityMarks>()
        .add_observer(adopt)
        .add_observer(release)
        .add_systems(
            PostUpdate,
            mark_seen_changes
                .after(VisibilitySystems::MarkNewlyHiddenEntitiesInvisible)
                .before(bevy::mesh::mark_3d_meshes_as_changed_if_their_assets_changed),
        );
}

/// Put a new sprite node on the material path: a slot written with the colour its own material was
/// spawned with, its look's shared material in place of its own, its [`SpriteState`]. Runs in the
/// flush that inserts the [`SpriteFx`], so the node is never drawn unadopted.
#[allow(clippy::too_many_arguments)]
fn adopt(
    add: On<Add<SpriteFx>>,
    mut commands: Commands,
    mut nodes: Query<&mut SpriteFx>,
    looks: Option<ResMut<SpriteLooks>>,
    slots: Option<ResMut<SpriteSlots>>,
    buffers: Option<ResMut<Assets<ShaderBuffer>>>,
    own: Option<Res<Assets<StandardMaterial>>>,
    materials: Option<ResMut<Assets<SpriteMaterial>>>,
) {
    let (Some(mut looks), Some(mut slots), Some(mut buffers), Some(own), Some(mut materials)) = (looks, slots, buffers, own, materials) else { return };
    if !looks.adopt {
        return;
    }
    let Ok(mut fx) = nodes.get_mut(add.entity) else { return };
    let Some(node) = own.get(&fx.material).cloned() else { return };
    let tag = slots.allocate(&mut buffers);
    slots.write(&mut buffers, [(&tag, linear(node.base_color))]);
    let key = LookKey::of(&node, &fx.sprite);
    let entry = looks.by_key.entry(key.clone()).or_insert_with(|| (materials.add(look_material(&node, &slots)), 0));
    entry.1 += 1;
    let look = entry.0.clone();
    // A node spawned after this frame's `animate` is drawn this frame with the uv it was spawned
    // with, as its own material would be. If its look has moved on (a flipbook), it draws through a
    // private copy of the look until `animate` shows it at the look's uv.
    let drawn = if materials.get(&look).is_some_and(|l| l.base.uv_transform == node.uv_transform) { look.clone() } else { materials.add(look_material(&node, &slots)) };
    // The node's own material is no longer drawn or written; dropping the handle frees it.
    fx.material = Handle::default();
    let state = SpriteState { colour: node.base_color, uv: node.uv_transform, tag: tag.clone(), look, key };
    commands.entity(add.entity).remove::<MeshMaterial3d<StandardMaterial>>().insert((tag, MeshMaterial3d(drawn), state));
}

/// Give a removed node's slot back (a despawn removes its [`SpriteState`]), and its look's entry when
/// it was the look's last node.
fn release(remove: On<Remove<SpriteState>>, nodes: Query<&SpriteState>, slots: Option<ResMut<SpriteSlots>>, looks: Option<ResMut<SpriteLooks>>) {
    let Ok(state) = nodes.get(remove.entity) else { return };
    if let Some(mut slots) = slots {
        slots.release(&state.tag);
    }
    if let Some(mut looks) = looks
        && let Some((_, count)) = looks.by_key.get_mut(&state.key)
    {
        *count -= 1;
        if *count == 0 {
            looks.by_key.remove(&state.key);
        }
    }
}

/// The private `StandardMaterial` an adopted node's own material would be now on `main`: its look's
/// material with the node's last shown colour and uv (S4).
pub fn demoted(state: &SpriteState, looks: &Assets<SpriteMaterial>) -> Option<StandardMaterial> {
    let look = looks.get(&state.look)?;
    Some(StandardMaterial { base_color: state.colour, uv_transform: state.uv, ..look.base.clone() })
}

/// Moving sprite nodes between their look and a private `StandardMaterial` (S4), for the systems that
/// edit a node's material as a whole: the cloak and build fade and the placement ghost.
#[derive(SystemParam)]
pub struct SpriteDemotion<'w, 's> {
    on_look: Query<'w, 's, &'static SpriteState, With<MeshMaterial3d<SpriteMaterial>>>,
    states: Query<'w, 's, &'static SpriteState>,
    looks: Option<Res<'w, Assets<SpriteMaterial>>>,
    slots: Option<Res<'w, SpriteSlots>>,
    buffers: Option<ResMut<'w, Assets<ShaderBuffer>>>,
}

impl SpriteDemotion<'_, '_> {
    /// Demote `node` if it is a sprite node drawn through its look: a private material equal to what
    /// its own material would be now on `main` replaces the look, and the caller edits it as it edits
    /// any other material. `None` for any other entity, demoted sprite nodes included.
    pub fn demote(&self, commands: &mut Commands, materials: &mut Assets<StandardMaterial>, node: Entity) -> Option<Handle<StandardMaterial>> {
        let state = self.on_look.get(node).ok()?;
        let own = materials.add(demoted(state, self.looks.as_deref()?)?);
        commands.entity(node).try_remove::<MeshMaterial3d<SpriteMaterial>>().try_insert(MeshMaterial3d(own.clone()));
        Some(own)
    }

    /// Put a demoted sprite node back on its look, its slot written again (its colour may have changed
    /// meanwhile). `false` if `node` is not an adopted sprite node.
    pub fn restore(&mut self, commands: &mut Commands, node: Entity) -> bool {
        let Ok(state) = self.states.get(node) else { return false };
        if let (Some(slots), Some(buffers)) = (self.slots.as_deref(), self.buffers.as_deref_mut()) {
            slots.write(buffers, [(&state.tag, linear(state.colour))]);
        }
        commands.entity(node).try_remove::<MeshMaterial3d<StandardMaterial>>().try_insert(MeshMaterial3d(state.look.clone()));
        true
    }
}

/// The shader, the material, the slots and adoption. Added by the map and viewer plugins (never in
/// one app).
pub struct SpriteMaterialPlugin;

impl Plugin for SpriteMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sprite.wesl");
        app.add_plugins(MaterialPlugin::<SpriteMaterial>::default());
        adoption(app);
    }

    fn finish(&self, app: &mut App) {
        // The buffer is an asset of the render plugin's storage; an app without rendering has no slots.
        if app.world().contains_resource::<Assets<ShaderBuffer>>() && !app.world().contains_resource::<SpriteSlots>() {
            let slots = SpriteSlots::new(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>());
            app.insert_resource(slots);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(capacity: u32) -> (Assets<ShaderBuffer>, SpriteSlots) {
        let mut buffers = Assets::<ShaderBuffer>::default();
        let s = SpriteSlots::with_capacity(&mut buffers, capacity);
        (buffers, s)
    }

    #[test]
    fn tags_are_unique_and_dense() {
        let (mut buffers, mut s) = slots(4);
        let tags: Vec<u32> = (0..37).map(|_| s.allocate(&mut buffers).value).collect();
        let mut sorted = tags.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), tags.len(), "a tag was handed out twice: {tags:?}");
        assert_eq!(tags, (0..37).collect::<Vec<_>>());
        assert_eq!(s.live(), 37);
    }

    #[test]
    fn a_slot_is_reused_after_its_node_is_despawned() {
        let mut app = App::new();
        app.init_resource::<Assets<ShaderBuffer>>();
        let s = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), 8);
        app.insert_resource(s);
        // The node's slot goes back on removal (the game's release observer keys on `SpriteState`).
        app.add_observer(|remove: On<Remove<MeshTag>>, tags: Query<&MeshTag>, mut slots: ResMut<SpriteSlots>| {
            slots.release(tags.get(remove.entity).unwrap());
        });
        let spawn = |app: &mut App| {
            let tag = app.world_mut().resource_scope(|w, mut s: Mut<SpriteSlots>| s.allocate(&mut w.resource_mut::<Assets<ShaderBuffer>>()));
            app.world_mut().spawn(tag).id()
        };
        let nodes: Vec<Entity> = (0..3).map(|_| spawn(&mut app)).collect();
        let tag = |app: &App, e: Entity| app.world().get::<MeshTag>(e).unwrap().value;
        assert_eq!(nodes.iter().map(|&e| tag(&app, e)).collect::<Vec<_>>(), [0, 1, 2]);
        app.world_mut().despawn(nodes[1]);
        assert_eq!(app.world().resource::<SpriteSlots>().live(), 2);
        let late = spawn(&mut app);
        assert_eq!(tag(&app, late), 1, "the despawned node's slot is handed out again");
        assert_eq!((tag(&app, nodes[0]), tag(&app, nodes[2])), (0, 2), "live tags are unchanged");
        // With no slot free, the next one is new.
        let next = spawn(&mut app);
        assert_eq!(tag(&app, next), 3);
    }

    #[test]
    fn the_buffer_doubles_and_keeps_live_tags_and_colours() {
        let (mut buffers, mut s) = slots(4);
        let handle = s.buffer();
        let tags: Vec<MeshTag> = (0..4).map(|_| s.allocate(&mut buffers)).collect();
        let colour = |i: u32| [i as f32 * 0.25, 1.0 - i as f32 * 0.125, 0.5, 1.0];
        s.write(&mut buffers, tags.iter().map(|t| (t, colour(t.value))));
        // A freed and re-taken slot does not grow the buffer.
        s.release(&tags[2]);
        let again = s.allocate(&mut buffers);
        assert_eq!((again.value, s.capacity()), (2, 4));
        // The fifth live node doubles it.
        let fifth = s.allocate(&mut buffers);
        assert_eq!((fifth.value, s.capacity()), (4, 8));
        assert_eq!(buffers.get(&handle).unwrap().buffer_size(), 8 * SLOT_BYTES);
        assert_eq!(s.buffer(), handle, "growth keeps the handle the materials bind");
        for t in &tags {
            assert_eq!(s.read(&buffers, t).map(|c| c.map(f32::to_bits)), Some(colour(t.value).map(f32::to_bits)), "slot {} after growth", t.value);
        }
        assert_eq!(s.read(&buffers, &fifth), Some([0.0; 4]), "a new slot is zero until written");
        let more: Vec<u32> = (0..4).map(|_| s.allocate(&mut buffers).value).collect();
        assert_eq!((more, s.capacity()), (vec![5, 6, 7, 8], 16));
    }

    #[test]
    fn a_look_material_has_colour_exactly_one_and_keeps_the_rest() {
        let (_buffers, s) = slots(1);
        let node = StandardMaterial {
            base_color: Color::srgb(0.3, 0.6, 0.9),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            uv_transform: bevy::math::Affine2::from_translation(Vec2::new(0.25, 0.5)),
            ..default()
        };
        let m = look_material(&node, &s);
        assert_eq!(linear(m.base.base_color).map(f32::to_bits), [1.0f32.to_bits(); 4]);
        assert_eq!((m.base.alpha_mode, m.base.unlit, m.base.double_sided, m.base.cull_mode), (AlphaMode::Add, true, true, None));
        assert_eq!(m.base.uv_transform, node.uv_transform);
        assert_eq!(m.extension.colours, s.buffer());
    }

    #[test]
    fn the_slot_colour_is_bevys_conversion_of_the_material_colour() {
        for c in [Color::srgb(0.2, 0.4, 0.8), Color::srgb(0.0, 1.0, 0.035), Color::srgb(0.5 * 0.3, 0.25 * 0.7, 0.999)] {
            let v = LinearRgba::from(c).to_vec4();
            assert_eq!(linear(c).map(f32::to_bits), v.to_array().map(f32::to_bits));
        }
    }

    #[test]
    fn the_registered_shader_path_is_the_one_the_material_asks_for() {
        let registered = bevy::asset::embedded_path!("src", "sprite.wesl");
        assert_eq!(format!("embedded://{}", registered.to_string_lossy().replace('\\', "/")), SHADER);
    }
}
