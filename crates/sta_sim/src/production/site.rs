//! Build sites: the one rule that says whether a structure may stand at a
//! point. `Sim::apply` checks it for a construction rig's `Order::Build`, and
//! the game's placement ghost calls it with the same inputs, so what the
//! ghost shows and what the sim accepts cannot differ.
//!
//! STA64 rule: a site is blocked by any other
//! object under the footprint, own or foreign ships included, and by the
//! edge of the map; the builder itself is ignored. Footprints are squares
//! derived from the unit's model ([`UnitDef::launch`](sta_content::UnitDef) `clear`,
//! the loosest circle round the model), taken at three quarters since a
//! bounding circle is looser than the hull. Positive-area overlap is refused;
//! exact edge contact is permitted. This is STA64's declared placement rule,
//! not a claim of measured native touching behavior.

use sta_content::ContentDb;
use sta_core::{ContentKey, Fixed, Vec3};

/// Shared XZ bounds for build admission and the displayed footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub min: (Fixed, Fixed),
    pub max: (Fixed, Fixed),
}

impl Bounds {
    pub fn centered(at: Vec3, half_x: Fixed, half_z: Fixed) -> Self {
        let offset = |c: Fixed, h: Fixed, add: bool| {
            Fixed::from_bits(if add {
                c.to_bits().saturating_add(h.to_bits())
            } else {
                c.to_bits().saturating_sub(h.to_bits())
            })
        };
        Self {
            min: (offset(at.x, half_x, false), offset(at.z, half_z, false)),
            max: (offset(at.x, half_x, true), offset(at.z, half_z, true)),
        }
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.min.0 < other.max.0
            && other.min.0 < self.max.0
            && self.min.1 < other.max.1
            && other.min.1 < self.max.1
    }
}

/// Footprint radius used when a def has no model-derived one, units.
pub const DEFAULT_FOOTPRINT: i32 = 50;

/// Why a site is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteError {
    /// The footprint reaches past the playable volume.
    OffMap,
    /// Another object stands under the footprint.
    Blocked,
}

/// Footprint radius of `def`, units.
pub fn footprint(db: &ContentDb, def: ContentKey) -> Fixed {
    let r = db
        .unit(def)
        .and_then(|d| d.launch.as_ref())
        .map(|l| l.clear)
        .filter(|c| *c > Fixed::ZERO)
        .unwrap_or(Fixed::from_int(DEFAULT_FOOTPRINT));
    r * Fixed::from_int(3) / Fixed::from_int(4)
}

/// Physical map bodies keep their existing typed extent. A resource volume
/// without a model uses the same 100-unit extent as the existing field display.
pub fn obstacle_radius(db: &ContentDb, def: ContentKey) -> Fixed {
    let Some(unit) = db.unit(def) else {
        return footprint(db, def);
    };
    match crate::world::EntityKind::of_def(unit) {
        crate::world::EntityKind::Resource if unit.radius <= Fixed::ZERO => Fixed::from_int(100),
        crate::world::EntityKind::Resource | crate::world::EntityKind::Prop
            if unit.radius > Fixed::ZERO =>
        {
            unit.radius
        }
        _ => footprint(db, def),
    }
}

pub fn unit_bounds(db: &ContentDb, def: ContentKey, at: Vec3) -> Bounds {
    let r = obstacle_radius(db, def);
    Bounds::centered(at, r, r)
}

pub fn region_bounds(region: &sta_content::Region) -> Option<Bounds> {
    use sta_content::RegionShape;
    match region.shape {
        RegionShape::Sphere { centre, radius } if radius > Fixed::ZERO => {
            Some(Bounds::centered(centre, radius, radius))
        }
        RegionShape::Box { min, max } if min.x < max.x && min.z < max.z => Some(Bounds {
            min: (min.x, min.z),
            max: (max.x, max.z),
        }),
        _ => None,
    }
}

/// Shared rectangle intersection test. The grid's outer border is clipped to
/// these exact bounds; cell spacing is presentation only.
pub fn check_bounds(
    bounds: Option<&(Vec3, Vec3)>,
    candidate: Bounds,
    others: impl IntoIterator<Item = Bounds>,
) -> Result<(), SiteError> {
    if let Some((lo, hi)) = bounds
        && (candidate.min.0 < lo.x
            || candidate.max.0 > hi.x
            || candidate.min.1 < lo.z
            || candidate.max.1 > hi.z)
    {
        return Err(SiteError::OffMap);
    }
    if others.into_iter().any(|other| candidate.overlaps(other)) {
        return Err(SiteError::Blocked);
    }
    Ok(())
}

/// Whether `what` may stand at `at`. `bounds` is the map's playable volume
/// (`None`: unbounded); `others` are the obstacles as `(position, footprint)`,
/// with the builder and anything that does not block (shots) already left
/// out. Only X and Z count.
pub fn check_site(
    db: &ContentDb,
    bounds: Option<&(Vec3, Vec3)>,
    what: ContentKey,
    at: Vec3,
    others: impl IntoIterator<Item = (Vec3, Fixed)>,
) -> Result<(), SiteError> {
    check_bounds(
        bounds,
        unit_bounds(db, what, at),
        others.into_iter().map(|(p, r)| Bounds::centered(p, r, r)),
    )
}

/// The obstacles in a view, for callers outside the sim (the placement
/// ghost): every unit with a content def except `ignore`. Additional pending
/// deploys and typed regions are supplied by the live placement adapter.
pub fn view_obstacles(
    db: &ContentDb,
    view: &crate::WorldView,
    ignore: Option<sta_core::EntityId>,
) -> Vec<(Vec3, Fixed)> {
    view.units
        .iter()
        .filter(|u| Some(u.id) != ignore && db.unit(u.def).is_some())
        .map(|u| (u.pos, obstacle_radius(db, u.def)))
        .collect()
}
