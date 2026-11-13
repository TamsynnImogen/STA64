//! The referee (NET-05, NET_DECISIONS N5, docs/NET.md): a headless seatless
//! peer that joins a room, re-simulates the match from `Start` and every
//! seat's turn records, checks every seat's digests against its own, serves
//! the resync of a diverged seat, and writes the artefacts `sta-bench desync`
//! reads when it finds one.
//!
//! ```text
//! relay --listen 127.0.0.1:7864
//! netplay host --relay 127.0.0.1:7864 --peers 2 --referee --ticks 9000
//! netplay join --relay 127.0.0.1:7864 --code K7M-Q2X --ticks 9000
//! referee --relay 127.0.0.1:7864 --code K7M-Q2X --out ./referee-out
//! ```
//!
//! Start it after the players have joined: a connection takes the lowest free
//! relay seat, and the players' seats come first. It prints `SEATED`, one
//! `VERDICT` line per diverging seat, and at the end
//! `DONE tick=<n> digest=<hex> diverged=<seats>`. Exit status: 0 the match was
//! verified (no seat differed), 3 a seat differed (reports written), 1 error.
//!
//! The content is the test fixture (`--content fake`), like `netplay`'s: the
//! game's own referee entry point hands `Referee::new` the real content and
//! map from the player's install (it must match the players' rules digest, or
//! the host refuses it with the differing key).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_core::PlayerId;
use sta_net::lockstep::Transport;
use sta_net::proto::{EngineId, Hello, RulesIdentity};
use sta_net::referee::{
    await_start, EndReason, Referee, RefereeLimits, RefereePolicy, RefereeStep,
};
use sta_net::transport::{ClientOptions, NetTransport, RoomCode};

fn die(msg: &str) -> ! {
    eprintln!("referee: {msg}");
    std::process::exit(1)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut relay, mut code, mut password) = (String::new(), String::new(), String::new());
    let mut out = PathBuf::from("referee-out");
    let mut host_seat = 0u8;
    let mut wait = 120u64;
    let mut limits = RefereeLimits::default();
    let mut policy = RefereePolicy::default();
    let mut engine = EngineId {
        name: "netplay".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        build: "dev".into(),
    };
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| die(&format!("{a} needs a value")));
        let num = |a: &str, v: String| -> u64 { v.parse().unwrap_or_else(|_| die(&format!("{a}: not a number"))) };
        match a.as_str() {
            "--relay" => relay = v(),
            "--code" => code = v(),
            "--password" => password = v(),
            "--out" => out = PathBuf::from(v()),
            "--host-seat" => host_seat = num(&a, v()) as u8,
            "--wait-seconds" => wait = num(&a, v()),
            "--idle-seconds" => limits.idle = Duration::from_secs(num(&a, v())),
            "--max-ticks" => limits.max_ticks = num(&a, v()),
            "--max-commands" => limits.max_logged_commands = num(&a, v()) as usize,
            "--resync-host" => policy.resync_host = true,
            "--engine-name" => engine.name = v(),
            "--engine-version" => engine.version = v(),
            "--engine-build" => engine.build = v(),
            "--content" => {
                if v() != "fake" {
                    die("--content: only `fake` is built in; the game supplies real content");
                }
            }
            "--help" | "-h" => {
                println!(
                    "usage: referee --relay ADDR --code CODE [--password P] [--out DIR] \
                     [--host-seat N] [--wait-seconds N] [--idle-seconds N] [--max-ticks N] \
                     [--max-commands N] [--resync-host] [--engine-name S --engine-version S \
                     --engine-build S] [--content fake]"
                );
                return;
            }
            other => die(&format!("unknown argument {other}")),
        }
    }
    if relay.is_empty() {
        die("--relay ADDR is required");
    }
    let code = RoomCode::parse(&code).unwrap_or_else(|| die("--code is not a room code"));
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let rules: &dyn RulesIdentity = db.digest();
    let mut t = NetTransport::join(relay.as_str(), code, &password, &ClientOptions::default())
        .unwrap_or_else(|e| die(&e.to_string()));
    let me = t.seat();
    let hello = Hello {
        engine,
        rules: rules.digest(),
        lineage: db.lineage().to_owned(),
        packs: vec![],
        name: "referee".into(),
    };
    let start = await_start(&mut t, PlayerId(host_seat), hello, Duration::from_secs(wait))
        .unwrap_or_else(|e| die(&e.to_string()));
    println!("JOINED slot {}", me.0);
    let control = t.control();
    let mut referee = Referee::new(&start, me, db.clone(), map, Box::new(t) as Box<dyn Transport>, policy, limits)
        .unwrap_or_else(|e| die(&e.to_string()));
    let mut announced = 0usize;
    let mut seated = false;
    let end = loop {
        let now = Instant::now();
        match referee.step_at(now) {
            RefereeStep::Ran(_) => {}
            RefereeStep::Stalled => std::thread::sleep(Duration::from_millis(1)),
            RefereeStep::Ended(why) => break why,
        }
        if !seated && referee.peer().router().seated() {
            seated = true;
            println!("SEATED");
        }
        let findings = referee.findings();
        for f in findings.iter().skip(announced) {
            println!("VERDICT {}", f.line());
        }
        announced = findings.len();
        if referee.report_due(now) {
            write_report(&mut referee, &out);
        }
    };
    let findings = referee.findings();
    if !findings.is_empty() {
        write_report(&mut referee, &out);
    }
    for f in findings.iter().skip(announced) {
        println!("VERDICT {}", f.line());
    }
    let diverged: Vec<String> = findings.iter().map(|f| f.seat.0.to_string()).collect();
    println!(
        "DONE tick={} digest={:016x} ended={} diverged=[{}]",
        referee.peer().tick(),
        referee.peer().sim().digest(),
        match end {
            EndReason::Closed => "closed",
            EndReason::Idle => "idle",
            EndReason::TooLong => "too-long",
            EndReason::LogFull => "log-full",
        },
        diverged.join(",")
    );
    control.leave();
    std::process::exit(if findings.is_empty() { 0 } else { 3 });
}

fn write_report(referee: &mut Referee, out: &PathBuf) {
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("referee: cannot create {}: {e}", out.display());
        return;
    }
    match referee.write_report(out, "referee") {
        Ok(f) => println!("REPORT {} {}", f.replay.display(), f.sidecar.display()),
        Err(e) => eprintln!("referee: cannot write the report: {e}"),
    }
}
