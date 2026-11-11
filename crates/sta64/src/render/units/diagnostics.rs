//! Opt-in visual diagnostics; reads the scene without changing it.
use super::*;

#[derive(Default)]
pub(super) struct Trace {
    model: Option<Option<String>>,
    ages: BTreeMap<Entity, u32>,
}

/// At most three snapshots per root, tracking up to 16 live matching roots. Run in Last
/// so the transforms and visibility reflect the completed render preparation.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn trace(
    mut state: Local<Trace>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    roots: Query<(Entity, &SimEntity, Option<&NeedsModel>, &Visibility, &GlobalTransform)>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    nodes: Query<(
        Option<&Name>,
        Option<&Visibility>,
        Option<&InheritedVisibility>,
        Option<&ViewVisibility>,
        Option<&GlobalTransform>,
    )>,
    parts: Query<(
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&crate::render::hulls::HullSource>,
        Option<&crate::render::hulls::HullPart>,
    )>,
    materials: Option<Res<Assets<StandardMaterial>>>,
) {
    let wanted = state
        .model
        .get_or_insert_with(|| std::env::var("STA64_TRACE_MODEL").ok().map(|s| s.to_ascii_lowercase()))
        .clone();
    let Some(wanted) = wanted.filter(|s| !s.is_empty()) else {
        return;
    };
    let (Some(link), Some(content), Some(materials)) = (link, content, materials) else {
        state.ages.clear();
        return;
    };
    // Bevy entity generations distinguish replacement roots and new matches even
    // when their simulation IDs repeat. Retired roots do not exhaust the bound.
    state.ages.retain(|root, _| roots.get(*root).is_ok());
    for (root, id, pending, visibility, transform) in &roots {
        let Some(unit) = link.curr().unit(id.0) else { continue };
        let Some(def) = content.0.unit(unit.def) else { continue };
        if !def.model.eq_ignore_ascii_case(&wanted) && !def.odf.eq_ignore_ascii_case(&wanted) {
            continue;
        }
        if !state.ages.contains_key(&root) && state.ages.len() >= 16 {
            continue;
        }
        let age = state.ages.entry(root).or_default();
        let report = matches!(*age, 0 | 2 | 60);
        *age = age.saturating_add(1);
        if !report {
            continue;
        }
        info!(
            "unit-visual id={} tick={} odf={} model={} pending={} progress={:?} flags={:?} position={:?} heading={:?} root_visibility={visibility:?} root_transform={:?}",
            id.0.0,
            link.curr().tick,
            def.odf,
            def.model,
            pending.is_some(),
            unit.progress,
            unit.flags,
            unit.pos.to_f32(),
            unit.heading,
            transform.affine()
        );
        let mut count = 0;
        for child in children.iter_descendants(root) {
            let Ok((standard, hull, hull_part)) = parts.get(child) else {
                continue;
            };
            let handle = standard.map(|s| &s.0).or_else(|| hull.map(|h| &h.source));
            let Some(handle) = handle else { continue };
            count += 1;
            let mat = materials.get(handle);
            let mut chain = Vec::new();
            let mut at = Some(child);
            // Parent paths are bounded even if malformed externally supplied data
            // reaches the scene. Normal model loading rejects cycles already.
            for _ in 0..128 {
                let Some(entity) = at else { break };
                if let Ok((name, vis, inherited, view, tf)) = nodes.get(entity) {
                    chain.push(format!(
                        "{} vis={vis:?} inherited={inherited:?} view={view:?} world={:?}",
                        name.map_or("<part>", |n| n.as_str()),
                        tf.map(|t| t.affine())
                    ));
                }
                if entity == root {
                    break;
                }
                at = parents.get(entity).ok().map(ChildOf::parent);
            }
            info!(
                "unit-visual id={} part={child:?} hull_shader={} material_loaded={} alpha={:?} hull_part={hull_part:?} texture={} scene={chain:?}",
                id.0.0,
                hull.is_some(),
                mat.is_some(),
                mat.map(|m| m.alpha_mode),
                mat.is_some_and(|m| m.base_color_texture.is_some())
            );
        }
        info!("unit-visual id={} material_parts={count}", id.0.0);
    }
}
