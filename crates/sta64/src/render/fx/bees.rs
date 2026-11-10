//! The construction ship's worker bees (RND-11 follow-up): how many, which model,
//! and where each one is at a given time.
//!
//! The games keep this in the construction ship's ODF: `numberOfWorkerBees` (6 for
//! the Dominion's ship, 12 for the Federation, Klingon and Romulan ones, 1 and
//! `workerBeeName = "empty"` for a base that builds without any), `workerBeeName` (the
//! SOD of one bee), `weldingBeamSprite` and `weldingBeamWidth` (2.0). Stock flies the
//! bees from the ship to spots on the new structure and welds there; Refit draws the
//! same (`bees.rs`, `[Bees]`). Here each bee circles the structure on its own
//! tilted orbit, bobbing and jittering a little, and welds a spot of the hull that
//! changes every second or two. Positions come from the animation clock and a seed
//! of the structure and the bee, so they replay.

use bevy::prelude::*;
use sta_odf::Odf;
use sta_sim::EntityId;

use super::impacts::rand;

/// Most bees a site draws, whatever the ODF says.
pub const MAX_BEES: u8 = 24;
/// `weldingBeamWidth` the beam's own width is tuned for.
pub const REF_BEAM_WIDTH: f32 = 2.0;

/// Orbit radius around the structure (times its radius) and how far it varies per bee.
const ORBIT: f32 = 1.05;
const ORBIT_SPREAD: f32 = 0.35;
/// Orbit speed, radians a second, and its spread across bees.
const SPEED: f32 = 0.45;
const SPEED_SPREAD: f32 = 0.5;
/// Height of the bobbing, and the size of the jitter, in structure radii.
const BOB: f32 = 0.22;
const JITTER: f32 = 0.035;
/// Where on the hull a bee welds, in structure radii from the centre.
pub const SURFACE: f32 = 0.55;
/// Seconds a bee welds one spot, and the share of it spent moving the beam on.
const SPOT_SECONDS: f32 = 1.8;
const SPOT_SWING: f32 = 0.18;

/// What a construction ship's ODF says about its bees.
#[derive(Debug, Clone, PartialEq)]
pub struct BeeSpec {
    /// Bees at a site; zero for a ship that builds with none.
    pub count: u8,
    /// SOD of one bee; `None`: the ODF has none (`empty`).
    pub model: Option<String>,
    /// `weldingBeamWidth`; zero when not set.
    pub width: f32,
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
    let beam = named("weldingbeamsprite");
    // A bee with no model and no beam is not drawn at all.
    let count = if model.is_none() && beam.is_none() {
        0
    } else {
        count
    };
    Some(BeeSpec {
        count,
        model,
        width: odf.f32("weldingbeamwidth").unwrap_or(0.0),
    })
}

/// One bee at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeePose {
    pub at: Vec3,
    /// Unit vector of its motion.
    pub fwd: Vec3,
    /// The point of the hull it welds.
    pub spot: Vec3,
}

/// Where a bee of `n` is at `now` seconds, round a structure at `center` of `radius`.
pub fn bee_pose(site: EntityId, bee: u8, n: u8, center: Vec3, radius: f32, now: f32) -> BeePose {
    let at = orbit(site, bee, n, center, radius, now);
    let ahead = orbit(site, bee, n, center, radius, now + 0.1);
    let fwd = (ahead - at).normalize_or(Vec3::X);

    // The spot: each cycle a new point on the hull, on the bee's side, blended in
    // from the last one while the beam swings over.
    let offset = rand(site, 0, bee, 9) * SPOT_SECONDS;
    let t = (now + offset) / SPOT_SECONDS;
    let cycle = t.floor();
    let into = t - cycle;
    let spot_of = |c: f32| {
        let tc = c * SPOT_SECONDS - offset;
        let side = (orbit(site, bee, n, center, radius, tc) - center).normalize_or(Vec3::Y);
        let r = |s: u8| rand(site, 1 + c.max(0.0) as u64, bee, 10 + s);
        let (y, a) = (r(0) * 2.0 - 1.0, r(1) * std::f32::consts::TAU);
        let ring = (1.0 - y * y).sqrt();
        let scatter = Vec3::new(ring * a.cos(), y, ring * a.sin());
        (side + scatter * 0.8).normalize_or(side)
    };
    let swing = (into / SPOT_SWING).clamp(0.0, 1.0);
    let swing = swing * swing * (3.0 - 2.0 * swing);
    let dir = spot_of(cycle - 1.0)
        .lerp(spot_of(cycle), swing)
        .normalize_or(Vec3::Y);
    let spot = center + dir * radius * SURFACE;
    BeePose { at, fwd, spot }
}

fn orbit(site: EntityId, bee: u8, n: u8, center: Vec3, radius: f32, now: f32) -> Vec3 {
    let r = |s: u8| rand(site, 0, bee, s);
    let tau = std::f32::consts::TAU;
    let dir = if r(2) < 0.5 { -1.0 } else { 1.0 };
    let speed = dir * SPEED * (1.0 - SPEED_SPREAD + SPEED_SPREAD * 2.0 * r(1));
    let phase = tau * bee as f32 / n.max(1) as f32 + r(3) * 0.6;
    let theta = phase + speed * now;
    let rad =
        radius * (ORBIT + ORBIT_SPREAD * r(0)) * (1.0 + 0.12 * (now * 0.55 + r(4) * tau).sin());
    let mut p = Vec3::new(theta.cos(), 0.0, theta.sin()) * rad;
    // Tilt the orbit about the x axis so the bees do not all ride one plane.
    p = Quat::from_rotation_x((r(5) - 0.5) * 1.3) * p;
    p.y += radius * BOB * (now * (0.8 + r(6)) + r(7) * tau).sin();
    // Flitting: a small quick wobble on top.
    let w = now * 2.7 + r(8) * tau;
    p += Vec3::new(w.sin(), (w * 1.3).cos(), (w * 0.7).sin()) * radius * JITTER;
    center + p
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
    fn bees_move_around_the_site_and_stay_near_it() {
        let (site, c, r) = (EntityId(7), Vec3::new(100.0, 0.0, -50.0), 40.0);
        for bee in 0..6 {
            let mut seen = Vec::new();
            for i in 0..40 {
                let p = bee_pose(site, bee, 6, c, r, i as f32 * 0.5);
                let d = p.at.distance(c);
                assert!(d > r * 0.6 && d < r * 2.0, "bee {bee} at {d}");
                assert!(
                    (p.spot.distance(c) - r * SURFACE).abs() < r * 0.01,
                    "spot off the hull"
                );
                assert!((p.fwd.length() - 1.0).abs() < 1e-4);
                seen.push(p);
            }
            let travelled: f32 = seen.windows(2).map(|w| w[0].at.distance(w[1].at)).sum();
            assert!(travelled > r * 4.0, "bee {bee} went {travelled}");
            assert!(
                seen.windows(2)
                    .any(|w| w[0].spot.distance(w[1].spot) > r * 0.1),
                "beam never moved"
            );
        }
        // Not all on one spot, and the same again on a replay.
        let a = bee_pose(site, 0, 6, c, r, 3.0);
        assert_ne!(a.at, bee_pose(site, 1, 6, c, r, 3.0).at);
        assert_eq!(a, bee_pose(site, 0, 6, c, r, 3.0));
        assert_ne!(a.at, bee_pose(EntityId(8), 0, 6, c, r, 3.0).at);
    }
}
