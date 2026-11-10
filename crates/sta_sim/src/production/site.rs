//! Build sites: the one rule that says whether a structure may stand at a
//! point. `Sim::apply` checks it for a construction rig's `Order::Build`, and
//! the game's placement ghost calls it with the same inputs, so what the
//! ghost shows and what the sim accepts cannot differ.
//!
//! Stock rule (Armada II `CanPlaceHere`): a site is blocked by any other
//! object under the footprint, own or foreign ships included, and by the
//! edge of the map; the builder itself is ignored. Footprints are circles
//! from the unit's model ([`UnitDef::launch`](sta_content::UnitDef) `clear`,
//! the loosest circle round the model), taken at three quarters since a
//! bounding circle is looser than the hull.

use sta_content::ContentDb;
use sta_core::{ContentKey, Fixed, Vec3};

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
    let r = footprint(db, what);
    if let Some((lo, hi)) = bounds
        && (at.x - r < lo.x || at.x + r > hi.x || at.z - r < lo.z || at.z + r > hi.z)
    {
        return Err(SiteError::OffMap);
    }
    for (p, pr) in others {
        let (dx, dz) = (p.x - at.x, p.z - at.z);
        let reach = r + pr;
        if dx * dx + dz * dz < reach * reach {
            return Err(SiteError::Blocked);
        }
    }
    Ok(())
}

/// The obstacles in a view, for callers outside the sim (the placement
/// ghost): every unit with a content def except `ignore`. Rigs' pending
/// deploys are not in a view, so a site another rig has claimed is only
/// refused by the sim.
pub fn view_obstacles(
    db: &ContentDb,
    view: &crate::WorldView,
    ignore: Option<sta_core::EntityId>,
) -> Vec<(Vec3, Fixed)> {
    view.units
        .iter()
        .filter(|u| Some(u.id) != ignore && db.unit(u.def).is_some())
        .map(|u| (u.pos, footprint(db, u.def)))
        .collect()
}

pub(super) fn entity_obstacles(
    db: &ContentDb,
    units: &crate::world::Entities,
    ignore: sta_core::EntityId,
) -> Vec<(Vec3, Fixed)> {
    units
        .iter()
        .filter(|e| e.id != ignore && e.kind != crate::world::EntityKind::Projectile && db.unit(e.def).is_some())
        .map(|e| (e.pos, footprint(db, e.def)))
        .collect()
}
