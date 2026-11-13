//! Everything drawn: models, sprites, particles, planets, nebulae, the sky and
//! the ships on the map (docs/WORKPLAN.md, RND lane).

pub mod anim;
pub mod construction;
pub mod cull;
pub mod fog;
pub mod fx;
pub mod hulls;
pub mod lod_switch;
pub mod materials;
pub mod mining;
pub mod model_scene;
pub mod moons;
pub mod nebula;
pub mod nebula_lowres;
pub mod overlays;
pub mod particles;
pub mod planet;
pub mod post;
pub mod profile;
pub mod quality;
pub mod rock;
pub mod shader_shadow;
pub mod shadows;
pub mod space;
pub mod stock;
pub mod sprites;
pub mod strategic;
pub mod surfaces;
pub mod textures;
pub mod units;

#[cfg(test)]
mod clock_guard;
#[cfg(test)]
mod gpu_shaders;
#[cfg(test)]
mod shadows_gpu;
