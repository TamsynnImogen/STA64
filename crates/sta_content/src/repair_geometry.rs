//! Authored repair node geometry from the simulation SOD (SIM-40, D9/D10).
//! This is the raw composed model transform, before external `ScaleSOD` or
//! entity placement. No docking-frame policy or missing-node fallback is inferred.

use std::collections::BTreeMap;

use sta_core::Vec3;

use crate::Warning;
use crate::def::UnitDef;
use crate::launch::{ModelSod, read_sod};

/// Full 3D transform of an authored `repairHardpoint` in SOD model space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepairAttachment {
    pub origin: Vec3,
    pub axes: [Vec3; 3],
}

fn representable(v: f32) -> bool {
    let v = f64::from(v);
    v.is_finite() && (-2147483648.0..2147483648.0).contains(&v)
}

/// Recompute attachments from readable simulation SODs, clearing stale values.
/// Missing files stay quiet; a readable SOD missing the authored node warns.
pub fn attach<'a>(
    vfs: &sta_vfs::Vfs,
    units: impl Iterator<Item = &'a mut UnitDef>,
) -> Vec<Warning> {
    let mut sods: BTreeMap<String, ModelSod> = BTreeMap::new();
    let mut warnings = Vec::new();
    for unit in units {
        unit.repair_attachment = None;
        let Some(node) = unit.repair.repair_hardpoint.as_deref() else {
            continue;
        };
        let key = unit
            .model
            .to_ascii_lowercase()
            .trim_end_matches(".sod")
            .to_owned();
        if key.is_empty() {
            continue;
        }
        let sod = sods
            .entry(key.clone())
            .or_insert_with(|| read_sod(vfs, &key));
        let ModelSod::Sod(sod) = sod else {
            continue;
        };
        let Some(index) = sod.find(node) else {
            warnings.push(Warning::new(
                format!("sod/{key}.sod"),
                format!(
                    "{}: repairHardpoint {node} is not in the readable SOD",
                    unit.odf
                ),
            ));
            continue;
        };
        let transform = sod.model_transform(index);
        if !transform.m.iter().copied().all(representable) {
            warnings.push(Warning::new(
                format!("sod/{key}.sod"),
                format!(
                    "{}: repairHardpoint {node} has a nonfinite or unrepresentable transform",
                    unit.odf
                ),
            ));
            continue;
        }
        unit.repair_attachment = Some(RepairAttachment {
            origin: Vec3::from_f32(transform.translation()),
            axes: [0, 1, 2].map(|axis| Vec3::from_f32(transform.axis(axis))),
        });
    }
    warnings
}
