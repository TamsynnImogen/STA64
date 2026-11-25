//! Fight and build check for the alpha testbed factions (TOOL-21,
//! docs/ALPHA_TESTBED.md). Headless sim only; no window, no renderer.
//!
//! `cargo run -p sta_bench --example testbed_fight -- <root> <mod> [map]`
//!
//! For every playable race (seat 0) against the next one (seat 1), on `map`
//! (default `tb_empty`):
//! - **fight**: each armed mobile unit (a weapon that attacks units: not only a
//!   mining beam) is spawned alone 500 units from an
//!   enemy mining freighter, ordered to attack it, and stepped 30 s. It passes when the
//!   target's shields or hull went down.
//! - **build**: each builder (a unit with build items) is spawned with every
//!   station of its race already standing (so tech-tree prerequisites hold) and
//!   a large stock; each build item is ordered in turn and must appear within
//!   twice its build time plus 30 s.
//!
//! Exit 1 if any unit fails.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, ContentKey, MapDef, OdfIndex, Races};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

const TICKS_PER_SECOND: u64 = 30;

struct Ctx {
    db: Arc<ContentDb>,
    map: Arc<MapDef>,
}

impl Ctx {
    fn sim(&self, races: [&str; 2]) -> Sim {
        let players = races
            .iter()
            .enumerate()
            .map(|(i, r)| PlayerSetup { id: PlayerId(i as u8), team: TeamId(i as u8), race: (*r).to_owned(), control: Control::Idle })
            .collect();
        let setup = GameSetup { seed: 1, map: Arc::clone(&self.map), content: Arc::clone(&self.db), players, options: GameOptions::default() };
        Sim::from_setup(&setup)
    }

    fn units_of(&self, race: &str) -> Vec<(ContentKey, String)> {
        self.db
            .units()
            .filter(|u| u.race.eq_ignore_ascii_case(race))
            .filter_map(|u| Some((self.db.key(&u.odf)?, u.odf.clone())))
            .collect()
    }
}

fn at(x: f32, z: f32) -> Vec3 {
    Vec3::from_f32([x, 0.0, z])
}

fn order(player: u8, seq: u32, o: Order) -> Command {
    Command { player: PlayerId(player), seq, kind: CommandKind::Order(o) }
}

fn pool(sim: &Sim, id: EntityId) -> Option<f32> {
    let v = sim.view();
    let u = v.units.iter().find(|u| u.id == id)?;
    Some(u.hull[0].to_f32() + u.shields.iter().map(|s| s[0].to_f32()).fold(0.0, f32::max))
}

fn fight(ctx: &Ctx, races: [&str; 2], unit: ContentKey) -> Result<String, String> {
    let mut sim = ctx.sim(races);
    let target_def = ctx.units_of(races[1]).into_iter().find(|(_, n)| n.ends_with("_miner")).ok_or("enemy has no miner")?.0;
    let target = sim.spawn_def(target_def, PlayerId(1), at(6500.0, 6000.0), (Fixed::from_int(-1), Fixed::ZERO)).ok_or("target did not spawn")?;
    let me = sim.spawn_def(unit, PlayerId(0), at(6000.0, 6000.0), (Fixed::ONE, Fixed::ZERO)).ok_or("did not spawn")?;
    let before = pool(&sim, target).ok_or("target missing")?;
    sim.step(&[order(0, 1, Order::Attack { units: vec![me], target })]);
    for _ in 0..30 * TICKS_PER_SECOND {
        sim.step(&[]);
    }
    match pool(&sim, target) {
        None => Ok("destroyed the target".into()),
        Some(after) if after < before - 0.5 => Ok(format!("target {before:.0} -> {after:.0}")),
        Some(_) => Err(format!("no damage in 30 s (target {before:.0})")),
    }
}

fn build(ctx: &Ctx, races: [&str; 2], builder: ContentKey) -> Vec<(String, Result<String, String>)> {
    let db = &ctx.db;
    let Some(def) = db.unit(builder) else { return vec![] };
    let items: Vec<ContentKey> = def.build.list.iter().filter_map(|r| if r.key != ContentKey::NONE { Some(r.key) } else { db.key(&r.odf) }).collect();
    let mut out = Vec::new();
    for item in items {
        let name = db.name(item).unwrap_or("?").to_owned();
        let mut sim = ctx.sim(races);
        // every station of the race stands far away, so prerequisites hold
        let mut x = 1000.0;
        for (k, _) in ctx.units_of(races[0]) {
            if db.unit(k).is_some_and(|u| u.movement.max_speed == Fixed::ZERO) {
                sim.spawn_def(k, PlayerId(0), at(x, 1000.0), (Fixed::ONE, Fixed::ZERO));
                x += 700.0;
            }
        }
        let Some(b) = sim.spawn_def(builder, PlayerId(0), at(6000.0, 6000.0), (Fixed::ONE, Fixed::ZERO)) else {
            out.push((name, Err("builder did not spawn".into())));
            continue;
        };
        let stock: Vec<Command> = (0..4u8)
            .map(|r| Command { player: PlayerId::BENCH, seq: u32::from(r) + 1, kind: CommandKind::Edit(sta_sim::command::Edit::Cargo(PlayerId(0), r, Fixed::from_int(100_000))) })
            .collect();
        sim.step(&stock);
        let mobile_builder = def.movement.max_speed != Fixed::ZERO;
        let place = mobile_builder.then(|| at(6000.0, 7500.0));
        let before = sim.view().units.iter().filter(|u| u.def == item && u.owner == PlayerId(0)).count();
        sim.step(&[order(0, 1, Order::Build { builder: b, what: item, at: place })]);
        let secs = db.unit(item).map(|u| u.build.ticks as f32 / TICKS_PER_SECOND as f32).unwrap_or(60.0);
        let limit = ((secs * 2.0 + 30.0) * TICKS_PER_SECOND as f32) as u64;
        let mut done = None;
        for t in 0..limit {
            sim.step(&[]);
            let v = sim.view();
            if v.units.iter().filter(|u| u.def == item && u.owner == PlayerId(0) && u.progress.is_none()).count() > before {
                done = Some(t);
                break;
            }
        }
        out.push((name, done.map(|t| format!("built in {:.0} s", t as f32 / TICKS_PER_SECOND as f32)).ok_or_else(|| format!("not built within {:.0} s", limit / TICKS_PER_SECOND))));
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(m)) = (args.first(), args.get(1)) else {
        eprintln!("usage: testbed_fight <root> <mod> [map]");
        std::process::exit(2);
    };
    let map_name = args.get(2).map(String::as_str).unwrap_or("tb_empty");
    let vfs = Vfs::mount_install(Path::new(root), Some(m)).expect("mount");
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).expect("content");
    let db = Arc::new(db);
    let (races, _) = Races::load(&vfs, &index);
    let bytes = vfs.read(&format!("bzn/{map_name}.bzn")).and_then(Result::ok).expect("map");
    let bzn = sta_bzn::parse(&bytes).expect("bzn");
    let map = Arc::new(MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map);
    let ctx = Ctx { db: Arc::clone(&db), map };

    let names = sta_content::race::skirmish_races(&races, &db);
    let mut failed = 0;
    for (i, race) in names.iter().enumerate() {
        let enemy = &names[(i + 1) % names.len()];
        let pair = [race.as_str(), enemy.as_str()];
        println!("== {race} vs {enemy}");
        for (key, name) in ctx.units_of(race) {
            let Some(u) = db.unit(key) else { continue };
            // Armed: a weapon that takes attack orders (not a mining or tractor beam, point defence or special).
            let armed = u.hardpoints.iter().any(|h| db.weapon(h.weapon.key).is_some_and(sta_sim::combat::weapons::attacks_units));
            if armed && u.movement.max_speed != Fixed::ZERO {
                let r = fight(&ctx, pair, key);
                failed += usize::from(r.is_err());
                println!("fight {name:22} {}", r.unwrap_or_else(|e| format!("FAIL {e}")));
            }
            if !u.build.list.is_empty() {
                for (item, r) in build(&ctx, pair, key) {
                    failed += usize::from(r.is_err());
                    println!("build {name:22} -> {item:22} {}", r.unwrap_or_else(|e| format!("FAIL {e}")));
                }
            }
        }
    }
    if failed > 0 {
        eprintln!("FAIL: {failed}");
        std::process::exit(1);
    }
    println!("OK");
}
