//! Live tooltip text for a particular selected-unit readout.

use bevy::prelude::*;
use sta_sim::{EntityId, Fixed};

use super::{layout::R, model::HudFeed, status::StatusFeed, stock::tooltip_text::Lookup};
use crate::ui::notices::Tooltip;

#[derive(Clone, Copy)]
pub(super) enum Readout {
    Hull,
    Shields,
    Crew,
    Energy,
    Construction,
    Deposit,
    System(usize),
}

#[derive(Component, Clone, Copy)]
pub(super) struct ReadoutTarget(pub EntityId, pub Readout);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReadoutText {
    label: (&'static str, &'static str),
    value: Value,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Pool {
        percent: Option<i128>,
        current: i128,
        capacity: i128,
    },
    Progress(i128),
    Infinite,
    System {
        health: i128,
        enabled: bool,
    },
}

fn localize(look: Lookup, (key, fallback): (&str, &str)) -> String {
    let value = look(key);
    if value == key { fallback.into() } else { value }
}

impl ReadoutText {
    pub(in crate::ui) fn text(&self, verbose: bool, look: Lookup) -> String {
        let label = localize(look, self.label);
        if label.is_empty() {
            return label;
        }
        let percent = |value| {
            format!(
                "{value}{}",
                localize(look, ("GUI_CP_PERCENT", "%")).replace("%%", "%")
            )
        };
        match self.value {
            Value::Pool {
                percent: share,
                current,
                capacity,
            } => {
                let value = share.map_or_else(|| current.to_string(), percent);
                if !verbose {
                    return format!("{label} {value}");
                }
                let open = localize(look, ("GUI_CP_START_EXTRA", "("));
                let slash = localize(look, ("GUI_CP_AMOUNT_SEPARATE", "/"));
                let close = localize(look, ("GUI_CP_END_EXTRA", ")"));
                let details = format!("{open}{current}{slash}{capacity}{close}");
                if share.is_some() {
                    format!("{label} {value} {details}")
                } else {
                    format!("{label} {details}")
                }
            }
            Value::Progress(share) => format!("{label} {}", percent(share)),
            Value::Infinite => format!("{label}: {}", localize(look, ("Unlimited", "Unlimited"))),
            Value::System { health, enabled } if verbose => {
                let detail = if !enabled {
                    localize(look, ("GUI_SD_REPAIR_DIS_SYS", "disabled."))
                } else if health == 100 {
                    localize(look, ("GUI_SD_REPAIR_FUNC_SYS", "fully functional."))
                } else {
                    percent(health)
                };
                format!("{label}: {detail}")
            }
            Value::System { .. } => label,
        }
    }
}

const SCALE: i128 = Fixed::ONE.to_bits() as i128;

fn bits(pair: [Fixed; 2]) -> [i128; 2] {
    pair.map(|value| i128::from(value.to_bits()))
}

fn percent(now: i128, max: i128) -> i128 {
    (now.clamp(0, max) * 100 + max / 2) / max
}

fn pool(pair: [i128; 2], shows_percent: bool) -> Option<Value> {
    let [now, max] = pair;
    if shows_percent && max <= 0 {
        return None;
    }
    Some(Value::Pool {
        percent: shows_percent.then(|| percent(now, max)),
        current: now / SCALE,
        capacity: max / SCALE,
    })
}

fn readout(
    feed: &HudFeed,
    status: Option<&StatusFeed>,
    target: ReadoutTarget,
) -> Option<ReadoutText> {
    let ReadoutTarget(id, kind) = target;
    let unit = feed.view.unit(id)?;
    // HealthBar and EnergyBar use a percentage followed by their pool in the verbose text
    // (Armada II executable, static). The shield pool uses the same display format in STA64.
    let (label, value) = match kind {
        Readout::Hull => (
            ("GUI_SD_BAR_HEALTH", "Hull integrity at"),
            pool(bits(unit.hull), true)?,
        ),
        Readout::Shields => {
            let sum = unit
                .shields
                .iter()
                .copied()
                .map(bits)
                .fold([0, 0], |sum, next| [sum[0] + next[0], sum[1] + next[1]]);
            (("Shields at", "Shields at"), pool(sum, true)?)
        }
        Readout::Energy => (
            ("GUI_SD_BAR_ENERGY", "Special energy at"),
            pool(bits(unit.energy?), true)?,
        ),
        Readout::Crew => (("GUI_SD_TEXT_CREW", "Crew"), pool(bits(unit.crew?), false)?),
        Readout::Construction => (
            ("GUI_SD_BAR_CONST", "Construction"),
            Value::Progress(percent(i128::from(unit.progress?.to_bits()), SCALE)),
        ),
        Readout::Deposit => {
            let row = super::status::node_status(status, id)?;
            let label = match row.resource {
                0 => ("GUI_CP_DILITHIUM_RES", "Dilithium"),
                1 => ("GUI_CP_LATINUM_RES", "Latinum"),
                2 => ("GUI_CP_METAL_RES", "Metal"),
                3 => ("GUI_CP_BIOMATTER_RES", "Biomatter"),
                _ => ("Resources remaining", "Resources remaining"),
            };
            let value = if row.infinite {
                Value::Infinite
            } else {
                pool(bits([row.amount, row.start]), false)?
            };
            (label, value)
        }
        Readout::System(system) => {
            let labels = [
                ("GUI_SD_REPAIR_SYS_ENG", "Engines"),
                ("GUI_SD_REPAIR_SYS_WPN", "Weapon control"),
                ("GUI_SD_REPAIR_SYS_SHLD", "Shield generator"),
                ("GUI_SD_REPAIR_SYS_LS", "Life support"),
                ("GUI_SD_REPAIR_SYS_SENS", "Sensors"),
            ];
            let state =
                status.and_then(|status| status.systems.iter().find(|row| row.entity == id));
            let health = state.map_or(Fixed::ONE, |row| row.health[system]);
            let enabled = state.is_none_or(|row| row.on[system]);
            (
                labels[system],
                Value::System {
                    health: percent(i128::from(health.to_bits()), SCALE),
                    enabled,
                },
            )
        }
    };
    Some(ReadoutText { label, value })
}

pub(super) fn bind(id: EntityId, kind: Readout) -> (ReadoutTarget, Tooltip) {
    (ReadoutTarget(id, kind), Tooltip::new(""))
}

/// The whole authored bar is hoverable, including its empty part at zero health.
pub(super) fn at(commands: &mut Commands, root: Entity, rect: R, id: EntityId, kind: Readout) {
    commands.entity(root).with_child((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(rect.x),
            top: Val::Px(rect.y),
            width: Val::Px(rect.w),
            height: Val::Px(rect.h),
            ..default()
        },
        bind(id, kind),
    ));
}

pub(super) fn update(
    feed: Res<HudFeed>,
    status: Option<Res<StatusFeed>>,
    mut parts: Query<(&ReadoutTarget, &mut Tooltip)>,
) {
    use super::stock::tooltip_text::StockTip;
    for (&target, mut tip) in &mut parts {
        let next = readout(&feed, status.as_deref(), target).map(StockTip::Readout);
        if tip.stock != next {
            tip.stock = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::{PlayerId, WorldSource, combat::damage::SystemView, fake::ScriptedSource};
    use std::sync::Arc;

    #[test]
    fn each_meter_and_system_uses_its_own_live_value_and_label() {
        let db = sta_content::fake::db();
        let mut source = ScriptedSource::from_json5(
            "{units:[{id:1,def:'fx_cruiser',owner:0,rails:[{at:0,pos:{x:0,y:0,z:0}}]}]}",
            &db,
        )
        .unwrap();
        let mut view = source.step(&[]).view;
        let unit = &mut Arc::make_mut(&mut view).units[0];
        unit.hull = [Fixed::from_int(80), Fixed::from_int(100)];
        unit.shields = [[Fixed::from_int(5), Fixed::from_int(20)]; 4];
        let id = unit.id;
        let mut status = StatusFeed {
            systems: vec![SystemView {
                entity: id,
                health: [Fixed::ONE; 5],
                on: [true; 5],
            }],
            ..default()
        };
        status.systems[0].health[4] = Fixed::from_f64(0.35);
        status.systems[0].on[1] = false;
        let feed = HudFeed {
            view,
            player: PlayerId(0),
            ..default()
        };
        let look = |key: &str| match key {
            "GUI_SD_REPAIR_SYS_SENS" => "Localized sensors".into(),
            "GUI_CP_PERCENT" => "%%".into(),
            key => key.into(),
        };
        let text = |kind, verbose| {
            readout(&feed, Some(&status), ReadoutTarget(id, kind))
                .unwrap()
                .text(verbose, &look)
        };
        assert_eq!(text(Readout::Hull, false), "Hull integrity at 80%");
        assert_eq!(text(Readout::Hull, true), "Hull integrity at 80% (80/100)");
        assert_eq!(text(Readout::Shields, true), "Shields at 25% (20/80)");
        assert_eq!(text(Readout::System(4), false), "Localized sensors");
        assert_eq!(text(Readout::System(4), true), "Localized sensors: 35%");
        assert_eq!(text(Readout::System(1), true), "Weapon control: disabled.");
    }
}
