//! Pure parsing and validation for per-instance hull appearance authoring keys.
//!
//! This module intentionally treats texture and node names as opaque data. It
//! never resolves, opens, or mutates an asset.

use sta_odf::Odf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubsystemKind {
    Engines,
    Weapons,
    Shields,
    LifeSupport,
    Sensors,
}

impl SubsystemKind {
    const ORDERED: [(Self, &'static str); 5] = [
        (Self::Engines, "engine"),
        (Self::Weapons, "weapon"),
        (Self::Shields, "shieldGenerator"),
        (Self::LifeSupport, "lifeSupport"),
        (Self::Sensors, "sensor"),
    ];
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubsystemNode {
    pub kind: SubsystemKind,
    /// The sparse, one-based ODF index.
    pub index: u8,
    /// The original node spelling from the ODF.
    pub node: String,
    /// An optional, same-index lowercase `explosion` value.
    pub explosion: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionPlacement {
    pub texture: String,
    pub hardpoint: String,
    /// `None` means the author omitted the key; explicit malformed vectors
    /// invalidate the placement instead of silently taking a default.
    pub offset: Option<[f32; 3]>,
    /// Kept in authored degrees for the renderer's transform adapter.
    pub rotation_degrees: Option<[f32; 3]>,
    pub size: [f32; 2],
    /// Explicit finite, positive model-space depth, when authored.
    pub depth: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogoPlacement {
    pub index: u8,
    pub projection: ProjectionPlacement,
    pub suffix: Option<String>,
    pub colour_key: Option<[u8; 3]>,
    pub flip_u: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DamageTarget {
    Hull,
    Sensors,
    Engines,
    Weapons,
    LifeSupport,
    ShieldGenerator,
}

impl DamageTarget {
    const ORDERED: [(Self, &'static str); 6] = [
        (Self::Hull, "hull"),
        (Self::Sensors, "sensors"),
        (Self::Engines, "engines"),
        (Self::Weapons, "weapons"),
        (Self::LifeSupport, "lifeSupport"),
        (Self::ShieldGenerator, "shieldGenerator"),
    ];
}

#[derive(Clone, Debug, PartialEq)]
pub struct DamagePlacement {
    pub target: DamageTarget,
    pub index: u8,
    pub projection: ProjectionPlacement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppearancePolicy {
    pub faction_suffix: Option<String>,
    pub subsystem_nodes: Vec<SubsystemNode>,
    pub logos: Vec<LogoPlacement>,
    pub damage: Vec<DamagePlacement>,
    /// Invalid explicit values produce `None`; absent values use documented 0.1.
    pub damage_threshold: Option<f32>,
    pub damage_preview: bool,
}

impl Default for AppearancePolicy {
    fn default() -> Self {
        Self {
            faction_suffix: None,
            subsystem_nodes: Vec::new(),
            logos: Vec::new(),
            damage: Vec::new(),
            damage_threshold: Some(0.1),
            damage_preview: false,
        }
    }
}

/// Parse presentation-only keys from a resolved ODF. Diagnostics identify
/// malformed authored keys; malformed entries are omitted from the policy.
pub fn parse_appearance_policy(odf: &Odf) -> (AppearancePolicy, Vec<String>) {
    let mut policy = AppearancePolicy::default();
    let mut diagnostics = Vec::new();

    if odf.raw("factionTextureSuffix").is_some() {
        match odf.str("factionTextureSuffix") {
            Some("") => {}
            Some(value) if valid_name(value, 32) => policy.faction_suffix = Some(value.to_owned()),
            _ => diagnostics.push(
                "factionTextureSuffix: expected at most 32 ASCII letters, digits, '_' or '-'"
                    .into(),
            ),
        }
    }

    for (kind, stem) in SubsystemKind::ORDERED {
        for index in 1..=64 {
            let key = format!("{stem}Mesh{index}");
            let Some(_raw) = odf.raw(&key) else { continue };
            let Some(node) = odf.str(&key).filter(|value| valid_name(value, 63)) else {
                diagnostics.push(format!("{key}: expected a non-empty node name of at most 63 ASCII letters, digits, '_' or '-'"));
                continue;
            };
            let explosion_key = format!("{key}explosion");
            let explosion = if odf.raw(&explosion_key).is_some() {
                match odf.str(&explosion_key) {
                    Some("") => None,
                    Some(value) if valid_name(value, 63) => Some(value.to_owned()),
                    _ => {
                        diagnostics.push(format!(
                            "{explosion_key}: invalid explosion reference; ignored"
                        ));
                        None
                    }
                }
            } else {
                None
            };
            policy.subsystem_nodes.push(SubsystemNode {
                kind,
                index,
                node: node.to_owned(),
                explosion,
            });
        }
    }

    if odf.raw("damageThreshold").is_some() {
        policy.damage_threshold = match odf.str("damageThreshold").and_then(parse_scalar) {
            Some(value) if value.is_finite() && value > 0.0 && value <= 1.0 => Some(value),
            _ => {
                diagnostics.push("damageThreshold: expected a finite number in (0, 1]".into());
                None
            }
        };
    }

    if odf.raw("damageDecalPreview").is_some() {
        match odf
            .str("damageDecalPreview")
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("1" | "true") => policy.damage_preview = true,
            Some("0" | "false") => policy.damage_preview = false,
            _ => diagnostics.push(
                "damageDecalPreview: expected 0, 1, true or false; preview remains disabled".into(),
            ),
        }
    }

    for (target, stem) in DamageTarget::ORDERED {
        for index in 1..=64 {
            let key = format!("{stem}Scorch{index}");
            if odf.raw(&key).is_some()
                && let Some(projection) = parse_projection(odf, &key, &key, &mut diagnostics)
            {
                policy.damage.push(DamagePlacement {
                    target,
                    index,
                    projection,
                });
            }
        }
    }

    for index in 1..=64 {
        let prefix = format!("logoDecal{index}");
        let hardpoint_key = format!("{prefix}Hardpoint");
        // Hardpoint presence is the explicit declaration of a logo placement.
        if odf.raw(&hardpoint_key).is_none() {
            continue;
        }
        let Some(projection) = parse_projection(odf, &prefix, "", &mut diagnostics) else {
            continue;
        };

        let suffix_key = format!("{prefix}Suffix");
        let suffix = if odf.raw(&suffix_key).is_some() {
            match odf.str(&suffix_key) {
                Some("") => None,
                Some(value) if valid_name(value, 32) => Some(value.to_owned()),
                _ => {
                    diagnostics.push(format!(
                        "{suffix_key}: invalid path-safe suffix; logo placement ignored"
                    ));
                    continue;
                }
            }
        } else {
            None
        };

        let colour_key_name = format!("{prefix}ColourKey");
        let colour_key = if odf.raw(&colour_key_name).is_some() {
            match parse_u8_triplet(odf.str(&colour_key_name).unwrap_or_default()) {
                Some(value) => Some(value),
                None => {
                    diagnostics.push(format!("{colour_key_name}: expected exactly three integers in 0..=255; logo placement ignored"));
                    continue;
                }
            }
        } else {
            None
        };

        let flip_key = format!("{prefix}FlipU");
        let flip_u = if odf.raw(&flip_key).is_some() {
            match odf
                .str(&flip_key)
                .map(str::trim)
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("1" | "true") => true,
                Some("0" | "false") => false,
                _ => {
                    diagnostics.push(format!(
                        "{flip_key}: expected 0, 1, true or false; logo placement ignored"
                    ));
                    continue;
                }
            }
        } else {
            false
        };
        policy.logos.push(LogoPlacement {
            index,
            projection,
            suffix,
            colour_key,
            flip_u,
        });
    }

    (policy, diagnostics)
}

/// Build the old `scorchTextureN` plus target-hardpoint entries. The caller
/// supplies only successfully loaded, nonblank `scorchTextureN` values, kept in
/// their original ODF index order; this function performs no VFS access.
pub fn legacy_damage(odf: &Odf, available_textures: &[String]) -> Vec<DamagePlacement> {
    let textures: Vec<&str> = available_textures
        .iter()
        .map(String::as_str)
        .filter(|texture| !texture.trim().is_empty())
        .collect();
    if textures.is_empty() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    for (target, stem) in DamageTarget::ORDERED {
        let has_explicit = (1..=64).any(|index| odf.raw(&format!("{stem}Scorch{index}")).is_some());
        if has_explicit {
            continue;
        }
        let hardpoints_key = format!("{stem}TargetHardpoints");
        let Some(raw) = odf.raw(&hardpoints_key) else {
            continue;
        };
        for (position, hardpoint) in parse_odf_string_list(raw).into_iter().take(64).enumerate() {
            let hardpoint = hardpoint.trim();
            if hardpoint.is_empty() {
                continue;
            }
            entries.push(DamagePlacement {
                target,
                index: (position + 1) as u8,
                projection: ProjectionPlacement {
                    texture: textures[position % textures.len()].to_owned(),
                    hardpoint: hardpoint.to_owned(),
                    offset: None,
                    rotation_degrees: None,
                    size: [6.0, 6.0],
                    depth: None,
                },
            });
        }
    }
    entries
}

/// Tokenize an ODF's quoted or whitespace-separated string list without
/// changing the original position of blank entries.
fn parse_odf_string_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut started = false;
    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                started = true;
            }
            ch if ch.is_whitespace() && !in_quote => {
                if started {
                    items.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(ch);
                started = true;
            }
        }
    }
    if started {
        items.push(current);
    }
    items
}

fn parse_projection(
    odf: &Odf,
    prefix: &str,
    texture_key: &str,
    diagnostics: &mut Vec<String>,
) -> Option<ProjectionPlacement> {
    let hardpoint_key = format!("{prefix}Hardpoint");
    let hardpoint = match odf
        .str(&hardpoint_key)
        .filter(|value| valid_name(value, 63))
    {
        Some(value) => value.to_owned(),
        None => {
            diagnostics.push(format!(
                "{hardpoint_key}: missing or invalid hardpoint; placement ignored"
            ));
            return None;
        }
    };
    let texture = if texture_key.is_empty() {
        // Logo texture selection is row-aligned later; the ODF placement does
        // not carry a texture value.
        String::new()
    } else {
        match odf
            .str(texture_key)
            .filter(|value| !value.trim().is_empty())
        {
            Some(value) => value.to_owned(),
            None => {
                diagnostics.push(format!(
                    "{texture_key}: missing or empty texture reference; placement ignored"
                ));
                return None;
            }
        }
    };

    let offset = optional_vector(odf, &format!("{prefix}Offset"), 3, diagnostics)?
        .map(|v| [v[0], v[1], v[2]]);
    let rotation_degrees = optional_vector(odf, &format!("{prefix}Rotation"), 3, diagnostics)?
        .map(|v| [v[0], v[1], v[2]]);
    let size_key = format!("{prefix}Size");
    let size = if odf.raw(&size_key).is_none() {
        [6.0, 6.0]
    } else {
        let values = vector_value(odf, &size_key, 2, diagnostics)?;
        if values.iter().any(|value| *value <= 0.0) {
            diagnostics.push(format!("{size_key}: expected positive dimensions; placement ignored"));
            return None;
        }
        [values[0], values[1]]
    };
    let depth_key = format!("{prefix}Depth");
    let depth = if odf.raw(&depth_key).is_some() {
        match odf
            .str(&depth_key)
            .and_then(parse_scalar)
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            Some(value) => Some(value),
            None => {
                diagnostics.push(format!(
                    "{depth_key}: expected a finite positive depth; placement ignored"
                ));
                return None;
            }
        }
    } else {
        None
    };
    Some(ProjectionPlacement {
        texture,
        hardpoint,
        offset,
        rotation_degrees,
        size,
        depth,
    })
}

/// `Some(None)` means omitted; `None` means a present but malformed value.
fn optional_vector(
    odf: &Odf,
    key: &str,
    count: usize,
    diagnostics: &mut Vec<String>,
) -> Option<Option<Vec<f32>>> {
    if odf.raw(key).is_none() {
        return Some(None);
    }
    vector_value(odf, key, count, diagnostics).map(Some)
}

fn vector_value(
    odf: &Odf,
    key: &str,
    count: usize,
    diagnostics: &mut Vec<String>,
) -> Option<Vec<f32>> {
    let Some(text) = odf.str(key) else {
        diagnostics.push(format!("{key}: missing vector value"));
        return None;
    };
    let parts: Vec<&str> = text
        .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
        .filter(|part| !part.is_empty())
        .collect();
    let parsed: Option<Vec<f32>> = parts.iter().map(|part| parse_scalar(part)).collect();
    let Some(values) = parsed else {
        diagnostics.push(format!("{key}: expected exactly {count} finite components"));
        return None;
    };
    if values.len() != count || values.iter().any(|value| !value.is_finite()) {
        diagnostics.push(format!("{key}: expected exactly {count} finite components"));
        return None;
    }
    Some(values)
}

fn parse_scalar(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    value.parse::<f32>().ok()
}

fn parse_u8_triplet(text: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = text
        .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}

fn valid_name(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn odf(text: &str) -> Odf {
        sta_odf::resolve("appearance", &mut |_| Some(text.to_owned())).unwrap()
    }

    #[test]
    fn parses_sparse_subsystem_rows_in_contract_order_and_ignores_orphan_explosions() {
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "sensorMesh4 = \"sensor_node\"\n",
            "sensorMesh4explosion = \"boom_sensor\"\n",
            "engineMesh2 = \"engine_right\"\n",
            "engineMesh2explosion = \"boom_right\"\n",
            "weaponMesh1 = \"gun\"\n",
            "weaponMesh2explosion = \"ignored_without_pair\"\n",
            "shieldGeneratorMesh64 = \"shield\"\n",
            "lifeSupportMesh7 = \"life\"\n",
            "lifeSupportMesh7explosion = \"\"\n",
            "sensorMesh65 = \"outside_range\"\n",
        )));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            policy
                .subsystem_nodes
                .iter()
                .map(|entry| (entry.kind, entry.index))
                .collect::<Vec<_>>(),
            vec![
                (SubsystemKind::Engines, 2),
                (SubsystemKind::Weapons, 1),
                (SubsystemKind::Shields, 64),
                (SubsystemKind::LifeSupport, 7),
                (SubsystemKind::Sensors, 4),
            ]
        );
        assert_eq!(
            policy.subsystem_nodes[0].explosion.as_deref(),
            Some("boom_right")
        );
        assert_eq!(policy.subsystem_nodes[1].explosion, None);
        assert_eq!(policy.subsystem_nodes[3].node, "life");
        assert_eq!(policy.subsystem_nodes[3].explosion, None);
    }

    #[test]
    fn faction_suffix_and_node_names_are_validated_without_rewriting_source_case() {
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "factionTextureSuffix = \"_K-1\"\n",
            "engineMesh1 = \"MiXeD_Node-1\"\n",
            "engineMesh2 = \"bad/path\"\n",
        )));
        assert_eq!(policy.faction_suffix.as_deref(), Some("_K-1"));
        assert_eq!(policy.subsystem_nodes[0].node, "MiXeD_Node-1");
        assert_eq!(diagnostics.len(), 1);
        let (invalid, diag) = parse_appearance_policy(&odf("factionTextureSuffix = \"../evil\"\n"));
        assert_eq!(invalid.faction_suffix, None);
        assert_eq!(diag.len(), 1);
    }

    #[test]
    fn parses_literal_projection_vectors_and_sparse_logo_metadata() {
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "logoDecal3Hardpoint = \"HP_Name\"\n",
            "logoDecal3Offset = \"1.25 -2 0.5\"\n",
            "logoDecal3Rotation = \"180 0 90\"\n",
            "logoDecal3Size = \"5.5 1.25\"\n",
            "logoDecal3Depth = \"2.75\"\n",
            "logoDecal3Suffix = \"_lower\"\n",
            "logoDecal3ColourKey = \"255 0 17\"\n",
            "logoDecal3FlipU = true\n",
            "damageThreshold = 0.25\n",
            "damageDecalPreview = 1\n",
            "hullScorch2 = \"scorch_tex\"\n",
            "hullScorch2Hardpoint = \"hp06\"\n",
            "hullScorch2Size = \"6 4\"\n",
        )));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(policy.damage_threshold, Some(0.25));
        assert!(policy.damage_preview);
        let logo = &policy.logos[0];
        assert_eq!(logo.index, 3);
        assert_eq!(logo.projection.texture, "");
        assert_eq!(logo.projection.offset, Some([1.25, -2.0, 0.5]));
        assert_eq!(logo.projection.rotation_degrees, Some([180.0, 0.0, 90.0]));
        assert_eq!(logo.projection.size, [5.5, 1.25]);
        assert_eq!(logo.projection.depth, Some(2.75));
        assert_eq!(logo.suffix.as_deref(), Some("_lower"));
        assert_eq!(logo.colour_key, Some([255, 0, 17]));
        assert!(logo.flip_u);
        assert_eq!(policy.damage[0].target, DamageTarget::Hull);
        assert_eq!(policy.damage[0].index, 2);
        assert_eq!(policy.damage[0].projection.texture, "scorch_tex");
    }

    #[test]
    fn explicit_invalid_placement_values_suppress_entries_instead_of_defaulting() {
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "logoDecal1Hardpoint = \"hp\"\n",
            "logoDecal1Size = \"0 1\"\n",
            "logoDecal2Hardpoint = \"hp\"\n",
            "logoDecal2Size = \"1 1\"\n",
            "logoDecal2Suffix = \"../bad\"\n",
            "logoDecal3Hardpoint = \"hp\"\n",
            "logoDecal3Size = \"1 nope\"\n",
            "logoDecal4Hardpoint = \"hp\"\n",
            "logoDecal4Size = \"1 1\"\n",
            "logoDecal4FlipU = maybe\n",
            "hullScorch1 = \"scorch\"\n",
            "hullScorch1Hardpoint = \"hp\"\n",
            "hullScorch1Size = \"1 1\"\n",
            "hullScorch1Offset = \"0 nope 1\"\n",
            "damageThreshold = 0\n",
            "damageDecalPreview = invalid\n",
        )));
        assert!(policy.logos.is_empty());
        assert!(policy.damage.is_empty());
        assert_eq!(policy.damage_threshold, None);
        assert!(!policy.damage_preview);
        assert_eq!(diagnostics.len(), 7, "{diagnostics:?}");
    }

    #[test]
    fn absent_projection_size_uses_documented_six_by_six_default() {
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "hullScorch1 = \"scorch\"\n",
            "hullScorch1Hardpoint = \"hp06\"\n",
        )));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let placement = &policy.damage[0].projection;
        assert_eq!(placement.size, [6.0, 6.0]);
        assert_eq!(placement.offset, None);
        assert_eq!(placement.rotation_degrees, None);
    }

    #[test]
    fn legacy_damage_cycles_textures_and_preserves_sparse_hardpoint_positions() {
        let textures = vec!["burn_a".to_owned(), "burn_b".to_owned()];
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "hullTargetHardpoints = \"hp01\" \"\" \"hp03\"\n",
            "sensorsTargetHardpoints = \"sensor_hp\"\n",
            "enginesTargetHardpoints = \"engine_hp\"\n",
            "enginesScorch1 = \"explicit_but_invalid\"\n",
            "enginesScorch1Hardpoint = \"engine_hp\"\n",
            "enginesScorch1Size = \"0 6\"\n",
            "weaponsTargetHardpoints = \"gun_a\" \"gun_b\"\n",
        )));
        assert!(policy.damage.is_empty());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let legacy = legacy_damage(
            &odf(concat!(
                "hullTargetHardpoints = \"hp01\" \"\" \"hp03\"\n",
                "sensorsTargetHardpoints = \"sensor_hp\"\n",
                "enginesTargetHardpoints = \"engine_hp\"\n",
                "enginesScorch1 = \"explicit_but_invalid\"\n",
                "weaponsTargetHardpoints = \"gun_a\" \"gun_b\"\n",
            )),
            &textures,
        );
        assert_eq!(
            legacy
                .iter()
                .map(|e| (e.target, e.index))
                .collect::<Vec<_>>(),
            vec![
                (DamageTarget::Hull, 1),
                (DamageTarget::Hull, 3),
                (DamageTarget::Sensors, 1),
                (DamageTarget::Weapons, 1),
                (DamageTarget::Weapons, 2),
            ]
        );
        assert_eq!(legacy[0].projection.texture, "burn_a");
        assert_eq!(legacy[1].projection.texture, "burn_a");
        assert_eq!(legacy[1].projection.hardpoint, "hp03");
        assert_eq!(legacy[2].projection.texture, "burn_a");
        assert_eq!(legacy[3].projection.texture, "burn_a");
        assert_eq!(legacy[4].projection.texture, "burn_b");
        assert!(
            legacy
                .iter()
                .all(|entry| entry.projection.size == [6.0, 6.0])
        );
    }

    #[test]
    fn legacy_damage_uses_caller_filtered_textures_and_no_output_when_none_are_available() {
        let empty = legacy_damage(&odf("hullTargetHardpoints = \"hp\"\n"), &[]);
        assert!(empty.is_empty());
        let textures = vec!["  ".to_owned(), "loaded".to_owned()];
        let result = legacy_damage(&odf("hullTargetHardpoints = \"hp\"\n"), &textures);
        assert_eq!(result[0].projection.texture, "loaded");
    }

    #[test]
    fn missing_values_disable_faction_and_absent_threshold_uses_documented_default() {
        let (policy, diagnostics) = parse_appearance_policy(&odf(concat!(
            "factionTextureSuffix = \"\"\n",
            "damageThreshold = 1.01\n",
        )));
        assert_eq!(policy.faction_suffix, None);
        assert_eq!(policy.damage_threshold, None);
        assert_eq!(diagnostics.len(), 1);
        let (defaulted, _) = parse_appearance_policy(&Odf::default());
        assert_eq!(defaulted.damage_threshold, Some(0.1));
    }
}
