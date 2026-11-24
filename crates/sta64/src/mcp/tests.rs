use super::*;
use sta_content::map::MapObject;
use sta_sim::command::LocalRouter;
use sta_sim::replay::Replay;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{EntityId, Fixed, Sim, TeamId, Vec3, WorldView};

const HUMAN: PlayerId = PlayerId(0);
const COMPUTER: PlayerId = PlayerId(1);

fn match_link() -> (SimLink, Arc<sta_content::ContentDb>, EntityId, EntityId) {
    let mut map = sta_content::fake::two_bases();
    let heading = (Fixed::ZERO, Fixed::ONE);
    map.objects.extend(map.starts.iter().map(|start| MapObject {
        odf: "fx_frigate".into(),
        owner: Some(start.slot),
        pos: start.pos,
        heading,
        label: None,
    }));
    let setup = GameSetup {
        seed: 1,
        players: vec![
            PlayerSetup {
                id: HUMAN,
                team: TeamId(0),
                race: "fx".into(),
                control: Control::Human,
            },
            PlayerSetup {
                id: COMPUTER,
                team: TeamId(1),
                race: "fx".into(),
                control: Control::Ai("test".into()),
            },
        ],
        map: Arc::new(map),
        content: Arc::new(sta_content::fake::db()),
        options: GameOptions::default(),
    };
    let link = SimLink::new(
        Box::new(Sim::from_setup(&setup)),
        Box::new(LocalRouter::new()),
        HUMAN,
    )
    .with_mcp_seat(COMPUTER, setup.map.bounds)
    .with_match_fog_lifted(true)
    .with_replay(Replay::new(&setup));
    let find = |seat| {
        link.real_curr()
            .units
            .iter()
            .find(|unit| unit.owner == seat)
            .unwrap()
            .id
    };
    let (ours, theirs) = (find(HUMAN), find(COMPUTER));
    (link, setup.content, ours, theirs)
}

fn request(
    link: &mut SimLink,
    db: &Arc<sta_content::ContentDb>,
    op: &str,
    lease: Option<&str>,
    arguments: Value,
) -> Result<Value, String> {
    respond(
        &Request {
            op: op.into(),
            lease: lease.map(str::to_owned),
            arguments,
        },
        COMPUTER,
        Some(link),
        Some(db),
        Instant::now(),
    )
}

struct TestAi(EntityId);

impl Controller for TestAi {
    fn think(&mut self, world: &WorldView, _: &[SimEvent], out: &mut Vec<Command>) {
        out.push(Command {
            player: COMPUTER,
            seq: world.tick as u32,
            kind: CommandKind::Order(Order::Stop {
                units: vec![self.0],
            }),
        });
    }
}

#[test]
fn seat_orders_replace_only_the_claimed_controller_and_are_replayed() {
    let (link, db, ours, theirs) = match_link();
    let mut link = link.with_controllers(vec![Box::new(TestAi(theirs))]);
    link.tick().unwrap(); // The AI schedules a command before the claim.
    assert!(request(&mut link, &db, "observe", None, json!({})).is_err());
    let claim = request(&mut link, &db, "claim", None, json!({})).unwrap();
    let lease = claim["lease"].as_str();
    assert!(request(&mut link, &db, "claim", None, json!({})).is_err());

    let observed = request(&mut link, &db, "observe", lease, json!({})).unwrap();
    assert!(
        observed["units"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == theirs.0)
    );
    assert!(
        observed["units"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != ours.0)
    );
    assert!(link.curr().unit(ours).is_some()); // The UI has lifted fog.
    assert!(
        request(
            &mut link,
            &db,
            "order",
            lease,
            json!({"action": "stop", "units": [ours.0]})
        )
        .is_err()
    );
    let movement = json!({"action": "move", "units": [theirs.0], "position": [4000, 0, 500]});
    let queued = request(&mut link, &db, "order", lease, movement.clone()).unwrap();
    link.submit(CommandKind::Order(Order::Stop { units: vec![ours] }));
    link.tick().unwrap();

    let replay = link.replay().unwrap();
    assert_eq!(replay.commands.len(), 2);
    assert_eq!(replay.commands[0].command.player, HUMAN);
    assert!(matches!(
        replay.commands[1].command.kind,
        CommandKind::Order(Order::Move { .. })
    ));
    let observed = request(&mut link, &db, "observe", lease, json!({})).unwrap();
    assert_eq!(observed["recent_orders"][0]["sequence"], queued["sequence"]);
    assert_eq!(observed["recent_orders"][0]["accepted"], true);

    request(&mut link, &db, "order", lease, movement).unwrap();
    request(&mut link, &db, "release", lease, json!({})).unwrap();
    assert!(request(&mut link, &db, "observe", lease, json!({})).is_err());
    for _ in 0..2 {
        link.tick().unwrap();
    }
    let replay = link.replay().unwrap();
    assert_eq!(replay.commands.len(), 3);
    assert!(matches!(
        replay.commands[2].command.kind,
        CommandKind::Order(Order::Stop { .. })
    ));
    assert_eq!(replay.commands[2].command.player, COMPUTER);
}

#[test]
fn expired_lease_drops_pending_orders() {
    let mut control = SeatControl::new(COMPUTER, (Vec3::ZERO, Vec3::ZERO));
    let now = Instant::now();
    let old = control.claim(now).unwrap();
    control
        .enqueue(Order::Stop {
            units: vec![EntityId(1)],
        })
        .unwrap();
    assert!(control.expire(now + LEASE_TIME));
    let mut commands = Vec::new();
    control.merge_orders(&mut commands);
    assert!(commands.is_empty());
    let new = control.claim(now + LEASE_TIME).unwrap();
    assert_ne!(old, new);
    assert!(control.authorize(Some(&old), now + LEASE_TIME).is_err());
}

#[cfg(unix)]
#[test]
#[ignore = "requires python3 and loopback sockets"]
fn stdio_client_observes_and_moves_a_ship_through_the_game_schedule() {
    use std::process::{Command as Process, Stdio};
    use std::thread;

    let endpoint = std::env::temp_dir().join(format!("sta64-mcp-game-{}.json", std::process::id()));
    let (link, db, ours, theirs) = match_link();
    let mut app = App::new();
    app.insert_resource(link).insert_resource(ContentRes(db));
    enable(
        &mut app,
        &LaunchConfig {
            mcp_seat: Some(COMPUTER.0),
            mcp_endpoint: Some(endpoint.clone()),
            ..Default::default()
        },
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut child = Process::new("python3")
        .arg(root.join("tools/test_sta64_mcp.py"))
        .arg("--live-game")
        .arg(&endpoint)
        .arg(ours.0.to_string())
        .arg(theirs.0.to_string())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        app.update();
        app.world_mut().resource_mut::<SimLink>().tick().unwrap();
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("STA64 MCP stdio test timed out");
        }
        thread::sleep(Duration::from_millis(5));
    }
    drop(app);
    assert!(!endpoint.exists());
}
