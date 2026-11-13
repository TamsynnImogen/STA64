//! Opt-in startup sweep of external clean-room A2 ports against installed maps.
//! No installed or port bytes are bundled in this repository.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_core::{PlayerId, TeamId};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_mission::{Labels, Resolution};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

const DEFAULT_STARTUP_TICKS: u32 = 30;
const MAX_STARTUP_TICKS: u32 = 36_000;

fn parse_startup_horizon(value: Option<OsString>) -> Result<u32, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_STARTUP_TICKS);
    };
    let value = value
        .into_string()
        .map_err(|_| "must be Unicode decimal digits".to_owned())?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must be a non-empty unsigned decimal integer".into());
    }
    let ticks = value
        .parse::<u32>()
        .map_err(|_| "must fit in an unsigned 32-bit integer".to_owned())?;
    if !(1..=MAX_STARTUP_TICKS).contains(&ticks) {
        return Err(format!("must be in 1..={MAX_STARTUP_TICKS}"));
    }
    Ok(ticks)
}

#[test]
fn startup_horizon_parser_accepts_bounds_and_uses_explicit_default() {
    for ticks in [1, 30, 600, 1800, 36_000] {
        assert_eq!(
            parse_startup_horizon(Some(OsString::from(ticks.to_string()))).unwrap(),
            ticks
        );
    }
    assert_eq!(parse_startup_horizon(None).unwrap(), 30);
}

#[test]
fn startup_horizon_parser_rejects_invalid_values_clearly() {
    for value in ["0", "36001", "-1", "1.5", "", " ", " 30", "4294967296"] {
        assert!(
            parse_startup_horizon(Some(OsString::from(value))).is_err(),
            "accepted {value:?}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let non_unicode = OsString::from_vec(vec![0xff]);
        assert_eq!(
            parse_startup_horizon(Some(non_unicode)).unwrap_err(),
            "must be Unicode decimal digits"
        );
    }
}

fn header_map(bytes: &[u8]) -> Result<String, String> {
    let header = String::from_utf8_lossy(bytes);
    let names: BTreeSet<String> = header
        .lines()
        .take(8)
        .flat_map(|line| line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')))
        .map(|word| word.trim_end_matches('.'))
        .filter(|word| {
            word.len() > 7
                && word.get(..3).is_some_and(|prefix| prefix.eq_ignore_ascii_case("a2_"))
                && word
                    .get(word.len() - 4..)
                    .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".bzn"))
        })
        .map(str::to_ascii_lowercase)
        .collect();
    if names.len() == 1 {
        Ok(names.into_iter().next().unwrap())
    } else {
        Err(format!(
            "expected one distinct a2_*.bzn token in first eight lines, found {names:?}"
        ))
    }
}

#[test]
fn header_map_accepts_sentence_punctuation_and_rejects_ambiguous_metadata() {
    assert_eq!(
        header_map(b"-- Evidence: a2_example01.bzn.\n-- Same: A2_EXAMPLE01.BZN").unwrap(),
        "a2_example01.bzn"
    );
    assert!(header_map(b"-- a2_example01.bzn and a2_example02.bzn.").is_err());
    assert!(header_map(b"-- No map metadata").is_err());
}

#[test]
#[ignore = "requires STA64_A2_INSTALL and STA64_A2_PORTS external directories"]
fn external_a2_campaign_ports_start_on_installed_maps() {
    let startup_ticks = parse_startup_horizon(std::env::var_os("STA64_A2_STARTUP_TICKS"))
        .unwrap_or_else(|error| panic!("invalid STA64_A2_STARTUP_TICKS: {error}"));
    eprintln!("A2 startup configuration: horizon_ticks={startup_ticks}");
    let install = PathBuf::from(std::env::var("STA64_A2_INSTALL").expect("set STA64_A2_INSTALL"));
    let ports = PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let mod_name = std::env::var("STA64_A2_MOD").unwrap_or_else(|_| "STA2 Classic".into());
    let mut vfs = Vfs::mount_install(Path::new(&install), Some(&mod_name)).expect("mount installed game");
    // Index installed content before the external Lua directory is made visible.
    let (index, index_warnings) = OdfIndex::build(&vfs);
    let (db, content_warnings) = ContentDb::load(&vfs, &index).expect("load installed content");
    eprintln!(
        "A2 startup: horizon_ticks={startup_ticks} profile={:?} content_hash={:#018x} index_warnings={} content_warnings={}",
        db.profile(),
        db.hash(),
        index_warnings.len(),
        content_warnings.len()
    );
    let package = std::env::var_os("STA64_A2_PACKAGE").map(PathBuf::from);
    if let Some(path) = &package {
        vfs.push_dir("staged A2 port aliases".into(), path.clone())
            .expect("mount staged aliases");
    } else {
        vfs.push_dir(
            "external A2 ports".into(),
            ports.parent().expect("missions directory has parent").to_path_buf(),
        )
        .expect("mount lower-priority external ports");
    }
    let labels = Labels::load(&vfs);
    let vfs = Arc::new(vfs);
    let db = Arc::new(db);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&ports)
        .expect("list external ports")
        .map(|entry| entry.expect("read external port directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lua")))
        .collect();
    files.sort_by_key(|path| path.file_name().unwrap().to_string_lossy().to_ascii_lowercase());

    let mut failures = Vec::new();
    let mut ran = 0;
    let mut resolved = 0;
    let mut packaging = Vec::new();
    for file in &files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let result = (|| -> Result<(), String> {
            let source = std::fs::read(file).map_err(|e| format!("read port: {e}"))?;
            let bzn_name = header_map(&source)?;
            let bzn_path = format!("bzn/{bzn_name}");
            let mission_path = format!("missions/{name}");
            let mounted = vfs
                .read(&mission_path)
                .ok_or_else(|| format!("VFS has no {mission_path}"))?
                .map_err(|e| format!("read VFS mission: {e}"))?;
            if mounted != source {
                return Err(format!(
                    "VFS mission bytes differ from external {name}; installed shadow or wrong mount"
                ));
            }
            let bzn_bytes = vfs
                .read(&bzn_path)
                .ok_or_else(|| format!("installed map missing: {bzn_path}"))?
                .map_err(|e| format!("read installed map: {e}"))?;
            let bzn = sta_bzn::parse(&bzn_bytes).map_err(|e| format!("parse {bzn_path}: {e}"))?;
            let resolution = sta_mission::resolve(&vfs, &labels, &bzn_path, &bzn);
            let matched = match &resolution {
                Resolution::Lua { path, .. } => {
                    vfs.read(path)
                        .ok_or_else(|| format!("resolved Lua path is missing: {path}"))?
                        .map_err(|e| format!("read resolved Lua path: {e}"))?
                        == source
                }
                _ => false,
            };
            if matched {
                resolved += 1;
            } else {
                packaging.push(format!("{name}: {bzn_path} resolves to {resolution:?}"));
            }
            let (map, map_warnings) = MapDef::from_bzn_in(&bzn, &db, &vfs, &index);
            let players = (0..8)
                .map(|id| {
                    let race = map
                        .objects
                        .iter()
                        .filter(|object| object.owner == Some(id))
                        .filter_map(|object| db.key(&object.odf).and_then(|key| db.unit(key)))
                        .map(|unit| unit.race.as_str())
                        .find(|race| !race.is_empty())
                        .unwrap_or("")
                        .to_owned();
                    PlayerSetup {
                        id: PlayerId(id),
                        team: TeamId(id),
                        race,
                        control: Control::Idle,
                    }
                })
                .collect();
            let setup = GameSetup {
                seed: 1,
                map: Arc::new(map),
                content: db.clone(),
                players,
                options: GameOptions::default(),
            };
            let (mut sim, spawn_warnings) = Sim::from_setup_with_warnings(&setup);
            let cfg = MissionConfig {
                seat: Some(PlayerId(0)),
                player_numbering: PlayerNumbering::ArmadaTeams,
                content: Some(db.clone()),
                labels: sim.labels.iter().map(|(id, label)| (id, label.clone())).collect(),
                view: Some((*sim.view()).clone()),
                paths: Vec::new(),
            };
            let startup_path = match &resolution {
                Resolution::Lua { path, .. } if matched => path.as_str(),
                _ => mission_path.as_str(),
            };
            let (layer, _) = vfs
                .resolve(startup_path)
                .ok_or_else(|| "startup path has no VFS layer".to_owned())?;
            let loaded_layer = vfs.layers()[layer].name();
            if package.is_some() && loaded_layer != "staged A2 port aliases" {
                return Err(format!(
                    "startup unexpectedly loaded from {loaded_layer}, not the staged package"
                ));
            }
            let mut runtime = MissionRuntime::from_vfs_with(vfs.clone(), startup_path, 1, cfg);
            let mut events = Vec::new();
            let mut rejected = Vec::new();
            for tick in 0..startup_ticks {
                let mut commands = Vec::new();
                runtime.think(&sim.view(), &events, &mut commands);
                let output = sim.step(&commands);
                rejected.extend(output.events.iter().filter_map(|event| {
                    match event {
                        SimEvent::Rejected { player, seq } => {
                            let command = commands
                                .iter()
                                .find(|c| c.player == *player && c.seq == *seq);
                            if let Some(CommandKind::Edit(Edit::Team(id, _))) = command.map(|c| &c.kind) {
                                // Diagnostic only: this is the state AFTER the step,
                                // not a claim about the precise rejection branch.
                                let target = sim.unit(*id).map(|u| (
                                    u.kind, u.owner, u.hull, u.flags, u.progress,
                                    db.unit(u.def).map(|d| (&d.odf, &d.class, d.resource)),
                                ));
                                eprintln!("A2 startup {name}: rejected Team tick={tick} target_after_step={target:?}");
                            }
                            Some((tick, player.0, *seq, command.map(|c| format!("{:?}", c.kind))))
                        }
                        _ => None,
                    }
                }));
                events = output.events;
            }
            ran += 1;
            let unsupported = runtime.unsupported();
            let handled_logs: Vec<_> = runtime
                .log()
                .into_iter()
                .filter(|line| {
                    let lower = line.to_ascii_lowercase();
                    ["could not", "failed", "missing"]
                        .iter()
                        .any(|word| lower.contains(word))
                })
                .collect();
            let handled_perception_errors = handled_logs
                .iter()
                .filter(|line| line.to_ascii_lowercase().contains("perceivedteam"))
                .count();
            eprintln!(
                "A2 startup {name}: horizon_ticks={startup_ticks} map={bzn_name} loaded_layer={loaded_layer:?} objects={} regions={} warnings(map/spawn)={}/{} resolve={resolution:?} errors={} objectives={} finished={} unsupported={unsupported:?} rejected={rejected:?} handled_log_count={} handled_perception_errors={handled_perception_errors} handled_log_samples={:?}",
                setup.map.objects.len(),
                setup.map.regions.len(),
                map_warnings.len(),
                spawn_warnings.len(),
                runtime.errors().len(),
                runtime.objectives().len(),
                runtime.finished(),
                handled_logs.len(),
                handled_logs.iter().take(3).collect::<Vec<_>>()
            );
            if !runtime.errors().is_empty() {
                failures.push(format!("{name}: Lua errors: {:?}", runtime.errors()));
            }
            if !rejected.is_empty() {
                failures.push(format!("{name}: rejected commands: {rejected:?}"));
            }
            if handled_perception_errors != 0 {
                failures.push(format!("{name}: {handled_perception_errors} handled perceivedTeam errors"));
            }
            Ok(())
        })();
        if let Err(error) = result {
            failures.push(format!("{name}: {error}"));
        }
    }
    eprintln!(
        "A2 startup summary: horizon_ticks={startup_ticks} ports={} ran={ran} resolved={resolved} packaging={packaging:?} failures={failures:?}",
        files.len()
    );
    assert_eq!(files.len(), 36, "expected all 36 external clean-room ports");
    assert_eq!(
        ran, 36,
        "every external port must complete a {startup_ticks}-tick startup"
    );
    if package.is_some() {
        assert_eq!(resolved, 36, "staged aliases must resolve every intended port");
    }
    assert!(failures.is_empty(), "A2 startup failures: {failures:?}");
}
