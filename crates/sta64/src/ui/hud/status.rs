//! Selection-scoped production and cargo presentation for the HUD.
//! The simulation owns these values; this module neither predicts work nor
//! issues commands. Keep this feed separate from `HudFeed` so external frame
//! runs can continue to supply their existing world-only feed.

use bevy::prelude::Resource;
use sta_sim::economy::view::CargoView;
use sta_sim::production::view::ProductionView;
use sta_sim::{ContentKey, Fixed};

use super::ExternalFeed;
use super::model::HudFeed;
use crate::sim_link::SimLink;

/// Optional records from the same tick as `HudFeed::view`. Empty data means
/// unavailable, never zero. The runtime copies `SimLink`'s immutable views.
#[derive(Resource, Default, Clone)]
pub(crate) struct StatusFeed {
    pub production: Vec<ProductionView>,
    pub cargo: Vec<CargoView>,
    /// Resource-node amounts (UI-48's dilithium and planet-metal bars).
    pub nodes: Vec<sta_sim::economy::view::NodeView>,
    /// UI-48: units with damaged or switched-off systems; the rest are whole and on.
    pub systems: Vec<sta_sim::combat::damage::SystemView>,
}

/// The stock slot a selected resource node yields. A harvestable latinum
/// field names its slot through its class, as in the simulation.
pub(super) fn node_slot(def: &sta_content::UnitDef) -> Option<usize> {
    if !sta_sim::world::EntityKind::mineable_def(def) {
        return None;
    }
    let slot = if def.class.0 == "latinum" { 1 } else { def.resource.unwrap_or(0) };
    (usize::from(slot) < sta_content::resources::SLOTS).then_some(usize::from(slot))
}

pub(super) fn node_status(feed: Option<&StatusFeed>, id: sta_sim::EntityId) -> Option<&sta_sim::economy::view::NodeView> {
    feed?.nodes.iter().find(|node| node.entity == id)
}

pub(super) fn node_fraction(feed: Option<&StatusFeed>, id: sta_sim::EntityId) -> f32 {
    match node_status(feed, id) {
        Some(n) if !n.infinite && n.start > Fixed::ZERO => {
            (n.amount.to_f32() / n.start.to_f32()).clamp(0.0, 1.0)
        }
        Some(n) if !n.infinite => 0.0,
        _ => 1.0,
    }
}

pub(super) fn population_text(feed: &HudFeed, id: sta_sim::EntityId) -> Option<String> {
    if !crate::input::select::target_seen(&feed.view, Some(feed.player), id) {
        return None;
    }
    let db = feed.content.as_deref()?;
    let def = db.unit(feed.view.unit(id)?.def)?;
    if def.planet_role != Some(sta_content::PlanetRole::Producer) {
        return None;
    }
    // An uncolonized planet need not have a colony row. Its cap is still data.
    let (current, cap) = match feed.view.colonies.binary_search_by_key(&id, |row| row.id) {
        Ok(i) => {
            let row = &feed.view.colonies[i];
            (row.population, row.max_population)
        }
        Err(_) => (Fixed::ZERO, db.rules().population()?.cap(def.planet_max_population)),
    };
    let whole = |n: Fixed| n.to_bits() / Fixed::ONE.to_bits();
    Some(crate::ui::i18n::trf("Population {0}/{1}", &[&whole(current), &whole(cap)]))
}

/// Keep optional status at the same simulation tick as the regular HUD feed.
/// External frame feeds and ended games must not inherit the last game's rows.
pub(super) fn sync(
    link: Option<bevy::prelude::Res<SimLink>>,
    external: Option<bevy::prelude::Res<ExternalFeed>>,
    mut feed: bevy::prelude::ResMut<StatusFeed>,
) {
    if external.is_some() || link.is_none() {
        feed.production.clear();
        feed.cargo.clear();
        feed.nodes.clear();
        feed.systems.clear();
    } else if let Some(link) = link {
        feed.nodes.clear();
        feed.nodes.extend_from_slice(link.nodes());
        feed.systems.clear();
        feed.systems.extend_from_slice(link.systems());
        feed.production.clear();
        feed.production.extend_from_slice(link.production());
        feed.cargo.clear();
        feed.cargo.extend_from_slice(link.cargo());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobState {
    /// The paid, started head is the active build.
    Building,
    /// The head has not yet been paid and cannot progress.
    AwaitingResources,
    /// A later job, or a paid head waiting for deployment.
    Queued,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueueItem {
    pub def: ContentKey,
    pub odf: Option<String>,
    pub model: Option<String>,
    pub name: String,
    pub state: JobState,
    /// Progress is meaningful only for the started, paid head.
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CargoStatus {
    pub amount: Fixed,
    pub capacity: Fixed,
    pub resource: Option<u8>,
    pub fraction: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct SelectedStatus {
    /// The selected object's own construction progress, including welded
    /// stations. `UnitView` is authoritative even after its builder dies.
    pub construction: Option<f32>,
    /// Only the local player's selected producer exposes its queue.
    pub queue: Vec<QueueItem>,
    /// Only the local player's selected mining craft exposes its hold.
    pub cargo: Option<CargoStatus>,
}

fn fraction(value: Fixed) -> f32 {
    value.to_f32().clamp(0.0, 1.0)
}

/// Return details only for one live selected object. A group selection has no
/// single queue or hold to display, and stale/missing ids carry no status.
pub(crate) fn selected(feed: &HudFeed, source: &StatusFeed) -> SelectedStatus {
    let units = feed.selected_units();
    let [unit] = units.as_slice() else {
        return SelectedStatus::default();
    };
    if !feed
        .view
        .visible_to(feed.player)
        .iter()
        .any(|seen| seen.id == unit.id)
    {
        return SelectedStatus::default();
    }
    let mut out = SelectedStatus {
        construction: unit.progress.filter(|p| *p < Fixed::ONE).map(fraction),
        ..Default::default()
    };
    if unit.owner != feed.player {
        return out;
    }
    if let Some(producer) = source.production.iter().find(|p| p.builder == unit.id) {
        out.queue = producer
            .jobs
            .iter()
            .enumerate()
            .map(|(index, job)| {
                let def = feed.content.as_deref().and_then(|db| db.unit(job.def));
                let active = index == 0 && job.paid && job.started;
                QueueItem {
                    def: job.def,
                    odf: def.map(|d| d.odf.clone()),
                    model: def.map(|d| d.model.clone()),
                    name: def.map_or_else(
                        || format!("#{}", job.def.0),
                        |d| {
                            super::model::craft_names::unit_name(d)
                                .unwrap_or(&d.odf)
                                .to_owned()
                        },
                    ),
                    state: if active {
                        JobState::Building
                    } else if index == 0 && !job.paid {
                        JobState::AwaitingResources
                    } else {
                        JobState::Queued
                    },
                    progress: active.then(|| fraction(job.progress)),
                }
            })
            .collect();
    }
    out.cargo = source.cargo.iter().find(|c| c.entity == unit.id).map(|c| {
        let capacity = c.capacity.to_f32();
        CargoStatus {
            amount: c.amount,
            capacity: c.capacity,
            resource: c.resource,
            fraction: if capacity > 0.0 {
                (c.amount.to_f32() / capacity).clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::ContentDb;
    use sta_sim::production::view::ProductionJobView;
    use sta_sim::view::UnitFlags;
    use sta_sim::{EntityId, PlayerId, UnitView, Vec3, WorldView};

    use super::*;

    fn unit(id: u32, owner: u8, progress: Option<Fixed>) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: ContentKey::NONE,
            owner: PlayerId(owner),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE, Fixed::ONE],
            shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE,
            progress,
            crew: None,
            energy: None,
            autonomy: None,
        }
    }

    fn feed(units: Vec<UnitView>, selected: Vec<EntityId>) -> HudFeed {
        HudFeed {
            view: Arc::new(WorldView {
                units,
                ..Default::default()
            }),
            player: PlayerId(1),
            selected,
            content: Some(Arc::new(ContentDb::builder().build().unwrap())),
            production: Vec::new(),
            names: Default::default(),
        }
    }

    #[test]
    fn queue_keeps_order_and_distinguishes_active_from_waiting() {
        let feed = feed(vec![unit(1, 1, None)], vec![EntityId(1)]);
        let source = StatusFeed {
            production: vec![ProductionView {
                portal: None,
                research: None,
                builder: EntityId(1),
                jobs: vec![
                    ProductionJobView {
                        def: ContentKey(11),
                        progress: Fixed::from_f64(0.4),
                        paid: true,
                        started: true,
                    },
                    ProductionJobView {
                        def: ContentKey(12),
                        progress: Fixed::ZERO,
                        paid: false,
                        started: false,
                    },
                ],
                site: None,
            }],
            ..Default::default()
        };
        let rows = selected(&feed, &source).queue;
        assert_eq!(
            rows.iter().map(|r| r.def).collect::<Vec<_>>(),
            [ContentKey(11), ContentKey(12)]
        );
        assert_eq!(rows[0].state, JobState::Building);
        assert!((rows[0].progress.unwrap() - 0.4).abs() < 0.001);
        assert_eq!(rows[1].state, JobState::Queued);
        assert_eq!(rows[1].progress, None);
    }

    #[test]
    fn idle_cargo_and_unfinished_station_use_live_values() {
        let source = StatusFeed {
            cargo: vec![CargoView {
                entity: EntityId(2),
                amount: Fixed::ZERO,
                capacity: Fixed::from_int(500),
                resource: None,
            }],
            ..Default::default()
        };
        let miner = feed(vec![unit(2, 1, None)], vec![EntityId(2)]);
        let cargo = selected(&miner, &source).cargo.unwrap();
        assert_eq!(
            (cargo.amount, cargo.capacity, cargo.resource, cargo.fraction),
            (Fixed::ZERO, Fixed::from_int(500), None, 0.0)
        );

        let station = feed(
            vec![unit(3, 1, Some(Fixed::from_f64(0.3)))],
            vec![EntityId(3)],
        );
        assert!((selected(&station, &source).construction.unwrap() - 0.3).abs() < 0.001);
    }

    #[test]
    fn enemy_and_group_selection_hide_private_status() {
        let source = StatusFeed {
            production: vec![ProductionView {
                portal: None,
                research: None,
                builder: EntityId(1),
                jobs: vec![ProductionJobView {
                    def: ContentKey(11),
                    progress: Fixed::ZERO,
                    paid: false,
                    started: false,
                }],
                site: None,
            }],
            cargo: vec![CargoView {
                entity: EntityId(1),
                amount: Fixed::from_int(5),
                capacity: Fixed::from_int(10),
                resource: Some(0),
            }],
                nodes: Vec::new(),
                systems: Vec::new(),
        };
        let enemy = feed(
            vec![unit(1, 2, Some(Fixed::from_f64(0.2)))],
            vec![EntityId(1)],
        );
        let shown = selected(&enemy, &source);
        assert!(shown.queue.is_empty());
        assert!(shown.cargo.is_none());
        assert!(shown.construction.is_some());
        let group = feed(
            vec![unit(1, 1, None), unit(2, 1, None)],
            vec![EntityId(1), EntityId(2)],
        );
        assert_eq!(selected(&group, &source), SelectedStatus::default());
    }

    #[test]
    fn selected_enemy_lost_to_fog_has_no_live_construction_status() {
        let source = StatusFeed::default();
        let mut enemy = feed(
            vec![unit(1, 2, Some(Fixed::from_f64(0.2)))],
            vec![EntityId(1)],
        );
        assert!(selected(&enemy, &source).construction.is_some());
        let mut hidden = (*enemy.view).clone();
        hidden.visibility = vec![(PlayerId(1), Vec::new())];
        enemy.view = Arc::new(hidden);
        assert_eq!(selected(&enemy, &source), SelectedStatus::default());
    }
}
