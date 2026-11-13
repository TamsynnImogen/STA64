//! Transient, per-instance subsystem part visibility and repair presentation.
//! This controller consumes actual subsystem HP; it never infers it from hull
//! health or changes simulation state. Systems use indices 0..5 in the view's
//! engines, weapons, shields, life support, sensors order.

use sta_core::EntityId;

/// Rendering instructions for a single subsystem root on this update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualPlan {
    /// Zero-based index into the valid candidate list, if one is selected.
    pub selected: Option<usize>,
    pub root_scale: f32,
    pub hide_descendants: bool,
    pub just_destroyed: bool,
    pub just_completed: bool,
    /// Positive normalized HP gain since the previous valid observation.
    pub repair_progress_delta: f32,
}

impl Default for VisualPlan {
    fn default() -> Self {
        Self {
            selected: None,
            root_scale: 1.0,
            hide_descendants: false,
            just_destroyed: false,
            just_completed: false,
            repair_progress_delta: 0.0,
        }
    }
}

/// One controller per entity and system. Destruction counts and selection are
/// deliberately presentation-only and need not survive a snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemVisualState {
    identity: Option<(EntityId, usize)>,
    previous_progress: Option<f32>,
    destroyed: bool,
    destruction_count: u32,
}

impl SystemVisualState {
    /// Update from `[hp, max_hp]`. Missing/invalid HP, system indices outside
    /// 0..5, and `EntityId::NONE` disable the visual without guessing damage.
    /// The first valid zero-HP observation selects a part without an explosion.
    pub fn update(
        &mut self,
        entity: EntityId,
        system: usize,
        health: Option<[f32; 2]>,
        candidate_count: usize,
    ) -> VisualPlan {
        if entity == EntityId::NONE || system >= 5 {
            *self = Self::default();
            return VisualPlan::default();
        }
        let Some([hp, max]) = health else {
            *self = Self::default();
            return VisualPlan::default();
        };
        if !hp.is_finite() || !max.is_finite() || max <= 0.0 || hp < 0.0 || hp > max {
            *self = Self::default();
            return VisualPlan::default();
        }
        let identity = (entity, system);
        if self.identity != Some(identity) {
            *self = Self {
                identity: Some(identity),
                ..Self::default()
            };
        }

        let progress = (hp / max).clamp(0.0, 1.0);
        let previous = self.previous_progress;
        let mut plan = VisualPlan::default();
        if previous.is_none() {
            // A loaded zero-HP system is already destroyed. A loaded partial
            // system has no known missing part and starts as intact.
            self.destroyed = hp == 0.0;
        } else if !self.destroyed && hp == 0.0 {
            self.destroyed = true;
            self.destruction_count = self.destruction_count.wrapping_add(1);
            plan.just_destroyed = true;
        } else if self.destroyed && hp == max {
            self.destroyed = false;
            plan.just_completed = true;
        }

        if self.destroyed {
            plan.selected =
                selected_candidate(entity, system, self.destruction_count, candidate_count);
            plan.root_scale = progress * progress * (3.0 - 2.0 * progress);
            plan.hide_descendants = true;
            plan.repair_progress_delta = previous.map_or(0.0, |old| (progress - old).max(0.0));
        }
        self.previous_progress = Some(progress);
        plan
    }
}

fn selected_candidate(
    entity: EntityId,
    system: usize,
    destruction_count: u32,
    count: usize,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    // Fixed integer avalanche. Neither HashMap iteration nor Rust's default
    // randomized hasher affects the choice. Modulo also remaps safely if the
    // valid candidate list becomes shorter after a mod or model refresh.
    let mut value = (entity.0 as u64)
        ^ ((system as u64) << 32)
        ^ (destruction_count as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    Some((value % count as u64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: EntityId = EntityId(7);

    #[test]
    fn destruction_partial_repair_and_completion_emit_once() {
        let mut state = SystemVisualState::default();
        assert_eq!(
            state.update(ID, 0, Some([10.0, 10.0]), 3),
            VisualPlan::default()
        );
        let destroyed = state.update(ID, 0, Some([0.0, 10.0]), 3);
        assert!(destroyed.just_destroyed && destroyed.hide_descendants);
        assert_eq!(destroyed.root_scale, 0.0);
        assert!(destroyed.selected.is_some_and(|n| n < 3));
        let repeat = state.update(ID, 0, Some([0.0, 10.0]), 3);
        assert_eq!(repeat.selected, destroyed.selected);
        assert!(!repeat.just_destroyed);
        let repair = state.update(ID, 0, Some([5.0, 10.0]), 3);
        assert_eq!(repair.root_scale, 0.5);
        assert_eq!(repair.repair_progress_delta, 0.5);
        assert!(repair.hide_descendants);
        assert_eq!(
            state
                .update(ID, 0, Some([5.0, 10.0]), 3)
                .repair_progress_delta,
            0.0
        );
        let complete = state.update(ID, 0, Some([10.0, 10.0]), 3);
        assert!(complete.just_completed);
        assert_eq!(complete.selected, None);
        assert!(!complete.hide_descendants);
        assert!(!state.update(ID, 0, Some([10.0, 10.0]), 3).just_completed);
        let next = state.update(ID, 0, Some([0.0, 10.0]), 3);
        assert!(next.just_destroyed);
        assert_eq!(next.selected, selected_candidate(ID, 0, 2, 3));
    }

    #[test]
    fn initial_destroyed_and_partial_state_do_not_invent_transition() {
        let mut loaded = SystemVisualState::default();
        let first = loaded.update(ID, 2, Some([0.0, 20.0]), 2);
        assert!(first.selected.is_some());
        assert!(!first.just_destroyed);
        assert!(!loaded.update(ID, 2, Some([0.0, 20.0]), 2).just_destroyed);
        let mut partial = SystemVisualState::default();
        assert_eq!(
            partial.update(ID, 2, Some([8.0, 20.0]), 2),
            VisualPlan::default()
        );
        assert!(partial.update(ID, 2, Some([0.0, 20.0]), 2).just_destroyed);
    }

    #[test]
    fn unknown_and_invalid_health_disable_visual_without_guessing() {
        let mut state = SystemVisualState::default();
        for input in [
            None,
            Some([0.0, 0.0]),
            Some([f32::NAN, 2.0]),
            Some([1.0, f32::INFINITY]),
            Some([-1.0, 2.0]),
            Some([3.0, 2.0]),
        ] {
            assert_eq!(state.update(ID, 0, input, 3), VisualPlan::default());
        }
        assert_eq!(
            state.update(ID, 5, Some([0.0, 2.0]), 3),
            VisualPlan::default()
        );
        assert_eq!(
            state.update(EntityId::NONE, 0, Some([0.0, 2.0]), 3),
            VisualPlan::default()
        );
        let after_unknown = state.update(ID, 0, Some([0.0, 2.0]), 3);
        assert!(!after_unknown.just_destroyed);
    }

    #[test]
    fn independent_entities_cycles_and_candidate_shrink() {
        let mut a = SystemVisualState::default();
        let mut b = SystemVisualState::default();
        a.update(ID, 1, Some([4.0, 4.0]), 64);
        b.update(EntityId(8), 1, Some([4.0, 4.0]), 64);
        let first_a = a.update(ID, 1, Some([0.0, 4.0]), 64);
        let first_b = b.update(EntityId(8), 1, Some([0.0, 4.0]), 64);
        assert_eq!(first_a.selected, selected_candidate(ID, 1, 1, 64));
        assert_eq!(first_b.selected, selected_candidate(EntityId(8), 1, 1, 64));
        let shrunk = a.update(ID, 1, Some([0.0, 4.0]), 1);
        assert_eq!(shrunk.selected, Some(0));
        assert!(!shrunk.just_destroyed);
        a.update(ID, 1, Some([4.0, 4.0]), 1);
        assert_eq!(a.update(ID, 1, Some([0.0, 4.0]), 1).selected, Some(0));
        let reused = a.update(EntityId(8), 1, Some([0.0, 4.0]), 2);
        assert!(!reused.just_destroyed);
    }
}
