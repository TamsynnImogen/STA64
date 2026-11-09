//! Marks: what happened, by tick, derived from the sim's own events.
//!
//! A mark is stamped with the tick of the step that produced it (the tick the
//! step started at), the same numbering the SIM packages' in-crate tests use:
//! a command given before tick 0 fires at tick 0, and the state that shows its
//! effect is the view at tick 1.
//!
//! Kinds and their fields (`obj` etc. name an object by scene name or `#id`):
//!
//! | kind | from | fields |
//! |---|---|---|
//! | `fire.start` | first `Fired` of a burst | `obj` shooter, `weapon`, `hp` |
//! | `fire.end` | last `Fired` of a burst + the weapon's duration | `obj`, `weapon`, `hp` |
//! | `hit` | `Hit` | `obj` target, `by`, `weapon`, `facing` |
//! | `shields.down` | `ShieldsDown` | `obj`, `facing` |
//! | `spawn` | `Spawned` | `obj` |
//! | `death` | `Destroyed` | `obj`, `by` (when known) |
//! | `build.start` | `BuildStarted` | `obj` builder, `what` |
//! | `built` | `Built` | `obj` builder, `id` |
//! | `delivered` | `Delivered` | `player`, `resource`, `amount` |
//! | `cloak` | `CloakChanged` | `obj`, `on` (1/0) |
//! | `rejected` | `Rejected` | `player`, `seq` |
//! | `eliminated` | `PlayerEliminated` | `player` |
//!
//! A burst is the run of `Fired` of one shooter and hardpoint in which each
//! shot comes no later than `FireGap` seconds after the previous one ended.
//! A burst that may still continue is not closed, so its `fire.end` does not
//! exist yet.

use std::collections::BTreeMap;

use sta_core::{EntityId, Tick};
use sta_sim::SimEvent;

use sta_content::ContentDb;
use sta_core::Fixed;

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Obj(EntityId),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub tick: Tick,
    pub kind: &'static str,
    pub fields: Vec<(&'static str, Val)>,
}

fn obj(id: EntityId) -> Val {
    Val::Obj(id)
}
fn text(s: impl ToString) -> Val {
    Val::Text(s.to_string())
}

type Burst = (Tick, Tick, String, Tick);

fn close(key: (u32, u8), burst: Burst, ended: &mut Vec<Mark>) {
    ended.push(Mark {
        tick: burst.1 + burst.3,
        kind: "fire.end",
        fields: vec![
            ("obj", obj(EntityId(key.0))),
            ("weapon", text(burst.2)),
            ("hp", text(key.1)),
        ],
    });
}

fn to_f64(f: Fixed) -> f64 {
    f.to_bits() as f64 / 4_294_967_296.0
}

/// Every mark up to `now`, ascending tick (stable), from the sim's events.
/// `fire_gap` is the scene's `[Marks] FireGap` in ticks.
pub fn derive_from(
    events: &[(Tick, SimEvent)],
    db: &ContentDb,
    fire_gap: Tick,
    now: Tick,
) -> Vec<Mark> {
    let wname = |k| db.name(k).unwrap_or("?").to_string();
    let mut out: Vec<Mark> = Vec::new();
    // (shooter, hardpoint) -> (start tick, last fired tick, weapon, duration)
    let mut bursts: BTreeMap<(u32, u8), Burst> = BTreeMap::new();
    let mut ended: Vec<Mark> = Vec::new();
    for (tick, e) in events {
        let tick = *tick;
        // A shot too late to continue its burst closes it first.
        if let SimEvent::Fired {
            shooter, hardpoint, ..
        } = e
        {
            let key = (shooter.0, *hardpoint);
            if bursts
                .get(&key)
                .is_some_and(|cur| tick > cur.1 + cur.3 + fire_gap)
            {
                let cur = bursts.remove(&key).expect("present");
                close(key, cur, &mut ended);
            }
        }
        match e {
            SimEvent::Fired {
                shooter,
                weapon,
                hardpoint,
                ..
            } => {
                let key = (shooter.0, *hardpoint);
                let dur = db.weapon(*weapon).map_or(0, |w| w.duration);
                match bursts.get_mut(&key) {
                    Some(cur) => {
                        cur.1 = tick;
                        cur.3 = dur;
                    }
                    None => {
                        bursts.insert(key, (tick, tick, wname(*weapon), dur));
                        out.push(Mark {
                            tick,
                            kind: "fire.start",
                            fields: vec![
                                ("obj", obj(*shooter)),
                                ("weapon", text(wname(*weapon))),
                                ("hp", text(hardpoint)),
                            ],
                        });
                    }
                }
            }
            SimEvent::Hit {
                target,
                by,
                weapon,
                facing,
                ..
            } => out.push(Mark {
                tick,
                kind: "hit",
                fields: vec![
                    ("obj", obj(*target)),
                    ("by", obj(*by)),
                    ("weapon", text(wname(*weapon))),
                    ("facing", text(facing)),
                ],
            }),
            SimEvent::ShieldsDown { id, facing } => out.push(Mark {
                tick,
                kind: "shields.down",
                fields: vec![("obj", obj(*id)), ("facing", text(facing))],
            }),
            SimEvent::Spawned { id } => out.push(Mark {
                tick,
                kind: "spawn",
                fields: vec![("obj", obj(*id))],
            }),
            SimEvent::Destroyed { id, by } => {
                let mut fields = vec![("obj", obj(*id))];
                if let Some(by) = by {
                    fields.push(("by", obj(*by)));
                }
                out.push(Mark {
                    tick,
                    kind: "death",
                    fields,
                });
            }
            SimEvent::BuildStarted { builder, what } => out.push(Mark {
                tick,
                kind: "build.start",
                fields: vec![("obj", obj(*builder)), ("what", text(wname(*what)))],
            }),
            SimEvent::Built { builder, id } => out.push(Mark {
                tick,
                kind: "built",
                fields: vec![("obj", obj(*builder)), ("id", obj(*id))],
            }),
            SimEvent::Delivered {
                player,
                resource,
                amount,
            } => out.push(Mark {
                tick,
                kind: "delivered",
                fields: vec![
                    ("player", text(player.0)),
                    ("resource", text(resource)),
                    ("amount", text(to_f64(*amount))),
                ],
            }),
            SimEvent::CloakChanged { id, on } => out.push(Mark {
                tick,
                kind: "cloak",
                fields: vec![("obj", obj(*id)), ("on", text(u8::from(*on)))],
            }),
            SimEvent::Rejected { player, seq } => out.push(Mark {
                tick,
                kind: "rejected",
                fields: vec![("player", text(player.0)), ("seq", text(seq))],
            }),
            SimEvent::PlayerEliminated { player } => out.push(Mark {
                tick,
                kind: "eliminated",
                fields: vec![("player", text(player.0))],
            }),
            _ => {}
        }
    }
    // Bursts the clock has closed by now.
    for (key, cur) in bursts {
        if now > cur.1 + cur.3 + fire_gap {
            close(key, cur, &mut ended);
        }
    }
    out.extend(ended.into_iter().filter(|m| m.tick <= now));
    out.sort_by_key(|m| m.tick);
    out
}

/// The marks file: one `TICK KIND k=v ...` line per mark; `name` turns an
/// object id into its scene name.
pub fn render_with(marks: &[Mark], name: impl Fn(EntityId) -> String) -> String {
    let mut out = String::new();
    for m in marks {
        out += &format!("{:>6} {}", m.tick, m.kind);
        for (k, v) in &m.fields {
            match v {
                Val::Obj(id) => out += &format!(" {k}={}", name(*id)),
                Val::Text(t) => out += &format!(" {k}={t}"),
            }
        }
        out.push('\n');
    }
    out
}

/// Kinds [`derive_from`] can emit (all in Refit's vocabulary; see
/// `tests/fixtures/refit_marks.txt`).
pub const KINDS: &[&str] = &[
    "fire.start", "fire.end", "hit", "shields.down", "spawn", "death", "build.start",
    "built", "delivered", "cloak", "rejected", "eliminated",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_pads_tick_and_names_objects() {
        let m = vec![
            Mark { tick: 7, kind: "hit", fields: vec![("obj", Val::Obj(EntityId(3))), ("facing", Val::Text("fore".into()))] },
            Mark { tick: 1234, kind: "spawn", fields: vec![("obj", Val::Obj(EntityId(9)))] },
        ];
        let s = render_with(&m, |id| if id.0 == 3 { "target".into() } else { format!("#{}", id.0) });
        assert_eq!(s, "     7 hit obj=target facing=fore\n  1234 spawn obj=#9\n");
    }

    #[test]
    fn no_events_no_marks() {
        let db = sta_content::fake::db();
        assert!(derive_from(&[], &db, 0, 10).is_empty());
        assert_eq!(render_with(&[], |_| String::new()), "");
    }
}
