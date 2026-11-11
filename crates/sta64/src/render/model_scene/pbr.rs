//! Native material mapping, separate from the unchanged legacy hull policies.
use super::*;
use crate::render::textures::TextureUse;
use bevy::mesh::UvChannel;
use sta_assets::model::pbr::{PbrAlpha, PbrMaterial, PbrModel, PbrTexture};

pub(super) fn material(
    loader: &mut Loader,
    model: &PbrModel,
    source: &PbrMaterial,
    normal_maps: bool,
    unlit: bool,
    stats: &mut SpawnStats,
) -> StandardMaterial {
    let mut texture = |reference: Option<PbrTexture>, usage| {
        reference.and_then(|reference| {
            let handle = loader.pbr(model, reference, usage);
            if handle.is_none() {
                stats.missing_textures.push(format!("native image {}", reference.image));
            }
            handle
        })
    };
    let uv = |reference: Option<PbrTexture>| match reference.map(|r| r.tex_coord) {
        Some(1) => UvChannel::Uv1,
        _ => UvChannel::Uv0,
    };
    StandardMaterial {
        base_color: Color::linear_rgba(
            source.base_color[0],
            source.base_color[1],
            source.base_color[2],
            source.base_color[3],
        ),
        base_color_texture: texture(source.base_color_texture, TextureUse::Color),
        base_color_channel: uv(source.base_color_texture),
        perceptual_roughness: source.roughness,
        metallic: source.metallic,
        metallic_roughness_texture: texture(source.metallic_roughness_texture, TextureUse::Data),
        metallic_roughness_channel: uv(source.metallic_roughness_texture),
        normal_map_texture: if normal_maps {
            texture(source.normal_texture, TextureUse::Normal(source.normal_scale))
        } else {
            None
        },
        normal_map_channel: uv(source.normal_texture),
        emissive: LinearRgba::rgb(source.emissive[0], source.emissive[1], source.emissive[2]),
        emissive_texture: texture(source.emissive_texture, TextureUse::Color),
        emissive_channel: uv(source.emissive_texture),
        occlusion_texture: texture(
            source.occlusion_texture,
            TextureUse::Occlusion(source.occlusion_strength),
        ),
        occlusion_channel: uv(source.occlusion_texture),
        alpha_mode: match source.alpha {
            PbrAlpha::Opaque => AlphaMode::Opaque,
            PbrAlpha::Mask(cut) => AlphaMode::Mask(cut),
            PbrAlpha::Blend => AlphaMode::Blend,
        },
        double_sided: source.double_sided,
        cull_mode: if source.double_sided { None } else { Some(Face::Back) },
        unlit: source.unlit || unlit,
        ..default()
    }
}
