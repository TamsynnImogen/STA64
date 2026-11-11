//! Native PBR annotations. Legacy SOD/M3D models leave this table absent.
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct PbrModel {
    pub images: Vec<PbrImage>,
    /// Unique shared Group.material key -> native material.
    pub materials: BTreeMap<String, PbrMaterial>,
    /// Shared model node index -> vertex attributes for its mesh.
    pub attributes: BTreeMap<usize, VertexAttributes>,
}

#[derive(Debug, Clone)]
pub struct PbrImage {
    pub name: Option<String>,
    /// Virtual path or buffer-view/data-URI description, never encoded URI data.
    pub origin: String,
    pub format: PbrImageFormat,
    pub data: Vec<u8>,
    pub content_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PbrImageFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, Default)]
pub struct VertexAttributes {
    pub normals: Option<Vec<[f32; 3]>>,
    /// LH tangent XYZ and handedness; renderer mirrors XYZ/Z and flips W back.
    pub tangents: Option<Vec<[f32; 4]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub uv1: Option<Vec<[f32; 2]>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PbrAlpha {
    Opaque,
    Mask(f32),
    Blend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PbrWrap {
    Clamp,
    Mirror,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PbrFilter {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PbrMinFilter {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PbrSampler {
    pub wrap_s: PbrWrap,
    pub wrap_t: PbrWrap,
    pub mag: Option<PbrFilter>,
    pub min: Option<PbrMinFilter>,
}

impl Default for PbrSampler {
    fn default() -> Self {
        Self {
            wrap_s: PbrWrap::Repeat,
            wrap_t: PbrWrap::Repeat,
            mag: None,
            min: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PbrTexture {
    pub image: usize,
    /// glTF UV sets 0 and 1, validated by the loader.
    pub tex_coord: u32,
    pub sampler: PbrSampler,
}

#[derive(Debug, Clone)]
pub struct PbrMaterial {
    pub name: Option<String>,
    /// Linear factors as authored by glTF, not sRGB byte colors.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub alpha: PbrAlpha,
    pub double_sided: bool,
    pub unlit: bool,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub base_color_texture: Option<PbrTexture>,
    pub normal_texture: Option<PbrTexture>,
    pub metallic_roughness_texture: Option<PbrTexture>,
    pub emissive_texture: Option<PbrTexture>,
    pub occlusion_texture: Option<PbrTexture>,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            name: None,
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            alpha: PbrAlpha::Opaque,
            double_sided: false,
            unlit: false,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            emissive_texture: None,
            occlusion_texture: None,
        }
    }
}
