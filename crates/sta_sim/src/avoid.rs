//! Unit separation and obstacle avoidance (SIM-21).
//!
//! # Radii
//!
//! A unit's body is a circle in the XZ plane of three quarters of its
//! definition's [`radius`](sta_content::UnitDef::radius) (the model's loosest
//! bounding circle times `ScaleSOD`; three quarters because that circle is
//! looser than the hull, the same factor the build-site rule uses for
//! footprints). A definition without a read model has radius zero and takes
//! no room, so content without models (and every sim made without content)
//! is untouched.
//!
//! # Who blocks whom
//!
//! - Craft keep out of each other, own and enemy: [`separate`] over a grid,
//!   each pair moving half the overlap, once per tick after kinematics.
//! - Solids are planets (resource-class defs that are not mining nodes),
//!   stations, props flagged `avoidMe = 1`, and asteroid fields. Craft are
//!   pushed out of them; stations and planets never move. A player's own
//!   stations are not solid to that player's craft: freighters dock at a
//!   station's centre ([`economy`](crate::economy)'s reach), rigs weld beside
//!   their structure and yards launch onto their own footprint.
//! - Moons that are mining nodes are not solid: miners work from beside them.
//!
//! # Steering
//!
//! Kinematics ([`kinematics`](crate::kinematics), both models) stays as it is
//! and only ever sees a point to fly to. Before it runs, a craft whose
//! straight leg to its target crosses a solid is given a detour point
//! instead: the tangent to the solid's circle (inflated by the craft's radius
//! and a margin) on the side the centre is not on, pushed a little further
//! along the circle so the craft never stalls on it. The real target is put
//! back right after the step, so orders, queues and arrival logic never see
//! the detour. Each tick is judged afresh from the craft's position, so no
//! state is kept. A target inside a solid is replaced, tick by tick, by the
//! nearest point just outside its edge, and the order ends once the craft is
//! within its deploy reach of that point.
//!
//! Boxes are steered round as their bounding circle.

use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use crate::Sim;
use crate::spatial::{Body, Grid, Solid, push_from_solid, separate};
use crate::world::EntityKind;

/// Grid cell width, `2^7` units.
const CELL_LOG2: u32 = 7;
/// Extra room kept round a solid when steering, units.
const MARGIN: i32 = 12;

fn three_quarters(r: Fixed) -> Fixed {
    r * Fixed::from_int(3) / Fixed::from_int(4)
}

/// An obstacle: the solid, and the player whose craft may overlap it.
pub(crate) struct Obstacle {
    pub(crate) solid: Solid,
    pub(crate) friend: Option<PlayerId>,
    /// Static regions have no actor; a moving solid never blocks itself.
    source: Option<EntityId>,
}

impl Obstacle {
    /// Whether `p` (with `slack`) is within the circle's bounding box: a
    /// cheap test that keeps far-off obstacles out of squared distances,
    /// which overflow past about 46000 units.
    fn near(&self, p: Vec3, slack: Fixed) -> bool {
        let (c, r) = self.circle();
        let reach = r + slack;
        (p.x - c.x).abs() <= reach && (p.z - c.z).abs() <= reach
    }

    /// Centre and radius of the circle steering goes round.
    fn circle(&self) -> (Vec3, Fixed) {
        match self.solid {
            Solid::Sphere { centre, radius } => (centre, radius),
            Solid::Box { min, max } => {
                let two = Fixed::from_int(2);
                let c = Vec3 {
                    x: (min.x + max.x) / two,
                    y: Fixed::ZERO,
                    z: (min.z + max.z) / two,
                };
                let (hx, hz) = ((max.x - min.x) / two, (max.z - min.z) / two);
                (c, (hx * hx + hz * hz).sqrt())
            }
        }
    }
}

impl Sim {
    /// The body radius of a definition (zero: takes no room).
    pub(crate) fn body_radius(&self, def: sta_core::ContentKey) -> Fixed {
        self.content
            .as_ref()
            .and_then(|db| db.unit(def))
            .map_or(Fixed::ZERO, |d| three_quarters(d.radius))
    }

    pub(crate) fn obstacles(&self) -> Vec<Obstacle> {
        let mut out: Vec<Obstacle> = crate::regions::solids(&self.regions)
            .into_iter()
            .map(|solid| Obstacle {
                solid,
                friend: None,
                source: None,
            })
            .collect();
        let Some(db) = self.content.as_ref() else {
            return out;
        };
        for e in self.units.iter() {
            let Some(d) = db.unit(e.def) else { continue };
            let radius = three_quarters(d.radius);
            if radius <= Fixed::ZERO {
                continue;
            }
            let solid = match e.kind {
                EntityKind::Resource => d.resource.is_none(),
                EntityKind::Station => true,
                EntityKind::Prop => d.avoid_me(),
                _ => false,
            };
            if solid {
                let friend = (e.kind == EntityKind::Station).then_some(e.owner);
                out.push(Obstacle {
                    solid: Solid::Sphere {
                        centre: e.pos,
                        radius,
                    },
                    friend,
                    source: Some(e.id),
                });
            }
        }
        out
    }

    /// Move targets out of solids and aim craft whose way is blocked round
    /// the blocker. Returns the real targets to put back after kinematics.
    pub(crate) fn steer_around(&mut self) -> Vec<(EntityId, Vec3)> {
        let obstacles = self.obstacles();
        if obstacles.is_empty() {
            return Vec::new();
        }
        let radii: Vec<Fixed> = self.units.iter().map(|u| self.body_radius(u.def)).collect();
        let margin = Fixed::from_int(MARGIN);
        let mut held = Vec::new();
        for (u, &ur) in self.units.iter_mut().zip(&radii) {
            if !u.mobile() || u.kind != EntityKind::Craft {
                continue;
            }
            let Some(real) = u.target else { continue };
            let mut t = real;
            let owner = u.owner;
            let mine = || obstacles.iter().filter(move |o| o.friend != Some(owner));
            let volume = self.confinement.active_volume(u);
            let solids: Vec<_> = if volume.is_some() { mine().map(|o| o.solid).collect() } else { Vec::new() };
            // A destination inside a solid ends at its edge: a margin beyond
            // it, so arriving there is neither touching (pushed back) nor a
            // leg that grazes the solid (steered round for ever).
            for o in mine().filter(|o| o.near(real, ur + margin)) {
                let (dx, dz) = push_from_solid(t, ur + margin, &o.solid);
                t.x = t.x + dx;
                t.z = t.z + dz;
            }
            let p = u.pos;
            if let Some(volume) = volume.filter(|v| v.check(t, false).outside) {
                // A pushed destination must never be clipped back into its
                // blocker. Choose a checked contact on the current side.
                use crate::boundary::contact::{contact, distance};
                let candidate = solids.iter().enumerate().filter_map(|(index, solid)| {
                    let point = contact(&volume, p, real, solid, ur, margin, &solids)?;
                    Some(((distance(point, real), distance(point, p), index), point))
                }).min_by_key(|(key, _)| *key).map(|(_, point)| point);
                let Some(candidate) = candidate else {
                    hold(u, real, &mut held);
                    continue;
                };
                t = candidate;
            }
            if t != real {
                // A craft cannot always land on a point it must approach
                // along an arc: the order is done within the craft's own
                // deploy reach of the edge (the same reach rigs use).
                let reach = crate::production::deploy_reach(&u.stats);
                let (dx, dz) = ((t.x - p.x).abs(), (t.z - p.z).abs());
                if dx <= reach && dz <= reach && (dx * dx + dz * dz).sqrt() <= reach {
                    u.target = None;
                    continue;
                }
                u.target = Some(t);
                held.push((u.id, real));
            }
            // The first solid on the straight leg.
            let (lx, lz) = (t.x - p.x, t.z - p.z);
            let len = (lx * lx + lz * lz).sqrt();
            if len == Fixed::ZERO {
                continue;
            }
            let (ux, uz) = (lx / len, lz / len);
            let mut hit: Option<(Fixed, Vec3, Fixed, Solid)> = None;
            for o in mine() {
                let (c, r) = o.circle();
                let reach = r + ur + margin / Fixed::from_int(2);
                // Compare boxes before squaring: distances here can be huge.
                let (lo_x, hi_x) = (p.x.min(t.x) - reach, p.x.max(t.x) + reach);
                let (lo_z, hi_z) = (p.z.min(t.z) - reach, p.z.max(t.z) + reach);
                if c.x < lo_x || c.x > hi_x || c.z < lo_z || c.z > hi_z {
                    continue;
                }
                let (cx, cz) = (c.x - p.x, c.z - p.z);
                let s = (cx * ux + cz * uz).max(Fixed::ZERO).min(len);
                let (qx, qz) = (cx - ux * s, cz - uz * s);
                if (qx * qx + qz * qz).sqrt() < reach && hit.is_none_or(|h| s < h.0) {
                    hit = Some((s, c, r + ur + margin, o.solid));
                }
            }
            let Some((_, c, ring, solid)) = hit else { continue };
            let preferred = detour(p, (ux, uz), c, ring, t.y);
            let target = if let Some(volume) = volume {
                use crate::boundary::contact::{contact, legal_target};
                let alternate = detours(p, (ux, uz), c, ring, t.y)[1];
                [preferred, alternate].into_iter().find(|&point| legal_target(&volume, p, point, ur, &solids))
                    .or_else(|| contact(&volume, p, t, &solid, ur, margin, &solids))
            } else { Some(preferred) };
            let Some(target) = target else {
                // `t != real` has already saved this order for restoration.
                hold(u, real, &mut held);
                continue;
            };
            u.target = Some(target);
            if t == real {
                held.push((u.id, real));
            }
        }
        held
    }

    /// Put back the targets [`steer_around`](Sim::steer_around) replaced.
    pub(crate) fn restore_targets(&mut self, held: Vec<(EntityId, Vec3)>) {
        for (id, t) in held {
            if let Some(u) = self.units.get_mut(id) {
                u.target = Some(t);
            }
        }
    }

    /// Keep craft out of each other and out of solids, once per tick.
    pub(crate) fn separate_units(&mut self) {
        if self.content.is_none() {
            return;
        }
        let bodies: Vec<Body> = self
            .units
            .iter()
            .filter(|u| u.kind == EntityKind::Craft)
            .map(|u| Body {
                id: u.id,
                pos: u.pos,
                radius: self.body_radius(u.def),
                movable: !self.production.is_welding(u.id),
            })
            .filter(|b| b.radius > Fixed::ZERO)
            .collect();
        if bodies.is_empty() {
            return;
        }
        let mut shifts = separate(&Grid::build(&bodies, CELL_LOG2));
        let obstacles = self.obstacles();
        for b in bodies.iter().filter(|body| body.movable) {
            let owner = self.units.get(b.id).map(|u| u.owner);
            let at = shifts.binary_search_by_key(&b.id, |x| x.0);
            let (mut sx, mut sz) = at.map_or((Fixed::ZERO, Fixed::ZERO), |i| shifts[i].1);
            let from = (sx, sz);
            for o in obstacles
                .iter()
                .filter(|o| o.friend.is_none() || o.friend != owner)
            {
                let p = Vec3 {
                    x: b.pos.x + sx,
                    y: b.pos.y,
                    z: b.pos.z + sz,
                };
                if !o.near(p, b.radius) {
                    continue;
                }
                let (dx, dz) = push_from_solid(p, b.radius, &o.solid);
                sx = sx + dx;
                sz = sz + dz;
            }
            if (sx, sz) != from {
                match at {
                    Ok(i) => shifts[i].1 = (sx, sz),
                    Err(i) => shifts.insert(i, (b.id, (sx, sz))),
                }
            }
        }
        for (id, (dx, dz)) in shifts {
            if let Some(u) = self.units.get_mut(id) {
                let previous = u.pos;
                u.pos.x = u.pos.x + dx;
                u.pos.z = u.pos.z + dz;
                if u.pos != previous {
                    let radius = bodies.binary_search_by_key(&id, |b| b.id).map_or(Fixed::ZERO, |i| bodies[i].radius);
                    constrain_position(&mut self.confinement, u, previous, radius, &obstacles);
                }
            }
        }
    }
}

fn hold(unit: &mut crate::Unit, real: Vec3, held: &mut Vec<(EntityId, Vec3)>) {
    unit.target = None;
    unit.speed = Fixed::ZERO;
    unit.omega = Fixed::ZERO;
    if held.last().is_none_or(|(id, _)| *id != unit.id) { held.push((unit.id, real)); }
}

/// Ordinary position update followed by checked solid-aware projection recovery.
/// Only actual projection pays for contact candidates. Bounds remain authoritative
/// if fully occluded; retaining a previous valid physical point is preferable.
pub(crate) fn constrain_position(
    confinement: &mut crate::boundary::state::Confinement,
    unit: &mut crate::Unit,
    previous: Vec3,
    radius: Fixed,
    obstacles: &[Obstacle],
) {
    let Some(volume) = confinement.position_updated(unit) else { return };
    use crate::boundary::contact::{contact, distance, legal_target};
    let solids: Vec<_> = obstacles.iter().filter(|o| o.source != Some(unit.id) && o.friend != Some(unit.owner)).map(|o| o.solid).collect();
    let desired = unit.pos;
    if legal_target(&volume, previous, desired, radius, &solids) { return; }
    let replacement = solids.iter().enumerate().filter_map(|(index, solid)| {
        let point = contact(&volume, previous, desired, solid, radius, Fixed::ZERO, &solids)?;
        Some(((distance(point, desired), distance(point, previous), index), point))
    }).min_by_key(|(key, _)| *key).map(|(_, point)| point);
    if let Some(point) = replacement {
        unit.pos = point;
    } else if legal_target(&volume, previous, previous, radius, &solids) {
        unit.pos = previous;
    }
}

/// The point to fly to from `p` heading `dir` to get round the circle
/// (`c`, `ring`): its tangent point on the side the centre is not, advanced
/// round the circle by a fixed angle so it is never reached.
fn detour(p: Vec3, dir: (Fixed, Fixed), c: Vec3, ring: Fixed, y: Fixed) -> Vec3 {
    detours(p, dir, c, ring, y)[0]
}

/// Keep the old preferred side first; confinement alone may select the other.
fn detours(p: Vec3, dir: (Fixed, Fixed), c: Vec3, ring: Fixed, y: Fixed) -> [Vec3; 2] {
    let (vx, vz) = (p.x - c.x, p.z - c.z);
    let d = (vx * vx + vz * vz).sqrt();
    let (nx, nz) = if d == Fixed::ZERO {
        (-dir.1, dir.0)
    } else {
        (vx / d, vz / d)
    };
    // Cosine and sine of the tangent angle at the centre (zero when inside).
    let (ca, sa) = if d > ring {
        let k = ring / d;
        (k, (Fixed::ONE - k * k).sqrt())
    } else {
        (Fixed::ONE, Fixed::ZERO)
    };
    // Plus a fixed look-ahead of 0.3 rad.
    let (cl, sl) = (Fixed::from_f64(0.955336), Fixed::from_f64(0.29552));
    let (cos, sin) = (ca * cl - sa * sl, sa * cl + ca * sl);
    let cand = |sgn: Fixed| (nx * cos - sgn * nz * sin, sgn * nx * sin + nz * cos);
    let side = dir.0 * (c.z - p.z) - dir.1 * (c.x - p.x);
    let pick = |(rx, rz): (Fixed, Fixed)| dir.0 * rz - dir.1 * rx;
    let (a, b) = (cand(Fixed::ONE), cand(-Fixed::ONE));
    // Aim at the candidate on the far side of the leg from the centre.
    let (first, second) = if side > Fixed::ZERO {
        if pick(a) < pick(b) { (a, b) } else { (b, a) }
    } else if pick(a) > pick(b) {
        (a, b)
    } else {
        (b, a)
    };
    [first, second].map(|(rx, rz)| Vec3 { x: c.x + rx * ring, y, z: c.z + rz * ring })
}

#[cfg(test)]
mod tests;
