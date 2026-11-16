use super::*;
use sta_content::{ContentDb, MapDef, MapObject, UnitDef};
use sta_sim::command::LocalRouter;
use sta_sim::replay::Replay;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{PlayerId, Sim, TeamId, Vec3 as SimVec3, WorldSource};
use std::sync::Arc;

fn setup(profile: Profile) -> GameSetup {
    let mut ship = UnitDef::new("test_ship", "craft");
    ship.hull = Fixed::from_int(500);
    ship.crew = Fixed::from_int(100);
    ship.shields.max = Fixed::from_int(100);
    ship.shields.recharge = Fixed::ONE;
    ship.movement = sta_content::MoveStats::from_f32(120.0, 1.0, 1.0, 1.0, 0.2);
    let mut base = UnitDef::new("test_base", "starbase");
    base.hull = Fixed::from_int(1000);
    base.crew = Fixed::from_int(100);
    let content = Arc::new(
        ContentDb::builder()
            .profile(profile)
            .features(profile.features())
            .unit(ship)
            .unit(base)
            .build()
            .unwrap(),
    );
    let mut map = MapDef::empty();
    for (odf, owner, x) in [
        ("test_ship", 0, 0),
        ("test_base", 0, -500),
        ("test_ship", 1, 2000),
        ("test_base", 1, 2500),
    ] {
        map.objects.push(MapObject {
            odf: odf.into(),
            owner: Some(owner),
            pos: SimVec3::from_f32([x as f32, 0.0, 0.0]),
            heading: (Fixed::ONE, Fixed::ZERO),
            label: None,
        });
    }
    GameSetup {
        seed: 47,
        map: Arc::new(map),
        content,
        players: [0, 1]
            .into_iter()
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "test".into(),
                control: Control::Human,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn link(s: &GameSetup) -> SimLink {
    SimLink::new(
        Box::new(Sim::from_setup(s)),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    )
    .with_replay(Replay::new(s))
}

fn app(profile: Profile) -> (App, GameSetup) {
    let s = setup(profile);
    let mut app = App::new();
    app.init_resource::<DebugDisplay>()
        .add_message::<CheatSubmitted>()
        .add_message::<Notice>()
        .add_message::<AppExit>()
        .insert_resource(link(&s))
        .insert_resource(ContentRes(s.content.clone()))
        .init_resource::<Selection>()
        .add_systems(Update, dispatch);
    (app, s)
}

fn send(app: &mut App, text: &str) {
    app.world_mut().write_message(CheatSubmitted(text.into()));
    app.update();
}

fn tick(app: &mut App) {
    let events = app.world_mut().resource_mut::<SimLink>().tick().unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, sta_sim::SimEvent::Rejected { .. })),
        "unexpected rejection: {events:?}"
    );
}

#[test]
fn catalogue_is_exact_and_follows_lineage() {
    for p in [
        Profile::Sta1Classic,
        Profile::Sta2Classic,
        Profile::Fo3,
        Profile::Fo4,
    ] {
        assert_eq!(code(p, "showmethemoney"), Some(Code::Money));
        assert_eq!(code(p, "kobayashimaru"), Some(Code::Win));
        for bad in [
            "SHOWMETHEMONEY",
            " showmethemoney",
            "showmethemoney ",
            "avoidance",
            "youstopmecold",
            "phonehome",
            "nomoreships",
        ] {
            assert_eq!(code(p, bad), None);
        }
    }
    assert_eq!(
        code(Profile::Sta1Classic, "canofwhoopass"),
        Some(Code::TacticalAi)
    );
    assert_eq!(code(Profile::Sta2Classic, "canofwhoopass"), None);
    assert_eq!(code(Profile::Sta1Classic, "kobayashimaru_lost"), None);
    for word in [
        "expl", "elim", "m", "dis", "levelup", "version", "crash", "cine", "fps", "dump",
    ] {
        assert!(code(Profile::Fo3, word).is_some(), "{word}");
        assert!(code(Profile::Fo4, word).is_some(), "{word}");
        assert_eq!(code(Profile::Sta2Classic, word), None);
    }
}

#[test]
fn repeated_money_uses_real_stock_and_the_profile_slots_and_replays() {
    for profile in [
        Profile::Sta1Classic,
        Profile::Sta2Classic,
        Profile::Fo3,
        Profile::Fo4,
    ] {
        let (mut app, s) = app(profile);
        send(&mut app, "showmethemoney");
        send(&mut app, "showmethemoney");
        tick(&mut app);
        let link = app.world().resource::<SimLink>();
        let stock = &link.real_curr().player(PlayerId(0)).unwrap().resources;
        let mut expected = vec![Fixed::ZERO; 4];
        for (slot, amount) in grants(profile) {
            expected[slot as usize] = Fixed::from_int(amount * 2);
        }
        assert_eq!(*stock, expected);
        assert!(
            link.real_curr()
                .player(PlayerId(1))
                .unwrap()
                .resources
                .iter()
                .all(|x| *x == Fixed::ZERO)
        );
        let replay = link.replay().unwrap();
        let played = Replay::decode(&replay.encode())
            .unwrap()
            .play(s.content.clone())
            .unwrap();
        assert_eq!(played.view().digest, link.real_curr().digest);
        let snapshot = app
            .world_mut()
            .resource_mut::<SimLink>()
            .snapshot()
            .unwrap();
        let loaded = Sim::from_snapshot(&snapshot, Some(s.content.clone())).unwrap();
        assert_eq!(loaded.view().digest, played.view().digest);
    }
}

#[test]
fn network_marker_refuses_every_command_without_changing_state_or_exiting() {
    let (mut app, s) = app(Profile::Fo4);
    app.insert_resource(NetworkedMatch);
    for word in [
        "showmethemoney",
        "expl",
        "elim",
        "m",
        "dis",
        "crash",
        "cine",
        "fps",
        "dump",
        "kobayashimaru",
    ] {
        send(&mut app, word);
    }
    tick(&mut app);
    assert!(
        app.world()
            .resource::<SimLink>()
            .replay()
            .unwrap()
            .commands
            .is_empty()
    );
    assert!(!app.world().resource::<DebugDisplay>().fps);
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
    let mut control = Sim::from_setup(&s);
    control.step(&[]);
    assert_eq!(
        app.world().resource::<SimLink>().real_curr().digest,
        control.view().digest
    );
}

#[test]
fn selected_enemy_moves_without_transferring_ownership_and_explodes_through_the_log() {
    let (mut app, s) = app(Profile::Fo4);
    let enemy = app
        .world()
        .resource::<SimLink>()
        .real_curr()
        .units
        .iter()
        .find(|u| u.owner == PlayerId(1) && u.def == s.content.key("test_ship").unwrap())
        .unwrap()
        .id;
    app.world_mut().resource_mut::<Selection>().0 = vec![enemy, enemy];
    let start = app
        .world()
        .resource::<SimLink>()
        .real_curr()
        .unit(enemy)
        .unwrap()
        .pos;
    send(&mut app, "m");
    for _ in 0..30 {
        tick(&mut app);
    }
    let u = app
        .world()
        .resource::<SimLink>()
        .real_curr()
        .unit(enemy)
        .unwrap();
    assert!(
        u.pos.x > start.x,
        "start={start:?}, now={u:?}, commands={:?}",
        app.world().resource::<SimLink>().replay().unwrap().commands
    );
    assert_eq!(u.owner, PlayerId(1));
    send(&mut app, "expl");
    tick(&mut app);
    let link = app.world().resource::<SimLink>();
    assert!(link.real_curr().unit(enemy).is_none());
    assert!(
        link.real_curr()
            .units
            .iter()
            .any(|u| u.owner == PlayerId(1))
    );
    assert_eq!(
        link.replay()
            .unwrap()
            .play(s.content)
            .unwrap()
            .view()
            .digest,
        link.real_curr().digest
    );
}

#[test]
fn elim_removes_the_selected_players_objects_and_preserves_the_other_seat() {
    let (mut app, s) = app(Profile::Fo4);
    let enemy = app
        .world()
        .resource::<SimLink>()
        .real_curr()
        .units
        .iter()
        .find(|u| u.owner == PlayerId(1))
        .unwrap()
        .id;
    app.world_mut().resource_mut::<Selection>().0 = vec![enemy];
    send(&mut app, "elim");
    tick(&mut app);
    let link = app.world().resource::<SimLink>();
    assert!(
        !link
            .real_curr()
            .units
            .iter()
            .any(|u| u.owner == PlayerId(1))
    );
    assert_eq!(
        link.real_curr()
            .units
            .iter()
            .filter(|u| u.owner == PlayerId(0))
            .count(),
        2
    );
    assert_eq!(
        link.replay()
            .unwrap()
            .play(s.content)
            .unwrap()
            .view()
            .digest,
        link.real_curr().digest
    );
}

#[test]
fn dis_drops_shields_and_holds_them_off_across_ticks_snapshot_and_replay() {
    let (mut app, s) = app(Profile::Fo4);
    let id = app
        .world()
        .resource::<SimLink>()
        .real_curr()
        .units
        .iter()
        .find(|u| u.def == s.content.key("test_ship").unwrap())
        .unwrap()
        .id;
    app.world_mut().resource_mut::<Selection>().0 = vec![id];
    send(&mut app, "dis");
    for _ in 0..30 {
        tick(&mut app);
    }
    let link = app.world().resource::<SimLink>();
    assert!(
        link.real_curr()
            .unit(id)
            .unwrap()
            .shields
            .iter()
            .all(|s| s[0] == Fixed::ZERO)
    );
    let played = link.replay().unwrap().play(s.content.clone()).unwrap();
    assert_eq!(played.view().digest, link.real_curr().digest);
    let bytes = app
        .world_mut()
        .resource_mut::<SimLink>()
        .snapshot()
        .unwrap();
    let mut loaded = Sim::from_snapshot(&bytes, Some(s.content)).unwrap();
    loaded.step(&[]);
    assert!(
        loaded
            .view()
            .unit(id)
            .unwrap()
            .shields
            .iter()
            .all(|s| s[0] == Fixed::ZERO)
    );
}

#[test]
fn missing_rank_ai_and_empty_selection_never_queue_fake_success() {
    let (mut app, _) = app(Profile::Fo4);
    for word in ["levelup", "expl", "elim", "m", "dis", "kobayashimaru"] {
        send(&mut app, word);
    }
    tick(&mut app);
    assert!(
        app.world()
            .resource::<SimLink>()
            .replay()
            .unwrap()
            .commands
            .is_empty()
    );
    let (mut app, _) = self::app(Profile::Sta1Classic);
    send(&mut app, "canofwhoopass");
    tick(&mut app);
    assert!(
        app.world()
            .resource::<SimLink>()
            .replay()
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn version_fps_and_crash_have_their_real_client_effects() {
    let (mut app, _) = app(Profile::Fo4);
    send(&mut app, "version");
    assert!(!app.world().resource::<Messages<Notice>>().is_empty());
    send(&mut app, "fps");
    assert!(app.world().resource::<DebugDisplay>().fps);
    send(&mut app, "fps");
    assert!(!app.world().resource::<DebugDisplay>().fps);
    send(&mut app, "crash");
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<AppExit>>()
            .drain()
            .collect::<Vec<_>>(),
        [AppExit::Error(std::num::NonZeroU8::new(1).unwrap())]
    );
}
