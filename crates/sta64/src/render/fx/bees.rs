//! The construction ship's worker bees (RND-11 follow-up): how many, which model,
//! and the presentation-only flight phases from its home hardpoint to the next
//! unrevealed station edge and back.
//!
//! The games keep this in the construction ship's ODF: `numberOfWorkerBees` (6 for
//! the Dominion's ship, 12 for the Federation, Klingon and Romulan ones, 1 and
//! `workerBeeName = "empty"` for a base that builds without any), `workerBeeName` (the
//! SOD of one bee), `weldingBeamSprite` and `weldingBeamWidth` (2.0). Stock flies the
//! bees from the ship to spots on the new structure and welds there; Refit draws the
//! same (`bees.rs`, `[Bees]`). `BeeMotion` uses simulation time and a current
//! edge target.

use bevy::prelude::*;
use sta_odf::Odf;
/// Most bees a site draws, whatever the ODF says.
pub const MAX_BEES: u8 = 24;
/// `weldingBeamWidth` the beam's own width is tuned for.
pub const REF_BEAM_WIDTH: f32 = 2.0;

/// What a construction ship's ODF says about its bees.
#[derive(Debug, Clone, PartialEq)]
pub struct BeeSpec {
    /// Bees at a site; zero for a ship that builds with none.
    pub count: u8,
    /// SOD of one bee; `None`: the ODF has none (`empty`).
    pub model: Option<String>,
    /// Constructor model nodes where bees wait and return, in ODF order.
    pub hardpoints: Vec<String>,
    /// `weldingBeamSprite`; stock class default is `phaser`.
    pub sprite: Option<String>,
    /// `weldingBeamWidth`; absent defaults to one, explicit zero disables the beam.
    pub width: f32,
    pub takeoff: f32,
    pub welding: f32,
    pub closure: f32,
}

impl BeeSpec {
    /// The beam's width relative to the one the look is tuned for.
    pub fn width_scale(&self) -> f32 {
        if self.width > 0.0 {
            self.width / REF_BEAM_WIDTH
        } else {
            1.0
        }
    }
}

/// The bees of a unit's resolved ODF; `None` if it has no `numberOfWorkerBees`
/// (a unit that is not a construction ship, or content without the key).
pub fn bee_spec(odf: &Odf) -> Option<BeeSpec> {
    let count = odf.i64("numberofworkerbees")?.clamp(0, MAX_BEES as i64) as u8;
    let named = |key: &str| {
        odf.str(key).map(str::trim).filter(|s| {
            !s.is_empty() && !s.eq_ignore_ascii_case("empty") && !s.eq_ignore_ascii_case("none")
        })
    };
    let model = named("workerbeename").map(str::to_owned);
    let sprite = if odf.raw("weldingbeamsprite").is_some() {
        named("weldingbeamsprite").map(str::to_owned)
    } else {
        Some("phaser".into())
    };
    // A bee with no model and no beam is not drawn at all.
    let count = if model.is_none() && sprite.is_none() {
        0
    } else {
        count
    };
    let timing = |key, fallback| {
        odf.f32(key)
            .filter(|v| v.is_finite())
            .map_or(fallback, |v| v.max(0.0))
    };
    Some(BeeSpec {
        count,
        model,
        hardpoints: odf.raw("workerbeehardpoints").into_iter().flat_map(str::split_whitespace)
            .map(|name| name.trim_matches('"')).filter(|name| !name.is_empty())
            .map(str::to_owned).collect(),
        sprite,
        width: timing("weldingbeamwidth", 1.0),
        takeoff: timing("timeuntiltakeoff", 2.0),
        welding: timing("timewelding", 0.5),
        closure: timing("timeuntilclosure", 2.0),
    })
}

/// Presentation-only flight state. All deadlines are in `AnimClock::now` seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeePhase {
    Waiting,
    Traveling,
    Welding,
    Returning,
    Closing,
    Done,
}

#[derive(Debug, Clone, Copy)]
pub struct BeeMotion {
    pub phase: BeePhase,
    pub at: Vec3,
    pub edge: Vec3,
    pub cycle: usize,
    ever_left: bool,
    blocked_on_edge: bool,
    since: f64,
    from: Vec3,
    destination: Vec3,
}

impl BeeMotion {
    pub fn new(now: f64, home: Vec3) -> Self {
        Self {
            phase: BeePhase::Waiting,
            at: home,
            edge: home,
            cycle: 0,
            ever_left: false,
            blocked_on_edge: false,
            since: now,
            from: home,
            destination: home,
        }
    }

    pub fn forward(&self) -> Vec3 {
        (self.destination - self.at).normalize_or(Vec3::Z)
    }

    /// Advance toward a pending chunk edge, or return from the actual last position.
    pub fn update(
        &mut self,
        now: f64,
        home: Vec3,
        edges: &[Vec3],
        bee: u8,
        spec: &BeeSpec,
        returning: bool,
    ) {
        const SPEED: f32 = 80.0;
        const STANDOFF: f32 = 3.0;
        if !now.is_finite() {
            return;
        }
        if returning
            && !matches!(
                self.phase,
                BeePhase::Returning | BeePhase::Closing | BeePhase::Done
            )
        {
            self.phase = BeePhase::Returning;
            self.from = self.at;
            self.since = now;
        }
        if !returning && matches!(self.phase, BeePhase::Traveling | BeePhase::Welding) {
            // The edge seam carries positions but no stable IDs. Treat small
            // movement of an animated edge as the same target; a larger jump
            // starts travel again with welding off.
            let nearby = edges.iter().copied().min_by(|a, b| {
                a.distance_squared(self.edge)
                    .total_cmp(&b.distance_squared(self.edge))
            });
            if nearby.is_none() {
                self.phase = BeePhase::Waiting;
                self.blocked_on_edge = true;
                return;
            }
            if let Some(next) =
                nearby.filter(|next| next.distance_squared(self.edge) <= STANDOFF * STANDOFF)
            {
                self.edge = next;
                if self.phase == BeePhase::Traveling {
                    self.destination = next + (home - next).normalize_or(Vec3::Y) * STANDOFF;
                }
            } else {
                self.edge = edges[(bee as usize + self.cycle) % edges.len()];
                self.from = self.at;
                self.destination = self.edge + (home - self.edge).normalize_or(Vec3::Y) * STANDOFF;
                self.since = now;
                self.phase = BeePhase::Traveling;
                return;
            }
        }
        // At most a few transitions in one frame; zero authored durations do not spin.
        for _ in 0..6 {
            match self.phase {
                BeePhase::Waiting => {
                    if !self.ever_left {
                        self.at = home;
                    }
                    if now < self.since + f64::from(spec.takeoff) {
                        break;
                    }
                    if edges.is_empty() { self.blocked_on_edge = true; break; }
                    self.edge = edges[(bee as usize + self.cycle) % edges.len()];
                    self.from = self.at;
                    self.destination =
                        self.edge + (home - self.edge).normalize_or(Vec3::Y) * STANDOFF;
                    self.since = if self.blocked_on_edge { now } else { self.since + f64::from(spec.takeoff) };
                    self.blocked_on_edge = false;
                    self.phase = BeePhase::Traveling;
                    self.ever_left = true;
                }
                BeePhase::Traveling => {
                    let travel = f64::from(self.from.distance(self.destination) / SPEED);
                    let fraction = if travel > 0.0 {
                        ((now - self.since) / travel).clamp(0.0, 1.0) as f32
                    } else {
                        1.0
                    };
                    self.at = self.from.lerp(self.destination, fraction);
                    if fraction < 1.0 {
                        break;
                    }
                    self.since += travel;
                    self.phase = BeePhase::Welding;
                }
                BeePhase::Welding => {
                    if now < self.since + f64::from(spec.welding) {
                        break;
                    }
                    self.since += f64::from(spec.welding);
                    self.cycle += 1;
                    self.from = self.at;
                    self.edge = edges[(bee as usize + self.cycle) % edges.len()];
                    self.destination =
                        self.edge + (home - self.edge).normalize_or(Vec3::Y) * STANDOFF;
                    self.phase = BeePhase::Traveling;
                }
                BeePhase::Returning => {
                    self.destination = home;
                    let travel = f64::from(self.from.distance(home) / SPEED);
                    let fraction = if travel > 0.0 {
                        ((now - self.since) / travel).clamp(0.0, 1.0) as f32
                    } else {
                        1.0
                    };
                    self.at = self.from.lerp(home, fraction);
                    if fraction < 1.0 {
                        break;
                    }
                    self.at = home;
                    self.since += travel;
                    self.phase = BeePhase::Closing;
                }
                BeePhase::Closing => {
                    self.at = home;
                    break;
                }
                BeePhase::Done => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn odf(text: &str) -> Odf {
        sta_odf::resolve("rig", &mut |_| Some(text.to_owned())).unwrap()
    }

    #[test]
    fn count_model_and_width_come_from_the_odf() {
        let fed = bee_spec(&odf("[X]\nnumberOfWorkerBees = 12\nworkerBeeName = \"fed_s_workbee\"\nweldingBeamSprite = \"weldbeam\"\nweldingBeamWidth = 2.0\n")).unwrap();
        assert_eq!(
            (fed.count, fed.model.as_deref(), fed.width),
            (12, Some("fed_s_workbee"), 2.0)
        );
        let dom = bee_spec(&odf("numberOfWorkerBees = 6\nworkerBeeName = \"dom_s_workbee\"\nweldingBeamSprite = \"weldbeam\"\nweldingBeamWidth = 5.0\n")).unwrap();
        assert_eq!(dom.count, 6);
        assert_eq!(dom.width_scale(), 2.5);
        // A base's ship with no bees to draw.
        let none = bee_spec(&odf("numberOfWorkerBees = 1\nworkerBeeName = \"empty\"\nweldingBeamSprite = \"empty\"\nweldingBeamWidth = 0\n")).unwrap();
        assert_eq!((none.count, none.model), (0, None));
        // Not a construction ship, or content that never says.
        assert_eq!(bee_spec(&odf("unitName = \"x\"\n")), None);
        // A mod's absurd count is capped.
        assert_eq!(
            bee_spec(&odf("numberOfWorkerBees = 900\nworkerBeeName = \"b\"\n"))
                .unwrap()
                .count,
            MAX_BEES
        );
    }

    #[test]
    fn authored_hardpoints_sprite_and_timings_keep_zero_overrides() {
        let spec = bee_spec(&odf("numberOfWorkerBees = 6\nworkerBeeName = \"bee\"\nworkerBeeHardpoints = \"bot1\" \"bot2\" \"bot3\" \"bot4\" \"bot5\" \"bot6\"\nweldingBeamSprite = \"weldbeam\"\nweldingBeamWidth = 0\ntimeUntilTakeoff = 0\ntimeWelding = 0\ntimeUntilClosure = 0\n")).unwrap();
        assert_eq!(
            spec.hardpoints,
            ["bot1", "bot2", "bot3", "bot4", "bot5", "bot6"]
        );
        assert_eq!(spec.sprite.as_deref(), Some("weldbeam"));
        assert_eq!(
            (spec.width, spec.takeoff, spec.welding, spec.closure),
            (0.0, 0.0, 0.0, 0.0)
        );
        let defaults = bee_spec(&odf("numberOfWorkerBees = 2\nworkerBeeName = \"bee\"\n")).unwrap();
        assert_eq!(defaults.sprite.as_deref(), Some("phaser"));
        assert_eq!(
            (
                defaults.width,
                defaults.takeoff,
                defaults.welding,
                defaults.closure
            ),
            (1.0, 2.0, 0.5, 2.0)
        );
        let clamped = bee_spec(&odf("numberOfWorkerBees = 1\nworkerBeeName = \"bee\"\ntimeUntilTakeoff = -3\ntimeWelding = -1\ntimeUntilClosure = -2\n")).unwrap();
        assert_eq!(
            (clamped.takeoff, clamped.welding, clamped.closure),
            (0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn phase_is_sim_timed_and_edge_changes_stop_welding() {
        let spec = bee_spec(&odf("numberOfWorkerBees = 1\nworkerBeeName = \"bee\"\n")).unwrap();
        let home = Vec3::ZERO;
        let edge = Vec3::new(80.0, 0.0, 0.0);
        let mut regular = BeeMotion::new(0.0, home);
        let mut irregular = regular;
        for now in [0.0, 1.0, 2.0, 2.5, 3.1] {
            regular.update(now, home, &[edge], 0, &spec, false);
        }
        for now in [0.0, 0.17, 1.23, 2.25, 2.75, 3.1] {
            irregular.update(now, home, &[edge], 0, &spec, false);
        }
        assert_eq!(regular.phase, BeePhase::Welding);
        assert_eq!(irregular.phase, regular.phase);
        assert_eq!(irregular.at, regular.at);
        assert_eq!(regular.edge, edge);
        let paused = regular.at;
        regular.update(3.1, home, &[edge], 0, &spec, false);
        assert_eq!(regular.at, paused);
        let next = Vec3::new(0.0, 80.0, 0.0);
        regular.update(3.2, home, &[next], 0, &spec, false);
        assert_eq!(regular.phase, BeePhase::Traveling);
        assert_eq!(regular.edge, next);
        assert_eq!(regular.at, paused);
        regular.update(3.3, home, &[], 0, &spec, false);
        assert_eq!(regular.phase, BeePhase::Waiting);
        assert_eq!(
            regular.at, paused,
            "missing edge must not snap the bee home"
        );
        regular.update(3.4, home, &[], 0, &spec, true);
        assert_eq!(regular.phase, BeePhase::Returning);
        assert_eq!(regular.at, paused);
        regular.update(4.5, home, &[], 0, &spec, true);
        assert_eq!(regular.phase, BeePhase::Closing);
        assert_eq!(regular.at, home);
    }

    #[test]
    fn late_or_reappearing_edges_start_travel_at_current_time() {
        let spec = bee_spec(&odf("numberOfWorkerBees = 1\nworkerBeeName = \"bee\"\n")).unwrap();
        let home = Vec3::ZERO;
        let edge = Vec3::new(80.0, 0.0, 0.0);
        let mut bee = BeeMotion::new(0.0, home);
        bee.update(8.0, home, &[], 0, &spec, false);
        assert_eq!(bee.phase, BeePhase::Waiting);
        bee.update(8.0, home, &[edge], 0, &spec, false);
        assert_eq!(bee.phase, BeePhase::Traveling);
        assert_eq!(bee.at, home, "late geometry must not fast-forward flight");
        bee.update(8.25, home, &[edge], 0, &spec, false);
        let partway = bee.at;
        assert!(partway.x > 0.0 && partway.x < edge.x);
        bee.update(8.3, home, &[], 0, &spec, false);
        assert_eq!(bee.at, partway);
        bee.update(20.0, home, &[edge], 0, &spec, false);
        assert_eq!(bee.phase, BeePhase::Traveling);
        assert_eq!(bee.at, partway, "reappearing edge must not teleport bee");
    }

}
