//! Stock nebulae: the games' own cloud sprites (the nebula ODF's SOD of billboard
//! puffs, see `render::nebula::NebulaBoards`), not the volumetric march.

use crate::render::profile::{Effect, ProfileEffects, RenderProfile};

/// Whether a nebula class with a gas recipe is drawn as volumetric gas. True only for
/// the Refit implementation of the scenery effect; Stock draws the class's own
/// billboard model. With no profile resource (tests), the Refit look as before.
pub fn volumetric(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    match (profile, effects) {
        (Some(p), Some(e)) => e.implementation(*p, Effect::Scenery) == RenderProfile::Refit,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_draws_billboards_refit_draws_gas() {
        let mut fx = ProfileEffects::default();
        fx.register_stock(Effect::Scenery);
        assert!(!volumetric(Some(&RenderProfile::Stock), Some(&fx)));
        assert!(volumetric(Some(&RenderProfile::Refit), Some(&fx)));
        assert!(volumetric(Some(&RenderProfile::Stock), Some(&ProfileEffects::default())));
        assert!(volumetric(None, None));
    }
}
