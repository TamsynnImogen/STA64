use std::{sync::Arc, time::Duration};

use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_sim::command::LocalRouter;
use sta_sim::setup::{GameOptions, GameSetup};
use sta_sim::{Fixed, PlayerId, Sim, WorldSource};

use super::*;
use crate::clock::{AnimClock, ClockMode};
use crate::sim_link::SimLinkPlugin;

fn app() -> (App, EntityId) {
    app_with_content(sta_content::fake::db())
}

fn app_with_content(content: sta_content::ContentDb) -> (App, EntityId) {
    let content = Arc::new(content);
    let setup = GameSetup {
        seed: 1,
        players: Vec::new(),
        map: Arc::new(sta_content::fake::two_bases()),
        content,
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim.spawn_def(
        setup.content.key("fx_cruiser").unwrap(),
        PlayerId(0),
        SimVec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
    ).unwrap();
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins((MinimalPlugins, StatesPlugin))
        .init_state::<AppState>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Aim>()
        .init_resource::<crate::ui::theme::Theme>()
        .add_message::<ButtonClicked>()
        .add_message::<crate::ui::theme::DropdownChanged>()
        .add_message::<Notice>()
        .add_message::<crate::game::flow::LeaveMatch>()
        .insert_resource(ContentRes(setup.content))
        .insert_resource(AnimClock::new(ClockMode::Wall, 0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
        .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
        .add_plugins((SimLinkPlugin, EditorPlugin));
    crate::app::configure_sets(&mut app);
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::InGame);
    app.update();
    (app, id)
}

/// Write Bevy's input resource in-process. No OS keyboard events are involved.
fn hotkey(app: &mut App) {
    chord(app, KeyCode::KeyE);
}

fn chord(app: &mut App, key: KeyCode) {
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.press(key);
    }
    app.update();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().reset_all();
}

fn press(app: &mut App, key: KeyCode) {
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
    app.update();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().reset_all();
}

fn point(app: &mut App, cursor: Vec2, at: Vec3) {
    app.world_mut().resource_mut::<Aim>().0 = Some(crate::input::select::AimState {
        cursor,
        ray: Ray3d::new(at + Vec3::Y * 45.0, Dir3::NEG_Y),
        clip_from_world: Mat4::IDENTITY,
        size: Vec2::new(1000.0, 800.0),
    });
}

fn mouse(app: &mut App, pressed: bool) {
    let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    buttons.clear();
    if pressed {
        buttons.press(MouseButton::Left);
    } else {
        buttons.release(MouseButton::Left);
    }
    app.update();
}

#[test]
fn ctrl_e_pauses_and_resumes_the_same_local_world() {
    let (mut app, id) = app();
    let tick = app.world().resource::<SimLink>().curr().tick;
    hotkey(&mut app);
    assert!(app.world().contains_resource::<Editing>());
    assert!(app.world().resource::<SimLink>().fog_lifted());
    let at = SimVec3::from_f32([29.0, 0.0, 43.0]);
    assert_eq!(
        app.world_mut().resource_mut::<SimLink>().edit_placement(PlacementEdit::Transform { id, at, quarter_turns: 1 }),
        Some(id),
    );
    app.update();
    {
        let link = app.world().resource::<SimLink>();
        assert_eq!(link.curr().tick, tick);
        let unit = link.curr().unit(id).unwrap();
        assert_eq!(unit.pos, at);
        assert_eq!(unit.heading, (Fixed::ONE, Fixed::ZERO));
    }
    hotkey(&mut app);
    app.update();
    assert!(!app.world().contains_resource::<Editing>());
    let link = app.world().resource::<SimLink>();
    assert!(!link.fog_lifted());
    assert!(link.curr().tick > tick);
    assert_eq!(link.curr().unit(id).unwrap().pos, at);
}

#[test]
fn editor_unit_visibility_survives_reveal_updates_and_restores_play_setting() {
    let (mut app, _) = app();
    app.add_plugins(crate::render::reveal::RevealPlugin);
    hotkey(&mut app);
    app.update();
    assert!(app.world().resource::<SimLink>().fog_lifted(), "reveal update must keep editor units visible");
    hotkey(&mut app);
    app.update();
    assert!(!app.world().resource::<SimLink>().fog_lifted(), "normal hidden-unit rules return");

    app.world_mut().resource_mut::<crate::render::reveal::Reveal>().fog = true;
    app.update();
    hotkey(&mut app);
    app.update();
    hotkey(&mut app);
    app.update();
    assert!(app.world().resource::<SimLink>().fog_lifted(), "a preexisting play reveal is restored");
}

#[test]
fn editor_shortcuts_do_not_reach_play_shortcuts() {
    #[derive(Default, Resource)]
    struct Seen(Vec<KeyCode>);

    let (mut app, _) = app();
    app.init_resource::<Seen>().add_systems(Update, |keys: Res<ButtonInput<KeyCode>>, mut seen: ResMut<Seen>| {
        seen.0.extend([KeyCode::F10, KeyCode::Tab].into_iter().filter(|key| keys.just_pressed(*key)));
    });
    press(&mut app, KeyCode::F10);
    press(&mut app, KeyCode::Tab);
    hotkey(&mut app);
    app.world_mut().resource_mut::<Editing>().palette.root = vec![palette::Entry {
        slot: 9,
        label: "Planets".into(),
        kind: EntryKind::Branch(Vec::new()),
    }];
    press(&mut app, KeyCode::F10);
    assert_eq!(app.world().resource::<Editing>().directory, [0]);
    press(&mut app, KeyCode::Tab);
    assert!(app.world().resource::<Editing>().directory.is_empty());
    assert_eq!(app.world().resource::<Seen>().0, [KeyCode::F10, KeyCode::Tab]);
}

#[test]
fn function_keys_navigate_sparse_slots_and_place_at_the_current_pointer() {
    let (mut app, _) = app();
    let map = sta_bzn::Map { version: 3013, size: [1000.0, 100.0, 1000.0], ..default() };
    let bytes = sta_bzn::write::write(&map).unwrap();
    let doc = Document::new("bzn/test.bzn".into(), bytes, map).unwrap();
    app.insert_resource(doc).insert_resource(AuthoredIds(BTreeMap::new()));
    hotkey(&mut app);

    let branch = |slot, children| palette::Entry { slot, label: "Group".into(), kind: EntryKind::Branch(children) };
    let leaf = palette::Entry { slot: 5, label: "Cruiser".into(), kind: EntryKind::Leaf { odf: "fx_cruiser".into(), neutral: true } };
    app.world_mut().resource_mut::<Editing>().palette.root = vec![branch(1, vec![branch(11, vec![leaf])])];

    press(&mut app, KeyCode::F1);
    assert!(app.world().resource::<Editing>().directory.is_empty());
    press(&mut app, KeyCode::F2);
    press(&mut app, KeyCode::F12);
    assert_eq!(app.world().resource::<Editing>().directory.len(), 2);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.world().resource::<Editing>().directory.len(), 1);
    press(&mut app, KeyCode::F12);
    press(&mut app, KeyCode::F6);
    assert!(app.world().resource::<Document>().objects.is_empty(), "missing aim cannot place");
    point(&mut app, Vec2::new(500.0, 400.0), Vec3::new(21.0, 0.0, -76.0));
    let root = app.world_mut().query_filtered::<Entity, With<panel::Root>>().single(app.world()).unwrap();
    app.world_mut().get_mut::<bevy::ui::RelativeCursorPosition>(root).unwrap().cursor_over = true;
    press(&mut app, KeyCode::F6);
    assert!(app.world().resource::<Document>().objects.is_empty(), "the menu covers the pointer");
    // Mouse buttons just select; an invalid placement attempt never arms one.
    mouse(&mut app, true);
    mouse(&mut app, false);
    assert!(app.world().resource::<Document>().objects.is_empty());
    press(&mut app, KeyCode::F6);
    let id = app.world().resource::<Selection>().0[0];
    let index = app.world().resource::<AuthoredIds>().0[&id];
    let at = [21.0, 0.0, 76.0];
    let unit = app.world().resource::<SimLink>().curr().unit(id).unwrap();
    assert_eq!(unit.pos.to_f32(), at);
    assert_eq!(unit.owner, sta_sim::world::NEUTRAL);

    let doc = app.world().resource::<Document>();
    let saved = sta_bzn::parse(&doc.bytes().unwrap()).unwrap();
    assert_eq!(&saved.objects[0].transform[9..], &at);
    assert_eq!(saved.objects[0].team, 0);
    assert_eq!(doc.object(index).unwrap().odf, "fx_cruiser");

    press(&mut app, KeyCode::Delete);
    assert!(app.world().resource::<SimLink>().curr().unit(id).is_none());
    assert!(app.world().resource::<Document>().object(index).is_none());

    let mut map = app.world().resource::<Document>().map.clone();
    map.objects.push(sta_bzn::MapObject { entity_id: i32::MAX, ..saved.objects[0].clone() });
    let bytes = sta_bzn::write::write(&map).unwrap();
    app.insert_resource(Document::new("bzn/test.bzn".into(), bytes, map).unwrap());
    let count = app.world().resource::<SimLink>().curr().units.len();
    press(&mut app, KeyCode::F6);
    assert_eq!(app.world().resource::<SimLink>().curr().units.len(), count,
        "a placement that cannot be saved must not appear in the world");
}

#[test]
fn two_completed_clicks_on_one_object_open_its_stats() {
    let (mut app, id) = app();
    hotkey(&mut app);
    app.world_mut().spawn((
        crate::sim_link::SimEntity(id),
        crate::render::units::SelectRadius(20.0),
        GlobalTransform::IDENTITY,
    ));
    let cursor = Vec2::new(540.0, 370.0);
    point(&mut app, cursor, Vec3::ZERO);
    mouse(&mut app, true);
    mouse(&mut app, false);
    assert_eq!(app.world().resource::<Selection>().0, [id]);
    assert!(app.world().resource::<Editing>().inspected.is_none());

    mouse(&mut app, true);
    assert!(app.world().resource::<Editing>().inspected.is_none());
    mouse(&mut app, false);
    assert_eq!(app.world().resource::<Editing>().inspected, Some(id));
    let close = app.world_mut().query::<&panel::Act>().iter(app.world())
        .any(|act| matches!(act, panel::Act::CloseStats));
    assert!(close, "the object stats panel opened");
    press(&mut app, KeyCode::Delete);
    assert!(app.world().resource::<SimLink>().curr().unit(id).is_some(), "stats holds editor tools");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.world().resource::<Editing>().inspected, Some(id), "Tab stays inside the form");
    press(&mut app, KeyCode::Escape);
    assert!(app.world().resource::<Editing>().inspected.is_none());

    // A move starting from the object is never the second click of a double-click.
    mouse(&mut app, true);
    mouse(&mut app, false);
    mouse(&mut app, true);
    point(&mut app, cursor + Vec2::X * 65.0, Vec3::new(62.0, 0.0, 43.0));
    mouse(&mut app, false);
    assert!(app.world().resource::<Editing>().inspected.is_none());
    assert_eq!(app.world().resource::<SimLink>().curr().unit(id).unwrap().pos.to_f32(),
        [62.0, 0.0, -43.0]);
}

#[test]
fn inspector_apply_is_atomic_and_undo_restores_both_world_and_map() {
    let (mut app, id) = app();
    let object = sta_bzn::MapObject {
        odf: "fx_cruiser".into(),
        entity_id: 12,
        team: 1,
        transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        initial_position: Some([0.0; 3]),
        ..default()
    };
    let map = sta_bzn::Map {
        version: 3013, size: [1000.0, 100.0, 1000.0], objects: vec![object], ..default()
    };
    let bytes = sta_bzn::write::write(&map).unwrap();
    app.insert_resource(Document::new("bzn/test.bzn".into(), bytes, map).unwrap());
    app.insert_resource(AuthoredIds(BTreeMap::from([(id, 0)])));
    app.init_resource::<crate::ui::hud::instance_names::InstanceNames>();
    hotkey(&mut app);
    app.world_mut().resource_mut::<Editing>().inspected = Some(id);
    app.update();
    let initial = app.world().resource::<SimLink>().real_curr().unit(id).unwrap().clone();
    let draft = |app: &mut App, key, value: &str| {
        let mut fields = app.world_mut().query::<(&panel::InspectorField, &mut TextField)>();
        let (_, mut field) = fields.iter_mut(app.world_mut()).find(|(field, _)| field.0 == key).expect(key);
        field.text = value.into();
    };
    let apply = |app: &mut App| {
        let mut buttons = app.world_mut().query::<(Entity, &panel::Act)>();
        let (entity, _) = buttons.iter(app.world()).find(|(_, act)| matches!(act, panel::Act::ApplyStats)).unwrap();
        app.world_mut().write_message(ButtonClicked(entity));
        app.update();
    };

    draft(&mut app, "x", "45");
    draft(&mut app, "hull", "999999");
    apply(&mut app);
    assert_eq!(app.world().resource::<Editing>().inspected, Some(id));
    assert!(!app.world().resource::<Editing>().inspector_error.is_empty());
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap(), &initial);
    assert!(app.world().resource::<Document>().properties(12).is_none());

    draft(&mut app, "hull", "25");
    draft(&mut app, "name", "Endeavour");
    draft(&mut app, "captain", "Imani");
    apply(&mut app);
    assert!(app.world().resource::<Editing>().inspected.is_none());
    let link = app.world().resource::<SimLink>();
    assert_eq!(link.real_curr().unit(id).unwrap().pos.x, Fixed::from_int(45));
    assert_eq!(link.real_curr().unit(id).unwrap().hull[0], Fixed::from_int(25));
    let doc = app.world().resource::<Document>();
    assert_eq!(doc.object(0).unwrap().instance_name, "Endeavour");
    assert_eq!(doc.properties(12).unwrap().captain.as_deref(), Some("Imani"));

    chord(&mut app, KeyCode::KeyZ);
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap(), &initial);
    assert!(app.world().resource::<Document>().properties(12).is_none());
    chord(&mut app, KeyCode::KeyY);
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.x, Fixed::from_int(45));
    assert_eq!(app.world().resource::<Document>().object(0).unwrap().instance_name, "Endeavour");
}

#[test]
fn a_resource_object_can_be_copied_pasted_and_undone() {
    let (mut app, _) = app();
    hotkey(&mut app);
    app.world_mut().resource_mut::<Editing>().palette.root = vec![palette::Entry {
        slot: 0,
        label: "Moon".into(),
        kind: EntryKind::Leaf { odf: "fx_moon".into(), neutral: true },
    }];
    let pointer = Vec2::new(500.0, 400.0);
    point(&mut app, pointer, Vec3::new(16.0, 0.0, 23.0));
    press(&mut app, KeyCode::F1);
    let source = app.world().resource::<Selection>().0[0];
    chord(&mut app, KeyCode::KeyC);
    point(&mut app, pointer, Vec3::new(86.0, 0.0, 94.0));
    chord(&mut app, KeyCode::KeyV);
    let copy = app.world().resource::<Selection>().0[0];
    let view = app.world().resource::<SimLink>().real_curr();
    assert_ne!(source, copy);
    assert_eq!(view.unit(copy).unwrap().owner, sta_sim::world::NEUTRAL);
    assert_eq!(view.unit(copy).unwrap().pos.to_f32(), [86.0, 0.0, -94.0]);
    chord(&mut app, KeyCode::KeyZ);
    assert!(app.world().resource::<SimLink>().real_curr().unit(copy).is_none());
}


#[test]
fn inspector_updates_a_healthy_subsystem_and_perceived_team_without_changing_ownership() {
    let mut ship = sta_content::UnitDef::new("fx_cruiser", "freighter");
    ship.race = "red".into();
    ship.hull = Fixed::from_int(100);
    ship.system_hit_points[0] = Fixed::from_int(40);
    let mut second = sta_content::UnitDef::new("test_ally", "craft");
    second.race = "blue".into();
    let content = sta_content::ContentDb::builder().unit(ship).unit(second).build().unwrap();
    let (mut app, id) = app_with_content(content);
    hotkey(&mut app);
    app.world_mut().resource_mut::<Editing>().inspected = Some(id);
    app.update();
    {
        let world = app.world_mut();
        let mut text = world.query::<(&panel::InspectorField, &mut TextField)>();
        let (_, mut health) = text.iter_mut(world).find(|(field, _)| field.0 == "engines").unwrap();
        assert_eq!(health.text, "1");
        health.text = "0.5".into();

        let mut checks = world.query::<(&panel::InspectorField, &mut Checkbox)>();
        for (field, mut check) in checks.iter_mut(world) {
            match field.0 {
                "engines_on" => check.on = false,
                "immortal" => check.on = true,
                _ => {}
            }
        }
        let mut choices = world.query::<(&panel::InspectorField, &mut Dropdown)>();
        for (field, mut choice) in choices.iter_mut(world) {
            match field.0 {
                "perceived_team" => choice.selected = 4,
                "race" => choice.selected = choice.options.iter().position(|race| race == "blue").unwrap(),
                _ => {}
            }
        }
    }
    let click = |app: &mut App, select: fn(&panel::Act) -> bool| {
        let world = app.world_mut();
        let mut buttons = world.query::<(Entity, &panel::Act)>();
        let (entity, _) = buttons.iter(world).find(|(_, act)| select(act)).unwrap();
        world.write_message(ButtonClicked(entity));
        app.update();
    };
    click(&mut app, |act| matches!(act, panel::Act::ApplyStats));
    {
        let link = app.world().resource::<SimLink>();
        let view = link.real_curr();
        let unit = view.unit(id).unwrap();
        assert_eq!(unit.owner, PlayerId(0));
        assert_eq!(view.perceived_owner(id), Some(PlayerId(3)));
        assert!(unit.flags.contains(sta_sim::view::UnitFlags::IMMORTAL));
        assert_eq!(view.attrs.races, vec![(id, "blue".into())]);
        let engines = view.appearance.systems(id).unwrap()[0].unwrap();
        assert!(!engines.enabled);
        assert_eq!(engines.health, [Fixed::from_int(20), Fixed::from_int(40)]);
    }

    app.world_mut().resource_mut::<Editing>().inspected = Some(id);
    app.update();
    click(&mut app, |act| matches!(act, panel::Act::RestoreStats));
    let view = app.world().resource::<SimLink>().real_curr();
    assert_eq!(view.perceived_owner(id), Some(PlayerId(0)));
    assert!(view.attrs.races.is_empty());
    assert!(!view.unit(id).unwrap().flags.contains(sta_sim::view::UnitFlags::IMMORTAL));
    let engines = view.appearance.systems(id).unwrap()[0].unwrap();
    assert!(engines.enabled);
    assert_eq!(engines.health, [Fixed::from_int(40); 2]);
}


#[test]
fn inspector_advanced_view_and_folds_preserve_pending_edits() {
    use crate::ui::theme::DropdownChanged;

    let (mut app, id) = app();
    hotkey(&mut app);
    app.world_mut().resource_mut::<Editing>().inspected = Some(id);
    app.update();

    let world = app.world_mut();
    let mut fields = world.query::<(Entity, &ChildOf, &panel::InspectorField)>();
    let (field, row, _) = fields.iter(world).find(|row| row.2.0 == "immortal").unwrap();
    let row = row.parent();
    assert_eq!(world.get::<Node>(row).unwrap().display, Display::Flex);
    world.get_mut::<Checkbox>(field).unwrap().on = true;

    let mut dropdowns = world.query::<(Entity, &Dropdown)>();
    let (view, _) = dropdowns.iter(world).find(|(_, choice)| choice.value() == Some("Advanced")).unwrap();
    let fold = {
        let mut captions = world.query::<(&ChildOf, &crate::ui::sprite_font::SpriteText)>();
        let (parent, _) = captions.iter(world).find(|(_, text)| text.text.eq_ignore_ascii_case("-  GameObject")).unwrap();
        parent.parent()
    };
    let show = |app: &mut App, index| {
        app.world_mut().write_message(DropdownChanged { entity: view, index });
        app.update();
    };
    show(&mut app, 0);
    assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::None);
    show(&mut app, 1);
    assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::Flex);

    app.world_mut().write_message(ButtonClicked(fold));
    app.update();
    assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::None);
    app.world_mut().write_message(ButtonClicked(fold));
    app.update();
    assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::Flex);
    assert!(app.world().get::<Checkbox>(field).unwrap().on);
    assert!(!app.world().resource::<SimLink>().real_curr().unit(id).unwrap()
        .flags.contains(sta_sim::view::UnitFlags::IMMORTAL));
}

#[test]
fn inspector_shield_switch_preserves_authored_pools_on_reopen() {
    let mut ship = sta_content::UnitDef::new("test_ship", "craft");
    ship.hull = Fixed::from_int(100);
    ship.shields.max = Fixed::from_int(50);
    ship.system_hit_points[2] = Fixed::from_int(40);
    let content = Arc::new(sta_content::ContentDb::builder().unit(ship).build().unwrap());
    let setup = GameSetup {
        seed: 1,
        players: Vec::new(),
        map: Arc::new(sta_content::MapDef::empty()),
        content: content.clone(),
        options: GameOptions::default(),
    };
    let spawn = || {
        let mut sim = Sim::from_setup(&setup);
        let id = sim.spawn_def(
            content.key("test_ship").unwrap(),
            PlayerId(0),
            SimVec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        ).unwrap();
        (sim, id)
    };
    let (sim, id) = spawn();
    let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
    let map = sta_bzn::Map { version: 3013, size: [1000.0, 100.0, 1000.0], ..default() };
    let bytes = sta_bzn::write::write(&map).unwrap();
    let mut doc = Document::new("bzn/test.bzn".into(), bytes, map).unwrap();
    let index = doc.push("test_ship", 1, [0.0; 3], [0.0, 1.0]).unwrap();
    let mut authored = AuthoredIds(BTreeMap::from([(id, index)]));
    let content = ContentRes(content.clone());
    for (key, value) in [("shields", "27"), ("shields_on", "0"), ("shields_on", "1")] {
        assert!(apply_inspector(id, &mut link, &content, None, Some(&mut doc), Some(&mut authored),
            None, &BTreeMap::from([(key, value)]), false).unwrap());
    }
    let shields = link.real_curr().unit(id).unwrap().shields;
    assert_eq!(shields[0], [Fixed::ZERO, Fixed::from_int(50)]);

    let (mut reopened, reopened_id) = spawn();
    let mut names = crate::ui::hud::instance_names::InstanceNames::default();
    doc.restore_properties(&mut reopened, &BTreeMap::from([(reopened_id, index)]), &mut names, false).unwrap();
    assert_eq!(reopened.view().unit(reopened_id).unwrap().shields, shields);
}
