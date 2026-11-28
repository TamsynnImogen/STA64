//! Armada II warp suppression (read from the executable, static).
//! Both entry and cruise are checked at the current position. Gravity wells
//! are passable at impulse, so they do not become obstacles for path planning.

use sta_core::{Fixed, Vec3};

use crate::Sim;
use crate::view::UnitFlags;
use crate::world::EntityKind;

pub(crate) struct GravityWells(Vec<(Vec3, Fixed)>, Vec<(Vec3, Fixed)>);

impl GravityWells {
    /// The fields of planets among the wells, centre and radius, in entity order: what the warp
    /// planner routes round (#1148). The other wells only stop warp.
    pub(crate) fn planet_fields(&self) -> &[(Vec3, Fixed)] {
        &self.1
    }

    pub(crate) fn suppresses(&self, position: Vec3, hull_radius: Fixed) -> bool {
        self.0.iter().any(|&(centre, radius)| {
            // The native test is sphere overlap, including the ship's own radius.
            // Square raw 32.32 differences in wide integers so large wells or
            // extreme map coordinates cannot overflow ordinary Fixed products.
            let reach = (i128::from(radius.to_bits()) + i128::from(hull_radius.max(Fixed::ZERO).to_bits())) as u128;
            let distance_sq = [(position.x, centre.x), (position.y, centre.y), (position.z, centre.z)]
                .into_iter()
                .map(|(a, b)| (i128::from(a.to_bits()) - i128::from(b.to_bits())).unsigned_abs().pow(2))
                .fold(0, u128::saturating_add);
            distance_sq < reach.pow(2)
        })
    }
}

impl Sim {
    /// Live fields, in entity order, sampled once before steering this tick.
    pub(crate) fn gravity_wells(&self) -> GravityWells {
        let Some(db) = self.content.as_deref().filter(|_| self.warp_allowed()) else {
            return GravityWells(Vec::new(), Vec::new());
        };
        let fields = db.rules().fields();
        if [fields.planet_dewarp_radius_multiplier, fields.black_hole_dewarp_radius_multiplier]
            .into_iter()
            .flatten()
            .all(|multiplier| multiplier <= Fixed::ZERO)
        {
            return GravityWells(Vec::new(), Vec::new());
        }
        let (mut wells, mut planets) = (Vec::new(), Vec::new());
        for unit in self.units.iter().filter(|unit| matches!(unit.kind, EntityKind::Resource | EntityKind::Prop) && !unit.flags.contains(UnitFlags::DYING)) {
            let Some(def) = db.unit(unit.def) else { continue };
            let Some(radius) = db.rules().warp_suppression_radius(def).filter(|r| *r > Fixed::ZERO) else { continue };
            wells.push((unit.pos, radius));
            if db.rules().planet_warp_suppression_radius(def).is_some() {
                planets.push((unit.pos, radius));
            }
        }
        GravityWells(wells, planets)
    }
}
