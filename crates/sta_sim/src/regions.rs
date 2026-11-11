//! What the map's regions do to units flying through them.
//!
//! # Asteroid fields
//!
//! The games' data (Fleet Operations 4 / Roots, stock Armada II) has no key
//! that slows, damages or hides ships in an asteroid field. A field is an
//! `asteroidfield`-class object (`fieldRadius`, `minAsteroids`..`maxAsteroids`,
//! `ScaleSOD`) whose rocks are physical objects, and Fleet Operations builds
//! its walls out of them: `trigger_blocking` (`fieldRadius = 400`) and
//! `trigger_blocking_sml` (100) are hidden one-rock fields whose rock model
//! is a sphere of exactly that radius. So a field is an obstacle: units do
//! not enter it. [`MapDef`](sta_content::MapDef) conversion already turns each
//! such object into a [`RegionKind::AsteroidField`] sphere of `fieldRadius`;
//! here the sphere (or box) becomes a [`Solid`] units are pushed out of.
//!
//! This is push-out only; steering round a field is [`avoid`](crate::avoid)'s
//! (SIM-21), which also gives units their radius (zero: a point).
//!
//! Nebulae are a different effect (speed coefficient, disabled systems) and
//! not handled here. A map without asteroid fields takes the early return and
//! its digests do not change.

use sta_content::{Region, RegionKind, RegionShape};

use crate::Sim;
use crate::spatial::{Solid, push_from_solid};
use sta_core::Fixed;

/// The obstacles among `regions`, in map order.
pub fn solids(regions: &[Region]) -> Vec<Solid> {
    regions
        .iter()
        .filter(|r| r.kind == RegionKind::AsteroidField)
        .filter_map(|r| match r.shape {
            RegionShape::Sphere { centre, radius } => Some(Solid::Sphere { centre, radius }),
            RegionShape::Box { min, max } => Some(Solid::Box { min, max }),
            _ => None,
        })
        .collect()
}

impl Sim {
    /// Push every moving unit out of the asteroid fields (X and Z only).
    pub(crate) fn push_out_of_fields(&mut self) {
        let solids = solids(&self.regions);
        if solids.is_empty() {
            return;
        }
        let radii: Vec<Fixed> = self.units.iter().map(|u| self.body_radius(u.def)).collect();
        let obstacles = if self.confinement.is_active() { self.obstacles() } else { Vec::new() };
        for (u, &radius) in self.units.iter_mut().zip(&radii).filter(|(u, _)| u.mobile()) {
            for s in &solids {
                let previous = u.pos;
                let (dx, dz) = push_from_solid(u.pos, radius, s);
                u.pos.x = u.pos.x + dx;
                u.pos.z = u.pos.z + dz;
                if u.pos != previous {
                    crate::avoid::constrain_position(&mut self.confinement, u, previous, radius, &obstacles);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
