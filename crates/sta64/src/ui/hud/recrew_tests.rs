//! Recrew HUD eligibility and command-queue coverage.

use std::sync::Arc;

use bevy::prelude::*;
use sta_content::{ContentDb, UnitDef};
use sta_sim::view::{UnitFlags, UnitView};
use sta_sim::{EntityId, Fixed, PlayerId, Vec3, WorldView};
use sta_sim::command::{CommandKind, Order};

use super::commands;
use super::model::{Act, Look, Page, command, grid};
use super::panels::GridCell;
use super::*;

const ME: PlayerId = PlayerId(1);

fn feed() -> HudFeed {
    let station = |odf: &str, facility: Option<bool>, has_crew: Option<bool>| {
        let mut def = UnitDef::new(odf, "starbase");
        def.facility = facility;
        def.has_crew = has_crew;
        def
    };
    let db = Arc::new(
        ContentDb::builder()
            .unit(station("station", None, None))
            .unit(station("no_facility", Some(false), None))
            .unit(station("no_crew", None, Some(false)))
            .unit(UnitDef::new("ship", "craft"))
            .build()
            .unwrap(),
    );
    let unit = |id: u32, odf: &str, owner: PlayerId, crew: Option<[i32; 2]>| UnitView {
        id: EntityId(id),
        def: db.key(odf).unwrap(),
        owner,
        pos: Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ZERO; 2]; 4],
        crew: crew.map(|c| c.map(Fixed::from_int)),
        energy: None,
        autonomy: None,
        flags: UnitFlags::SELECTABLE,
        progress: None,
    };
    HudFeed {
        view: Arc::new(WorldView {
            units: vec![
                unit(1, "station", ME, Some([50, 100])),
                unit(2, "station", ME, Some([100, 100])),
                unit(3, "station", ME, None),
                unit(4, "station", ME, Some([0, 0])),
                unit(5, "station", PlayerId(2), Some([50, 100])),
                unit(6, "ship", ME, Some([50, 100])),
                unit(7, "no_facility", ME, Some([50, 100])),
                unit(8, "no_crew", ME, Some([50, 100])),
                unit(9, "station", ME, Some([50, 100])),
                unit(10, "station", ME, Some([50, 100])),
            ],
            ..Default::default()
        }),
        player: ME,
        selected: vec![EntityId(1)],
        content: Some(db),
    }
}

fn button(feed: &HudFeed) -> Option<super::model::Button> {
    grid(feed, Page::Top)
        .into_iter()
        .find(|b| b.act == Act::Recrew)
}

#[test]
fn station_button_follows_live_crew_and_queues_only_when_ready() {
    let mut f = feed();
    let b = button(&f).unwrap();
    assert_eq!(b.look, Look::Ready);
    assert_eq!(b.sprite_names, ["b_recrew"]);
    assert_eq!(command(&f, &Act::Recrew), Some((
        CommandKind::Order(Order::Recrew { units: vec![EntityId(1)], on: None }),
        vec![EntityId(1)],
    )));
    f.selected = vec![EntityId(2)];
    assert_eq!(button(&f).unwrap().look, Look::Locked);
    assert_eq!(command(&f, &Act::Recrew), None);
    Arc::make_mut(&mut f.view).units[1].crew = Some([Fixed::from_int(99), Fixed::from_int(100)]);
    assert_eq!(button(&f).unwrap().look, Look::Ready);
    assert_eq!(command(&f, &Act::Recrew), Some((
        CommandKind::Order(Order::Recrew { units: vec![EntityId(2)], on: None }),
        vec![EntityId(2)],
    )));
    Arc::make_mut(&mut f.view).units[1].crew = Some([Fixed::from_int(100), Fixed::from_int(100)]);
    assert_eq!(button(&f).unwrap().look, Look::Locked);
    assert_eq!(command(&f, &Act::Recrew), None);
}

#[test]
fn only_owned_finished_live_capacity_stations_qualify() {
    let mut f = feed();
    Arc::make_mut(&mut f.view).units[8].progress = Some(Fixed::from_f64(0.5));
    Arc::make_mut(&mut f.view).units[9].flags = UnitFlags::SELECTABLE.with(UnitFlags::DYING, true);
    for id in 3..=10 {
        f.selected = vec![EntityId(id)];
        assert!(button(&f).is_none(), "id {id}");
    }
    f.selected = (1..=10).map(EntityId).collect();
    assert_eq!(
        commands::recrew_stations(&f, &f.own_selection()),
        [EntityId(1)]
    );
    assert_eq!(command(&f, &Act::Recrew), Some((
        CommandKind::Order(Order::Recrew { units: vec![EntityId(1)], on: None }),
        vec![EntityId(1)],
    )));
    assert_eq!(button(&f).unwrap().look, Look::Ready);
    f.selected = vec![EntityId(2), EntityId(6)];
    assert_eq!(button(&f).unwrap().look, Look::Locked);
    f.selected = vec![EntityId(2), EntityId(6), EntityId(5)];
    assert_eq!(
        button(&f).unwrap().look,
        Look::Locked,
        "ship and enemy cannot activate a full own station"
    );
}

#[test]
fn ready_click_queues_one_real_order_without_warning() {
    let f = feed();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<ButtonClicked>()
        .add_message::<SelectionChanged>()
        .add_message::<OrderIssued>()
        .add_message::<Notice>()
        .insert_resource(f.clone())
        .init_resource::<HudState>()
        .init_resource::<CommandQueue>()
        .init_resource::<Selection>()
        .init_resource::<OrderTargets>()
        .add_systems(Update, press);
    let cell = app.world_mut().spawn(GridCell(Act::Recrew)).id();
    app.world_mut()
        .resource_mut::<Messages<ButtonClicked>>()
        .write(ButtonClicked(cell));
    app.update();
    assert_eq!(app.world().resource::<CommandQueue>().0, [
        CommandKind::Order(Order::Recrew { units: vec![EntityId(1)], on: None }),
    ]);
    let notices: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<Notice>>()
        .drain()
        .collect();
    assert!(notices.is_empty());

    app.world_mut().insert_resource(HudFeed {
        selected: vec![EntityId(2)],
        ..f
    });
    app.world_mut()
        .resource_mut::<Messages<ButtonClicked>>()
        .write(ButtonClicked(cell));
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<Notice>>()
            .drain()
            .next()
            .is_none()
    );
    assert_eq!(app.world().resource::<CommandQueue>().0.len(), 1);

    app.world_mut().insert_resource(HudFeed {
        selected: vec![EntityId(99)],
        ..feed()
    });
    app.world_mut()
        .resource_mut::<Messages<ButtonClicked>>()
        .write(ButtonClicked(cell));
    app.update();
    assert_eq!(app.world().resource::<CommandQueue>().0.len(), 1);
    assert!(app.world_mut().resource_mut::<Messages<Notice>>().drain().next().is_none());
}
