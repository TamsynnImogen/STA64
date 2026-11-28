//! A unit in its dying phase leaves the local selection and the control groups (#1100, client side).

use super::*;
use crate::input::select::ControlGroups;
use bevy::ecs::system::RunSystemOnce;
use sta_sim::command::LocalRouter;
use sta_sim::view::UnitFlags;
use sta_sim::{ContentKey, Fixed, TickOutput, UnitView};

struct Frozen(Arc<WorldView>);

impl WorldSource for Frozen {
    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput { view: self.0.clone(), events: vec![] }
    }
    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }
}

fn unit(id: u32, flags: UnitFlags) -> UnitView {
    UnitView {
        id: EntityId(id),
        def: ContentKey(1),
        owner: PlayerId(0),
        pos: SimVec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ONE; 2]; 4],
        crew: None,
        energy: None,
        autonomy: None,
        flags,
        progress: None,
    }
}

fn ids(v: &[u32]) -> Vec<EntityId> {
    v.iter().map(|i| EntityId(*i)).collect()
}

/// A world of units 1 to 3, unit 2 in its dying phase, with the given selection and groups.
fn app_with(selection: &[u32], groups: [&[u32]; 2]) -> App {
    let view = Arc::new(WorldView {
        units: vec![unit(1, UnitFlags::default()), unit(2, UnitFlags::DYING), unit(3, UnitFlags::default())],
        ..Default::default()
    });
    let mut app = App::new();
    app.add_message::<SelectionChanged>()
        .insert_resource(SimLink::new(Box::new(Frozen(view)), Box::new(LocalRouter::new()), PlayerId(0)))
        .insert_resource(Selection(ids(selection)));
    let mut g = ControlGroups::default();
    g.groups[0] = ids(groups[0]);
    g.groups[1] = ids(groups[1]);
    app.insert_resource(g);
    app
}

fn changes(app: &mut App) -> usize {
    app.world_mut().resource_mut::<Messages<SelectionChanged>>().drain().count()
}

#[test]
fn a_dying_unit_leaves_the_selection_and_the_groups_without_a_select_voice_trigger() {
    let mut app = app_with(&[1, 2], [&[2, 3], &[1]]);
    app.world_mut().run_system_once(drop_dying_from_selection).unwrap();
    assert_eq!(app.world().resource::<Selection>().0, ids(&[1]));
    let groups = app.world().resource::<ControlGroups>();
    assert_eq!((groups.groups[0].clone(), groups.groups[1].clone()), (ids(&[3]), ids(&[1])));
    // The select voice plays on `SelectionChanged`: a death must not send it, whoever stays selected.
    assert_eq!(changes(&mut app), 0);
    // Picked again in a later frame (INTERIM guard), it goes again, still without the message.
    app.world_mut().resource_mut::<Selection>().0 = ids(&[2]);
    app.world_mut().run_system_once(drop_dying_from_selection).unwrap();
    assert!(app.world().resource::<Selection>().0.is_empty());
    assert_eq!(changes(&mut app), 0);
}

#[derive(Resource, Default)]
struct Saw(Vec<(&'static str, Vec<EntityId>)>);

fn saw_in_input(s: Res<Selection>, mut saw: ResMut<Saw>) {
    saw.0.push(("input", s.0.clone()));
}
fn saw_in_render(s: Res<Selection>, mut saw: ResMut<Saw>) {
    saw.0.push(("render", s.0.clone()));
}
fn saw_in_ui(s: Res<Selection>, mut saw: ResMut<Saw>) {
    saw.0.push(("ui", s.0.clone()));
}
fn saw_in_audio(s: Res<Selection>, mut saw: ResMut<Saw>) {
    saw.0.push(("audio", s.0.clone()));
}

#[test]
fn the_prune_runs_after_the_input_and_before_the_interface() {
    // Through the real set order and the real plugin: what the input systems hold when the frame starts is the
    // old selection (they may have picked a dying unit), and every reader of render, interface and audio sees it
    // already pruned.
    let mut app = app_with(&[1, 2], [&[], &[]]);
    crate::app::configure_sets(&mut app);
    app.add_plugins(MinimalPlugins)
        .add_plugins(SimLinkPlugin)
        .init_resource::<Saw>()
        .add_systems(Update, saw_in_input.in_set(crate::app::GameSet::Input))
        .add_systems(Update, saw_in_render.in_set(crate::app::GameSet::Render))
        .add_systems(Update, saw_in_ui.in_set(crate::app::GameSet::Ui))
        .add_systems(Update, saw_in_audio.in_set(crate::app::GameSet::Audio));
    app.update();
    let saw = &app.world().resource::<Saw>().0;
    assert_eq!(
        saw.as_slice(),
        [("input", ids(&[1, 2])), ("render", ids(&[1])), ("ui", ids(&[1])), ("audio", ids(&[1]))],
        "the readers' view of the selection, in the order they ran"
    );
    assert_eq!(changes(&mut app), 0, "and no select voice is asked for");
}

#[test]
fn with_nothing_dying_the_selection_is_left_alone_and_quiet() {
    // Every other profile: no unit is ever flagged dying, so nothing is pruned or announced.
    let mut app = app_with(&[1, 3], [&[1, 3], &[]]);
    app.world_mut().run_system_once(drop_dying_from_selection).unwrap();
    assert_eq!(app.world().resource::<Selection>().0, ids(&[1, 3]));
    assert_eq!(app.world().resource::<ControlGroups>().groups[0], ids(&[1, 3]));
    assert_eq!(changes(&mut app), 0);
}
