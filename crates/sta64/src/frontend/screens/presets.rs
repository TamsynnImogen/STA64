//! The one last-saved Instant Action setup. IDs are resolved against the mounted
//! content on load; a preset never selects a map or race by its list position.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use sta_content::StartingResources;

use super::data::{RANDOM_SIDE, SetupFog, SetupStartMarkers};
use super::{ScreenData, Seat, SeatControl, SetupDraft};
use crate::clock::MatchSpeed;

const FILE_NAME: &str = "instant-action-setup.cfg";
const VERSION: &str = "1";
const MAX_BYTES: u64 = 64 * 1024;

pub(super) fn save(draft: &SetupDraft, data: &ScreenData) -> Result<(), String> {
    let dir = crate::platform::config_dir().ok_or("No per-user config folder is available")?;
    save_at(&dir.join(FILE_NAME), draft, data)
}

pub(super) fn load(data: &ScreenData) -> Result<SetupDraft, String> {
    let dir = crate::platform::config_dir().ok_or("No per-user config folder is available")?;
    load_at(&dir.join(FILE_NAME), data)
}

fn save_at(path: &Path, draft: &SetupDraft, data: &ScreenData) -> Result<(), String> {
    validate(draft, data)?;
    let map = &data.maps[draft.map].def.name;
    let mut text = format!(
        "version={VERSION}\nmap={}\nseed={}\nseat_count={}\nobserving={}\nreturn_row={}\nspeed={}\nresources={}\nofficer_limit={}\ndirectors_cut={}\ncolour={}\nfog={}\nrandom_placement={}\npreview_visible={}\nstart_markers={}\n",
        encode(map),
        draft.seed,
        draft.seats.len(),
        draft.observing,
        draft.return_row,
        speed_key(draft.speed),
        resources_key(draft.options.starting_resources),
        draft.options.officer_limit,
        draft.options.directors_cut,
        draft
            .colour
            .map_or_else(|| "default".to_owned(), |n| n.to_string()),
        draft.fog.key(),
        draft.random_placement,
        draft.preview_visible,
        draft.start_markers.key()
    );
    for (i, seat) in draft.seats.iter().enumerate() {
        let control = match seat.control {
            SeatControl::Human => "human",
            SeatControl::Easy => "easy",
            SeatControl::Computer => "computer",
            SeatControl::Hard => "hard",
            SeatControl::Mcp => "mcp",
            SeatControl::Idle => "idle",
        };
        text.push_str(&format!(
            "seat.{i}.control={control}\nseat.{i}.race={}\nseat.{i}.team={}\n",
            encode(&seat.race),
            seat.team
        ));
    }
    crate::ui::settings::write_atomic(path, &text).map_err(|e| {
        format!(
            "Cannot save Instant Action settings to {}: {e}",
            path.display()
        )
    })
}

fn load_at(path: &Path, data: &ScreenData) -> Result<SetupDraft, String> {
    let file = std::fs::File::open(path).map_err(|e| {
        format!(
            "Cannot open Instant Action settings at {}: {e}",
            path.display()
        )
    })?;
    let len = file
        .metadata()
        .map_err(|e| format!("Cannot inspect Instant Action settings: {e}"))?
        .len();
    if len > MAX_BYTES {
        return Err("Instant Action settings file is too large".into());
    }
    let mut text = String::new();
    file.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("Cannot read Instant Action settings: {e}"))?;
    if text.len() as u64 > MAX_BYTES {
        return Err("Instant Action settings file is too large".into());
    }
    let mut fields = BTreeMap::new();
    for (line, raw) in text.lines().enumerate() {
        let raw = raw.trim_end_matches('\r');
        if raw.is_empty() {
            continue;
        }
        let (key, value) = raw
            .split_once('=')
            .ok_or_else(|| format!("Invalid Instant Action settings line {}", line + 1))?;
        if key.is_empty() || fields.insert(key, value).is_some() {
            return Err(format!(
                "Invalid or repeated Instant Action setting '{key}'"
            ));
        }
    }
    let get = |key: &str| {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| format!("Instant Action setting '{key}' is missing"))
    };
    if get("version")? != VERSION {
        return Err("Unsupported Instant Action settings version".into());
    }
    let map_name = decode(get("map")?)?;
    let map = data
        .maps
        .iter()
        .position(|m| m.def.name == map_name)
        .ok_or_else(|| format!("Saved map '{map_name}' is not available in the mounted content"))?;
    let slots = data.maps[map].slots();
    let count = number::<usize>(get("seat_count")?, "seat_count")?;
    if count != slots || count < 2 {
        return Err("Saved seat count does not match the mounted map".into());
    }
    let races = data.races();
    let mut seats = Vec::with_capacity(count);
    for i in 0..count {
        let control = match get(&format!("seat.{i}.control"))? {
            "human" => SeatControl::Human,
            "easy" => SeatControl::Easy,
            "computer" => SeatControl::Computer,
            "hard" => SeatControl::Hard,
            "mcp" => SeatControl::Mcp,
            "idle" => SeatControl::Idle,
            _ => return Err(format!("Invalid control for seat {}", i + 1)),
        };
        let race = decode(get(&format!("seat.{i}.race"))?)?;
        let race = if race == RANDOM_SIDE {
            race
        } else {
            races
                .iter()
                .find(|r| r.eq_ignore_ascii_case(&race))
                .cloned()
                .ok_or_else(|| {
                    format!("Saved race '{race}' is not available in the mounted content")
                })?
        };
        let team = number::<u8>(get(&format!("seat.{i}.team"))?, "team")?;
        seats.push(Seat {
            control,
            race,
            team,
        });
    }
    let mut draft = SetupDraft::new(data);
    draft.map = map;
    draft.seed = number(get("seed")?, "seed")?;
    draft.seats = seats;
    draft.observing = boolean(get("observing")?, "observing")?;
    draft.return_row = number(get("return_row")?, "return_row")?;
    draft.speed = match get("speed")? {
        "slow" => MatchSpeed::Slow,
        "normal" => MatchSpeed::Normal,
        "fast" => MatchSpeed::Fast,
        _ => return Err("Invalid Instant Action speed".into()),
    };
    draft.options.starting_resources = match get("resources")? {
        "normal" => StartingResources::Normal,
        "lots" => StartingResources::Lots,
        _ => return Err("Invalid Instant Action starting resources".into()),
    };
    draft.options.officer_limit = number(get("officer_limit")?, "officer_limit")?;
    let directors_cut = fields.get("directors_cut").map_or(Ok(false), |value| boolean(value, "directors_cut"))?;
    draft.options.directors_cut = directors_cut && data.content.profile() == sta_content::Profile::Sta1Classic;
    draft.colour = match get("colour")? {
        "default" => None,
        value => Some(number(value, "colour")?),
    };
    draft.fog = SetupFog::from_key(get("fog")?).ok_or("Invalid Instant Action fog mode")?;
    draft.random_placement = boolean(get("random_placement")?, "random_placement")?;
    draft.preview_visible = boolean(get("preview_visible")?, "preview_visible")?;
    draft.start_markers = SetupStartMarkers::from_key(get("start_markers")?)
        .ok_or("Invalid Instant Action start marker mode")?;
    validate(&draft, data)?;
    Ok(draft)
}

fn validate(draft: &SetupDraft, data: &ScreenData) -> Result<(), String> {
    let map = data
        .maps
        .get(draft.map)
        .ok_or("Selected map is not available")?;
    let slots = map.slots();
    if slots < 2 || draft.seats.len() != slots {
        return Err("Seat count does not match the selected map".into());
    }
    if draft.return_row >= slots {
        return Err("Saved player return row is outside the map".into());
    }
    if draft.seats.iter().filter(|seat| seat.control == SeatControl::Mcp).count() > 1 {
        return Err("Instant Action settings can reserve one MCP slot".into());
    }
    let humans = draft
        .seats
        .iter()
        .enumerate()
        .filter(|(_, s)| s.control == SeatControl::Human)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if humans.len() != usize::from(!draft.observing)
        || (!draft.observing && humans[0] != draft.return_row)
    {
        return Err("Saved local player and observer mode disagree".into());
    }
    let races = data.races();
    if races.is_empty()
        || draft.seats.iter().any(|s| {
            s.team as usize >= slots
                || (s.race != RANDOM_SIDE && !races.iter().any(|r| r.eq_ignore_ascii_case(&s.race)))
        })
    {
        return Err("Saved race or team is not available for this map".into());
    }
    let limits = &data.content.rules().officer_limits;
    if (!limits.is_empty() && draft.options.officer_limit as usize >= limits.len())
        || (limits.is_empty() && draft.options.officer_limit != 0)
    {
        return Err("Saved officer limit is not available in the mounted content".into());
    }
    if draft
        .colour
        .is_some_and(|n| n >= data.colours.swatches().len())
    {
        return Err("Saved colour is not available in the mounted palette".into());
    }
    match (draft.random_placement, draft.start_markers) {
        (
            false,
            SetupStartMarkers::All | SetupStartMarkers::Available | SetupStartMarkers::Allies,
        )
        | (true, SetupStartMarkers::All | SetupStartMarkers::None) => {}
        _ => return Err("Saved start marker mode does not match placement mode".into()),
    }
    Ok(())
}

fn speed_key(speed: MatchSpeed) -> &'static str {
    match speed {
        MatchSpeed::Slow => "slow",
        MatchSpeed::Normal => "normal",
        MatchSpeed::Fast => "fast",
    }
}

fn resources_key(resources: StartingResources) -> &'static str {
    match resources {
        StartingResources::Normal => "normal",
        StartingResources::Lots => "lots",
    }
}

fn boolean(value: &str, key: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("Invalid boolean for '{key}'")),
    }
}

fn number<T: std::str::FromStr>(value: &str, key: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("Invalid number for '{key}'"))
}

// Keep the file ASCII and escape line breaks, controls and literal percent signs.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte == b'%' || !(0x20..0x7f).contains(&byte) {
            out.push_str(&format!("%{byte:02X}"));
        } else {
            out.push(char::from(byte));
        }
    }
    out
}

fn decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .ok_or("Invalid percent escape in Instant Action settings")?;
            let digit = |b: u8| {
                (b as char)
                    .to_digit(16)
                    .map(|n| n as u8)
                    .ok_or("Invalid percent escape in Instant Action settings")
            };
            out.push((digit(hex[0])? << 4) | digit(hex[1])?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "Invalid UTF-8 in Instant Action setting".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sta64-preset-{name}-{}-{:?}.cfg",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn round_trip_uses_map_identity_and_all_choices() {
        let mut data = ScreenData::fake();
        let mut draft = SetupDraft::new(&data);
        draft.set_map(&data, 1);
        draft.seed = 42;
        draft.set_observing(true);
        draft.seats[0].control = SeatControl::Mcp;
        draft.seats[1].control = SeatControl::Easy;
        draft.seats[1].race = "fx".into();
        draft.seats[2].control = SeatControl::Hard;
        draft.seats[3].race = RANDOM_SIDE.into();
        draft.seats[2].team = 0;
        draft.speed = MatchSpeed::Fast;
        draft.options.starting_resources = StartingResources::Lots;
        draft.colour = Some(0);
        draft.fog = SetupFog::ShroudOnly;
        draft.random_placement = true;
        draft.preview_visible = false;
        draft.start_markers = SetupStartMarkers::None;
        let file = path("roundtrip");
        save_at(&file, &draft, &data).unwrap();
        data.maps.reverse();
        let loaded = load_at(&file, &data).unwrap();
        assert_eq!(loaded.map, 0);
        assert_eq!(loaded.seats, draft.seats);
        assert_eq!(loaded.seed, 42);
        assert_eq!(loaded.speed, MatchSpeed::Fast);
        assert_eq!(loaded.options, draft.options);
        assert!(loaded.observing);
        assert_eq!(loaded.colour, Some(0));
        assert_eq!(loaded.fog, SetupFog::ShroudOnly);
        assert!(loaded.random_placement);
        assert!(!loaded.preview_visible);
        assert_eq!(loaded.start_markers, SetupStartMarkers::None);
        std::fs::remove_file(file).unwrap();
    }

    /// The match option "warp allowed" is not part of a preset (INTERIM, a TODO line): a preset made before the
    /// option existed, and one saved with it off, load with it on.
    #[test]
    fn a_preset_loads_with_warp_allowed_on_whatever_was_chosen() {
        let data = ScreenData::fake();
        let file = path("warp-option");
        let mut draft = SetupDraft::new(&data);
        draft.options.allow_warp = false;
        save_at(&file, &draft, &data).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("warp"), "the option is not written");
        assert!(load_at(&file, &data).unwrap().options.allow_warp, "a preset without the option loads it on");
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn old_preset_defaults_directors_cut_off_and_invalid_value_is_rejected() {
        let mut data = ScreenData::fake();
        data.races = std::sync::Arc::new(sta_content::Races::from_defs(vec![sta_content::RaceDef {
            stem: "fx".into(),
            name: "fx".into(),
            display_key: None,
            slot: Some(0),
            interface: None,
            interface_sprites: None,
            single_player_music: None,
            multiplayer_music: None,
            music_tracks: Vec::new(),
            standard_units: Vec::new(),
            minimal_units: Vec::new(),
            stock: [0; 4],
            lots_stock: [0; 4],
            normal_crew: None,
            lots_crew: None,
        }]));
        let other_profile = data.clone();
        data.content = std::sync::Arc::new(sta_content::ContentDb::builder()
            .profile(sta_content::Profile::Sta1Classic).build().unwrap());
        let file = path("directors-legacy");
        let mut draft = SetupDraft::new(&data);
        draft.options.directors_cut = true;
        save_at(&file, &draft, &data).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(load_at(&file, &data).unwrap().options.directors_cut);
        assert!(!load_at(&file, &other_profile).unwrap().options.directors_cut);
        std::fs::write(&file, text.replace("directors_cut=true\n", "")).unwrap();
        assert!(!load_at(&file, &data).unwrap().options.directors_cut);
        std::fs::write(&file, text.replace("directors_cut=true", "directors_cut=maybe")).unwrap();
        assert!(load_at(&file, &data).unwrap_err().contains("directors_cut"));
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn invalid_file_and_missing_content_do_not_return_partial_draft() {
        let data = ScreenData::fake();
        let file = path("invalid");
        let draft = SetupDraft::new(&data);
        save_at(&file, &draft, &data).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, text.replace("seat.0.team=0", "seat.0.team=999")).unwrap();
        assert!(load_at(&file, &data).unwrap_err().contains("team"));
        std::fs::write(&file, text.replace("map=", "map=missing-")).unwrap();
        assert!(load_at(&file, &data).unwrap_err().contains("map"));
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn invalid_draft_does_not_replace_last_saved_setup() {
        let data = ScreenData::fake();
        let file = path("preserve");
        let mut draft = SetupDraft::new(&data);
        save_at(&file, &draft, &data).unwrap();
        let before = std::fs::read(&file).unwrap();
        draft.seats[0].race = "missing-race".into();
        assert!(save_at(&file, &draft, &data).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::remove_file(file).unwrap();
    }
}
