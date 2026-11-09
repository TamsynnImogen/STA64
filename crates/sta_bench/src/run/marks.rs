//! Marks for the in-process runner: the derivation and file format live in
//! [`crate::marks`] (shared with `sta64 --headless`); this adds the scenario
//! checks that need a [`Bench`].

use sta_core::EntityId;

use super::bench::Bench;
pub use crate::marks::{Mark, Val};

/// Every mark up to the bench's current tick, ascending tick (stable).
pub fn derive(b: &Bench) -> Vec<Mark> {
    crate::marks::derive_from(&b.events, &b.cfg.content, b.fire_gap, b.tick())
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
