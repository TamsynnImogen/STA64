//! Object properties staged by the editor. A2 commits the object dialog on OK;
//! the Fleet Ops inspector extends the basic fields with class-specific rows.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_bzn::MapObject;
use sta_sim::source::{Inspection, ObjectProperties};
use sta_sim::world::{EntityKind, NEUTRAL};
use sta_sim::{EntityId, Fixed, PlayerId, Vec3 as SimVec3};

use crate::sim_link::{ContentRes, SimLink};

pub(super) struct Row {
    pub section: &'static str,
    pub label: &'static str,
    pub key: Option<&'static str>,
    pub value: String,
}

const POS: [&str; 3] = ["x", "y", "z"];
const ANGLE: [&str; 3] = ["yaw", "pitch", "roll"];
const FACING: [&str; 4] = ["fore", "aft", "port", "starboard"];
const SYSTEMS: [&str; 5] = ["engines", "weapons", "shield generator", "life support", "sensors"];

fn fixed_text(value: Fixed) -> String {
    (value.to_bits() as f64 / (1u64 << 32) as f64).to_string()
}

fn basis(object: &MapObject) -> Mat3 {
    Mat3::from_cols_array(&object.transform[..9].try_into().expect("three basis vectors"))
}

fn angles(object: Option<&MapObject>, heading: (Fixed, Fixed)) -> [f32; 3] {
    if let Some(matrix) = object.map(basis)
        && matrix.is_finite() && matrix.determinant().abs() > 0.00001
    {
        let rotation = Mat3::from_cols(matrix.x_axis.normalize(), matrix.y_axis.normalize(), matrix.z_axis.normalize());
        let (yaw, pitch, roll) = Quat::from_mat3(&rotation).to_euler(EulerRot::YXZ);
        [yaw, pitch, roll].map(f32::to_degrees)
    } else {
        [heading.0.to_f32().atan2(heading.1.to_f32()).to_degrees(), 0.0, 0.0]
    }
}

fn push(rows: &mut Vec<Row>, section: &'static str, label: &'static str, key: Option<&'static str>, value: impl Into<String>) {
    rows.push(Row { section, label, key, value: value.into() });
}

pub(super) fn rows(
    id: EntityId,
    link: &SimLink,
    content: Option<&ContentRes>,
    object: Option<&MapObject>,
    extras: Option<&ObjectProperties>,
    can_change_identity: bool,
) -> Vec<Row> {
    let Some(unit) = link.real_curr().unit(id) else { return Vec::new() };
    let Some(db) = content.map(|content| content.0.as_ref()) else { return Vec::new() };
    let Some(def) = db.unit(unit.def) else { return Vec::new() };
    let mut rows = Vec::new();
    push(&mut rows, "Object", "ODF", None, &def.odf);
    push(&mut rows, "Object", "Class", None, &def.class.0);
    push(&mut rows, "Object", "Entity ID", None, id.0.to_string());
    let identity_key = |key| can_change_identity.then_some(key);
    push(&mut rows, "Object", "Script label", identity_key("label"), object.map_or("", |o| &o.label));
    push(&mut rows, "Object", "Display name", identity_key("name"), object.map_or("", |o| &o.instance_name));
    let team = if unit.owner == NEUTRAL { 0 } else { u16::from(unit.owner.0) + 1 };
    let shown_team = object.map_or_else(|| i32::from(team), |object| object.team);
    push(&mut rows, "Object", "Team (0 neutral)", identity_key("team"), shown_team.to_string());

    if EntityKind::of_def(def) == EntityKind::Craft {
        let identity = crate::ui::hud::model::craft_names::identity(def, id);
        let captain = extras.and_then(|properties| properties.captain.as_ref());
        let registry = extras.and_then(|properties| properties.registry.as_ref());
        push(&mut rows, "Craft identity", "Captain", Some("captain"), captain.or(identity.captain.as_ref()).map_or("", String::as_str));
        push(&mut rows, "Craft identity", "Captain uses ODF (0/1)", Some("captain_default"), if captain.is_none() { "1" } else { "0" });
        push(&mut rows, "Craft identity", "Registry", Some("registry"), registry.or(identity.registry.as_ref()).map_or("", String::as_str));
        push(&mut rows, "Craft identity", "Registry uses ODF (0/1)", Some("registry_default"), if registry.is_none() { "1" } else { "0" });
    }

    let pos = object.map_or_else(|| unit.pos.to_f32(), |object| object.transform[9..12].try_into().expect("position"));
    for (axis, value) in POS.into_iter().zip(pos) {
        push(&mut rows, "Position", axis, Some(axis), value.to_string());
    }
    for (axis, value) in ANGLE.into_iter().zip(angles(object, unit.heading)) {
        push(&mut rows, "Rotation (degrees)", axis, Some(axis), value.to_string());
    }

    if def.hull > Fixed::ZERO || unit.hull[1] > Fixed::ZERO {
        push(&mut rows, "Condition", "Hull", Some("hull"), fixed_text(unit.hull[0]));
        push(&mut rows, "Condition", "Maximum hull", Some("max_hull"), fixed_text(unit.hull[1]));
    }
    if let Some(crew) = unit.crew {
        push(&mut rows, "Condition", "Crew", Some("crew"), fixed_text(crew[0]));
        push(&mut rows, "Condition", "Maximum crew", Some("max_crew"), fixed_text(crew[1]));
    }
    if let Some(energy) = unit.energy {
        push(&mut rows, "Condition", "Special energy", Some("energy"), fixed_text(energy[0]));
        push(&mut rows, "Condition", "Maximum energy", None, fixed_text(energy[1]));
    }
    if def.shields.max > Fixed::ZERO || unit.shields[0][1] > Fixed::ZERO {
        let label = if db.features().directional_shields { "Maximum per facing" } else { "Maximum shields" };
        push(&mut rows, "Shields", label, Some("max_shields"), fixed_text(unit.shields[0][1]));
        if db.features().directional_shields {
            for (side, shield) in FACING.into_iter().zip(unit.shields) {
                push(&mut rows, "Shields", side, Some(side), fixed_text(shield[0]));
            }
        } else {
            push(&mut rows, "Shields", "Current shields", Some("shields"), fixed_text(unit.shields[0][0]));
        }
        if extras.and_then(|properties| properties.shields)
            .is_some_and(|faces| faces.iter().any(|face| face[1] != faces[0][1]))
        {
            push(&mut rows, "Shields", "Imported unequal capacities", None, "Preserved in map data");
        }
    }
    if let Some(systems) = link.systems().iter().find(|row| row.entity == id) {
        for (index, (name, value)) in SYSTEMS.into_iter().zip(systems.health).enumerate() {
            if def.system_hit_points[index] > Fixed::ZERO {
                push(&mut rows, "Subsystem health (0–1)", name, Some(name), fixed_text(value));
            }
        }
    }
    if let Some(node) = link.nodes().iter().find(|node| node.entity == id) {
        push(&mut rows, "Resource", "Amount", (!node.infinite).then_some("resource"),
            if node.infinite { "Unlimited".into() } else { fixed_text(node.amount) });
        push(&mut rows, "Resource", "Capacity", None, fixed_text(node.start));
    }
    if def.planet_role == Some(sta_content::PlanetRole::Producer) {
        let population = link.real_curr().colonies.iter().find(|colony| colony.id == id)
            .map_or(Fixed::ZERO, |colony| colony.population);
        let cap = db.rules().population().map_or(Fixed::ZERO, |rules| rules.cap(def.planet_max_population));
        push(&mut rows, "Planet", "Population", Some("population"), fixed_text(population));
        push(&mut rows, "Planet", "Maximum population", None, fixed_text(cap));
    }
    rows
}

pub(super) struct Plan {
    pub edit: Inspection,
    pub object: Option<MapObject>,
    pub saved: ObjectProperties,
    pub changed: bool,
}

fn number(input: &str, field: &str) -> Result<f64, String> {
    input.trim().parse::<f64>().ok()
        .filter(|n| n.is_finite() && n.abs() <= f64::from(i32::MAX))
        .ok_or_else(|| format!("Enter a finite number for {field}"))
}

fn fixed(input: &str, field: &str) -> Result<Fixed, String> {
    number(input, field).map(Fixed::from_f64)
}

fn valid_text(value: &str, field: &str, max: usize) -> Result<String, String> {
    if value.len() > max || value.contains(['\0', '\n', '\r']) {
        return Err(format!("{field} must fit in {max} UTF-8 bytes"));
    }
    Ok(value.to_owned())
}

fn as_pair(now: Fixed, max: Fixed, field: &str) -> Result<[Fixed; 2], String> {
    if now < Fixed::ZERO || max < now {
        return Err(format!("{field} must be between zero and its maximum"));
    }
    Ok([now, max])
}

#[allow(clippy::too_many_arguments)]
pub(super) fn plan(
    id: EntityId,
    link: &SimLink,
    content: &ContentRes,
    object: Option<&MapObject>,
    extras: Option<&ObjectProperties>,
    can_change_identity: bool,
    inputs: &BTreeMap<&str, &str>,
    restore: bool,
) -> Result<Plan, String> {
    let unit = link.real_curr().unit(id).ok_or("Object no longer available")?;
    let db = &content.0;
    let def = db.unit(unit.def).ok_or("Object definition is unavailable")?;
    let rows = rows(id, link, Some(content), object, extras, can_change_identity);
    let changed = |key: &str| -> Option<&str> {
        let initial = rows.iter().find(|row| row.key == Some(key))?;
        inputs.get(key).copied().filter(|value| *value != initial.value)
    };
    let changed_fixed = |key| changed(key).map(|value| fixed(value, key)).transpose();
    let read_fixed = |key, otherwise| changed_fixed(key).map(|value| value.unwrap_or(otherwise));
    let read_team = |input: &str| {
        input.trim().parse::<u8>().ok().filter(|team| *team <= 8).ok_or("Team must be 0 through 8")
    };
    let mut object = object.cloned();
    let mut saved = extras.cloned().unwrap_or_default();
    let mut live = ObjectProperties::default();
    let mut owner = unit.owner;
    let mut at = unit.pos;
    let mut heading = unit.heading;
    let mut name = changed("name").map(|s| valid_text(s, "Display name", 64)).transpose()?;
    let label = changed("label").map(|s| valid_text(s, "Script label", 127)).transpose()?;
    if let Some(input) = changed("team") {
        let team = read_team(input)?;
        owner = if team == 0 { NEUTRAL } else { PlayerId(team - 1) };
        if let Some(object) = &mut object {
            object.team = i32::from(team);
        }
    }
    if let Some(object) = &mut object && let Some(label) = &label {
        object.label = label.clone();
    }

    if POS.iter().any(|key| changed(key).is_some()) {
        let pos = object.as_ref().map_or_else(|| unit.pos.to_f32(), |object| object.transform[9..12].try_into().expect("position"));
        let mut next = pos.map(f64::from);
        for (axis, value) in POS.into_iter().zip(&mut next) {
            if let Some(input) = changed(axis) {
                *value = number(input, axis)?;
            }
        }
        let next = next.map(|value| value as f32);
        at = SimVec3::from_f32(next);
        if let Some(object) = &mut object {
            object.transform[9..12].copy_from_slice(&next);
            object.initial_position = Some(next);
        }
    }
    if ANGLE.iter().any(|key| changed(key).is_some()) {
        let mut next = angles(object.as_ref(), heading);
        for (axis, value) in ANGLE.into_iter().zip(&mut next) {
            if let Some(input) = changed(axis) {
                *value = number(input, axis)?.rem_euclid(360.0) as f32;
            }
        }
        let [yaw, pitch, roll] = next.map(f32::to_radians);
        let rotation = Mat3::from_quat(Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll));
        heading = (Fixed::from_f64(f64::from(yaw.sin())), Fixed::from_f64(f64::from(yaw.cos())));
        if let Some(object) = &mut object {
            let scale = basis(object);
            let matrix = Mat3::from_cols(rotation.x_axis * scale.x_axis.length(), rotation.y_axis * scale.y_axis.length(), rotation.z_axis * scale.z_axis.length());
            object.transform[..9].copy_from_slice(&matrix.to_cols_array());
        }
    }

    if restore {
        let default = sta_sim::world::Entity::from_def(def, db.features(), owner, at, heading);
        live.hull = (def.hull > Fixed::ZERO || unit.hull[1] > Fixed::ZERO).then_some(default.hull);
        live.shields = (def.shields.max > Fixed::ZERO || unit.shields[0][1] > Fixed::ZERO).then(|| default.shields.view());
        live.crew = default.crew;
        live.energy = unit.energy.map(|energy| energy[1]);
        if link.systems().iter().any(|system| system.entity == id) {
            live.systems = Some([Fixed::ONE; 5]);
        }
        saved = ObjectProperties::default();
        live.resource = link.nodes().iter().find(|node| node.entity == id && node.amount != node.start)
            .map(|node| node.start);
        if link.real_curr().colonies.iter().any(|colony| colony.id == id) {
            live.population = Some(Fixed::ZERO);
            saved.population = live.population;
        }
        if can_change_identity {
            name = Some(String::new());
        }
    } else {
        if changed("hull").is_some() || changed("max_hull").is_some() {
            live.hull = Some(as_pair(read_fixed("hull", unit.hull[0])?, read_fixed("max_hull", unit.hull[1])?, "Hull")?);
            saved.hull = live.hull;
        }
        if let Some(crew) = unit.crew && (changed("crew").is_some() || changed("max_crew").is_some()) {
            live.crew = Some(as_pair(read_fixed("crew", crew[0])?, read_fixed("max_crew", crew[1])?, "Crew")?);
            saved.crew = live.crew;
        }
        if changed("max_shields").is_some() || changed("shields").is_some() || FACING.iter().any(|key| changed(key).is_some()) {
            let max = read_fixed("max_shields", unit.shields[0][1])?;
            let mut faces = unit.shields;
            for (side, face) in FACING.into_iter().zip(&mut faces) {
                let key = if db.features().directional_shields { side } else { "shields" };
                *face = as_pair(read_fixed(key, face[0])?, max, "Shields")?;
            }
            live.shields = Some(faces);
            saved.shields = live.shields;
        }
        if let Some(system) = link.systems().iter().find(|system| system.entity == id)
            && SYSTEMS.iter().any(|key| changed(key).is_some())
        {
            let mut health = system.health;
            for (key, value) in SYSTEMS.into_iter().zip(&mut health) {
                *value = read_fixed(key, *value)?;
            }
            live.systems = Some(health);
            saved.systems = live.systems;
        }
        if let Some(value) = changed_fixed("energy")? {
            live.energy = Some(value);
            saved.energy = live.energy;
        }
        if let Some(value) = changed_fixed("resource")? {
            live.resource = Some(value);
            saved.resource = live.resource;
        }
        if let Some(value) = changed_fixed("population")? {
            let cap = db.rules().population().map_or(Fixed::ZERO, |rules| rules.cap(def.planet_max_population));
            if value < Fixed::ZERO || value > cap {
                return Err("Population must be between zero and its maximum".into());
            }
            live.population = Some(value);
            saved.population = live.population;
        }
        for (field, defaults, value) in [
            ("captain", "captain_default", &mut saved.captain),
            ("registry", "registry_default", &mut saved.registry),
        ] {
            if changed(field).is_none() && changed(defaults).is_none() {
                continue;
            }
            match inputs.get(defaults).map(|input| input.trim()) {
                Some("1") => {
                    // Typing a value opts into the override unless the reset flag was also changed.
                    *value = if changed(defaults).is_some() { None }
                        else { changed(field).map(|entry| valid_text(entry, field, 511)).transpose()? };
                }
                Some("0") => *value = inputs.get(field).map(|entry| valid_text(entry, field, 511)).transpose()?,
                _ => return Err(format!("Enter 0 or 1 for {defaults}")),
            }
        }
    }

    if let Some(value) = &name {
        saved.instance_name = Some(value.clone());
        if let Some(object) = &mut object {
            object.instance_name = value.clone();
        }
    }
    let changed = restore || owner != unit.owner || at != unit.pos || heading != unit.heading
        || label.is_some() || name.is_some() || live != ObjectProperties::default()
        || extras.map_or(saved != ObjectProperties::default(), |before| *before != saved);
    Ok(Plan {
        edit: Inspection { id, owner, at, heading, label, name, properties: live },
        object,
        saved,
        changed,
    })
}
