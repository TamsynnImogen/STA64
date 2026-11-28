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
fn a_dying_unit_leaves_the_selection_and_the_groups_and_the_change_is_announced() {
    let mut app = app_with(&[1, 2], [&[2, 3], &[1]]);
    app.world_mut().run_system_once(drop_dying_from_selection).unwrap();
    assert_eq!(app.world().resource::<Selection>().0, ids(&[1]));
    let groups = app.world().resource::<ControlGroups>();
    assert_eq!((groups.groups[0].clone(), groups.groups[1].clone()), (ids(&[3]), ids(&[1])));
    assert_eq!(changes(&mut app), 1);
    // Picked again in a later frame, it goes again.
    app.world_mut().resource_mut::<Selection>().0 = ids(&[2]);
    app.world_mut().run_system_once(drop_dying_from_selection).unwrap();
    assert!(app.world().resource::<Selection>().0.is_empty());
    assert_eq!(changes(&mut app), 1);
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
