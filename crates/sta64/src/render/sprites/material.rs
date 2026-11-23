//! The sprite nodes' material and colour slots (ADR 0009, S1 and S2).
//!
//! - [`SpriteMaterial`]: one shared material per look (texture, blend mode, uv), Bevy's
//!   `StandardMaterial` extended with [`SpriteTint`], which binds the colour buffer as a plain,
//!   non-bindless storage buffer at binding 100. Its fragment shader is `sprite.wesl`, Bevy's forward
//!   fragment shader with one added multiply (S3); the prepass keeps Bevy's own.
//! - [`SpriteSlots`]: one `[f32; 4]` linear colour per sprite node in one `ShaderBuffer`, at the slot
//!   its `MeshTag` names. Slots come from a free list and never move, so a live node's tag never
//!   changes; the buffer grows by doubling.
//!
//! Step 1 of the ADR's implementation: the mechanism and its proofs only. No game node uses it yet;
//! `animate`, the cloak and build fade and the ghost tint are unchanged (step 2 adopts the nodes).
// Step 2 (adoption, #1139) is the API's first user outside the tests; remove this then.
#![allow(dead_code)]

use bevy::asset::{embedded_asset, AssetPath, RenderAssetUsages};
use bevy::mesh::MeshTag;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, BufferUsages};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

/// A look's shared material: per-look state in the base, the node's colour from its slot.
pub type SpriteMaterial = ExtendedMaterial<StandardMaterial, SpriteTint>;

/// Where the material asks for its fragment shader (registered by [`SpriteMaterialPlugin`], so a mounted
/// file can shadow it by path, ADR 0004 D12).
pub const SHADER: &str = "embedded://sta64/render/sprites/sprite.wesl";

/// The extension: the colour buffer every look's material binds. Not bindless (S2), so the combined
/// material is not either: one bind group per look and a plain storage binding in the shader.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SpriteTint {
    /// [`SpriteSlots::buffer`]; `sprite_colours` in `sprite.wesl`.
    #[storage(100, read_only)]
    pub colours: Handle<ShaderBuffer>,
}

impl MaterialExtension for SpriteTint {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse(SHADER).into_owned())
    }
}

/// A look's shared material from the material a node would have on its own: the same texture,
/// blend mode and uv, with the material colour exactly 1 (linear white, whose conversion is the
/// identity), so the shader's multiply by the slot gives the per-node material's product (S3).
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

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Slots held by live nodes.
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
    pub fn read(&self, buffers: &Assets<ShaderBuffer>, tag: &MeshTag) -> Option<[f32; 4]> {
        buffers.get(&self.buffer)?.cast_slice::<[f32; 4]>()?.get(tag.value as usize).copied()
    }
}

/// Bytes per slot: one `vec4<f32>`.
const SLOT_BYTES: u64 = 16;

/// The shader, the material and the slots. Added by the map and viewer plugins (never in one app).
pub struct SpriteMaterialPlugin;

impl Plugin for SpriteMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sprite.wesl");
        app.add_plugins(MaterialPlugin::<SpriteMaterial>::default());
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
        // What step 2's adoption does on removal: the node's slot goes back.
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
