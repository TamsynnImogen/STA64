//! Client-only gains for the six subsystem emissive channels.

use sta_sim::{appearance::SystemAppearance, Fixed};

/// Channel order: warp, impulse, shields, life support, sensors, weapons.
#[derive(Default)]
pub(super) struct EmissiveState {
    destroyed: [bool; 5],
}

impl EmissiveState {
    pub(super) fn gains(
        &mut self,
        systems: Option<&[Option<SystemAppearance>; 5]>,
        speed: Fixed,
        impulse_cap: Fixed,
    ) -> [f32; 6] {
        let mut gains = [1.0; 6];

        // The native motion fallback has no explicit warp state here. Its
        // velocity test uses squared speed > 0.01, then warp above 105% of
        // the positive impulse cap. The idle/impulse/warp gains follow the
        // native fallback. The view does not expose gravity-well transitions.
        let velocity = speed.to_f32();
        if velocity * velocity > 0.01 {
            let cap = impulse_cap.to_f32();
            let threshold = cap * 1.05;
            if cap > 0.0 && threshold.is_finite() && velocity * velocity > threshold * threshold {
                gains[0] = 2.0;
            } else {
                gains[0] = 1.25;
                gains[1] = 1.5;
            }
        }

        if let Some(systems) = systems {
            // AppearanceView order: engines, weapons, shields, life, sensors.
            // Missing/zero-max records are unknown and remain visible. An
            // enabled system with ordinary partial damage stays lit. Once
            // observed at zero health, it stays dark until fully repaired.
            let mut dark = |index: usize| {
                let Some(s) = systems[index].filter(|s| s.health[1] > Fixed::ZERO) else {
                    return false;
                };
                if s.health[0] <= Fixed::ZERO {
                    self.destroyed[index] = true;
                } else if s.health[0] >= s.health[1] {
                    self.destroyed[index] = false;
                }
                !s.enabled || self.destroyed[index]
            };
            if dark(0) {
                gains[0] = 0.0;
                gains[1] = 0.0;
            }
            for (system, channel) in [(2, 2), (3, 3), (4, 4), (1, 5)] {
                if dark(system) {
                    gains[channel] = 0.0;
                }
            }
        }
        gains
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(enabled: bool, current: i32, maximum: i32) -> Option<SystemAppearance> {
        Some(SystemAppearance {
            enabled,
            health: [Fixed::from_int(current), Fixed::from_int(maximum)],
        })
    }

    fn gains(systems: Option<&[Option<SystemAppearance>; 5]>, speed: i32, cap: i32) -> [f32; 6] {
        EmissiveState::default().gains(systems, Fixed::from_int(speed), Fixed::from_int(cap))
    }

    #[test]
    fn motion_fallback_has_exact_idle_impulse_and_warp_gains() {
        assert_eq!(gains(None, 0, 100), [1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(gains(None, 100, 100), [1.25, 1.5, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(gains(None, 106, 100), [2.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(gains(None, 106, 0), [1.25, 1.5, 1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn destroyed_engine_is_dark_even_if_enabled_and_during_partial_repair() {
        let mut systems = [None; 5];
        let mut state = EmissiveState::default();
        systems[0] = system(true, 5, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100))[..2],
            [2.0, 1.0]
        );
        systems[0] = system(true, 0, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100))[..2],
            [0.0, 0.0]
        );
        systems[0] = system(true, 5, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100))[..2],
            [0.0, 0.0]
        );
        systems[0] = system(true, 20, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100))[..2],
            [2.0, 1.0]
        );
        systems[0] = system(true, 5, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100))[..2],
            [2.0, 1.0]
        );
    }

    #[test]
    fn switched_off_system_restores_at_full_health() {
        let mut systems = [None; 5];
        systems[2] = system(false, 20, 20);
        assert_eq!(gains(Some(&systems), 0, 100)[2], 0.0);
        systems[2] = system(true, 20, 20);
        assert_eq!(gains(Some(&systems), 0, 100)[2], 1.0);
    }

    #[test]
    fn life_and_sensors_are_isolated_and_unknown_fails_open() {
        let mut systems = [None; 5];
        systems[3] = system(true, 0, 20);
        assert_eq!(
            gains(Some(&systems), 0, 100),
            [1.0, 1.0, 1.0, 0.0, 1.0, 1.0]
        );
        systems[3] = None;
        systems[4] = system(false, 0, 0);
        assert_eq!(
            gains(Some(&systems), 0, 100),
            [1.0, 1.0, 1.0, 1.0, 1.0, 1.0]
        );
        systems[4] = system(true, 0, 20);
        assert_eq!(
            gains(Some(&systems), 0, 100),
            [1.0, 1.0, 1.0, 1.0, 0.0, 1.0]
        );
        systems[4] = None;
        systems[1] = system(true, 0, 20);
        assert_eq!(
            gains(Some(&systems), 0, 100),
            [1.0, 1.0, 1.0, 1.0, 1.0, 0.0]
        );
    }
}
