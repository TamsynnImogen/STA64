//! Immutable production presentation, derived from persisted queue/site records.
//! This optional `WorldSource` view leaves serialized `WorldView` unchanged.

use super::{Production, launch, orbit, ticks_of};
use crate::world::Entities;
use sta_content::ContentDb;
use sta_core::{ContentKey, EntityId, Fixed, Vec3};
use std::collections::BTreeMap;

/// A producer's ordered queue and current site, ascending builder id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionView {
    pub builder: EntityId,
    pub jobs: Vec<ProductionJobView>,
    pub site: Option<ConstructionSiteView>,
    /// A yard's aperture during the interval between build and release.
    pub portal: Option<LaunchPortalView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchPortalView {
    pub def: ContentKey,
    pub pos: Vec3,
    pub heading: (Fixed, Fixed),
    pub open: Fixed,
}

/// Only a paid, started head is an actual in-yard hull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionJobView {
    pub def: ContentKey,
    pub progress: Fixed,
    pub paid: bool,
    pub started: bool,
}

/// Precise pose from the same launch/deployment geometry production uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructionSiteView {
    pub def: ContentKey,
    pub pos: Vec3,
    pub heading: (Fixed, Fixed),
    /// A welded station has a real id; a pending deploy or in-yard hull does
    /// not. Renderers must not invent simulation ids for these previews.
    pub entity: Option<EntityId>,
}

fn progress(done: u64, ticks: u64) -> Fixed {
    let ticks = ticks.max(1);
    Fixed::from_bits(((u128::from(done.min(ticks)) << 32) / u128::from(ticks)) as i64)
}

impl Production {
    pub(crate) fn view(&self, db: &ContentDb, units: &Entities) -> Vec<ProductionView> {
        let mut rows = BTreeMap::new();
        for (builder, queue) in self.queues.iter() {
            let Some(unit) = units.get(builder) else {
                continue;
            };
            let jobs: Vec<_> = queue
                .iter()
                .map(|job| ProductionJobView {
                    def: job.def,
                    progress: progress(job.done, ticks_of(db, job.def)),
                    paid: job.paid,
                    started: job.started,
                })
                .collect();
            if jobs.is_empty() {
                continue;
            }
            let launch = db.unit(unit.def).and_then(|builder| db.launch_wormhole(builder).map(|gate| (builder, gate)));
            let portal = launch.and_then(|(builder, gate)| {
                let head = queue.first().filter(|job| job.started)?;
                let elapsed = head.done.checked_sub(ticks_of(db, head.def))?;
                let out = launch::through_wormhole(builder, unit.pos, unit.heading);
                Some(LaunchPortalView { def: gate.key, pos: out.pos, heading: out.heading, open: progress(elapsed, gate.wormhole_open_ticks()) })
            });
            let site = jobs
                .first()
                .filter(|job| job.started && job.paid && launch.is_none())
                .map(|job| {
                    let (pos, heading) =
                        db.unit(unit.def).map_or((unit.pos, unit.heading), |def| {
                            if def.is_evolver() {
                                return (unit.pos, unit.heading);
                            }
                            let out = launch::place(def, unit.pos, unit.heading);
                            (out.pos, out.heading)
                        });
                    ConstructionSiteView {
                        def: job.def,
                        pos,
                        heading,
                        entity: None,
                    }
                });
            rows.insert(
                builder,
                ProductionView {
                    builder,
                    jobs,
                    site,
                    portal,
                },
            );
        }
        for (builder, deploy) in self.deploys.iter() {
            if units.get(builder).is_none() {
                continue;
            }
            let heading = if db.unit(deploy.def).is_some_and(orbit::is_orbital) {
                let (planets, _) = orbit::from_entities(db, units);
                orbit::owner(&planets, deploy.at).map_or((Fixed::ZERO, Fixed::ONE), |p| {
                    orbit::slot(&p, deploy.at).heading
                })
            } else {
                (Fixed::ZERO, Fixed::ONE)
            };
            rows.insert(
                builder,
                ProductionView {
                    builder,
                    jobs: vec![ProductionJobView {
                        def: deploy.def,
                        progress: Fixed::ZERO,
                        paid: deploy.paid,
                        started: false,
                    }],
                    site: Some(ConstructionSiteView {
                        def: deploy.def,
                        pos: deploy.at,
                        heading,
                        entity: None,
                    }),
                    portal: None,
                },
            );
        }
        for (id, weld) in self.welds.iter() {
            let Some(unit) = units.get(id) else { continue };
            // A live station keeps welding after its constructor dies. Keep
            // that site; consumers use the child's owner/visibility then.
            rows.insert(
                weld.builder,
                ProductionView {
                    builder: weld.builder,
                    jobs: vec![ProductionJobView {
                        def: unit.def,
                        progress: progress(weld.done, weld.ticks),
                        paid: true,
                        started: true,
                    }],
                    site: Some(ConstructionSiteView {
                        def: unit.def,
                        pos: unit.pos,
                        heading: unit.heading,
                        entity: Some(id),
                    }),
                    portal: None,
                },
            );
        }
        rows.into_values().collect()
    }
}
