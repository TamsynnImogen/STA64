//! Client-only gains for the six subsystem emissive channels.

use std::time::Duration;

use sta_sim::{EntityId, Fixed, appearance::SystemAppearance};

/// A2FOExtensions holds each flicker decision for 90 ms and salts it by craft
/// and native subsystem record. Use the stable entity ID in place of its pointer.
#[derive(Clone, Copy)]
pub(super) struct FlickerPhase {
    bucket: u32,
    craft: EntityId,
}

impl FlickerPhase {
    pub(super) fn new(elapsed: Duration, craft: EntityId) -> Self {
        Self {
            bucket: (elapsed.as_millis() / 90) as u32,
            craft,
        }
    }

    pub(super) fn visible(self, system: usize) -> bool {
        // Translate AppearanceView slots to A2FO's subsystem record indices.
        let record = [1_u32, 2, 4, 3, 0][system];
        let mut value = self.bucket ^ self.craft.0 ^ record.wrapping_mul(0x9e3779b9);
        value ^= value >> 16;
        value = value.wrapping_mul(0x7feb352d);
        value ^= value >> 15;
        value = value.wrapping_mul(0x846ca68b);
        value ^= value >> 16;
        (value & 7) < 5
    }
}

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
        phase: FlickerPhase,
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
                self.destroyed[index] || (!s.enabled && !phase.visible(index))
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

    fn phase(ms: u64) -> FlickerPhase {
        FlickerPhase::new(Duration::from_millis(ms), EntityId(7))
    }

    fn gains(systems: Option<&[Option<SystemAppearance>; 5]>, speed: i32, cap: i32) -> [f32; 6] {
        EmissiveState::default().gains(systems, Fixed::from_int(speed), Fixed::from_int(cap), phase(90))
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
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100), phase(90))[..2],
            [2.0, 1.0]
        );
        systems[0] = system(true, 0, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100), phase(90))[..2],
            [0.0, 0.0]
        );
        systems[0] = system(true, 5, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100), phase(90))[..2],
            [0.0, 0.0]
        );
        systems[0] = system(true, 20, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100), phase(90))[..2],
            [2.0, 1.0]
        );
        systems[0] = system(true, 5, 20);
        assert_eq!(
            state.gains(Some(&systems), Fixed::from_int(106), Fixed::from_int(100), phase(90))[..2],
            [2.0, 1.0]
        );
    }

    #[test]
    fn disabled_engines_and_shields_flicker_independently() {
        let mut systems = [None; 5];
        systems[0] = system(false, 20, 20);
        systems[2] = system(false, 20, 20);
        let mut state = EmissiveState::default();
        let mut gains = |ms| state.gains(Some(&systems), Fixed::ZERO, Fixed::ZERO, phase(ms));
        assert_eq!(gains(89), [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(gains(90), [1.0, 1.0, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(gains(179), gains(90));
        assert_eq!(gains(180), [0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        assert!(FlickerPhase::new(Duration::from_millis(180), EntityId(8)).visible(0));
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
