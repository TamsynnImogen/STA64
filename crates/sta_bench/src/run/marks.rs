//! Marks for the in-process runner: the derivation and file format live in
//! [`crate::marks`] (shared with `sta64 --headless`); this adds the scenario
//! checks that need a [`Bench`].

use std::collections::{BTreeMap, VecDeque};

use sta_core::{ContentKey, EntityId, TICK_HZ, Tick, Vec3};
use sta_sim::SimEvent;
use sta_sim::command::Target;

use super::bench::Bench;
use super::to_f64;
pub use crate::marks::{Mark, Val, ordnance_label};

/// Every mark up to the bench's current tick, ascending tick (stable): the
/// event marks, then the measuring marks the scene's `[Marks]` asks for
/// (`Ordnance=1`, `State=<ms>`), the commands' (`bank`), and with `SimTime=1`
/// a `t=` (sim seconds) first on every mark.
pub fn derive(b: &Bench) -> Vec<Mark> {
    let mut out = crate::marks::derive_from(&b.events, &b.cfg.content, b.fire_gap, b.tick());
    let cfg = &b.marks_cfg;
    if cfg.ordnance {
        out.extend(ordnance(b));
    }
    if let Some(ms) = cfg.state_ms {
        let every = ((f64::from(ms) * f64::from(TICK_HZ) / 1000.0).round() as Tick).max(1);
        out.extend(states(b, every));
    }
    out.extend(b.command_marks.iter().cloned());
    out.sort_by_key(|m| m.tick);
    if cfg.sim_time {
        for m in &mut out {
            m.fields.insert(0, ("t", Val::Text(format!("{:.3}", m.tick as f64 / f64::from(TICK_HZ)))));
        }
    }
    out
}

fn xyz(p: Vec3) -> String {
    format!("{:.1},{:.1},{:.1}", to_f64(p.x), to_f64(p.y), to_f64(p.z))
}

fn pos_at(b: &Bench, tick: Tick, id: EntityId) -> Option<Vec3> {
    b.view_at(tick).ok()?.units.iter().find(|u| u.id == id).map(|u| u.pos)
}

/// `ord.new` per shot and `ord.gone` when a shot's hit lands (Refit's
/// `Ordnance=1`). A hit consumes the oldest open shot of that shooter and
/// weapon; a shot that never hits has no `ord.gone` (the sim has no event for
/// ordnance that runs out), and later hits of a beam's one shot none either.
fn ordnance(b: &Bench) -> Vec<Mark> {
    let db = &b.cfg.content;
    let skip = &b.marks_cfg.ordnance_skip;
    let mut open: BTreeMap<(u32, ContentKey), VecDeque<(u64, Tick)>> = BTreeMap::new();
    let mut out = Vec::new();
    let mut n = 0u64;
    for (tick, e) in &b.events {
        match e {
            SimEvent::Fired { shooter, weapon, target, .. } => {
                let odf = db.name(*weapon).unwrap_or("?").to_ascii_lowercase();
                if skip.iter().any(|p| odf.starts_with(p.as_str())) {
                    continue;
                }
                n += 1;
                open.entry((shooter.0, *weapon)).or_default().push_back((n, *tick));
                let tgt = match target {
                    Target::Entity(id) => Val::Obj(*id),
                    Target::Point(p) => Val::Text(xyz(*p)),
                    _ => Val::Text("none".into()),
                };
                let at = pos_at(b, *tick, *shooter).map_or_else(|| "?".into(), xyz);
                out.push(Mark {
                    tick: *tick,
                    kind: "ord.new",
                    fields: vec![
                        ("id", Val::Text(n.to_string())),
                        ("obj", Val::Obj(*shooter)),
                        ("weapon", Val::Text(ordnance_label(db, *weapon))),
                        ("target", tgt),
                        ("at", Val::Text(at)),
                    ],
                });
            }
            SimEvent::Hit { target, by, weapon, at, .. } => {
                let Some((id, fired)) = open.get_mut(&(by.0, *weapon)).and_then(VecDeque::pop_front) else { continue };
                let tgt = pos_at(b, *tick, *target);
                let d = tgt.map_or_else(
                    || "?".into(),
                    |t| {
                        let (dx, dy, dz) = (to_f64(at.x) - to_f64(t.x), to_f64(at.y) - to_f64(t.y), to_f64(at.z) - to_f64(t.z));
                        format!("{:.1}", (dx * dx + dy * dy + dz * dz).sqrt())
                    },
                );
                out.push(Mark {
                    tick: *tick,
                    kind: "ord.gone",
                    fields: vec![
                        ("id", Val::Text(id.to_string())),
                        ("obj", Val::Obj(*by)),
                        ("weapon", Val::Text(ordnance_label(db, *weapon))),
                        ("at", Val::Text(xyz(*at))),
                        ("life", Val::Text(format!("{:.3}", (tick - fired) as f64 / f64::from(TICK_HZ)))),
                        ("tgt_at", Val::Text(tgt.map_or_else(|| "?".into(), xyz))),
                        ("d", Val::Text(d)),
                    ],
                });
            }
            _ => {}
        }
    }
    out
}

/// `state` every `every` ticks for each of the scene's named objects (Refit's
/// `State=<ms>`): `pos`, `hull`, `sh` (the fore facing; one pool in A2),
/// and `sp` when the unit has special energy. `q`, `ma`, `swa` are not written:
/// no queue length in the view, no autonomy in the sim.
fn states(b: &Bench, every: Tick) -> Vec<Mark> {
    let mut out = Vec::new();
    let mut tick = 0;
    while tick <= b.tick() {
        if let Ok(v) = b.view_at(tick) {
            for u in &v.units {
                if b.name_of(u.id).starts_with('#') {
                    continue;
                }
                let mut fields = vec![
                    ("obj", Val::Obj(u.id)),
                    ("pos", Val::Text(xyz(u.pos))),
                    ("hull", Val::Text(format!("{:.1}", to_f64(u.hull[0])))),
                    ("sh", Val::Text(format!("{:.1}", to_f64(u.shields[0][0])))),
                ];
                if let Some(e) = u.energy {
                    fields.push(("sp", Val::Text(format!("{:.1}", to_f64(e[0])))));
                }
                out.push(Mark { tick, kind: "state", fields });
            }
        }
        tick += every;
    }
    out
}

pub fn render(b: &Bench, marks: &[Mark]) -> String {
    crate::marks::render_with(marks, |id: EntityId| b.name_of(id))
}

/// A mark spec: `kind k=v k=v`.
#[derive(Debug, Clone)]
pub struct Spec {
    pub kind: String,
    pub fields: Vec<(String, String)>,
}

pub fn parse_spec(s: &str) -> Result<Spec, String> {
    let mut w = s.split_whitespace();
    let kind = w.next().ok_or("empty mark")?.to_owned();
    let mut fields = Vec::new();
    for f in w {
        let (k, v) = f
            .split_once('=')
            .ok_or_else(|| format!("`{f}`: expected key=value"))?;
        fields.push((k.to_owned(), v.to_owned()));
    }
    Ok(Spec { kind, fields })
}

impl Spec {
    pub fn matches(&self, m: &Mark, b: &Bench) -> bool {
        if self.kind != m.kind {
            return false;
        }
        self.fields.iter().all(|(k, want)| {
            m.fields.iter().any(|(mk, v)| {
                mk == k
                    && match v {
                        Val::Obj(id) => b.resolve(want).is_ok_and(|w| w == *id),
                        Val::Text(t) => t.eq_ignore_ascii_case(want),
                    }
            })
        })
    }
}
