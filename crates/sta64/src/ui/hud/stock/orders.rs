//! Immediate orders from the stock command palette.

use sta_sim::command::{AlertState, AutonomyKind, AutonomyLevel, CommandKind, Order, TradeAction, TradeResource};
use sta_sim::world::EntityKind;
use sta_sim::{EntityId, Fixed, UnitView, view::UnitFlags};

use super::commands::NativeCommand;
use super::model::{HudFeed, finished};

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Scout,
    SearchAndDestroy,
    Alert(AlertState),
    Autonomy(AutonomyKind, AutonomyLevel),
    Load(TradeResource),
    Buy(TradeResource),
    Sell(TradeResource),
}

impl Kind {
    pub(super) fn read(record: &NativeCommand) -> Option<Self> {
        use {AlertState as A, AutonomyKind as K, AutonomyLevel as L};
        Some(match record.command.to_ascii_uppercase().as_str() {
            "SCOUT" => Self::Scout,
            "SEARCH_AND_DESTROY" => Self::SearchAndDestroy,
            "GREEN_ALERT" => Self::Alert(A::Green),
            "YELLOW_ALERT" => Self::Alert(A::Yellow),
            "RED_ALERT" => Self::Alert(A::Red),
            "MOVE_LOW" => Self::Autonomy(K::Movement, L::Low),
            "MOVE_MED" => Self::Autonomy(K::Movement, L::Medium),
            "MOVE_HIGH" => Self::Autonomy(K::Movement, L::High),
            "SPECIAL_WEAPON_AUTONOMY_NONE" => Self::Autonomy(K::SpecialWeapon, L::Low),
            "SPECIAL_WEAPON_AUTONOMY_MED" => Self::Autonomy(K::SpecialWeapon, L::Medium),
            "SPECIAL_WEAPON_AUTONOMY_HIGH" => Self::Autonomy(K::SpecialWeapon, L::High),
            "GIVE_RESOURCE" => Self::Load(TradeResource::from_native(record.param?)?),
            "BUY_RESOURCE" => Self::Buy(TradeResource::from_native(record.param?)?),
            "SELL_RESOURCE" => Self::Sell(TradeResource::from_native(record.param?)?),
            _ => return None,
        })
    }

    pub(super) fn eligible(self, feed: &HudFeed, record: &NativeCommand, u: &UnitView) -> bool {
        if u.owner != feed.player || !finished(u) || u.hull[0] <= Fixed::ZERO {
            return false;
        }
        let Some(def) = feed.def(u) else { return false };
        let kind = EntityKind::of_def(def);
        let mobile = kind == EntityKind::Craft && def.movement.max_speed > Fixed::ZERO
            && !u.flags.contains(UnitFlags::ENGINES_OFF)
            && !u.flags.contains(UnitFlags::BUILDING);
        let can_order = match self {
            Self::Scout | Self::SearchAndDestroy => mobile,
            Self::Alert(_) | Self::Autonomy(..) => matches!(kind, EntityKind::Craft | EntityKind::Station),
            Self::Load(_) => feed.cargo_ship(u),
            Self::Buy(_) | Self::Sell(_) => feed.can_trade(u) && def.trade_station.is_some(),
        };
        can_order && record.source.iter().all(|key| match key.as_str() {
            "ship" => kind == EntityKind::Craft,
            "facility" => kind == EntityKind::Station,
            "mobile" => mobile,
            "cargo_ship" => feed.cargo_ship(u),
            "trade_station" => feed.can_trade(u) && def.trade_station.is_some(),
            // Tara's command requirements are drawn from the resolved unit ODF.
            _ => def.raw.bool(key) == Some(true),
        })
    }

    pub(super) fn command(self, feed: &HudFeed, record: &NativeCommand) -> Option<(CommandKind, Vec<EntityId>)> {
        let units: Vec<_> = feed.selected_units().into_iter()
            .filter(|u| self.eligible(feed, record, u))
            .map(|u| u.id)
            .collect();
        if units.is_empty() {
            return None;
        }
        let ids = units.clone();
        let order = match self {
            Self::Scout => Order::Scout { units: ids },
            Self::SearchAndDestroy => Order::SearchAndDestroy { units: ids },
            Self::Alert(level) => Order::Alert { units: ids, level },
            Self::Autonomy(kind, level) => Order::Autonomy { units: ids, kind, level },
            Self::Load(resource) => Order::Trade { units: ids, action: TradeAction::Load(resource) },
            Self::Buy(resource) => Order::Trade { units: ids, action: TradeAction::Buy(resource) },
            Self::Sell(resource) => Order::Trade { units: ids, action: TradeAction::Sell(resource) },
        };
        Some((CommandKind::Order(order), units))
    }

    pub(super) fn active(self, feed: &HudFeed, record: &NativeCommand) -> bool {
        let mut units = feed.selected_units().into_iter().filter(|u| self.eligible(feed, record, u)).peekable();
        units.peek().is_some() && units.all(|u| match self {
            Self::Alert(level) => {
                let rows = &feed.view.attrs.alerts;
                let current = rows.binary_search_by_key(&u.id, |row| row.0)
                    .map_or(AlertState::Red, |index| rows[index].1);
                current == level
            }
            Self::Autonomy(kind, level) => {
                let slot = usize::from(kind == AutonomyKind::SpecialWeapon);
                u.autonomy.is_some_and(|levels| levels[slot] == level as u8)
            }
            Self::Scout | Self::SearchAndDestroy | Self::Load(_) | Self::Buy(_) | Self::Sell(_) => false,
        })
    }
}
