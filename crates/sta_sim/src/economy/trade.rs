//! Armada II cargo routes and station exchanges. The station shares its
//! physical berth with regular freighters.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, TradeStationDef};
use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use super::{Economy, dist, dock, per_tick, set_target};
use crate::attrs::Attrs;
use crate::command::{TradeAction, TradeResource};
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, Fnv, Table};
use crate::{Rng, Sim};

const LATINUM: u8 = 1;
const LOT: Fixed = Fixed::from_int(100);
const SALE_PRICE: Fixed = Fixed::from_int(70);
const BUY_PRICE: Fixed = Fixed::from_bits((1000i64 << 32) / 7);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Cargo {
    Goods {
        station: EntityId,
        origin: Vec3,
        value: Fixed,
    },
    Resource(TradeResource),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Hold {
    cargo: Cargo,
    amount: Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Route {
    Goods,
    Loading(TradeResource),
    Shipping(TradeResource),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Unload,
    Load,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Dock {
    station: EntityId,
    joined: Option<u64>,
    phase: Option<Phase>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Merchant {
    source: Option<EntityId>,
    destination: Option<EntityId>,
    route: Option<Route>,
    hold: Option<Hold>,
    dock: Option<Dock>,
}

pub(crate) type TradeRec = Vec<(EntityId, Merchant)>;

fn eligible(unit: &Entity) -> bool {
    unit.owner.is_player()
        && unit.hull[0] > Fixed::ZERO
        && !unit.flags.contains(UnitFlags::DYING)
        && unit.progress.is_none_or(|p| p >= Fixed::ONE)
}

#[derive(Clone, Copy)]
pub(crate) struct TradeRules<'a>(pub &'a ContentDb, pub &'a Attrs);

impl<'a> TradeRules<'a> {
    fn allows(self, unit: &Entity) -> bool {
        eligible(unit)
            && self
                .0
                .unit(unit.def)
                .is_some_and(|def| self.0.can_trade(self.1.race(unit.id).unwrap_or(&def.race)))
    }

    fn station(self, unit: &Entity) -> Option<&'a TradeStationDef> {
        if !self.allows(unit) {
            return None;
        }
        self.0.unit(unit.def)?.trade_station.as_ref()
    }
}

pub(super) fn capacity(db: &ContentDb, unit: &Entity) -> Option<Fixed> {
    let def = db.unit(unit.def)?;
    def.trade_cargo_capacity
}

fn sum(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(a.to_bits().saturating_add(b.to_bits()))
}

fn product(a: Fixed, b: Fixed) -> Fixed {
    let bits = (i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 32;
    Fixed::from_bits(bits.clamp(0, i128::from(i64::MAX)) as i64)
}

fn spatial_distance(a: Vec3, b: Vec3) -> Fixed {
    let squared = [(a.x, b.x), (a.y, b.y), (a.z, b.z)]
        .into_iter()
        .map(|(a, b)| i128::from(a.to_bits()) - i128::from(b.to_bits()))
        .map(|delta| delta.unsigned_abs().saturating_pow(2))
        .fold(0u128, u128::saturating_add);
    Fixed::from_bits(squared.isqrt().min(i64::MAX as u128) as i64)
}

impl Merchant {
    fn target(self) -> Option<EntityId> {
        match self.route? {
            Route::Goods => {
                let source = self.source?;
                let destination = self.destination?;
                if matches!(self.hold.map(|h| h.cargo), Some(Cargo::Goods { station, .. }) if station == source)
                {
                    Some(destination)
                } else {
                    Some(source)
                }
            }
            Route::Loading(_) => self.source,
            Route::Shipping(_) => self.destination,
        }
    }

    fn source(&mut self, station: EntityId) {
        self.source = Some(station);
        if self.destination == Some(station) {
            self.destination = None;
        }
    }

    fn destination(&mut self, station: EntityId) {
        self.destination = Some(station);
        if self.source == Some(station) {
            self.source = None;
        }
    }

    fn resume(&mut self) {
        if matches!(self.route, Some(Route::Loading(_))) {
            return;
        }
        if let Some(Hold {
            cargo: Cargo::Resource(resource),
            ..
        }) = self.hold
        {
            self.route = self.destination.map(|_| Route::Shipping(resource));
        } else {
            self.route = self.source.zip(self.destination).map(|_| Route::Goods);
        }
    }

    fn cargo_matches_route(self) -> bool {
        match (self.route, self.hold.map(|h| h.cargo)) {
            (None, _) | (Some(Route::Goods), None | Some(Cargo::Goods { .. })) => true,
            (Some(Route::Loading(_)), None) => true,
            (Some(Route::Loading(res) | Route::Shipping(res)), Some(Cargo::Resource(held))) => {
                res == held
            }
            _ => false,
        }
    }

    fn stop(&mut self) {
        self.route = None;
        self.dock = None;
    }
}

impl Sim {
    pub(crate) fn order_trade(
        &mut self,
        player: PlayerId,
        ids: &[EntityId],
        action: TradeAction,
    ) -> bool {
        let Some(db) = self.content.clone() else {
            return false;
        };
        let rules = TradeRules(&db, &self.attrs);
        let actors: Vec<_> = ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|&id| {
                self.units.get(id).is_some_and(|u| {
                    rules.allows(u)
                        && (!player.is_player() || u.owner == player)
                        && !self.production.is_welding(id)
                })
            })
            .collect();
        if let TradeAction::Buy(resource) | TradeAction::Sell(resource) = action {
            let Some(station) = actors
                .iter()
                .filter_map(|&id| self.units.get(id))
                .find(|u| rules.station(u).is_some())
            else {
                return false;
            };
            // One click trades one lot, even when several stations were selected.
            return self.economy.exchange(
                station.owner,
                resource,
                matches!(action, TradeAction::Buy(_)),
            );
        }
        let endpoint = match action {
            TradeAction::Source(id) | TradeAction::Destination(id) | TradeAction::Auto(id) => {
                let Some(station) = self.units.get(id).filter(|u| rules.station(u).is_some())
                else {
                    return false;
                };
                Some(station.id)
            }
            _ => None,
        };

        let mut accepted = false;
        let mut redirected = Vec::new();
        for id in actors {
            let ship = self.units.get(id).expect("validated actor");
            if !ship.orderable()
                || capacity(&db, ship).is_none_or(|max| max <= Fixed::ZERO)
                || endpoint == Some(id)
            {
                continue;
            }
            let mut trader = self.economy.traders.get(id).copied().unwrap_or_default();
            let was_active = trader.route.is_some();
            let action = match action {
                TradeAction::Auto(station) => {
                    if matches!(trader.hold.map(|h| h.cargo), Some(Cargo::Resource(_))) {
                        TradeAction::Destination(station)
                    } else if trader.source.is_none() {
                        TradeAction::Source(station)
                    } else if trader.source == Some(station) && trader.destination.is_none() {
                        continue;
                    } else {
                        TradeAction::Destination(station)
                    }
                }
                explicit => explicit,
            };
            match action {
                TradeAction::Source(station) => {
                    trader.source(station);
                    trader.resume();
                }
                TradeAction::Destination(station) => {
                    trader.destination(station);
                    // An early destination stays set while the ship loads.
                    trader.resume();
                }
                TradeAction::Load(resource) => {
                    let source = trader
                        .source
                        .and_then(|id| self.units.get(id))
                        .filter(|u| rules.station(u).is_some())
                        .or_else(|| {
                            self.units
                                .iter()
                                .filter(|u| u.owner == ship.owner && rules.station(u).is_some())
                                .min_by_key(|u| (spatial_distance(ship.pos, u.pos), u.id))
                        });
                    let Some(source) = source else { continue };
                    match trader.hold {
                        Some(Hold {
                            cargo: Cargo::Resource(held),
                            ..
                        }) if resource != held => continue,
                        // Armada II starts a shipment by clearing its old goods.
                        Some(Hold {
                            cargo: Cargo::Goods { .. },
                            ..
                        }) => trader.hold = None,
                        _ => {}
                    }
                    trader.source(source.id);
                    trader.destination = None;
                    trader.route = Some(Route::Loading(resource));
                }
                _ => unreachable!("station action or context endpoint handled above"),
            }
            accepted = true;
            if trader.route.is_some() || was_active {
                redirected.push((id, trader));
            } else {
                self.economy.traders.insert(id, trader);
            }
        }
        let taken: Vec<_> = redirected.iter().map(|(id, _)| *id).collect();
        self.clear_scripted(player, &taken);
        self.weapons.stop_attack(&self.units, player, &taken);
        self.economy.cancel(&self.units, player, &taken);
        for (id, mut trader) in redirected {
            self.approach_warp.remove(id);
            self.watchdog.remove(id);
            self.movement.clear(id);
            set_target(&mut self.units, id, None);
            trader.dock = None;
            self.economy.traders.insert(id, trader);
        }
        accepted
    }
}

impl Economy {
    fn exchange(&mut self, player: PlayerId, resource: TradeResource, buy: bool) -> bool {
        let slot = match resource {
            TradeResource::Dilithium => 0,
            TradeResource::Metal => 2,
            _ => return false,
        };
        let (debit, price, credit, amount) = if buy {
            (LATINUM, BUY_PRICE, slot, LOT)
        } else {
            (slot, LOT, LATINUM, SALE_PRICE)
        };
        if !self.spend(player, debit, price) {
            return false;
        }
        self.credit(player, credit, amount);
        true
    }

    fn credit(&mut self, player: PlayerId, stock: u8, amount: Fixed) {
        if amount > Fixed::ZERO {
            let slot = &mut self.slot(player)[usize::from(stock)];
            *slot = sum(*slot, amount);
        }
    }

    fn load_resource(&mut self, player: PlayerId, resource: TradeResource, amount: Fixed) -> Fixed {
        let Some(stock) = resource.stock_slot() else {
            return self.withdraw_crew(player, amount);
        };
        let Some(i) = self.stock.binary_search_by_key(&player, |r| r.0).ok() else {
            return Fixed::ZERO;
        };
        let slot = &mut self.stock[i].1[usize::from(stock)];
        let taken = amount.min((*slot).max(Fixed::ZERO));
        *slot = *slot - taken;
        taken
    }

    fn unload_resource(&mut self, player: PlayerId, resource: TradeResource, amount: Fixed) {
        if let Some(stock) = resource.stock_slot() {
            self.credit(player, stock, amount);
        } else {
            let crew = sum(self.crew_of(player).unwrap_or_default(), amount);
            self.set_personnel(player, Some(crew), self.officer_base(player));
        }
    }

    fn pay_goods(
        &mut self,
        ship: &Entity,
        station: &Entity,
        units: &Entities,
        cargo: Cargo,
        share: Fixed,
    ) {
        let Cargo::Goods {
            station: source,
            origin,
            value,
        } = cargo
        else {
            return;
        };
        if source == station.id {
            return;
        }
        // CargoShip::mGetCargoProfit uses pickup-to-dock distance, with
        // reductions for proximity and for two stations of the same owner.
        let distance = spatial_distance(origin, station.pos);
        let base = (i128::from(distance.to_bits()) + (5i128 << 32)) / (10i128 << 32);
        let mut profit = Fixed::from_int(base as i32);
        if distance < Fixed::from_int(1300) {
            profit = profit / Fixed::from_int(10);
        } else if distance < Fixed::from_int(3200) {
            profit = profit / Fixed::from_int(5);
        }
        if units
            .get(source)
            .is_some_and(|source| eligible(source) && source.owner == station.owner)
        {
            profit = profit / Fixed::from_int(10);
        }
        let profit = product(profit, value);
        let commission = product(profit, share).min(profit);
        self.credit(ship.owner, LATINUM, profit - commission);
        self.credit(station.owner, LATINUM, commission);
    }

    pub(super) fn suspend_trade(&mut self, id: EntityId) {
        if let Some(trader) = self.traders.get_mut(id) {
            trader.stop();
        }
    }

    pub(super) fn trade_queue_ahead(
        &self,
        id: EntityId,
        ticket: dock::Ticket,
    ) -> impl Iterator<Item = EntityId> + '_ {
        self.traders.iter().filter_map(move |(other, trader)| {
            let dock = trader.dock?;
            let joined = dock.joined?;
            (other != id
                && dock.station == ticket.station
                && dock.phase.is_none()
                && ticket
                    .joined
                    .is_none_or(|ours| (joined, other) < (ours, id)))
            .then_some(other)
        })
    }

    /// Only the ship's current dock stops being an obstacle while approaching.
    pub(crate) fn trade_dock_of(&self, id: EntityId) -> Option<EntityId> {
        self.traders.get(id)?.dock.map(|dock| dock.station)
    }

    pub(super) fn trade_berths(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.traders.iter().filter_map(|(_, trader)| {
            let dock = trader.dock?;
            dock.phase.map(|_| dock.station)
        })
    }

    pub(crate) fn step_trade(
        &mut self,
        units: &mut Entities,
        rules: TradeRules<'_>,
        tick: u64,
        rng: &mut Rng,
    ) {
        if self.traders.is_empty() {
            return;
        }
        let mut occupied = self.docked_stations(units);
        occupied.extend(self.trade_berths());
        let ids: Vec<_> = self
            .traders
            .iter()
            .filter(|(_, t)| t.route.is_some())
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            let mut trader = *self.traders.get(id).expect("existing trade row");
            self.step_merchant(units, rules, rng, (tick, id), &mut trader, &mut occupied);
            self.traders.insert(id, trader);
        }
    }

    fn step_merchant(
        &mut self,
        units: &mut Entities,
        rules: TradeRules<'_>,
        rng: &mut Rng,
        (tick, id): (u64, EntityId),
        trader: &mut Merchant,
        occupied: &mut BTreeSet<EntityId>,
    ) {
        let target = trader
            .dock
            .filter(|d| d.phase.is_some())
            .map(|d| d.station)
            .or_else(|| trader.target());
        let db = rules.0;
        let Some(ship) = units.get(id) else { return };
        let Some(max) = capacity(db, ship).filter(|&c| c > Fixed::ZERO && rules.allows(ship))
        else {
            trader.stop();
            set_target(units, id, None);
            return;
        };
        let Some((station, def)) = target.and_then(|id| {
            let station = units.get(id)?;
            Some((station, rules.station(station)?))
        }) else {
            trader.stop();
            set_target(units, id, None);
            return;
        };

        let berth = dock::Berth::of(db, station);
        let mut dock = trader
            .dock
            .filter(|d| d.station == station.id)
            .unwrap_or(Dock {
                station: station.id,
                joined: None,
                phase: None,
            });
        if dock.phase.is_none() {
            let turn = super::dock::Ticket {
                station: station.id,
                joined: dock.joined.or(Some(tick)),
            };
            let ahead = self
                .trade_queue_ahead(id, turn)
                .chain(self.queue_ahead(id, turn))
                .filter_map(|other| units.get(other))
                .map(|other| super::dock::span(db, other.def, berth.authored))
                .fold(Fixed::ZERO, sum);
            let arrival = dock::arrival(db, ship.def, berth.authored);
            let berth_range = if berth.authored {
                arrival
            } else {
                arrival + berth.extent
            };
            let open = !occupied.contains(&station.id) && ahead == Fixed::ZERO;
            let queue_span = dock::span(db, ship.def, berth.authored);
            let waiting = berth.waiting_point(sum(ahead, queue_span));
            let goal = if open { berth.pos } else { waiting };
            if dock.joined.is_none()
                && (dist(ship.pos, berth.pos) <= berth.extent + queue_span * Fixed::from_int(2)
                    || dist(ship.pos, waiting) <= arrival)
            {
                dock.joined = Some(tick);
            }
            if !open || dist(ship.pos, berth.pos) > berth_range {
                trader.dock = Some(dock);
                set_target(units, id, (dist(ship.pos, goal) > arrival).then_some(goal));
                return;
            }
            occupied.insert(station.id);
            dock.joined.get_or_insert(tick);
            dock.phase = Some(match trader.route {
                Some(Route::Shipping(_)) => Phase::Unload,
                Some(Route::Goods) if trader.hold.is_some() => Phase::Unload,
                _ => Phase::Load,
            });
        }

        let rate = per_tick(def.transfer_rate).max(Fixed::from_bits(1));
        let route = trader.route.expect("active merchant");
        let station_pos = station.pos;
        let station_id = station.id;
        let owner = station.owner;
        let ship_owner = ship.owner;
        match dock.phase.expect("docked merchant") {
            Phase::Unload => {
                if let Some(hold) = &mut trader.hold {
                    let amount = hold.amount.min(rate);
                    hold.amount = hold.amount - amount;
                    match hold.cargo {
                        Cargo::Goods { .. } if hold.amount == Fixed::ZERO => {
                            self.pay_goods(ship, station, units, hold.cargo, def.profit_share);
                        }
                        Cargo::Resource(resource) => self.unload_resource(owner, resource, amount),
                        _ => {}
                    }
                }
                if trader.hold.is_none_or(|h| h.amount == Fixed::ZERO) {
                    trader.hold = None;
                    if route == Route::Goods {
                        dock.phase = Some(Phase::Load);
                    } else {
                        trader.stop();
                    }
                }
            }
            Phase::Load => match route {
                Route::Goods => {
                    let mut hold = trader.hold.unwrap_or_else(|| {
                        let value = if def.goods.is_empty() {
                            Fixed::ONE
                        } else {
                            def.goods[(rng.next_u64() % def.goods.len() as u64) as usize].value
                        };
                        Hold {
                            cargo: Cargo::Goods {
                                station: station_id,
                                origin: station_pos,
                                value,
                            },
                            amount: Fixed::ZERO,
                        }
                    });
                    hold.amount = sum(hold.amount, rate).min(max);
                    trader.hold = Some(hold);
                    if hold.amount == max {
                        dock.phase = None;
                    }
                }
                Route::Loading(resource) => {
                    let held = trader.hold.map_or(Fixed::ZERO, |h| h.amount);
                    let wanted = (max - held).min(rate);
                    let taken = self.load_resource(ship_owner, resource, wanted);
                    let amount = held + taken;
                    trader.hold = (amount > Fixed::ZERO).then_some(Hold {
                        cargo: Cargo::Resource(resource),
                        amount,
                    });
                    if amount == max || taken < wanted {
                        trader.route = if amount > Fixed::ZERO && trader.destination.is_some() {
                            Some(Route::Shipping(resource))
                        } else {
                            None
                        };
                        dock.phase = None;
                    }
                }
                Route::Shipping(_) => trader.stop(),
            },
        }
        trader.dock = dock.phase.filter(|_| trader.route.is_some()).map(|_| dock);
        if let Some(ship) = units.get_mut(id) {
            ship.target = None;
            ship.speed = Fixed::ZERO;
            ship.omega = Fixed::ZERO;
            if berth.authored {
                ship.pos = berth.pos;
                ship.heading = berth.heading;
            }
        }
    }

    pub(crate) fn remove_trade_entity(&mut self, units: &mut Entities, entity: EntityId) {
        if units.contains(entity) {
            // Capture cancels the route. The new owner takes the physical hold.
            if let Some(trader) = self.traders.get_mut(entity) {
                trader.stop();
                trader.source = None;
                trader.destination = None;
            }
        } else {
            self.traders.remove(entity);
        }
        for (id, trader) in self.traders.iter_mut() {
            let mut changed = false;
            if trader.source == Some(entity) {
                trader.source = None;
                changed = true;
            }
            if trader.destination == Some(entity) {
                trader.destination = None;
                changed = true;
            }
            if changed && trader.route.is_some() {
                trader.stop();
                set_target(units, id, None);
            }
        }
    }

    pub(crate) fn trade_rows(&self) -> TradeRec {
        self.traders
            .iter()
            .map(|(id, trader)| (id, *trader))
            .collect()
    }

    pub(crate) fn restore_trade(
        &mut self,
        rows: TradeRec,
        units: &Entities,
        db: Option<&ContentDb>,
        tick: u64,
    ) -> Option<()> {
        let mut occupied = self.docked_stations(units);
        for &(id, trader) in &rows {
            let db = db?;
            let max = capacity(db, units.get(id)?)?;
            let valid_station = |station| {
                id != station
                    && units
                        .get(station)
                        .and_then(|u| db.unit(u.def))
                        .is_some_and(|d| d.trade_station.is_some())
            };
            if trader.source.is_some_and(|s| !valid_station(s))
                || trader
                    .destination
                    .is_some_and(|s| !valid_station(s) || trader.source == Some(s))
                || trader
                    .hold
                    .is_some_and(|h| h.amount <= Fixed::ZERO || h.amount > max)
                || !trader.cargo_matches_route()
                || trader.route.is_some() && trader.target().is_none()
                || trader.dock.is_some_and(|d| {
                    trader.route.is_none()
                        || !valid_station(d.station)
                        || (Some(d.station) != trader.source
                            && Some(d.station) != trader.destination)
                        || d.joined.is_some_and(|joined| joined > tick)
                        || d.phase.is_some() && (d.joined.is_none() || !occupied.insert(d.station))
                })
            {
                return None;
            }
        }
        self.traders = Table::from_rows(rows)?;
        Some(())
    }

    pub(super) fn hash_trade(&self, hash: &mut Fnv) {
        if self.traders.is_empty() {
            return;
        }
        hash.u64(0x7472_6164_655f_7631);
        // Same canonical rows as the snapshot. Each byte affects lockstep.
        let bytes = postcard::to_allocvec(&self.trade_rows()).expect("trade rows serialize");
        for byte in bytes {
            hash.u64(u64::from(byte));
        }
    }

    pub(super) fn trade_cargo(
        &self,
        db: &ContentDb,
        unit: &Entity,
    ) -> Option<super::view::CargoView> {
        let capacity = capacity(db, unit)?;
        let hold = self.traders.get(unit.id).and_then(|t| t.hold);
        Some(super::view::CargoView {
            entity: unit.id,
            capacity,
            amount: hold.map_or(Fixed::ZERO, |h| h.amount),
            resource: hold.and_then(|h| match h.cargo {
                Cargo::Resource(resource) => resource.stock_slot(),
                Cargo::Goods { .. } => None,
            }),
        })
    }
}

#[cfg(test)]
#[path = "trade_tests.rs"]
mod tests;
