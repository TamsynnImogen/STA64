//! Everything drawn: models, sprites, particles, planets, nebulae, the sky and
//! the ships on the map (docs/WORKPLAN.md, RND lane).

pub mod fx;
pub mod hulls;
pub mod materials;
pub mod model_scene;
pub mod nebula;
pub mod overlays;
pub mod particles;
pub mod planet;
pub mod post;
pub mod rock;
pub mod space;
pub mod sprites;
pub mod strategic;
pub mod surfaces;
pub mod textures;
pub mod units;

#[cfg(test)]
mod clock_guard;
#[cfg(test)]
mod gpu_shaders;
