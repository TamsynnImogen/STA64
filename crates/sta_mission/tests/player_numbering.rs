//! Explicit A2/BZN team numbering over core player slots. The default MMM
//! mode remains zero-based; this test uses only synthetic content and a sim.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, WorldSource};

fn scene() -> (Sim, Arc<ContentDb>, [EntityId; 3]) {
    let db = Arc::new(fake::db());
    let setup = GameSetup {
        seed: 23,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: (0..8)
            .map(|n| PlayerSetup {
                id: PlayerId(n),
                team: TeamId(n),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let heading = (Fixed::ZERO, Fixed::ONE);
    let first = sim
        .spawn_def(
            db.key("fx_cruiser").unwrap(),
            PlayerId(0),
            Vec3::from_f32([100.0, 0.0, 0.0]),
            heading,
        )
        .unwrap();
    let eighth = sim
        .spawn_def(
            db.key("fx_cruiser").unwrap(),
            PlayerId(7),
            Vec3::from_f32([800.0, 0.0, 0.0]),
            heading,
        )
        .unwrap();
    let neutral = sim
        .spawn_def(
            db.key("fx_moon").unwrap(),
            NEUTRAL,
            Vec3::from_f32([0.0, 0.0, 800.0]),
            heading,
        )
        .unwrap();
    let out = sim.step(&[
        Command {
            player: PlayerId::SCRIPT,
            seq: 1,
            kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(123))),
        },
        Command {
            player: PlayerId::SCRIPT,
            seq: 2,
            kind: CommandKind::Edit(Edit::Cargo(PlayerId(7), 0, Fixed::from_int(789))),
        },
    ]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    (sim, db, [first, eighth, neutral])
}

fn runtime(sim: &Sim, db: Arc<ContentDb>, numbering: PlayerNumbering) -> MissionRuntime {
    let cfg = MissionConfig {
        seat: Some(PlayerId(0)),
        content: Some(db),
        view: Some((*sim.view()).clone()),
        player_numbering: numbering,
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("numbers.lua", "", 23, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn armada_team_numbers_round_trip_find_players_neutral_and_spawn_owner() {
    let (mut sim, db, [first, eighth, neutral]) = scene();
    let mut rt = runtime(&sim, db, PlayerNumbering::ArmadaTeams);
    let observed: (u8, u8, u8, bool, bool, bool, bool) = rt.eval(&format!(
        "local a=EntityFinder():findOne({{player=1}}); local b=EntityFinder():findOne({{player=8}}); local n=EntityFinder():findOne({{player=0}}); \
         return Player(1).number, Player.get(8).number, Player(0).number, a.id=={}, b.id=={}, n.id=={}, a.player==Player(1)",
        first.0, eighth.0, neutral.0
    )).unwrap();
    assert_eq!(observed, (1, 8, 0, true, true, true, true));
    let local: (u8, u8) = rt.eval("return Game.localSlot, Game.localPlayer.number").unwrap();
    assert_eq!(local, (0, 1));
    let details: (f64, f64, String, u8, u8) = rt.eval(&format!(
        "return Player(1).resources.dilithium, Player(8).resources.dilithium, Player(1):getRelation(Player(8)), A2.GetRealTeam({}), A2.GetRealTeam({})",
        eighth.0, neutral.0
    )).unwrap();
    assert_eq!(details, (123.0, 789.0, "Enemy".into(), 8, 0));
    let mapped: (bool, String, bool, bool, bool, bool) = rt.eval(&format!(
        "local a=EntityFinder():findOne({{player=Player(1)}}); local b=EntityFinder():findOne({{player=Player(8)}}); \
         return a.id=={} and b.id=={}, Player(1):getRelation(8), a:canPlayerSee(1), a:canPlayerSee(Player(1)), a:canPlayerSee(0), a:canPlayerSee(Player(0))",
        first.0, eighth.0
    )).unwrap();
    assert_eq!(mapped, (true, "Enemy".into(), true, true, false, false));
    rt.eval::<()>("Player(1):setRelation(8, 'Ally')").unwrap();
    let relation: (String, String) = rt
        .eval("return Player(1):getRelation(8), Player(1):getRelation(Player(8))")
        .unwrap();
    assert_eq!(relation, ("Ally".into(), "Ally".into()));

    rt.eval::<()>("h=Entity.add('fx_cruiser',8,Vector(900,0,0)); h:giveOrder(Order.Go,Vector(950,0,0))")
        .unwrap();
    let request = rt.take_commands();
    assert_eq!(request.len(), 2);
    assert!(matches!(
        &request[0].kind,
        CommandKind::Edit(Edit::SetRelation { from: PlayerId(0), to: PlayerId(7), .. })
    ));
    assert!(matches!(
        &request[1].kind,
        CommandKind::Edit(Edit::SpawnRequest {
            player: PlayerId(7),
            ..
        })
    ));
    let out = sim.step(&request);
    let born = out
        .events
        .iter()
        .find_map(|e| match e {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].player, PlayerId(7));
    assert_eq!(
        deferred[0].kind,
        CommandKind::Order(Order::Move {
            units: vec![born],
            to: Vec3::from_f32([950.0, 0.0, 0.0]),
            queue: false
        })
    );
    let resolved: (u32, u8, u8) = rt.eval("return h.id, h.player.number, A2.GetRealTeam(h.id)").unwrap();
    assert_eq!(resolved, (born.0, 8, 8));
    assert!(
        !sim.step(&deferred)
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
}

#[test]
fn default_slots_remain_zero_based_and_invalid_native_numbers_are_atomic() {
    let (mut sim, db, [first, _, _]) = scene();
    let mut slots = runtime(&sim, db.clone(), PlayerNumbering::Slots);
    let observed: (u8, bool, u8) = slots
        .eval(&format!(
            "return Player(0).number, EntityFinder():findOne({{player=0}}).id=={}, A2.GetRealTeam({})",
            first.0, first.0
        ))
        .unwrap();
    assert_eq!(observed, (0, true, 0));
    slots
        .eval::<()>("h=Entity.add('fx_cruiser',0,Vector(300,0,0))")
        .unwrap();
    let commands = slots.take_commands();
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        &commands[0].kind,
        CommandKind::Edit(Edit::SpawnRequest {
            player: PlayerId(0),
            ..
        })
    ));
    assert!(
        !sim.step(&commands)
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );

    let mut native = runtime(&sim, db, PlayerNumbering::ArmadaTeams);
    let invalid: (bool, bool, bool) = native
        .eval("return Player(9)==nil, Player.get(1.5)==nil, Player(-1)==nil")
        .unwrap();
    assert_eq!(invalid, (true, true, true));
    for script in [
        "Entity.add('fx_cruiser',9,Vector(0,0,0))",
        "Entity.add('fx_cruiser',1.5,Vector(0,0,0))",
        "Entity.add('fx_cruiser',0,Vector(0,0,0))",
        "EntityFinder():findOne({player=9})",
        "EntityFinder():find({player=1.5})",
        "EntityFinder():find(9)",
    ] {
        assert!(native.eval::<()>(script).is_err(), "{script}");
        assert!(native.take_commands().is_empty(), "{script} queued a partial command");
    }
    let unrestricted: (bool, bool) = native
        .eval("return #(EntityFinder():find({})) > 0, #(EntityFinder():find({player=nil})) > 0")
        .unwrap();
    assert_eq!(unrestricted, (true, true));
    assert!(
        native
            .eval::<()>("m=Monitor(); m:hook(Monitor.Enters,Vector(0,0,0),100,function()end,9)")
            .is_err()
    );
    let hooks: usize = native.eval("return m.hooks").unwrap();
    assert_eq!(hooks, 0);
    assert!(native.take_commands().is_empty());
}
