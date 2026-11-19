//! Why a player cannot pay for a build, from what a client sees ([`PlayerView`]).
//!
//! The sim's own payment test ([`super::can_pay`]) and this one share [`first_short`], so a client
//! that asks before it sends an order gets the answer the sim will give, and can name what is short.
//! The officer test is the admission rule of [`super::officers`] over the view's `[census, limit]`.
//! The one thing a view cannot carry is the seat race's latinum factor (a team the data frees of
//! latinum); a client asking for such a seat sees the unscaled latinum cost.

use sta_content::ContentDb;
use sta_core::{ContentKey, Fixed};

use crate::view::PlayerView;

/// What is missing for a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortfall {
    /// The stock of this resource id is below the cost.
    Resource(usize),
    /// The officer census plus the build's officers would pass the limit.
    Officers,
}

/// The first resource id whose positive `cost` is above what `have` holds.
pub(super) fn first_short(have: &[Fixed], cost: &[Fixed]) -> Option<usize> {
    cost.iter().enumerate().find(|&(i, &c)| c > Fixed::ZERO && have.get(i).is_none_or(|&h| h < c)).map(|(i, _)| i)
}

/// Why `p` cannot start building `def` now, `None` when it can. A view limit of 0 reads as "no
/// officer limit" (the view shows `[0, 0]` for a seat without one).
pub fn build_shortfall(db: &ContentDb, p: &PlayerView, def: ContentKey) -> Option<Shortfall> {
    let d = db.unit(def)?;
    if let Some(i) = first_short(&p.resources, &d.build.cost) {
        return Some(Shortfall::Resource(i));
    }
    let cost = d.build.officers.max(Fixed::ZERO);
    let [census, limit] = p.officers;
    (limit > Fixed::ZERO && cost > Fixed::ZERO && Fixed::from_bits(census.to_bits().saturating_add(cost.to_bits())) > limit).then_some(Shortfall::Officers)
}
