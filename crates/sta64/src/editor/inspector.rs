//! Object properties staged by the editor. A2 commits the object dialog on OK;
//! the Fleet Ops inspector extends the basic fields with class-specific rows.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use sta_bzn::MapObject;
use sta_sim::source::{Inspection, ObjectProperties};
use sta_sim::view::UnitFlags;
use sta_sim::world::{EntityKind, NEUTRAL};
use sta_sim::{EntityId, Fixed, PlayerId, Vec3 as SimVec3};

use crate::sim_link::{ContentRes, SimLink};

pub(super) enum Control {
    Text,
    Check,
    Team,
    Race(Vec<String>),
}

pub(super) struct Row {
    pub group: Option<&'static str>,
    pub advanced: bool,
    pub control: Control,
    pub section: &'static str,
    pub label: &'static str,
    pub key: Option<&'static str>,
    pub value: String,
}

const POS: [&str; 3] = ["x", "y", "z"];
const ANGLE: [&str; 3] = ["yaw", "pitch", "roll"];
const FACING: [&str; 4] = ["fore", "aft", "port", "starboard"];
// In simulation slot order. The property grid puts Shield first.
const SYSTEMS: [&str; 5] = ["engines", "weapons", "shield generator", "life support", "sensors"];
const ENABLED: [&str; 5] = ["engines_on", "weapons_on", "shields_on", "life_support_on", "sensors_on"];
const GROUPS: [&str; 5] = ["Engines", "Weapons", "Shield", "Lifesupport", "Sensor"];

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

#[derive(Clone, Copy)]
struct Section {
    name: &'static str,
    group: Option<&'static str>,
    advanced: bool,
}

impl Section {
    const fn new(name: &'static str) -> Self {
        Self { name, group: None, advanced: false }
    }

    const fn group(self, group: &'static str) -> Self {
        Self { group: Some(group), ..self }
    }

    const fn advanced(self) -> Self {
        Self { advanced: true, ..self }
    }

    fn row(self, rows: &mut Vec<Row>, label: &'static str, key: Option<&'static str>, value: impl Into<String>) {
        self.control(rows, label, key, value, Control::Text);
    }

    fn control(self, rows: &mut Vec<Row>, label: &'static str, key: Option<&'static str>, value: impl Into<String>, control: Control) {
        rows.push(Row {
            section: self.name, group: self.group, advanced: self.advanced,
            key, label, value: value.into(), control,
        });
    }

    fn check(self, rows: &mut Vec<Row>, label: &'static str, key: &'static str, enabled: bool) {
        self.control(rows, label, Some(key), if enabled { "1" } else { "0" }, Control::Check);
    }
}

fn is_craft(def: &sta_content::UnitDef) -> bool {
    matches!(EntityKind::of_def(def), EntityKind::Craft | EntityKind::Station)
}

fn subclass(def: &sta_content::UnitDef) -> Option<&'static str> {
    match def.class.0.as_str() {
        "freighter" => Some("Freighter"),
        "cargo_ship" => Some("CargoShip"),
        "colony" => Some("Colony"),
        "constructionrig" => Some("ConstructionRig"),
        "starbase" => Some("Starbase"),
        "shipyard" | "buildyard" => Some("Shipyard"),
        "research" => Some("ResearchStation"),
        "mining" | "p_mining" => Some("MiningStation"),
        "trading_station" => Some("TradingStation"),
        "wormhole" => Some("Wormhole"),
        "planet" | "planet_obj" => Some("Planet"),
        _ if def.resource.is_some() && !is_craft(def) => Some("Resource"),
        _ => None,
    }
}

pub(super) fn class_chain(id: EntityId, link: &SimLink, content: Option<&ContentRes>) -> String {
    let mut chain = String::from("Entity > GameObject");
    if let Some(def) = link.real_curr().unit(id).and_then(|unit| content?.0.unit(unit.def)) {
        if is_craft(def) {
            chain.push_str(" > Craft");
        }
        if EntityKind::producer_def(def) && is_craft(def) {
            chain.push_str(" > Producer");
        }
        if let Some(class) = subclass(def) {
            chain.push_str(" > ");
            chain.push_str(class);
        }
    }
    chain
}

fn author_team(owner: PlayerId) -> i32 {
    if owner.is_player() { i32::from(owner.0) + 1 } else { 0 }
}

fn team_row(section: Section, rows: &mut Vec<Row>, label: &'static str, key: Option<&'static str>, team: i32) {
    section.control(rows, label, key, team.to_string(),
        if (0..=8).contains(&team) { Control::Team } else { Control::Text });
}

fn carrier_rows(rows: &mut Vec<Row>, db: &sta_content::ContentDb, def: &sta_content::UnitDef) {
    let section = Section::new("GameObject").group("Carrier").advanced();
    let weapons: Vec<_> = def.hardpoints.iter().filter_map(|hp| db.weapon(hp.weapon.key)).collect();
    if weapons.is_empty() {
        return;
    }
    let regular: Vec<_> = weapons.iter().filter(|weapon| !sta_sim::specials::is_special(weapon)).collect();
    section.row(rows, "Regular weapon mounts", None, regular.len().to_string());
    section.row(rows, "Special weapon mounts", None, (weapons.len() - regular.len()).to_string());
    for (caption, value) in [
        ("Min regular range", regular.iter().map(|weapon| weapon.range).min()),
        ("Max regular range", regular.iter().map(|weapon| weapon.range).max()),
        ("Min weapon range", weapons.iter().map(|weapon| weapon.range).min()),
        ("Max weapon range", weapons.iter().map(|weapon| weapon.range).max()),
    ] {
        if let Some(range) = value {
            section.row(rows, caption, None, fixed_text(range));
        }
    }
}

pub(super) fn rows(
    id: EntityId,
    link: &SimLink,
    content: Option<&ContentRes>,
    object: Option<&MapObject>,
    extras: Option<&ObjectProperties>,
    can_change_identity: bool,
) -> Vec<Row> {
    let view = link.real_curr();
    let Some(unit) = view.unit(id) else { return Vec::new() };
    let Some(db) = content.map(|content| content.0.as_ref()) else { return Vec::new() };
    let Some(def) = db.unit(unit.def) else { return Vec::new() };
    let mut rows = Vec::new();
    let entity = Section::new("Entity").advanced();
    entity.row(&mut rows, "Entity type", None, &def.class.0);
    entity.row(&mut rows, "ID", None, format!("{:#010X}", id.0));
    let game_object = Section::new("GameObject");
    let identity_key = |key| can_change_identity.then_some(key);
    game_object.row(&mut rows, "Label", identity_key("label"), object.map_or("", |o| &o.label));
    let race = view.attrs.races.iter().find(|row| row.0 == id).map_or(&def.race, |row| &row.1);
    let races: BTreeSet<_> = db.units().filter(|def| !def.race.is_empty())
        .map(|def| def.race.as_str()).chain((!race.is_empty()).then_some(race.as_str())).collect();
    let races = std::iter::once("ODF default".to_owned()).chain(races.into_iter().map(str::to_owned)).collect();
    game_object.control(&mut rows, "Race", Some("race"), race, Control::Race(races));
    let team = object.map_or_else(|| author_team(unit.owner), |object| object.team);
    team_row(game_object, &mut rows, "Team", identity_key("team"), team);
    let perceived = view.perceived_owner(id).unwrap_or(unit.owner);
    team_row(game_object, &mut rows, "Perceived Team", Some("perceived_team"), author_team(perceived));

    let pos = object.map_or_else(|| unit.pos.to_f32(), |object| object.transform[9..12].try_into().expect("position"));
    for ((key, label), value) in POS.into_iter().zip(["X", "Y", "Z"]).zip(pos) {
        game_object.group("Position").row(&mut rows, label, Some(key), value.to_string());
    }
    let [yaw, pitch, roll] = angles(object, unit.heading);
    for (label, key, value) in [("Roll", "roll", roll), ("Pitch", "pitch", pitch), ("Yaw", "yaw", yaw)] {
        game_object.row(&mut rows, label, Some(key), value.to_string());
    }

    if def.hull > Fixed::ZERO || unit.hull[1] > Fixed::ZERO {
        game_object.row(&mut rows, "Cur Health", Some("hull"), fixed_text(unit.hull[0]));
        game_object.row(&mut rows, "Max Health", Some("max_hull"), fixed_text(unit.hull[1]));
    }
    let energy = unit.energy.unwrap_or([Fixed::ZERO; 2]);
    game_object.row(&mut rows, "Cur Special Energy", unit.energy.map(|_| "energy"), fixed_text(energy[0]));
    game_object.row(&mut rows, "Max Special Energy", None, fixed_text(energy[1]));
    game_object.advanced().check(&mut rows, "Cannot die", "immortal", unit.flags.contains(UnitFlags::IMMORTAL));
    game_object.advanced().row(&mut rows, "ODF", None, format!("{}.odf", def.odf));
    carrier_rows(&mut rows, db, def);

    let craft = Section::new("Craft");
    let name_section = if is_craft(def) { craft } else { Section::new(subclass(def).unwrap_or("GameObject")) };
    use crate::ui::hud::model::craft_names;
    let identity = craft_names::identity(def, id);
    let name = view.attrs.names.iter().find(|row| row.0 == id).map(|row| row.1.as_str())
        .or_else(|| object.map(|object| object.instance_name.as_str()).filter(|name| !name.is_empty()))
        .unwrap_or(&identity.name);
    name_section.row(&mut rows, "DisplayName", identity_key("name"), name);

    if def.shields.max > Fixed::ZERO || unit.shields[0][1] > Fixed::ZERO {
        if db.features().directional_shields {
            let shields = craft.group("Directional shields");
            for ((key, label), shield) in FACING.into_iter().zip(["Fore", "Aft", "Port", "Starboard"]).zip(unit.shields) {
                shields.row(&mut rows, label, Some(key), fixed_text(shield[0]));
            }
            shields.row(&mut rows, "Maximum per facing", Some("max_shields"), fixed_text(unit.shields[0][1]));
        } else {
            craft.row(&mut rows, "curShields", Some("shields"), fixed_text(unit.shields[0][0]));
            craft.row(&mut rows, "maxShields", Some("max_shields"), fixed_text(unit.shields[0][1]));
        }
        if extras.and_then(|properties| properties.shields)
            .is_some_and(|faces| faces.iter().any(|face| face[1] != faces[0][1]))
        {
            craft.row(&mut rows, "Imported unequal capacities", None, "Preserved in map data");
        }
    }
    if let Some(crew) = unit.crew {
        craft.row(&mut rows, "maxCrew", Some("max_crew"), fixed_text(crew[1]));
        craft.row(&mut rows, "curCrew", Some("crew"), fixed_text(crew[0]));
    }
    if is_craft(def) {
        if let Some(ship_type) = &def.stats.ship_type {
            craft.advanced().row(&mut rows, "Ship Type", None, ship_type);
        }
        let captain = extras.and_then(|properties| properties.captain.as_ref());
        let registry = extras.and_then(|properties| properties.registry.as_ref());
        let section = craft.group("Identity");
        section.row(&mut rows, "Captain", Some("captain"), captain.or(identity.captain.as_ref()).map_or("", String::as_str));
        section.check(&mut rows, "Captain uses ODF", "captain_default", captain.is_none());
        section.row(&mut rows, "Registry", Some("registry"), registry.or(identity.registry.as_ref()).map_or("", String::as_str));
        section.check(&mut rows, "Registry uses ODF", "registry_default", registry.is_none());
        let (health, enabled) = system_state(id, link);
        for i in [2, 0, 1, 3, 4] {
            let cap = def.system_hit_points[i];
            if cap <= Fixed::ZERO {
                continue;
            }
            let system = Section::new("CraftSystem").group(GROUPS[i]).advanced();
            system.check(&mut rows, "Active", ENABLED[i], enabled[i]);
            system.row(&mut rows, "Max hit points", None, fixed_text(cap));
            system.row(&mut rows, "Health fraction", Some(SYSTEMS[i]), fixed_text(health[i]));
            if let Some(stats) = view.appearance.systems(id).and_then(|systems| systems[i]) {
                system.row(&mut rows, "Current hit points", None, fixed_text(stats.health[0]));
            }
        }
    }
    if let Some(job) = link.production().iter().find(|queue| queue.builder == id)
        .and_then(|queue| queue.jobs.first().filter(|job| job.started))
    {
        let producer = Section::new("Producer").advanced();
        if let Some(product) = db.unit(job.def) {
            producer.row(&mut rows, "Building", None, &product.odf);
        }
        producer.row(&mut rows, "Build progress", None, fixed_text(job.progress));
    }
    if let Some(node) = link.nodes().iter().find(|node| node.entity == id) {
        Section::new("Resource").row(&mut rows, "Amount", (!node.infinite).then_some("resource"),
            if node.infinite { "Unlimited".into() } else { fixed_text(node.amount) });
        Section::new("Resource").row(&mut rows, "Capacity", None, fixed_text(node.start));
    }
    if def.planet_role == Some(sta_content::PlanetRole::Producer) {
        let population = link.real_curr().colonies.iter().find(|colony| colony.id == id)
            .map_or(Fixed::ZERO, |colony| colony.population);
        let cap = db.rules().population().map_or(Fixed::ZERO, |rules| rules.cap(def.planet_max_population));
        Section::new("Planet").row(&mut rows, "Population", Some("population"), fixed_text(population));
        Section::new("Planet").row(&mut rows, "Maximum population", None, fixed_text(cap));
    }
    rows
}

fn system_state(id: EntityId, link: &SimLink) -> ([Fixed; 5], [bool; 5]) {
    link.systems().iter().find(|system| system.entity == id)
        .map_or(([Fixed::ONE; 5], [true; 5]), |system| (system.health, system.on))
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
    let changed_check = |key| changed(key).map(|input| match input {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("Invalid checkbox value for {key}")),
    }).transpose();
    let to_owner = |team| if team == 0 { NEUTRAL } else { PlayerId(team - 1) };
    let mut saved = extras.cloned().unwrap_or_default();
    let mut live = ObjectProperties::default();
    let mut owner = unit.owner;
    let mut at = unit.pos;
    let mut heading = unit.heading;
    let mut name = changed("name").map(|s| valid_text(s, "Display name", 64)).transpose()?;
    let label = changed("label").map(|s| valid_text(s, "Script label", 127)).transpose()?;
    if let Some(input) = changed("team") {
        let team = read_team(input)?;
        owner = to_owner(team);
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
        live.race = Some(String::new());
        live.perceived_owner = Some(owner);
        live.immortal = Some(false);
        if is_craft(def) && def.system_hit_points.iter().any(|cap| *cap > Fixed::ZERO) {
            for (slot, cap) in live.systems_enabled.iter_mut().zip(def.system_hit_points) {
                *slot = (cap > Fixed::ZERO).then_some(true);
            }
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
        if SYSTEMS.iter().any(|key| changed(key).is_some()) {
            let (mut health, _) = system_state(id, link);
            for (key, value) in SYSTEMS.into_iter().zip(&mut health) {
                *value = read_fixed(key, *value)?;
            }
            live.systems = Some(health);
            saved.systems = live.systems;
        }
        for (i, key) in ENABLED.into_iter().enumerate() {
            if let Some(value) = changed_check(key)? {
                live.systems_enabled[i] = Some(value);
                saved.systems_enabled[i] = Some(value);
            }
        }
        if live.systems_enabled[2] == Some(false) && live.shields.is_none() && unit.shields[0][1] > Fixed::ZERO {
            let mut faces = saved.shields.unwrap_or(unit.shields);
            for face in &mut faces {
                face[0] = Fixed::ZERO;
            }
            saved.shields = Some(faces);
        }
        if let Some(input) = changed("race") {
            let race = input.trim();
            if race.is_empty() || def.race.eq_ignore_ascii_case(race) {
                live.race = Some(String::new());
                saved.race = None;
            } else {
                let valid = db.units().find(|def| def.race.eq_ignore_ascii_case(race))
                    .ok_or("Enter a race defined by the active mod")?;
                let race = valid_text(&valid.race, "Race", sta_sim::attrs::MAX_RACE_BYTES)?;
                live.race = Some(race.clone());
                saved.race = Some(race);
            }
        }
        if let Some(input) = changed("perceived_team") {
            let perceived_owner = to_owner(read_team(input)?);
            live.perceived_owner = Some(perceived_owner);
            saved.perceived_owner = (perceived_owner != owner).then_some(perceived_owner);
        } else if saved.perceived_owner == Some(owner) {
            saved.perceived_owner = None;
        }
        if let Some(value) = changed_check("immortal")? {
            live.immortal = Some(value);
            saved.immortal = value.then_some(true);
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
