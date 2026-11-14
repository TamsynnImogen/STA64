//! Windowless tests of the lobby (UI-11). Everything runs over an in-process relay on
//! 127.0.0.1 with an OS-chosen port (no external network); every wait is a loop on a real
//! condition with a deadline, never a fixed sleep.
//!
//! - [`Session`] level: two and three clients host, join, change seats, launch, and play
//!   through the lockstep router with equal digests; each refusal, kick, host leaving, a
//!   content mismatch with the differing key named.
//! - Screen level: two `App`s click through the Multiplayer, Host, Join and Lobby screens;
//!   layout in every window shape and UI scale.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::window::{PrimaryWindow, WindowResolution};
use sta_content::{ContentDb, RaceDef, Races};
use sta_core::{Fixed, PlayerId, Vec3};
use sta_net::lockstep::{Peer, StepOutcome};
use sta_net::proto::Refusal;
use sta_net::transport::{ClientOptions, ConnectError, NetTransport, RefuseReason, Relay, RelayConfig, RoomCode};
use sta_sim::WorldSource;
use sta_sim::command::{Command, CommandKind, Edit};

use super::layout::{self, MSlot, region};
use super::view::{DropCol, LobbyDrop, LobbyField, MpAct, Placed};
use super::*;
use crate::app::{AppState, configure_sets};
use crate::frontend::screens::stock::layout::fit_box;
use crate::frontend::screens::{Screen, ScreensConfig, ScreensPlugin};
use crate::ui::theme::{DropdownOption, KitFocus, KitKey, Theme, click, kit_click, kit_key};

pub(crate) const DEADLINE: Duration = Duration::from_secs(20);

fn race(name: &str, slot: i64) -> RaceDef {
    RaceDef {
        stem: name.into(), name: name.into(), display_key: None, slot: Some(slot), interface: None, interface_sprites: None,
        single_player_music: None, multiplayer_music: None,
        standard_units: Vec::new(), minimal_units: Vec::new(), stock: [0; 4],
        lots_stock: [0; 4], normal_crew: None, lots_crew: None,
    }
}

/// The fake install with four playable sides (a lobby needs sides to choose from).
fn data() -> ScreenData {
    let mut d = ScreenData::fake();
    d.races = Arc::new(Races::from_defs(vec![race("fed", 0), race("kli", 1), race("rom", 2), race("bor", 3)]));
    d
}

pub(crate) fn relay() -> Relay {
    Relay::bind("127.0.0.1:0", RelayConfig::default()).expect("a relay on an OS-chosen port")
}

fn cfg(name: &str) -> SessionCfg {
    let mut c = SessionCfg::from_data(&data(), name);
    c.seed = Some(0x5eed);
    c
}

fn opts() -> ClientOptions {
    ClientOptions::default()
}

fn host_on(r: &Relay, password: &str, name: &str) -> Session {
    connect_host(&r.local_addr().to_string(), password, cfg(name), &opts()).expect("host")
}

fn join_on(r: &Relay, code: RoomCode, password: &str, c: SessionCfg) -> Result<Session, ConnectError> {
    connect_guest(&r.local_addr().to_string(), code, password, c, &opts())
}

/// Update every session until `cond` holds, or fail at the deadline.
pub(crate) fn pump(sessions: &mut [&mut Session], what: &str, cond: impl Fn(&[&mut Session]) -> bool) {
    let end = Instant::now() + DEADLINE;
    loop {
        for s in sessions.iter_mut() {
            s.update(Instant::now());
        }
        if cond(sessions) {
            return;
        }
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The table with each viewer's own marker removed, so two machines' views compare.
fn seats(s: &Session) -> Vec<SeatView> {
    s.view().seats.into_iter().map(|mut x| {
        x.me = false;
        x
    }).collect()
}

fn kinds(s: &Session) -> Vec<Kind> {
    s.view().seats.iter().map(|x| x.kind).collect()
}

fn find(s: &Session, slot: usize) -> SeatView {
    s.view().seats[slot].clone()
}

// ---------------------------------------------------------------------------
// The state machine
// ---------------------------------------------------------------------------

#[test]
fn computers_move_off_the_slots_joiners_take() {
    let plan = |kinds: &[Kind]| -> Vec<SlotPlan> { kinds.iter().enumerate().map(|(i, k)| SlotPlan { kind: *k, race: format!("r{i}"), team: i as u8 }).collect() };
    let p = plan(&[Kind::Human, Kind::Computer, Kind::Open, Kind::Open]);
    // Nobody joined: the plan stands.
    assert_eq!(place_computers_around_humans(&p, 0b1).iter().map(|x| x.kind).collect::<Vec<_>>(), [Kind::Human, Kind::Computer, Kind::Open, Kind::Open]);
    // A joiner lands on slot 1 (the lowest free): the computer moves to the highest open slot and keeps its side.
    let q = place_computers_around_humans(&p, 0b11);
    assert_eq!(q.iter().map(|x| x.kind).collect::<Vec<_>>(), [Kind::Human, Kind::Human, Kind::Open, Kind::Computer]);
    assert_eq!(q[3].race, "r1");
    // No room left: it is dropped, never doubled up.
    let full = place_computers_around_humans(&plan(&[Kind::Human, Kind::Computer, Kind::Open]), 0b111);
    assert_eq!(full.iter().filter(|x| x.kind == Kind::Computer).count(), 0);
    // Idle seats move the same way; a human on an open slot displaces nothing.
    let idle = place_computers_around_humans(&plan(&[Kind::Human, Kind::Idle, Kind::Open]), 0b11);
    assert_eq!(idle.iter().map(|x| x.kind).collect::<Vec<_>>(), [Kind::Human, Kind::Human, Kind::Idle]);
}

#[test]
fn words_and_addresses() {
    assert_eq!(normalize_relay(" relay.example "), Some("relay.example:7864".into()));
    assert_eq!(normalize_relay("127.0.0.1:9000"), Some("127.0.0.1:9000".into()));
    assert_eq!(normalize_relay("[::1]:9000"), Some("[::1]:9000".into()));
    assert_eq!(normalize_relay("::1"), Some("[::1]:7864".into()));
    assert_eq!(normalize_relay("  "), None);
    assert_eq!(clean_name("  Jean\u{7}-Luc  ", ""), "Jean-Luc");
    assert_eq!(clean_name("   ", "Player 2"), "Player 2");
    assert!(clean_name(&"x".repeat(100), "").len() <= 32);
    for (r, word) in [
        (RefuseReason::NoRoom, "no room with that code"),
        (RefuseReason::BadPassword, "password is wrong"),
        (RefuseReason::Full, "full"),
        (RefuseReason::Closed, "closed"),
        (RefuseReason::RateLimited, "Too many tries"),
    ] {
        assert!(refuse_reason_text(r).contains(word), "{r:?}: {}", refuse_reason_text(r));
    }
    assert!(Ended::ConnectionLost("the relay stopped answering".into()).text().contains("lost"));
    assert!(Ended::HostLeft.text().contains("host left"));
    assert!(Ended::NoAnswer.text().contains("did not answer"));
    assert!(rejoin_unavailable().contains("NET-04"));
}

/// One digest checkpoint every 30 ticks.
fn run_peers(peers: &mut [Peer], ticks: u64) -> Vec<Vec<(u64, u64)>> {
    let end = Instant::now() + Duration::from_secs(120);
    let mut logs = vec![Vec::new(); peers.len()];
    loop {
        let mut done = true;
        for (i, p) in peers.iter_mut().enumerate() {
            if p.tick() >= ticks {
                continue;
            }
            done = false;
            match p.step() {
                StepOutcome::Ran { .. } => {
                    if p.tick() % 30 == 0 {
                        logs[i].push((p.tick(), p.sim().digest()));
                    }
                }
                StepOutcome::Stalled { .. } => {}
                StepOutcome::Halted => panic!("seat {i} desynced at tick {}", p.tick()),
            }
        }
        if done {
            return logs;
        }
        assert!(Instant::now() < end, "the match stalled");
        std::thread::yield_now();
    }
}

/// The host spawns a few ships for each person, as `netplay` does, so the digests mean something.
fn seed_ships(peer: &mut Peer, db: &ContentDb, players: &[u8]) {
    let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
    let mut seq = 0;
    for &p in players {
        for k in 0..3 {
            peer.submit(Command {
                player: PlayerId::SCRIPT,
                seq,
                kind: CommandKind::Edit(Edit::Spawn {
                    odf: if k % 2 == 0 { cruiser } else { frigate },
                    player: PlayerId(p),
                    pos: Vec3::from_f32([-150.0 + 60.0 * f32::from(p), 0.0, (k * 70 - 100) as f32]),
                    heading: (Fixed::ZERO, Fixed::ONE),
                }),
            });
            seq += 1;
        }
    }
}

/// What every peer must agree on about a launched game.
fn assert_same_game(matches: &[&NetMatch]) {
    let first = matches[0];
    for m in &matches[1..] {
        assert_eq!(m.start, first.start, "every peer got the same Start");
        assert_eq!(m.setup.seed, first.setup.seed);
        assert_eq!(m.setup.map.name, first.setup.map.name);
        assert_eq!(m.setup.options, first.setup.options);
        let key = |m: &NetMatch| m.setup.players.iter().map(|p| (p.id, p.team, p.race.clone())).collect::<Vec<_>>();
        assert_eq!(key(m), key(first), "the same seats, teams and sides");
    }
}

#[test]
fn two_clients_host_join_choose_launch_and_play_in_step() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    let code = host.code();
    let mut guest = join_on(&r, code, "", cfg("Riker")).expect("join");
    pump(&mut [&mut host, &mut guest], "the guest seated on both screens", |s| {
        s[1].phase == Phase::Open && s[0].view().seats[1].kind == Kind::Human && s[1].view().seats.get(1).is_some_and(|x| x.kind == Kind::Human)
    });
    assert_eq!(find(&host, 1).name, "Riker");
    assert_eq!(seats(&host), seats(&guest), "both tables agree");
    assert_eq!(guest.view().seats.iter().filter(|x| x.me).count(), 1);
    assert!(guest.view().seats[1].me);

    // Seat changes travel both ways.
    guest.choose("kli", 1);
    host.set_slot_race(0, "rom");
    host.set_slot_team(0, 0);
    pump(&mut [&mut host, &mut guest], "side changes propagate", |s| find(s[0], 1).race == "kli" && find(s[1], 0).race == "rom" && find(s[0], 1).team == 1);
    assert_eq!(seats(&host), seats(&guest));
    // A side the install does not have is ignored.
    guest.choose("nonesuch", 1);

    // The host's settings reach the guest, and take a ready mark back.
    let mut o = host.view().options;
    o.starting_resources = sta_content::StartingResources::Lots;
    host.set_options(o);
    guest.set_ready(true);
    pump(&mut [&mut host, &mut guest], "options arrive and the ready mark lands", |s| {
        s[1].view().options.starting_resources == sta_content::StartingResources::Lots && find(s[0], 1).ready
    });
    assert_eq!(host.launch_block(), None);
    assert_eq!(find(&host, 1).race, "kli", "the refused side did not change the seat");

    // A change of the host's settings takes the ready marks back (the guest must agree to the new ones).
    host.set_options(GameOptions::default());
    assert!(!find(&host, 1).ready);
    assert!(host.launch_block().unwrap().contains("Riker"));
    pump(&mut [&mut host, &mut guest], "options change reaches the guest", |s| !find(s[1], 1).ready);
    guest.set_ready(true);
    pump(&mut [&mut host, &mut guest], "ready again", |s| find(s[0], 1).ready);

    host.launch().expect("launch");
    pump(&mut [&mut host, &mut guest], "the guest sees Start", |s| s[1].phase == Phase::Launched);
    assert_eq!(host.phase, Phase::Launched);
    let (mh, mg) = (host.take_match().unwrap(), guest.take_match().unwrap());
    assert_same_game(&[&mh, &mg]);
    assert_eq!((mh.me, mg.me), (PlayerId(0), PlayerId(1)));
    assert_eq!(mh.start.options.starting_resources, sta_content::StartingResources::Normal);
    assert_eq!(mh.start.seats.iter().map(|x| (x.slot.0, x.race.as_str(), x.team.0)).collect::<Vec<_>>(), [(0, "rom", 0), (1, "kli", 1)]);

    // The match runs through the lockstep router: equal digests tick for tick.
    let db = data().content.clone();
    let mut peers = vec![mh.into_peer().unwrap(), mg.into_peer().unwrap()];
    seed_ships(&mut peers[0], &db, &[0, 1]);
    let logs = run_peers(&mut peers, 240);
    assert_eq!(logs[0], logs[1], "digests differ");
    assert_eq!(logs[0].len(), 8);
    assert_eq!(peers[0].sim().digest(), peers[1].sim().digest());
    assert!(peers[0].sim().view().units.len() >= 6, "the host's ships exist");
}

#[test]
fn three_clients_with_a_computer_seat_start_the_same_game() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    host.set_map(1).expect("the four-seat map");
    let code = host.code();
    let mut a = join_on(&r, code, "", cfg("Riker")).unwrap();
    let mut b = join_on(&r, code, "", cfg("Data")).unwrap();
    // The host plans a computer on slot 2, where a joiner may land: the relay seats people lowest first.
    host.set_slot_kind(2, Kind::Computer);
    host.set_slot_kind(3, Kind::Computer);
    pump(&mut [&mut host, &mut a, &mut b], "everyone seated", |s| {
        s[1].phase == Phase::Open && s[2].phase == Phase::Open && kinds(s[0]).iter().filter(|k| **k == Kind::Human).count() == 3 && seats(s[1]) == seats(s[0]) && seats(s[2]) == seats(s[0])
    });
    // Two joiners took slots 1 and 2; the computer that was planned on 2 moved to the free slot 3 and the other one found no room.
    assert_eq!(kinds(&host), [Kind::Human, Kind::Human, Kind::Human, Kind::Computer]);
    // Each player's choices reach the other two.
    a.choose("bor", 0);
    b.choose("rom", 2);
    host.set_slot_race(3, "kli");
    pump(&mut [&mut host, &mut a, &mut b], "choices reach everyone", |s| {
        s.iter().all(|x| find(x, 1).race == "bor" && find(x, 2).race == "rom" && find(x, 2).team == 2 && find(x, 3).race == "kli")
    });
    // Seats that do not exist cannot be edited by others: a joiner cannot set the host's seat.
    a.set_slot_race(0, "bor");
    assert_ne!(find(&host, 0).race, "bor");
    a.set_ready(true);
    b.set_ready(true);
    pump(&mut [&mut host, &mut a, &mut b], "all ready", |s| s[0].launch_block().is_none());
    host.launch().expect("launch");
    pump(&mut [&mut host, &mut a, &mut b], "start reaches both guests", |s| s[1].phase == Phase::Launched && s[2].phase == Phase::Launched);
    let (mh, ma, mb) = (host.take_match().unwrap(), a.take_match().unwrap(), b.take_match().unwrap());
    assert_same_game(&[&mh, &ma, &mb]);
    assert_eq!(mh.start.seats.iter().map(|x| matches!(x.occupant, Occupant::Peer)).collect::<Vec<_>>(), [true, true, true, false]);
    assert!(matches!(&mh.start.seats[3].occupant, Occupant::Ai(n) if n == SKIRMISH_AI));
    let db = data().content.clone();
    let mut peers = vec![mh.into_peer().unwrap(), ma.into_peer().unwrap(), mb.into_peer().unwrap()];
    seed_ships(&mut peers[0], &db, &[0, 1, 2]);
    let logs = run_peers(&mut peers, 180);
    assert_eq!(logs[0], logs[1]);
    assert_eq!(logs[0], logs[2]);
    assert_eq!(logs[0].len(), 6);
}

#[test]
fn a_bad_code_a_bad_password_a_full_room_and_a_closed_room_say_so() {
    let r = relay();
    // No such room: the relay says so; the words say what to check.
    let nowhere = RoomCode::parse("ZZZ-ZZZ").unwrap();
    let err = join_on(&r, nowhere, "", cfg("Riker")).unwrap_err();
    assert!(matches!(err, ConnectError::Refused(RefuseReason::NoRoom)), "{err:?}");
    assert!(connect_error_text(&err, "relay").contains("no room with that code"));

    let mut host = host_on(&r, "swordfish", "Picard");
    let code = host.code();
    // Bad password, and none at all: distinguishable from a missing room (an open design question).
    for pw in ["wrong", ""] {
        let err = join_on(&r, code, pw, cfg("Riker")).unwrap_err();
        assert!(matches!(err, ConnectError::Refused(RefuseReason::BadPassword)), "{err:?}");
        assert!(connect_error_text(&err, "relay").contains("password is wrong"));
    }
    let mut a = join_on(&r, code, "swordfish", cfg("Riker")).expect("the right password");
    pump(&mut [&mut host, &mut a], "seated", |s| s[1].phase == Phase::Open);

    // The map has two seats: a third player is refused by the host as full, in words.
    let mut c = join_on(&r, code, "swordfish", cfg("Worf")).expect("the relay has seats");
    pump(&mut [&mut host, &mut a, &mut c], "the host refuses the third", |s| s[2].ended().is_some());
    assert_eq!(c.ended(), Some(&Ended::Refused(Refusal::Full)));
    assert_eq!(c.ended().unwrap().text(), "The game is full.");
    pump(&mut [&mut host, &mut a], "the refused seat is gone", |s| kinds(s[0]).iter().filter(|k| **k == Kind::Human).count() == 2);

    // The host closes the room: the relay turns newcomers away (once it has the host's word; until then a
    // newcomer is let in and the host itself says the game is closed).
    host.set_room_open(false);
    let end = Instant::now() + DEADLINE;
    loop {
        match join_on(&r, code, "swordfish", cfg("Worf")) {
            Err(err) => {
                assert!(matches!(err, ConnectError::Refused(RefuseReason::Closed)), "{err:?}");
                assert!(connect_error_text(&err, "relay").contains("closed"));
                break;
            }
            Ok(mut early) => {
                pump(&mut [&mut host, &mut early], "the host says the game is closed", |s| s[1].ended().is_some());
                assert_eq!(early.ended(), Some(&Ended::Refused(Refusal::Closed)));
            }
        }
        assert!(Instant::now() < end, "the relay never closed the room");
    }
    pump(&mut [&mut host, &mut a], "the closed flag reaches the guest", |s| s[1].view().closed);
    host.set_room_open(true);
    pump(&mut [&mut host, &mut a], "open again", |s| !s[1].view().closed);

    // A relay with all eight seats taken refuses the ninth.
    let r2 = relay();
    let mut big = host_on(&r2, "", "Picard");
    let mut taken: Vec<NetTransport> = (0..7).map(|_| NetTransport::join(r2.local_addr(), big.code(), "", &opts()).unwrap()).collect();
    let err = join_on(&r2, big.code(), "", cfg("Late")).unwrap_err();
    assert!(matches!(err, ConnectError::Refused(RefuseReason::Full)), "{err:?}");
    assert!(connect_error_text(&err, "relay").contains("eight seats"));
    big.update(Instant::now());
    taken.clear();
}

#[test]
fn an_unreachable_relay_says_where_it_looked() {
    // A port nobody listens on: bind, note it, release it.
    let addr = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
    let err = connect_host(&addr, "", cfg("Picard"), &opts()).expect_err("nobody there");
    let text = connect_error_text(&err, &addr);
    assert!(text.starts_with("Cannot reach the relay at 127.0.0.1:"), "{text}");
}

#[test]
fn another_content_is_refused_with_the_differing_key_named() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    // The joiner's rules are not the host's: a content database with one unit and none of the rest.
    let mut other = sta_content::UnitDef::new("fx_cruiser", "craft");
    other.race = "fx".into();
    let db2 = Arc::new(ContentDb::builder().unit(other).build().unwrap());
    let mut c = cfg("Riker");
    c.content = db2.clone();
    let mut joiner = join_on(&r, host.code(), "", c).unwrap();
    pump(&mut [&mut host, &mut joiner], "the host names a key", |s| s[1].ended().is_some());
    // What the protocol says the first differing key is.
    let Ended::Refused(Refusal::RulesDiffer(diff)) = joiner.ended().unwrap().clone() else { panic!("{:?}", joiner.ended()) };
    let want = sta_net::proto::rules_refusal(host.cfg.content.digest(), db2.digest().digest(), &RulesIdentity::entries(db2.digest())).unwrap();
    assert_eq!(want, Some(Refusal::RulesDiffer(diff.clone())));
    assert!(!diff.key.is_empty());
    let text = joiner.ended().unwrap().text();
    assert!(text.contains(&diff.key), "the message names the key: {text}");
    assert!(text.starts_with("Your game rules differ from the host's."), "{text}");
    // The host's table never showed the refused player.
    pump(&mut [&mut host, &mut joiner], "the refused joiner's seat is gone", |_| true);
    assert_eq!(kinds(&host).iter().filter(|k| **k == Kind::Human).count(), 1);
    assert!(host.launch_block().is_some());
}

#[test]
fn another_build_is_refused_too() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    let mut c = cfg("Riker");
    c.engine.build = "somewhere-else".into();
    let mut joiner = join_on(&r, host.code(), "", c).unwrap();
    pump(&mut [&mut host, &mut joiner], "refused", |s| s[1].ended().is_some());
    let text = joiner.ended().unwrap().text();
    assert!(text.contains("another build") && text.contains(&host.cfg.engine.build), "{text}");
}

#[test]
fn a_kicked_player_is_told_and_may_come_back() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    host.set_map(1).unwrap();
    let code = host.code();
    let mut a = join_on(&r, code, "", cfg("Riker")).unwrap();
    pump(&mut [&mut host, &mut a], "seated", |s| s[1].phase == Phase::Open && find(s[0], 1).kind == Kind::Human);
    host.kick(1);
    assert_eq!(find(&host, 1).kind, Kind::Open, "the host's table is free at once");
    pump(&mut [&mut host, &mut a], "the kicked player is told", |s| s[1].ended().is_some());
    assert_eq!(a.ended(), Some(&Ended::Refused(Refusal::Kicked)));
    assert_eq!(a.ended().unwrap().text(), "The host removed you from the room.");
    // Nothing keeps them out (the cheapest answer to an open question, `after_kick`): the same player joins again.
    let mut again = join_on(&r, code, "", cfg("Riker")).expect("the room is still open to them");
    pump(&mut [&mut host, &mut again], "back in the room", |s| s[1].phase == Phase::Open && s[0].view().seats.iter().any(|x| x.kind == Kind::Human && x.name == "Riker"));
    // The host cannot kick itself or an empty seat.
    host.kick(0);
    host.kick(3);
    assert_eq!(find(&host, 0).kind, Kind::Human);
}

#[test]
fn a_player_who_ignores_a_kick_is_cut_off_by_the_relay() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    let mut a = join_on(&r, host.code(), "", cfg("Riker")).unwrap();
    pump(&mut [&mut host, &mut a], "seated", |s| s[1].phase == Phase::Open && find(s[0], 1).kind == Kind::Human);
    // The relay closes the connection without a word: the player is told what is known.
    host.control().kick(PlayerId(1)).unwrap();
    pump(&mut [&mut host, &mut a], "cut off", |s| s[1].ended().is_some());
    assert_eq!(a.ended(), Some(&Ended::RelayClosed));
    assert!(a.ended().unwrap().text().contains("closed your connection"));
    pump(&mut [&mut host, &mut a], "the host sees the seat empty", |s| find(s[0], 1).kind == Kind::Open);
}

#[test]
fn the_host_leaving_ends_the_room_for_the_others() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    host.set_map(1).unwrap();
    let code = host.code();
    let mut a = join_on(&r, code, "", cfg("Riker")).unwrap();
    let mut b = join_on(&r, code, "", cfg("Data")).unwrap();
    pump(&mut [&mut host, &mut a, &mut b], "seated", |s| s[1].phase == Phase::Open && s[2].phase == Phase::Open);
    host.leave();
    assert_eq!(host.ended(), Some(&Ended::Left));
    pump(&mut [&mut a, &mut b], "both are told", |s| s[0].ended().is_some() && s[1].ended().is_some());
    for g in [&a, &b] {
        assert_eq!(g.ended(), Some(&Ended::HostLeft));
        assert_eq!(g.ended().unwrap().text(), "The host left, so the room is closed.");
    }
    // A newcomer finds the room closed.
    let err = join_on(&r, code, "", cfg("Worf")).unwrap_err();
    assert!(matches!(err, ConnectError::Refused(RefuseReason::Closed | RefuseReason::NoRoom)), "{err:?}");
}

#[test]
fn a_guest_leaving_frees_the_seat_and_chat_reaches_everyone() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    host.set_map(1).unwrap();
    let code = host.code();
    let mut a = join_on(&r, code, "", cfg("Riker")).unwrap();
    let mut b = join_on(&r, code, "", cfg("Data")).unwrap();
    pump(&mut [&mut host, &mut a, &mut b], "seated", |s| s[1].phase == Phase::Open && s[2].phase == Phase::Open && seats(s[2]).len() == 4 && find(s[2], 2).kind == Kind::Human);
    a.say("Make it so.");
    b.say("  Engage!  ");
    host.say("");
    pump(&mut [&mut host, &mut a, &mut b], "chat reaches everyone", |s| s[0].chat().len() == 2 && s[1].chat().len() == 2 && s[2].chat().len() == 2);
    let lines = |s: &Session| s.chat().iter().map(|c| format!("{}: {}", c.from, c.text)).collect::<Vec<_>>();
    assert!(lines(&host).contains(&"Riker: Make it so.".to_owned()) && lines(&host).contains(&"Data: Engage!".to_owned()), "{:?}", lines(&host));
    assert!(lines(&a).contains(&"Data: Engage!".to_owned()) && lines(&b).contains(&"Riker: Make it so.".to_owned()));
    a.leave();
    pump(&mut [&mut host, &mut b], "the seat is free on both screens", |s| find(s[0], 1).kind == Kind::Open && find(s[1], 1).kind == Kind::Open);
    // The map cannot shrink under a seated player.
    host.set_map(0).expect_err("player in seat 2 needs a four-seat map");
    assert_eq!(host.view().seats.len(), 4);
}

#[test]
fn a_guest_without_the_hosts_map_cannot_ready() {
    let r = relay();
    let mut host = host_on(&r, "", "Picard");
    let mut c = cfg("Riker");
    c.maps.clear();
    let mut a = join_on(&r, host.code(), "", c).unwrap();
    pump(&mut [&mut host, &mut a], "seated", |s| s[1].phase == Phase::Open && !s[1].view().seats.is_empty());
    let why = a.ready_block().unwrap();
    assert!(why.contains("You do not have the map fx_two_bases"), "{why}");
    a.set_ready(true);
    assert!(!find(&host, 1).ready);
    // A map of the same name but another digest is not the host's.
    a.leave();
    pump(&mut [&mut host], "the seat is free", |s| find(s[0], 1).kind == Kind::Open);
    let mut c = cfg("Data");
    let mut def = (*c.maps[0].def).clone();
    def.starts.pop();
    c.maps[0] = MapInfo { digest: proto::map_digest(&def).unwrap(), slots: 1, def: Arc::new(def), ..c.maps[0].clone() };
    let mut d = join_on(&r, host.code(), "", c).unwrap();
    pump(&mut [&mut host, &mut d], "seated", |s| s[1].phase == Phase::Open && !s[1].view().seats.is_empty());
    assert!(d.ready_block().unwrap().contains("is not the host's"), "{:?}", d.ready_block());
    d.set_ready(true);
    assert!(!find(&host, 1).ready);
}

// ---------------------------------------------------------------------------
// The screens
// ---------------------------------------------------------------------------

fn app_with(data: ScreenData, window: Option<(u32, u32)>, scale: f32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.add_plugins(crate::ui::profile::UiProfilePlugin);
    app.insert_resource(data)
        .insert_resource(UiScale(scale))
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Main, ..default() })
        .add_plugins(ScreensPlugin);
    if let Some((w, h)) = window {
        app.world_mut().spawn((Window { resolution: WindowResolution::new(w, h), ..default() }, PrimaryWindow));
    }
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    app
}

fn app() -> App {
    app_with(data(), None, 1.0)
}

pub(crate) fn settle(app: &mut App) {
    for _ in 0..4 {
        app.update();
    }
}

pub(crate) fn go(app: &mut App, s: Screen) {
    app.world_mut().resource_mut::<NextState<Screen>>().set(s);
    settle(app);
}

pub(crate) fn screen_of(app: &App) -> Screen {
    *app.world().resource::<State<Screen>>().get()
}

pub(crate) fn texts(app: &mut App) -> Vec<String> {
    app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect()
}

pub(crate) fn has_text(app: &mut App, want: &str) -> bool {
    let want = want.to_lowercase();
    texts(app).iter().any(|t| t.to_lowercase().contains(&want))
}

pub(crate) fn act_entity(app: &mut App, want: MpAct) -> Option<Entity> {
    app.world_mut().query::<(Entity, &MpAct)>().iter(app.world()).find(|(_, a)| **a == want).map(|(e, _)| e)
}

pub(crate) fn press(app: &mut App, want: MpAct) {
    let e = act_entity(app, want).unwrap_or_else(|| panic!("no enabled {want:?} on {:?}", screen_of(app)));
    click(app.world_mut(), e);
    settle(app);
}

fn field_entity(app: &mut App, which: LobbyField) -> Entity {
    app.world_mut().query::<(Entity, &LobbyField)>().iter(app.world()).find(|(_, f)| **f == which).map(|(e, _)| e).unwrap_or_else(|| panic!("no {which:?} field"))
}

/// Type into a field the way a player does: focus it, then keys.
pub(crate) fn type_into(app: &mut App, which: LobbyField, text: &str) {
    let e = field_entity(app, which);
    if let Some(mut f) = app.world_mut().get_mut::<crate::ui::theme::TextField>(e) {
        f.text.clear();
        f.cursor = 0;
    }
    app.world_mut().resource_mut::<KitFocus>().0 = Some(e);
    for ch in text.chars() {
        kit_key(app.world_mut(), KitKey { code: KeyCode::KeyA, text: Some(ch.to_string()), shift: false });
    }
    settle(app);
}

pub(crate) fn session_of(app: &App) -> &Session {
    app.world().resource::<Multiplayer>().session.as_ref().expect("a session")
}

/// Update the apps until `cond`, or fail at the deadline.
pub(crate) fn pump_apps(apps: &mut [&mut App], what: &str, cond: impl Fn(&mut [&mut App]) -> bool) {
    let end = Instant::now() + DEADLINE;
    loop {
        for a in apps.iter_mut() {
            a.update();
        }
        if cond(apps) {
            return;
        }
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(crate) fn in_screen(a: &App, s: Screen) -> bool {
    screen_of(a) == s
}

/// Host through the screens: Multiplayer, name and relay typed, Host a game, Create room.
pub(crate) fn host_through_screens(app: &mut App, relay: &Relay, name: &str, password: &str) {
    go(app, Screen::Main);
    let mp_button = app.world_mut().query::<(Entity, &crate::frontend::screens::Act)>().iter(app.world()).find(|(_, a)| **a == crate::frontend::screens::Act::Goto(Screen::Multiplayer)).map(|(e, _)| e).expect("the Multiplayer entry");
    click(app.world_mut(), mp_button);
    settle(app);
    assert_eq!(screen_of(app), Screen::Multiplayer);
    type_into(app, LobbyField::Name, name);
    type_into(app, LobbyField::Relay, &relay.local_addr().to_string());
    press(app, MpAct::Goto(Screen::HostGame));
    assert_eq!(screen_of(app), Screen::HostGame);
    if !password.is_empty() {
        type_into(app, LobbyField::Password, password);
    }
    press(app, MpAct::StartHost);
}

/// Join through the screens, the name and relay typed on the first and the code on the second.
pub(crate) fn join_through_screens(app: &mut App, relay: &Relay, name: &str, code: &str, password: &str) {
    go(app, Screen::Multiplayer);
    type_into(app, LobbyField::Name, name);
    type_into(app, LobbyField::Relay, &relay.local_addr().to_string());
    press(app, MpAct::Goto(Screen::JoinGame));
    type_into(app, LobbyField::Code, code);
    if !password.is_empty() {
        type_into(app, LobbyField::Password, password);
    }
    press(app, MpAct::StartJoin);
}

pub(crate) fn pick(app: &mut App, drop: Entity, index: usize) {
    kit_click(app.world_mut(), drop);
    settle(app);
    let row = app.world_mut().query::<(Entity, &DropdownOption)>().iter(app.world()).find(|(_, o)| o.0 == index).map(|(e, _)| e).unwrap();
    kit_click(app.world_mut(), row);
    settle(app);
}

pub(crate) fn drop_of(app: &mut App, slot: usize, col: DropCol) -> Entity {
    app.world_mut().query::<(Entity, &LobbyDrop)>().iter(app.world()).find(|(_, d)| d.slot == slot && d.col == col).map(|(e, _)| e).unwrap_or_else(|| panic!("no dropdown {slot} {col:?}"))
}

#[test]
fn the_main_menu_opens_the_multiplayer_screen() {
    let mut app = app();
    assert_eq!(screen_of(&app), Screen::Main);
    use crate::frontend::screens::{Act, stock::{Placed, Slot}};
    let off: Vec<_> = app.world_mut().query::<(&Placed, &crate::ui::theme::WidgetState)>()
        .iter(app.world()).filter(|(_, state)| **state == crate::ui::theme::WidgetState::Disabled)
        .map(|(placed, _)| placed.0).collect();
    assert_eq!(off.len(), 3);
    for slot in [Slot::MainMapEditor, Slot::MainReplayIntro, Slot::MainCredits] {
        assert!(off.contains(&slot), "{slot:?} is unavailable");
    }
    let multiplayer = app.world_mut().query::<(Entity, &Act)>().iter(app.world())
        .find(|(_, act)| **act == Act::Goto(Screen::Multiplayer)).unwrap().0;
    crate::ui::theme::click(app.world_mut(), multiplayer);
    settle(&mut app);
    assert_eq!(screen_of(&app), Screen::Multiplayer);
    for want in ["Your name", "Relay address", "Host a game", "Join a game", "Previous menu"] {
        assert!(has_text(&mut app, want), "{want}: {:?}", texts(&mut app));
    }
    press(&mut app, MpAct::Goto(Screen::JoinGame));
    for want in ["Room code", "Password", "Rejoin a running game"] {
        assert!(has_text(&mut app, want), "{want}");
    }
    // Rejoin is there and off, with its reason in words (NET-04).
    assert!(act_entity(&mut app, MpAct::Rejoin).is_none());
    assert!(has_text(&mut app, "NET-04"));
    press(&mut app, MpAct::Goto(Screen::Multiplayer));
    press(&mut app, MpAct::Goto(Screen::Main));
    assert_eq!(screen_of(&app), Screen::Main);
}

#[test]
fn two_apps_host_join_choose_launch_and_play_in_step() {
    let r = relay();
    let (mut h, mut g) = (app(), app());
    host_through_screens(&mut h, &r, "Picard", "");
    pump_apps(&mut [&mut h], "the host is in the lobby", |a| in_screen(a[0], Screen::Lobby));
    let code = session_of(&h).code().to_string();
    assert!(has_text(&mut h, &format!("Room code: {code}")));
    assert!(has_text(&mut h, "Picard (host)"));
    // Alone in a room, the host cannot launch, and says why.
    assert!(act_entity(&mut h, MpAct::Launch).is_none());
    assert!(has_text(&mut h, "At least two seats must be played"));

    join_through_screens(&mut g, &r, "Riker", &code.to_lowercase(), "");
    pump_apps(&mut [&mut h, &mut g], "both lobbies show both players", |a| {
        in_screen(a[1], Screen::Lobby) && session_of(a[0]).view().seats[1].kind == Kind::Human && session_of(a[1]).view().seats.len() == 2 && session_of(a[1]).view().seats[1].kind == Kind::Human
    });
    assert!(has_text(&mut h, "Riker") && has_text(&mut g, "Picard (host)") && has_text(&mut g, "Riker"));
    assert!(has_text(&mut h, "Waiting for Riker to be ready."));
    assert!(act_entity(&mut g, MpAct::ToggleReady).is_some(), "the joiner's Launch is Ready");
    assert!(act_entity(&mut g, MpAct::Launch).is_none());

    // The joiner picks a side and a team from its dropdowns; the host's screen follows.
    let side = drop_of(&mut g, 1, DropCol::Side);
    pick(&mut g, side, 1);
    let team = drop_of(&mut g, 1, DropCol::Team);
    pick(&mut g, team, 0);
    pump_apps(&mut [&mut h, &mut g], "the choices reach the host", |a| {
        let v = session_of(a[0]).view();
        v.seats[1].race == "kli" && v.seats[1].team == 0
    });
    // It cannot edit the host's seat: no dropdown there on its screen.
    assert!(app_has_no_drop(&mut g, 0, DropCol::Side));
    // The host picks its own.
    let hs = drop_of(&mut h, 0, DropCol::Side);
    pick(&mut h, hs, 2);
    pump_apps(&mut [&mut h, &mut g], "the host's side reaches the joiner", |a| session_of(a[1]).view().seats[0].race == "rom");
    // Chat: typed in the line, said with Enter.
    type_into(&mut g, LobbyField::Chat, "Hello, Captain");
    kit_key(g.world_mut(), KitKey::code(KeyCode::Enter));
    pump_apps(&mut [&mut h, &mut g], "the chat reaches the host's log", |a| has_text(a[0], "Riker: Hello, Captain"));
    assert!(has_text(&mut g, "Riker: Hello, Captain"), "the speaker sees its own line");

    // Ready, then the host launches.
    press(&mut g, MpAct::ToggleReady);
    pump_apps(&mut [&mut h, &mut g], "the host may launch", |a| act_entity(a[0], MpAct::Launch).is_some());
    press(&mut h, MpAct::Launch);
    pump_apps(&mut [&mut h, &mut g], "both apps have a match", |a| a[0].world().resource::<NetLaunch>().0.is_some() && a[1].world().resource::<NetLaunch>().0.is_some());
    let (mh, mg) = (h.world_mut().resource_mut::<NetLaunch>().0.take().unwrap(), g.world_mut().resource_mut::<NetLaunch>().0.take().unwrap());
    assert_same_game(&[&mh, &mg]);
    assert_eq!(mh.start.seats[1].race, "kli");
    assert_eq!(mh.start.seats[1].name, "Riker");
    let db = data().content.clone();
    let mut peers = vec![mh.into_peer().unwrap(), mg.into_peer().unwrap()];
    seed_ships(&mut peers[0], &db, &[0, 1]);
    let logs = run_peers(&mut peers, 150);
    assert_eq!(logs[0], logs[1]);
    // The names and the relay are remembered for next time.
    let s = h.world().resource::<crate::frontend::screens::Settings>();
    assert_eq!((s.player_name.as_str(), s.relay.as_str()), ("Picard", r.local_addr().to_string().as_str()));
}

fn app_has_no_drop(app: &mut App, slot: usize, col: DropCol) -> bool {
    !app.world_mut().query::<&LobbyDrop>().iter(app.world()).any(|d| d.slot == slot && d.col == col)
}

#[test]
fn a_host_plans_computers_and_the_room_follows() {
    let r = relay();
    let mut h = app();
    host_through_screens(&mut h, &r, "Picard", "");
    pump_apps(&mut [&mut h], "in the lobby", |a| in_screen(a[0], Screen::Lobby));
    let kind = drop_of(&mut h, 1, DropCol::Kind);
    pick(&mut h, kind, 1);
    assert_eq!(session_of(&h).view().seats[1].kind, Kind::Computer);
    assert!(has_text(&mut h, "Computer"));
    // A person and a computer make a game; no one else needs to be ready.
    assert!(act_entity(&mut h, MpAct::Launch).is_some());
    let kind = drop_of(&mut h, 1, DropCol::Kind);
    pick(&mut h, kind, 2);
    assert_eq!(session_of(&h).view().seats[1].kind, Kind::Idle);
    assert!(act_entity(&mut h, MpAct::Launch).is_none());
    // Resources and officers are the host's.
    let before = session_of(&h).view().options.clone();
    press(&mut h, MpAct::CycleResources);
    assert_ne!(session_of(&h).view().options.starting_resources, before.starting_resources);
    // The room can be closed and opened.
    press(&mut h, MpAct::ToggleOpen);
    assert!(session_of(&h).view().closed && has_text(&mut h, "Room closed to new players"));
    press(&mut h, MpAct::ToggleOpen);
    assert!(!session_of(&h).view().closed);
}

#[test]
fn the_host_picks_the_map_on_the_map_select_screen_and_returns() {
    let r = relay();
    let mut h = app();
    host_through_screens(&mut h, &r, "Picard", "");
    pump_apps(&mut [&mut h], "in the lobby", |a| in_screen(a[0], Screen::Lobby));
    assert_eq!(session_of(&h).view().seats.len(), 2);
    press(&mut h, MpAct::PickMap);
    assert_eq!(screen_of(&h), Screen::MapSelect);
    // Cancel goes back to the room, not to Instant Action.
    let cancel = h.world_mut().query::<(Entity, &crate::frontend::screens::Act)>().iter(h.world()).find(|(_, a)| **a == crate::frontend::screens::Act::Goto(Screen::Setup)).map(|(e, _)| e).unwrap();
    click(h.world_mut(), cancel);
    settle(&mut h);
    assert_eq!(screen_of(&h), Screen::Lobby);
    press(&mut h, MpAct::PickMap);
    h.world_mut().resource_mut::<crate::frontend::screens::Browse>().map = 1;
    let take = h.world_mut().query::<(Entity, &crate::frontend::screens::Act)>().iter(h.world()).find(|(_, a)| **a == crate::frontend::screens::Act::TakeMap).map(|(e, _)| e).unwrap();
    click(h.world_mut(), take);
    settle(&mut h);
    assert_eq!(screen_of(&h), Screen::Lobby);
    assert_eq!(session_of(&h).view().seats.len(), 4, "the room has the four-seat map");
    assert!(has_text(&mut h, "Four Corners"));
    // Instant Action's own flow still returns to Game Setup.
    go(&mut h, Screen::Setup);
    press_act(&mut h, crate::frontend::screens::Act::PickMap);
    press_act(&mut h, crate::frontend::screens::Act::TakeMap);
    assert_eq!(screen_of(&h), Screen::Setup);
}

fn press_act(app: &mut App, want: crate::frontend::screens::Act) {
    let e = app.world_mut().query::<(Entity, &crate::frontend::screens::Act)>().iter(app.world()).find(|(_, a)| **a == want).map(|(e, _)| e).unwrap();
    click(app.world_mut(), e);
    settle(app);
}

#[test]
fn refusals_show_in_words_on_the_screen_that_tried() {
    let r = relay();
    let mut h = app();
    host_through_screens(&mut h, &r, "Picard", "swordfish");
    pump_apps(&mut [&mut h], "in the lobby", |a| in_screen(a[0], Screen::Lobby));
    let code = session_of(&h).code().to_string();

    // No relay typed: asked for, nothing sent.
    let mut g = app();
    go(&mut g, Screen::JoinGame);
    type_into(&mut g, LobbyField::Code, &code);
    press(&mut g, MpAct::StartJoin);
    assert!(has_text(&mut g, "Enter the relay's address first"), "{:?}", texts(&mut g));
    // A code that is not a code.
    go(&mut g, Screen::Multiplayer);
    type_into(&mut g, LobbyField::Relay, &r.local_addr().to_string());
    go(&mut g, Screen::JoinGame);
    let e = field_entity(&mut g, LobbyField::Code);
    g.world_mut().get_mut::<crate::ui::theme::TextField>(e).unwrap().text = "nope".into();
    g.world_mut().write_message(crate::ui::theme::TextChanged { entity: e });
    settle(&mut g);
    press(&mut g, MpAct::StartJoin);
    assert!(has_text(&mut g, "That is not a room code"));

    // A well-formed code nobody has.
    join_through_screens(&mut g, &r, "Riker", "ZZZ-ZZZ", "swordfish");
    pump_apps(&mut [&mut h, &mut g], "no such room, in words", |a| has_text(a[1], "no room with that code"));
    assert_eq!(screen_of(&g), Screen::JoinGame, "the player stays where they can try again");
    // The wrong password.
    go(&mut g, Screen::JoinGame);
    for f in [LobbyField::Code, LobbyField::Password] {
        let e = field_entity(&mut g, f);
        g.world_mut().get_mut::<crate::ui::theme::TextField>(e).unwrap().text.clear();
    }
    type_into(&mut g, LobbyField::Code, &code);
    type_into(&mut g, LobbyField::Password, "guess");
    press(&mut g, MpAct::StartJoin);
    pump_apps(&mut [&mut h, &mut g], "bad password, in words", |a| has_text(a[1], "The password is wrong."));
}

#[test]
fn an_unreachable_relay_shows_in_words_on_the_screen() {
    let addr = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
    let mut g = app();
    go(&mut g, Screen::Multiplayer);
    type_into(&mut g, LobbyField::Relay, &addr);
    press(&mut g, MpAct::Goto(Screen::HostGame));
    press(&mut g, MpAct::StartHost);
    pump_apps(&mut [&mut g], "the failure in words", |a| has_text(a[0], "Cannot reach the relay at 127.0.0.1:"));
    assert_eq!(screen_of(&g), Screen::HostGame);
    // The buttons are back: the player can try again.
    assert!(act_entity(&mut g, MpAct::StartHost).is_some());
}

#[test]
fn a_content_mismatch_shows_the_key_the_kick_and_the_host_leaving_on_the_joiners_screen() {
    let r = relay();
    let mut h = app();
    host_through_screens(&mut h, &r, "Picard", "");
    pump_apps(&mut [&mut h], "in the lobby", |a| in_screen(a[0], Screen::Lobby));
    let code = session_of(&h).code().to_string();

    // Other rules.
    let mut other = sta_content::UnitDef::new("fx_cruiser", "craft");
    other.race = "fx".into();
    let mut d = data();
    d.content = Arc::new(ContentDb::builder().unit(other).build().unwrap());
    let mut odd = app_with(d.clone(), None, 1.0);
    join_through_screens(&mut odd, &r, "Worf", &code, "");
    pump_apps(&mut [&mut h, &mut odd], "refused with a key", |a| has_text(a[1], "Your game rules differ from the host's."));
    let Some(Refusal::RulesDiffer(diff)) = sta_net::proto::rules_refusal(h.world().resource::<ScreenData>().content.digest(), d.content.digest().digest(), &RulesIdentity::entries(d.content.digest())).unwrap() else { panic!() };
    assert!(has_text(&mut odd, &diff.key), "the key {} is on the screen: {:?}", diff.key, texts(&mut odd));
    assert_eq!(screen_of(&odd), Screen::JoinGame);

    // A kick.
    let mut g = app();
    join_through_screens(&mut g, &r, "Riker", &code, "");
    pump_apps(&mut [&mut h, &mut g], "seated", |a| in_screen(a[1], Screen::Lobby) && session_of(a[0]).view().seats[1].kind == Kind::Human && session_of(a[1]).phase == Phase::Open);
    press(&mut h, MpAct::Kick(1));
    pump_apps(&mut [&mut h, &mut g], "the kicked player's screen", |a| has_text(a[1], "The host removed you from the room."));
    assert_eq!(screen_of(&g), Screen::JoinGame);
    assert_eq!(session_of(&h).view().seats[1].kind, Kind::Open);

    // The host leaves.
    join_through_screens(&mut g, &r, "Riker", &code, "");
    pump_apps(&mut [&mut h, &mut g], "seated again", |a| in_screen(a[1], Screen::Lobby) && session_of(a[0]).view().seats[1].kind == Kind::Human && session_of(a[1]).phase == Phase::Open);
    press(&mut h, MpAct::Leave);
    assert_eq!(screen_of(&h), Screen::Multiplayer);
    pump_apps(&mut [&mut h, &mut g], "the room is gone", |a| has_text(a[1], "The host left, so the room is closed."));
    assert_eq!(screen_of(&g), Screen::JoinGame);
    assert!(h.world().resource::<Multiplayer>().session.is_none());
}

#[test]
fn player_name_and_relay_are_kept_in_the_settings_file() {
    use crate::ui::settings::Stored;
    let mut s = Stored::default();
    s.settings.player_name = "Jean-Luc".into();
    s.settings.relay = "relay.example:7864".into();
    let text = s.to_text();
    assert!(text.contains("player_name = Jean-Luc") && text.contains("relay = relay.example:7864"), "{text}");
    let (back, warnings) = Stored::parse(&text);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!((back.settings.player_name.as_str(), back.settings.relay.as_str()), ("Jean-Luc", "relay.example:7864"));
    // Nothing written when nothing is set (the default file does not change).
    assert!(!Stored::default().to_text().contains("player_name"));
    // The Multiplayer screen starts from them.
    let mut app = app();
    {
        let mut st = app.world_mut().resource_mut::<crate::frontend::screens::Settings>();
        st.player_name = "Jean-Luc".into();
        st.relay = "relay.example:7864".into();
    }
    go(&mut app, Screen::Multiplayer);
    assert_eq!(app.world().resource::<MpFields>().name, "Jean-Luc");
    let e = field_entity(&mut app, LobbyField::Relay);
    assert_eq!(app.world().get::<crate::ui::theme::TextField>(e).unwrap().text, "relay.example:7864");
}

// ---- layout ---------------------------------------------------------------------

/// Windows: 21:9 (3440x1440), 16:9, 16:10 and 4:3, at the UI scales in use.
const WINDOWS: [(&str, u32, u32); 4] = [("21:9", 3440, 1440), ("16:9", 1920, 1080), ("16:10", 1920, 1200), ("4:3", 1600, 1200)];
const SCALES: [f32; 3] = [1.0, 1.25, 1.333];

fn node_rect(app: &App, e: Entity) -> Rect {
    let n = app.world().get::<Node>(e).unwrap();
    let px = |v: Val| match v {
        Val::Px(p) => p,
        other => panic!("not px: {other:?}"),
    };
    Rect::new(px(n.left), px(n.top), px(n.left) + px(n.width), px(n.top) + px(n.height))
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x - 0.01 && b.min.x < a.max.x - 0.01 && a.min.y < b.max.y - 0.01 && b.min.y < a.max.y - 0.01
}

/// Each part of `screen` stands in its region, inside the box, clear of the others, and
/// its widget fits the region, for every window shape and UI scale.
fn check_layout(screen: Screen, want: &[MSlot], rows: usize) {
    let r = relay();
    for (name, w, h) in WINDOWS {
        for scale in SCALES {
            let mut app = app_with(data(), Some((w, h)), scale);
            // A lobby needs a room with a second player in it, so the Kick buttons are there.
            let mut _guest = None;
            if screen == Screen::Lobby {
                host_through_screens(&mut app, &r, "Picard", "");
                pump_apps(&mut [&mut app], "in the lobby", |a| in_screen(a[0], Screen::Lobby));
                if rows > 0 {
                    let g = join_on(&r, session_of(&app).code(), "", cfg("Riker")).unwrap();
                    pump_apps(&mut [&mut app], "the guest is seated", |a| session_of(a[0]).view().seats[1].kind == Kind::Human);
                    _guest = Some(g);
                }
            } else {
                go(&mut app, screen);
            }
            settle(&mut app);
            let ctx = format!("{screen:?} {name} x{scale}");
            let avail = Vec2::new(w as f32 / scale, h as f32 / scale);
            let b = fit_box(avail);
            assert_eq!(app.world().resource::<crate::frontend::screens::stock::StockBox>().0, b, "{ctx}: the box");
            let theme = app.world().resource::<Theme>().clone();
            let all: Vec<(MSlot, Entity)> = app.world_mut().query::<(Entity, &Placed)>().iter(app.world()).map(|(e, p)| (p.0, e)).collect();
            for slot in want {
                assert!(all.iter().any(|(s, _)| s == slot), "{ctx}: no {slot:?}");
            }
            let mut rects = Vec::new();
            for (slot, e) in &all {
                let r = node_rect(&app, *e);
                let want_r = region(*slot).px(b);
                for (got, exp) in [(r.min.x, want_r.min.x), (r.min.y, want_r.min.y), (r.max.x, want_r.max.x), (r.max.y, want_r.max.y)] {
                    assert!((got - exp).abs() < 0.01, "{ctx}: {slot:?} {r:?} is not its region {want_r:?}");
                }
                assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= b.x + 0.01 && r.max.y <= b.y + 0.01, "{ctx}: {slot:?} leaves the box");
                if let Some(kid) = app.world().get::<Children>(*e).and_then(|k| k.first().copied())
                    && let Some(n) = app.world().get::<Node>(kid)
                {
                    if let Val::Px(kh) = n.height {
                        assert!(kh <= r.height() + 0.5, "{ctx}: {slot:?} widget {kh}px high in a {}px region", r.height());
                    }
                    if let Val::Px(kw) = n.width {
                        assert!(kw <= r.width() + 0.5, "{ctx}: {slot:?} widget {kw}px wide in a {}px region", r.width());
                    }
                }
                rects.push((*slot, r));
            }
            for (i, (sa, ra)) in rects.iter().enumerate() {
                for (sb, rb) in &rects[i + 1..] {
                    assert!(!overlap(*ra, *rb), "{ctx}: {sa:?} overlaps {sb:?}");
                }
            }
            if screen == Screen::Lobby {
                assert!(region(MSlot::Side(0)).px(b).height() + 0.01 >= theme.button_h, "{ctx}: rows are too tight");
            }
            assert!(b.x / b.y >= layout_min() - 0.01 && b.x / b.y <= layout_max() + 0.01, "{ctx}: aspect");
        }
    }
}

fn layout_min() -> f32 {
    crate::frontend::screens::stock::layout::MIN_ASPECT
}

fn layout_max() -> f32 {
    crate::frontend::screens::stock::layout::MAX_ASPECT
}

#[test]
fn connection_screen_layout_in_every_window_and_scale() {
    check_layout(Screen::Multiplayer, &[MSlot::Title, MSlot::NameLabel, MSlot::NameField, MSlot::RelayLabel, MSlot::RelayField, MSlot::HostBtn, MSlot::JoinBtn, MSlot::Back], 0);
}

#[test]
fn host_form_layout_in_every_window_and_scale() {
    check_layout(Screen::HostGame, &[MSlot::Title, MSlot::HostPassLabel, MSlot::HostPassField, MSlot::CreateBtn, MSlot::Back], 0);
}

#[test]
fn join_form_layout_in_every_window_and_scale() {
    check_layout(Screen::JoinGame, &[MSlot::Title, MSlot::CodeLabel, MSlot::CodeField, MSlot::JoinPassLabel, MSlot::JoinPassField, MSlot::JoinNowBtn, MSlot::RejoinBtn, MSlot::RejoinNote, MSlot::Back], 0);
}

#[test]
fn lobby_layout_in_every_window_and_scale() {
    let mut want = vec![MSlot::Title, MSlot::HeadPlayer, MSlot::HeadSide, MSlot::HeadTeam, MSlot::Log, MSlot::ChatLine, MSlot::MapInfo, MSlot::Preview, MSlot::SelectMap, MSlot::Resources, MSlot::Officers, MSlot::RoomInfo, MSlot::OpenToggle, MSlot::Status, MSlot::Leave, MSlot::Launch];
    // The fake map has two seats; the second is held by a joiner the host can kick.
    for i in 0..2 {
        want.extend([MSlot::Light(i), MSlot::Player(i), MSlot::Side(i), MSlot::Team(i)]);
    }
    want.push(MSlot::Kick(1));
    check_layout(Screen::Lobby, &want, 1);
}
