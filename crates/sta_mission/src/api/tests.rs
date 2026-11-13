//! Per-function tests: scripts drive a real `Sim` on `fake::db()` and a
//! `two_bases()` map with a few units added.

use std::sync::Arc;

use sta_content::MapObject;
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::{Relation, WorldView};
use sta_sim::{Sim, SimEvent, WorldSource};

use super::{MissionConfig, ObjectiveState};
use crate::runtime::MissionRuntime;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn obj(odf: &str, owner: u8, pos: Vec3, label: &str) -> MapObject {
    MapObject { odf: odf.into(), owner: Some(owner), pos, heading: (Fixed::ONE, Fixed::ZERO), label: Some(label.into()) }
}

/// Ids by spawn order: 1 moon, 2 hero (p0 cruiser), 3 wing (p0 frigate),
/// 4 foe (p1 frigate, 300 from hero), 5 far (p1 miner).
const HERO: EntityId = EntityId(2);
const FOE: EntityId = EntityId(4);

struct Game {
    sim: Sim,
    rt: MissionRuntime,
    view: Arc<WorldView>,
    events: Vec<SimEvent>,
    seen: Vec<SimEvent>,
}

fn game(script: &str) -> Game {
    let mut map = sta_content::fake::two_bases();
    map.objects.push(obj("fx_cruiser", 0, v(-1000, 0), "hero"));
    map.objects.push(obj("fx_frigate", 0, v(-900, 200), "wing"));
    map.objects.push(obj("fx_frigate", 1, v(-700, 0), "foe"));
    map.objects.push(obj("fx_miner", 1, v(3000, 0), "far"));
    let content = Arc::new(sta_content::fake::db());
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(map),
        content: content.clone(),
        players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
        options: GameOptions::default(),
    };
    let sim = Sim::from_setup(&setup);
    let cfg = MissionConfig {
        seat: Some(PlayerId(0)),
        content: Some(content),
        labels: sim.labels.iter().map(|(id, l)| (id, l.clone())).collect(),
        view: Some((*sim.view()).clone()),
        paths: Vec::new(),
        player_numbering: super::PlayerNumbering::Slots,
    };
    let rt = MissionRuntime::from_source_with("missions/T.lua", script, 1, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let view = sim.view();
    Game { sim, rt, view, events: Vec::new(), seen: Vec::new() }
}

impl Game {
    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            let mut out = Vec::new();
            self.rt.think(&self.view, &self.events, &mut out);
            let o = self.sim.step(&out);
            self.view = o.view;
            self.events = o.events;
            self.seen.extend(self.events.iter().cloned());
        }
        assert!(self.rt.errors().is_empty(), "{:?}", self.rt.errors());
        assert!(!self.seen.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", self.seen);
    }

    fn global<T: mlua::FromLua + crate::runtime::EvalResult>(&self, name: &str) -> T {
        self.rt.eval(&format!("return {name}")).unwrap()
    }

    fn pos(&self, id: EntityId) -> Option<[f32; 3]> {
        self.view.unit(id).map(|u| u.pos.to_f32())
    }
}

#[test]
fn converted_order_namespace_dispatches_attack_and_stop_through_the_sim() {
    let mut g = game(
        r#"
        assert(rawequal(GameObject.Order, Order))
        assert(type(GameObject.Order.Attack) == 'string')
        HERO_HANDLE = EntityFinder():findOne('hero')
        HERO_HANDLE:giveOrder(GameObject.Order.Attack, EntityFinder():findOne('foe'))
        "#,
    );
    let mut replay = g.sim.clone();
    let attack = g.rt.take_commands();
    assert_eq!(attack.len(), 1);
    assert_eq!(attack[0].player, PlayerId(0));
    assert_eq!(attack[0].kind, CommandKind::Order(Order::Attack { units: vec![HERO], target: FOE }));
    let out = g.sim.step(&attack);
    assert_eq!(replay.step(&attack).events, out.events);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));

    let mut commands = Vec::new();
    g.rt.think(&out.view, &out.events, &mut commands);
    assert!(commands.is_empty());
    g.rt.eval::<()>("HERO_HANDLE:giveOrder(GameObject.Order.Stop)").unwrap();
    let stop = g.rt.take_commands();
    assert_eq!(stop.len(), 1);
    assert_eq!(stop[0].player, PlayerId(0));
    assert_eq!(stop[0].kind, CommandKind::Order(Order::Stop { units: vec![HERO] }));
    let out = g.sim.step(&stop);
    assert_eq!(replay.step(&stop).events, out.events);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert_eq!(g.sim.digest(), replay.digest());
    assert!(g.rt.unsupported().is_empty());
}

#[test]
fn filter_and_point_metatables_can_queue_orders_without_context_borrow_panics() {
    let mut g = game(r#"
        local h = EntityFinder():findOne('hero')
        local filter = setmetatable({}, { __index = function(_, name)
            if name == 'player' then h:giveOrder(Order.Stop); return Player(0) end
        end })
        FILTERED = {}
        for _, u in ipairs(EntityFinder():find(filter)) do FILTERED[#FILTERED+1] = u.id end
        local p = setmetatable({y=0,z=0}, { __index = function(_, name)
            if name == 'x' then h:giveOrder(Order.Stop); return -600 end
        end })
        h:giveOrder(Order.Go, p)
        Monitor():hook(Monitor.Enters, p, 120, function() end)
    "#);
    assert_eq!(g.global::<Vec<u32>>("FILTERED"), [HERO.0, 3]);
    let commands = g.rt.take_commands();
    assert_eq!(commands.len(), 4);
    assert_eq!(commands.iter().filter(|c| matches!(c.kind, CommandKind::Order(Order::Stop { .. }))).count(), 3);
    assert_eq!(commands.iter().filter(|c| matches!(c.kind, CommandKind::Order(Order::Move { .. }))).count(), 1);
    assert!(!g.sim.step(&commands).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(g.rt.unsupported().is_empty());
}

#[test]
fn static_craft_and_producer_ancestry_remains_available_to_mission_finders() {
    use super::entity::is_type;
    // The parsed sensor label is already a craft category in sta_content.
    assert!(sta_content::odf::CRAFT_CLASSES.contains(&"sensor"));
    for class in ["sensor", "shipyard", "starbase", "miningstation"] {
        assert!(is_type(class, "Craft"), "{class} must be a Craft");
        assert!(is_type(class, "GameObject"));
        assert!(is_type(class, class));
    }
    for class in ["shipyard", "starbase", "miningstation"] {
        assert!(is_type(class, "Producer"));
    }
    assert!(is_type("Starbase", "Shipyard"));
    assert!(!is_type("sensor", "Producer"));
    assert!(!is_type("shipyard", "Starbase"));
    assert!(!is_type("planet", "Craft"));
    assert!(!is_type("unknown", "Craft"));
}

#[test]
fn entity_properties_read_the_view_and_content() {
    let g = game(
        r#"
        local h = EntityFinder():findOne('hero')
        ID, ODF, CLASS, OWNER, LABEL, VALID = h.id, h.odf, h.class, h.player.number, h.label, h.valid
        HULL, MAXHULL, X = h.hull, h.maxHull, h.position.x
        ISCRAFT, ISSTAR, ISENT, ISGO = h:isType(Entity.Type.Craft), h:isType(Entity.Type.Starbase), h:isType(Entity.Type.Entity), h:isType(Entity.Type.GameObject)
        RACE = h.race
        SAME = (h == EntityFinder():findOne('hero'))
        DIFF = (h == EntityFinder():findOne('foe'))
        CREW = h.crew
        "#,
    );
    assert_eq!(g.global::<u32>("ID"), HERO.0);
    assert_eq!(g.global::<String>("ODF"), "fx_cruiser.odf");
    assert_eq!(g.global::<String>("CLASS"), "craft");
    assert_eq!(g.global::<u8>("OWNER"), 0);
    assert_eq!(g.global::<String>("LABEL"), "hero");
    assert!(g.global::<bool>("VALID"));
    assert_eq!(g.global::<f64>("HULL"), 800.0);
    assert_eq!(g.global::<f64>("MAXHULL"), 800.0);
    assert_eq!(g.global::<f64>("X"), -1000.0);
    assert!(g.global::<bool>("ISCRAFT") && g.global::<bool>("ISENT") && g.global::<bool>("ISGO"));
    assert!(!g.global::<bool>("ISSTAR"));
    assert_eq!(g.global::<String>("RACE"), "fx");
    assert!(g.global::<bool>("SAME") && !g.global::<bool>("DIFF"));
    assert_eq!(g.global::<f64>("CREW"), 300.0);
}

#[test]
fn finder_filters_and_orders_results() {
    let g = game(
        r#"
        local f = EntityFinder()
        local function ids(t) local r = {} for i, e in ipairs(t) do r[i] = e.id end return table.concat(r, ',') end
        ALL = ids(f:find({}))
        P0 = ids(f:find({ player = 0 }))
        P1 = ids(f:find({ player = Player(1) }))
        FRIG = ids(f:find({ odf = 'fx_frigate' }))
        CLS = ids(f:find({ class = 'freighter' }))
        BYTYPE = ids(f:find({ type = Entity.Type.Craft }))
        NEAR = ids(f:find({ center = Vector(-1000, 0, 0), radius = 350 }))
        NEARENT = ids(f:find({ center = f:findOne('hero'), radius = 150, player = 0 }))
        NAME = ids(f:find('foe'))
        NONE = (f:findOne('nope') == nil)
        NONE2 = #f:find({ label = 'nope' })
        STATIC = EntityFinder.findOne('far').id
        BYNUM = ids(f:find(1))
        "#,
    );
    assert_eq!(g.global::<String>("ALL"), "1,2,3,4,5");
    assert_eq!(g.global::<String>("P0"), "2,3");
    assert_eq!(g.global::<String>("P1"), "4,5");
    assert_eq!(g.global::<String>("FRIG"), "3,4");
    assert_eq!(g.global::<String>("CLS"), "5");
    assert_eq!(g.global::<String>("BYTYPE"), "2,3,4,5");
    assert_eq!(g.global::<String>("NEAR"), "2,3,4");
    assert_eq!(g.global::<String>("NEARENT"), "2");
    assert_eq!(g.global::<String>("NAME"), "4");
    assert!(g.global::<bool>("NONE"));
    assert_eq!(g.global::<i64>("NONE2"), 0);
    assert_eq!(g.global::<u32>("STATIC"), 5);
    assert_eq!(g.global::<String>("BYNUM"), "4,5");
}

#[test]
fn give_order_moves_the_unit() {
    let mut g = game(
        r#"
        local hero, sent = EntityFinder():findOne('hero'), false
        MMM.register(function(t)
          if not sent then sent = true; hero:giveOrder(Order.Go, Vector(-1000, 0, 1500)) end
        end)
        "#,
    );
    let start = g.pos(HERO).unwrap();
    g.run(300);
    let end = g.pos(HERO).unwrap();
    assert!(end[2] > start[2] + 100.0, "{start:?} -> {end:?}");
}

#[test]
fn orders_become_player_commands_and_edits_come_from_script() {
    let mut g = game(
        r#"
        local f = EntityFinder()
        local hero, foe = f:findOne('hero'), f:findOne('foe')
        hero:giveOrder(Order.Attack, foe)
        hero:giveOrder(Order.Go, foe, true)
        hero:giveOrder(Order.Stop)
        hero:giveOrder(Order.SpecialWeaponHigh, Vector(1, 2, 3))
        hero:cloak(); hero:decloak()
        hero:giveOrder(Order.Patrol, foe)
        foe:giveOrder(Order.Go)
        foe:remove()
        foe.invincible = true
        foe.hull = 10
        foe.player = 0
        Player(0).resources.metal = 500
        "#,
    );
    let cmds = g.rt.take_commands();
    let kinds: Vec<_> = cmds.iter().map(|c| (c.player.0, c.seq)).collect();
    // Orders come from the unit's owner (0 for hero, 1 for foe), edits from SCRIPT (254).
    assert_eq!(kinds, [(0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (254, 1), (254, 2), (254, 3), (254, 4), (254, 5)]);
    assert!(matches!(&cmds[0].kind, CommandKind::Order(Order::Attack { target, .. }) if *target == FOE));
    assert!(matches!(&cmds[1].kind, CommandKind::Order(Order::Move { queue: true, .. })));
    assert!(matches!(&cmds[3].kind, CommandKind::Order(Order::Special { slot: 0, .. })));
    assert!(matches!(&cmds[4].kind, CommandKind::Order(Order::Cloak { on: true, .. })));
    assert!(matches!(&cmds[10].kind, CommandKind::Edit(Edit::Cargo(PlayerId(0), 2, _))));
    let unsupported = g.rt.unsupported();
    assert_eq!(
        unsupported,
        [("giveOrder Go: no point".to_owned(), 1), ("giveOrder Patrol".to_owned(), 1)]
    );
    g.run(1);
}

#[test]
fn monitor_on_destroy_fires_once() {
    let mut g = game(
        r#"
        FIRED, WHO, OWNER = 0, nil, nil
        local foe = EntityFinder():findOne('foe')
        M = Monitor()
        M:hook(Monitor.Destroyed, foe, function(dead)
          FIRED = FIRED + 1; WHO = dead.id; OWNER = dead.odf; VALID = dead.valid
        end)
        Monitor.timer(3, function() foe:remove() end)
        "#,
    );
    g.run(60);
    assert_eq!(g.global::<i64>("FIRED"), 0);
    g.run(120);
    assert_eq!(g.global::<i64>("FIRED"), 1);
    assert_eq!(g.global::<u32>("WHO"), FOE.0);
    assert!(!g.global::<bool>("VALID"));
    assert!(g.seen.iter().any(|e| matches!(e, SimEvent::Destroyed { id, .. } if *id == FOE)));
    g.run(60);
    assert_eq!(g.global::<i64>("FIRED"), 1);
    assert_eq!(g.global::<String>("M.state"), "Stopped");
}

#[test]
fn monitor_on_destroy_fires_when_combat_kills() {
    let mut g = game(
        r#"
        FIRED = 0
        local f = EntityFinder()
        local hero, foe = f:findOne('hero'), f:findOne('foe')
        Monitor.destroyed(foe, function() FIRED = FIRED + 1 end)
        hero:giveOrder(Order.Attack, foe)
        "#,
    );
    g.run(3000);
    assert!(g.view.unit(FOE).is_none(), "foe survived");
    assert_eq!(g.global::<i64>("FIRED"), 1);
}

#[test]
fn monitor_enters_fires_for_newcomers_only() {
    let mut g = game(
        r#"
        ENTERED = {}
        local hero, sent = EntityFinder():findOne('hero'), false
        -- hero starts at x=-1000: outside this ball; foe (-700) is inside.
        M = Monitor()
        M:hook(Monitor.Enters, Vector(-700, 0, 0), 120, function(u) ENTERED[#ENTERED + 1] = u.id end)
        MMM.register(function()
          if not sent then sent = true; hero:giveOrder(Order.Go, Vector(-700, 0, 100)) end
        end)
        "#,
    );
    g.run(600);
    let ids: Vec<u32> = g.global("ENTERED");
    assert_eq!(ids, [HERO.0], "{:?}", g.pos(HERO));
}

#[test]
fn monitor_timer_once_and_repeating_stop_and_unhook() {
    let mut g = game(
        r#"
        ONCE, EVERY, STOPPED = 0, 0, 0
        Monitor.timer(2, function() ONCE = ONCE + 1 end)
        Monitor.timer(1, function() EVERY = EVERY + 1 end, true)
        local m = Monitor()
        m:hook(Monitor.Timer, 3, function() STOPPED = STOPPED + 1 end)
        HOOKS = m.hooks
        m:stop()
        STATE = m.state
        BAD = Monitor()
        BAD:hook('Nonsense', 1)
        "#,
    );
    g.run(5 * 30 + 1);
    assert_eq!(g.global::<i64>("ONCE"), 1);
    assert_eq!(g.global::<i64>("EVERY"), 5);
    assert_eq!(g.global::<i64>("STOPPED"), 0);
    assert_eq!(g.global::<i64>("HOOKS"), 1);
    assert_eq!(g.global::<String>("STATE"), "Stopped");
    assert_eq!(g.rt.unsupported(), [("Monitor:hook Nonsense".to_owned(), 1)]);
}

#[test]
fn monitor_stop_all_and_error_isolation() {
    let mut g = game(
        r#"
        N, E = 0, 0
        Monitor.timer(1, function() N = N + 1 end, true)
        Monitor.timer(1, function() E = E + 1; error('boom') end, true)
        MMM.register(function(t) if t >= 3.5 then Monitor.stopAll() end end)
        "#,
    );
    for _ in 0..(5 * 30) {
        let mut out = Vec::new();
        g.rt.think(&g.view, &g.events, &mut out);
        let o = g.sim.step(&out);
        g.view = o.view;
        g.events = o.events;
    }
    assert_eq!(g.global::<i64>("N"), 3);
    assert_eq!(g.global::<i64>("E"), 3);
    assert_eq!(g.rt.errors().len(), 3);
    assert!(g.rt.errors()[0].contains("boom"));
}

#[test]
fn objectives_add_complete_fail() {
    let mut g = game(
        r#"
        local list = Objectives.objectives
        A = list:add('Destroy the foe')
        B = list:add{ title = 'Protect the hero', description = 'Do not lose it' }
        C = Objectives.objectives:add('Third')
        BEFORE = A.state
        A:complete()
        C:fail()
        B.description = 'changed'
        COUNT, COUNT2, LEN = Objectives.count, list.count, #list
        VISIBLE = Objectives.visible
        Objectives.visible = false
        TITLE = list:get(2).title
        STATES = A.state .. B.state .. C.state
        Objectives.load('whatever.obj')
        "#,
    );
    g.run(2);
    assert_eq!(g.global::<String>("BEFORE"), "active");
    assert_eq!((g.global::<i64>("COUNT"), g.global::<i64>("COUNT2"), g.global::<i64>("LEN")), (3, 3, 3));
    assert!(g.global::<bool>("VISIBLE"));
    assert_eq!(g.global::<String>("TITLE"), "Protect the hero");
    assert_eq!(g.global::<String>("STATES"), "completeactivefailed");
    let o = g.rt.objectives();
    assert_eq!(o.len(), 3);
    assert_eq!(o[0].state, ObjectiveState::Complete);
    assert_eq!(o[1].state, ObjectiveState::Active);
    assert_eq!(o[1].description, "changed");
    assert_eq!(o[2].state, ObjectiveState::Failed);
    assert_eq!((o[0].id, o[1].id, o[2].id), (1, 2, 3));
}

#[test]
fn player_relations_and_overrides() {
    let mut g = game(
        r#"
        local p0, p1 = Player(0), Player.get(1)
        R01, R10, SELF = p0:getRelation(p1), p1:getRelation(0), p0:getRelation(p0)
        p0:setRelation(p1, Relation.Ally)
        R01B, R10B = p0:getRelation(1), p1:getRelation(p0)
        p1:setRelation(p0, Relation.Neutral)
        R10C = p1:getRelation(p0)
        NAME, NUM, LOCAL = p0.name, p1.number, Game.localPlayer.number
        EQ = (Player(1) == p1) and (Player(0) ~= p1)
        OWNER_IS_P0 = EntityFinder():findOne('hero').player == p0
        p0:loadAIP('whatever.aip')
        UNKNOWN = p0.nothingHere.deeper()
        "#,
    );
    g.run(1);
    assert_eq!(g.global::<String>("R01"), "Enemy");
    assert_eq!(g.global::<String>("R10"), "Enemy");
    assert_eq!(g.global::<String>("SELF"), "Ally");
    assert_eq!(g.global::<String>("R01B"), "Ally");
    assert_eq!(g.global::<String>("R10B"), "Enemy");
    assert_eq!(g.global::<String>("R10C"), "Neutral");
    assert_eq!(g.global::<String>("NAME"), "Player 0");
    assert_eq!((g.global::<u8>("NUM"), g.global::<u8>("LOCAL")), (1, 0));
    assert!(g.global::<bool>("EQ") && g.global::<bool>("OWNER_IS_P0"));
    assert_eq!(g.rt.relation(PlayerId(0), PlayerId(1)), Relation::Ally);
    assert_eq!(g.rt.relation(PlayerId(1), PlayerId(0)), Relation::Neutral);
}

#[test]
fn resources_read_and_edit() {
    let mut g = game(
        r#"
        local p, done = Player(0), false
        MMM.register(function()
          if not done then done = true; p.resources.metal = 500; p.resources.dilithium = 40; p.resources.bogus = 1 end
          METAL = p.resources.metal
        end)
        "#,
    );
    g.run(5);
    assert_eq!(g.global::<f64>("METAL"), 500.0);
    assert_eq!(g.view.player(PlayerId(0)).unwrap().resources[0].to_f32(), 40.0);
    assert_eq!(g.rt.unsupported(), [("resources.bogus = x".to_owned(), 1)]);
}

#[test]
fn game_mission_and_print() {
    let mut g = game(
        r#"
        print('hello', 1)
        MMM.register(function(t) TIME = Game.time; SEED = Game.seed; SLOT = Game.localSlot; Mission.finish(true) end)
        "#,
    );
    g.run(30);
    assert!(g.rt.finished());
    assert_eq!(g.rt.log(), ["hello\t1"]);
    assert_eq!(g.global::<f64>("TIME"), 29.0 / 30.0);
    assert_eq!((g.global::<i64>("SEED"), g.global::<i64>("SLOT")), (1, 0));
}

#[test]
fn vector_arithmetic() {
    let g = game(
        r#"
        local a, b = Vector(1, 2, 3), Vector(4, 6, 3)
        local s, d, m = a + b, b - a, a * 2
        S = s.x .. ',' .. s.y .. ',' .. s.z
        D = (b - a):length()
        M = m.z
        DOT = a * b
        DIST = a:distance(b)
        EQ = (a == Vector(1, 2, 3))
        "#,
    );
    assert_eq!(g.global::<String>("S"), "5,8,6");
    assert_eq!(g.global::<f64>("D"), 5.0);
    assert_eq!(g.global::<f64>("M"), 6.0);
    assert_eq!(g.global::<f64>("DOT"), 4.0 + 12.0 + 9.0);
    assert_eq!(g.global::<f64>("DIST"), 5.0);
    assert!(g.global::<bool>("EQ"));
}

#[test]
fn same_script_same_commands() {
    let src = r#"
        local f = EntityFinder()
        MMM.register(function(t)
          for _, e in ipairs(f:find({ player = 0 })) do
            e:giveOrder(Order.Go, Vector(math.random(-500, 500), 0, math.random(-500, 500)))
          end
        end)
        "#;
    let cmds = |()| {
        let mut g = game(src);
        let mut all = Vec::new();
        for _ in 0..30 {
            let mut out = Vec::new();
            g.rt.think(&g.view, &[], &mut out);
            all.extend(out);
        }
        all
    };
    assert_eq!(cmds(()), cmds(()));
}

/// Without a view the finders see nothing and nothing errors.
#[test]
fn empty_world_is_safe() {
    let mut rt = MissionRuntime::from_source(
        "missions/T.lua",
        "local e = EntityFinder():findOne('x'); R = e == nil\nMMM.register(function() local n = #EntityFinder():find({}) end)",
        1,
    );
    let mut out = Vec::new();
    for _ in 0..3 {
        rt.think(&WorldView::default(), &[], &mut out);
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(out.is_empty());
}

#[test]
fn finder_constructor_predicates_filter_and_combine_with_query_filters() {
    let g = game(r#"
        local f = EntityFinder(function(e) return e.player.number == 0 end)
        local function ids(t) local r={} for i,e in ipairs(t) do r[i]=e.id end return table.concat(r, ',') end
        ALL = ids(f:find())
        FRIG = ids(f:find({odf='FX_FRIGATE.ODF'}))
        FIRST = f:findOne().id
        STATIC = ids(EntityFinder.find(function(e) return e.label == 'foe' end))
        BYFILE = EntityFinder():findOne('FX_CRUISER.ODF').id
        RELATION = Player.Relation.Enemy == Relation.Enemy
    "#);
    assert_eq!(g.global::<String>("ALL"), "2,3");
    assert_eq!(g.global::<String>("FRIG"), "3");
    assert_eq!(g.global::<u32>("FIRST"), HERO.0);
    assert_eq!(g.global::<String>("STATIC"), "4");
    assert_eq!(g.global::<u32>("BYFILE"), HERO.0);
    assert!(g.global::<bool>("RELATION"));
}

#[test]
fn finder_predicates_can_reenter_the_api_and_queue_edits() {
    let mut g = game(r#"
        local f = EntityFinder(function(e)
            if e.player.number == 0 then e.hull = 10; return true end
            return false
        end)
        COUNT = #f:find()
    "#);
    assert_eq!(g.global::<u32>("COUNT"), 2);
    assert_eq!(g.rt.take_commands().len(), 2);
    assert!(g.rt.unsupported().is_empty());
}

#[test]
fn finder_predicates_are_isolated_and_errors_remain_visible() {
    let g = game(r#"
        local a = EntityFinder(function(e) return e.label == 'hero' end)
        local b = EntityFinder(function(e) return e.label == 'foe' end)
        A, B = a:findOne().id, b:findOne().id
        OK = pcall(function() EntityFinder(function() error('predicate failed') end):find() end)
        INVALID = pcall(function() EntityFinder(3) end)
        AGAIN = a:findOne().id
    "#);
    assert_eq!(g.global::<u32>("A"), HERO.0);
    assert_eq!(g.global::<u32>("B"), FOE.0);
    assert!(!g.global::<bool>("OK"));
    assert!(!g.global::<bool>("INVALID"));
    assert_eq!(g.global::<u32>("AGAIN"), HERO.0);
}

#[test]
fn officers_and_crew_assignments_queue_the_typed_edits_and_are_not_counted() {
    let mut g = game("MMM.register(function() Player(0).maxOfficers = 777; Player(0).resources.crew = 321; EntityFinder():find()[2].crew = 5 end)");
    let mut out = Vec::new();
    g.rt.think(&g.view, &g.events, &mut out);
    let edits: Vec<_> = out.iter().filter_map(|c| match &c.kind { CommandKind::Edit(e) => Some(e.clone()), _ => None }).collect();
    assert!(edits.contains(&Edit::MaxOfficers(PlayerId(0), Fixed::from_int(777))), "{edits:?}");
    assert!(edits.contains(&Edit::TeamCrew(PlayerId(0), Fixed::from_int(321))), "{edits:?}");
    assert!(edits.contains(&Edit::UnitCrew(HERO, Fixed::from_int(5))), "{edits:?}");
    let unsupported = g.rt.unsupported();
    assert!(unsupported.iter().all(|(n, _)| !n.contains("maxOfficers") && !n.contains("resources.crew") && !n.contains("Entity.crew")), "{unsupported:?}");
}

#[test]
fn personnel_assignment_edge_cases_are_recorded_not_aborting() {
    let mut g = game("MMM.register(function() local e = EntityFinder():find()[2]; e.crew = nil; e.crew = 0/0; Player(0).maxOfficers = 5; DONE = true end)");
    for _ in 0..3 {
        let mut out = Vec::new();
        g.rt.think(&g.view, &g.events, &mut out);
        let o = g.sim.step(&out);
        g.view = o.view;
        g.events = o.events;
    }
    assert!(g.global::<bool>("DONE"), "the callback was not aborted: {:?}", g.rt.errors());
    let names: Vec<String> = g.rt.unsupported().into_iter().map(|(n, _)| n).collect();
    // A seat with no officer limit takes the script's (SIM-54: a raw write in the original).
    assert!(!names.iter().any(|n| n.contains("Player.maxOfficers")), "{names:?}");
    for want in ["Entity.crew = non-number", "Entity.crew = non-finite"] {
        assert!(names.iter().any(|n| n.contains(want)), "{want} in {names:?}");
    }
    let mut g = game("MMM.register(function() Player(0).maxOfficers = 0/0 end)");
    let mut out = Vec::new();
    g.rt.think(&g.view, &g.events, &mut out);
    assert!(g.rt.errors().iter().any(|e| e.contains("requires a finite number")), "{:?}", g.rt.errors());
}

#[test]
fn entity_shields_is_a_fraction_of_the_maximum_applied_by_the_sim() {
    let mut g = game(
        "MMM.register(function()
            local e = EntityFinder():find()[2]
            e.shields = 0.0
            DONE = true
        end)",
    );
    g.run(3);
    assert!(g.global::<bool>("DONE"));
    assert!(g.rt.unsupported().iter().all(|(n, _)| !n.contains("Entity.shields")));
    let hero = g.sim.units.get(HERO).unwrap();
    assert!(hero.shields.now[0] < hero.shields.max, "{:?}", hero.shields);
}

// ---- SIM-54: attribute assignments, read back at once and applied by the sim ----

#[test]
fn max_hull_is_a_per_unit_clock_that_reads_back_at_once_and_reaches_the_sim() {
    // The Roots idiom: a unit's maxHull as a counter, `x.maxHull = x.maxHull + 1.0`, several times
    // in one hook; the other cruiser-class unit is untouched.
    let mut g = game(
        "MMM.register(function()
            if DONE == true then return end
            local e = EntityFinder():find()[2]
            e.maxHull = 1
            for i = 1, 3 do e.maxHull = e.maxHull + 1.0 end
            SEEN = e.maxHull
            e.hullValue = 7000
            HULL = e.hullValue
            e.name = 'Relay Station'
            NAME = e.name
            e.race = Race.find('fx')
            RACE = e.race
            DONE = true
        end)",
    );
    g.run(3);
    assert!(g.global::<bool>("DONE"));
    assert_eq!(g.global::<f64>("SEEN"), 4.0);
    assert_eq!(g.global::<f64>("HULL"), 7000.0);
    assert_eq!(g.global::<String>("NAME"), "Relay Station");
    assert_eq!(g.global::<String>("RACE"), "fx");
    let hero = g.sim.units.get(HERO).unwrap();
    assert_eq!(hero.hull, [Fixed::from_int(7000), Fixed::from_int(4)]);
    assert_eq!(g.sim.units.get(EntityId(3)).unwrap().hull[1], g.sim.content().unwrap().unit(g.sim.units.get(EntityId(3)).unwrap().def).unwrap().hull);
    assert_eq!(g.view.attrs.names, vec![(HERO, "Relay Station".to_string())]);
    let unsupported = g.rt.unsupported();
    assert!(unsupported.iter().all(|(n, _)| !n.contains("maxHull") && !n.contains("hullValue") && !n.contains("Entity.name")), "{unsupported:?}");
}

#[test]
fn assignments_on_a_unit_added_this_tick_are_held_and_read_back_until_it_lands() {
    let mut g = game(
        "MMM.register(function()
            if DONE == true then return end
            local n = Entity.add('fx_frigate', Player(0), Vector(500, 0, 500))
            n.maxCrew = 20
            n.crew = 12
            n.maxHull = 99
            n.maxShields = 360
            n.shields = 0.5
            n.name = 'Reinforcement'
            n.forward = Vector(1, 0, 0)
            READ = n.maxHull
            NAME = n.name
            DONE = true
        end)",
    );
    g.run(4);
    assert!(g.global::<bool>("DONE"));
    assert_eq!(g.global::<f64>("READ"), 99.0);
    assert_eq!(g.global::<String>("NAME"), "Reinforcement");
    let id = g.sim.units.iter().map(|u| u.id).max().unwrap();
    let u = g.sim.units.get(id).unwrap();
    assert_eq!(u.crew, Some([Fixed::from_int(12), Fixed::from_int(20)]));
    assert_eq!(u.hull[1], Fixed::from_int(99));
    assert_eq!(u.shields.max, Fixed::from_int(360));
    assert!(u.shields.now[0] >= Fixed::from_int(180) && u.shields.now[0] < Fixed::from_int(190), "{:?}", u.shields);
    assert_eq!(u.heading, (Fixed::ONE, Fixed::ZERO));
    assert_eq!(g.view.attrs.names, vec![(id, "Reinforcement".to_string())]);
    let unsupported = g.rt.unsupported();
    assert!(unsupported.iter().all(|(n, _)| !n.contains("pending")), "{unsupported:?}");
}

#[test]
fn entity_forward_and_omega_queue_sim_edits_and_bad_input_is_recorded() {
    let mut g = game(
        "MMM.register(function()
            local e = EntityFinder():find()[2]
            e.forward = Vector(0, 3, -4)
            e.omega = Vector(0, 0.5, 0)
            e.forward = Vector(0, 5, 0)
            e.forward = Vector(0/0, 0, 1)
            e.omega = 5
            DONE = true
        end)",
    );
    g.run(3);
    assert!(g.global::<bool>("DONE"), "bad input must not abort the callback");
    let hero = g.sim.units.get(HERO).unwrap();
    // y is ignored for the facing; (0, -4) normalised is -z.
    assert_eq!(hero.heading, (sta_core::Fixed::ZERO, -sta_core::Fixed::ONE));
    // The omega edit reached the sim and was accepted; the unit's engines are on, so it changed nothing.
    assert_eq!(hero.omega, sta_core::Fixed::ZERO);
    let names: Vec<String> = g.rt.unsupported().into_iter().map(|(n, _)| n).collect();
    assert!(names.iter().any(|n| n.contains("no horizontal direction")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("Entity.forward = a Vector without finite")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("Entity.omega = non-Vector")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("simulation rejected")), "{names:?}");
}

#[test]
fn recorded_assignments_keep_the_callback_running_and_name_each_gap() {
    let mut g = game(
        "MMM.register(function()
            local p = Player(0)
            p.race = Race.find('fx'); p.color = Color(1, 0, 0); p.name = 'x'
            local e = EntityFinder():find()[2]
            e.movementAutonomy = 1
            e.specialWeaponAutonomy = 1
            DONE = true
        end)",
    );
    g.run(2);
    assert!(g.global::<bool>("DONE"));
    let names: Vec<String> = g.rt.unsupported().into_iter().map(|(n, _)| n).collect();
    for want in ["Player.color", "Player.name", "Entity.movementAutonomy", "Entity.specialWeaponAutonomy"] {
        assert!(names.iter().any(|n| n.starts_with(want)), "{want} in {names:?}");
    }
    // SIM-54: race and name are applied, not counted, and read back.
    assert!(!names.iter().any(|n| n.contains("race") || n.contains("Entity.name")), "{names:?}");
}

#[test]
fn condition_monitor_and_text_input_callbacks_get_a_hook_object_like_installed_scripts() {
    let mut g = game(
        "seen = {}
        local m = Monitor('done', false, nil, function() return true end, nil)
        m:hook('onDone', MonitorHook.Type.OnTrue, nil, function(monitor, hook)
            seen[#seen+1] = hook.argument; seen[#seen+1] = hook.id
        end, 7)",
    );
    g.run(2);
    assert_eq!(g.global::<i64>("seen[1]"), 7);
    assert_eq!(g.global::<String>("seen[2]"), "onDone");
}

// ---- LUA-13: an end that rests on a gap is blocked, any other stands -------

use crate::runtime::MissionEndState;

/// Ids: 2 hero (p0), 3 wing (p0), 4 foe (p1), 5 far (p1). `Colonize` raises
/// and is counted unsupported: a gap call that names both handles.
fn end_state(script: &str) -> MissionEndState {
    let mut g = game(script);
    g.run(5);
    g.rt.mission_end(g.view.tick)
}

#[test]
fn an_end_with_no_gap_on_its_path_stands_even_with_unsupported_calls_elsewhere() {
    // A gap call on foe (4) and far (5), then a real loss decided on the hero
    // (2) and a count over the hero's team, which has nothing tainted.
    let s = "pcall(A2.Colonize, 4, 5)\nif A2.IsAlive(2) and A2.CountUnits(A2.GetRealTeam(2), 0) > 0 then A2.FailMission(A2.GetTime(), 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: false });
    let s = "pcall(A2.Colonize, 3, 5)\nif A2.IsAlive(2) then A2.SucceedMission(0, 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: true });
}

#[test]
fn an_end_read_off_a_tainted_handle_is_blocked_and_not_a_win_or_a_loss() {
    let s = "pcall(A2.Colonize, 3, 5)\nif A2.IsAlive(3) then A2.SucceedMission(0, 0) end";
    match end_state(s) {
        MissionEndState::Blocked(why) => assert!(why.contains("SucceedMission") && why.contains("IsAlive") && why.contains("object 3"), "{why}"),
        other => panic!("{other:?}"),
    }
    // The same on a handle named by a call that is not bound at all.
    let s = "A2.NoteGap('CommandAllShips', 2)\nif A2.IsCompromised(2, 99) then A2.FailMission(0, 0) end";
    assert!(matches!(end_state(s), MissionEndState::Blocked(_)));
}

#[test]
fn a_team_count_is_tainted_only_over_a_team_with_a_tainted_object() {
    // Foe (4) belongs to team B: a count over team B or over any team is read
    // off a gap; a count over the hero's team is not.
    let s = "pcall(A2.Colonize, 4, 5)\nif A2.CountUnits(A2.GetRealTeam(4), 0) > 0 then A2.SucceedMission(0, 0) end";
    assert!(matches!(end_state(s), MissionEndState::Blocked(_)));
    let s = "pcall(A2.Colonize, 4, 5)\nif A2.CountUnits(-1, 0) > 0 then A2.SucceedMission(0, 0) end";
    assert!(matches!(end_state(s), MissionEndState::Blocked(_)));
    let s = "pcall(A2.Colonize, 4, 5)\nif A2.CountUnits(A2.GetRealTeam(2), 0) > 0 then A2.SucceedMission(0, 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: true });
}

#[test]
fn a_handle_that_never_existed_blocks_only_after_a_handle_producing_gap() {
    let s = "if not A2.IsAlive(0) then A2.FailMission(0, 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: false });
    // Some other unsupported call changes nothing for it.
    let s = "pcall(A2.Colonize, 3, 5)\nif not A2.IsAlive(0) then A2.FailMission(0, 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: false });
    let s = "A2.NoteGap('BuildObject', 'x')\nif not A2.IsAlive(0) then A2.FailMission(0, 0) end";
    assert!(matches!(end_state(s), MissionEndState::Blocked(_)));
}

#[test]
fn a_read_after_the_end_call_does_not_block_it_and_the_last_call_wins() {
    let s = "A2.FailMission(0, 0)\npcall(A2.Colonize, 3, 5)\nlocal _ = A2.IsAlive(3)";
    assert_eq!(end_state(s), MissionEndState::Ended { won: false });
    // Reads never clear: a call after a tainted read is blocked, whatever it is.
    let s = "pcall(A2.Colonize, 3, 5)\nlocal _ = A2.IsAlive(3)\nA2.FailMission(0, 0)\nA2.SucceedMission(0, 0)";
    match end_state(s) {
        MissionEndState::Blocked(why) => assert!(why.contains("SucceedMission"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Two clean calls: the last one decides.
    assert_eq!(end_state("A2.FailMission(0, 0)\nA2.SucceedMission(0, 0)"), MissionEndState::Ended { won: true });
    assert_eq!(end_state("A2.SucceedMission(0, 0)\nA2.FailMission(0, 0)"), MissionEndState::Ended { won: false });
}

#[test]
fn a_raising_read_and_a_non_handle_integer_are_judged_apart() {
    // IsShipAliveAndOffMap has no data: its answer is a guess, so it is a tainted read.
    let s = "pcall(A2.IsShipAliveAndOffMap, 3)\nif A2.IsAlive(2) then A2.SucceedMission(0, 0) end";
    assert!(matches!(end_state(s), MissionEndState::Blocked(_)));
    // A failed call whose first integer is no object (an objective index) taints nothing.
    let s = "pcall(A2.Colonize, 900, 901)\nif A2.CountUnits(A2.GetRealTeam(4), 0) > 0 then A2.SucceedMission(0, 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: true });
}

#[test]
fn a_gap_that_changes_who_fights_taints_every_later_read() {
    // A world-effect gap with no object, then a win decided on a healthy hero.
    let s = "A2.NoteGap('DisableAllAITeams')\nif A2.IsAlive(2) then A2.SucceedMission(0, 0) end";
    assert!(matches!(end_state(s), MissionEndState::Blocked(_)));
}

#[test]
fn a_gap_that_is_not_a_world_effect_taints_no_unrelated_read() {
    let s = "A2.NoteGap('CollisionPrevention', 1)\nif A2.IsAlive(2) then A2.SucceedMission(0, 0) end";
    assert_eq!(end_state(s), MissionEndState::Ended { won: true });
}
