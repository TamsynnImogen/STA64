use super::*;
use crate::production::view::ProductionView;

fn row(sim: &Sim, builder: EntityId) -> ProductionView {
    sim.production_view()
        .into_iter()
        .find(|row| row.builder == builder)
        .expect("production row")
}

#[test]
fn queue_view_tracks_paid_active_head_and_exact_launch_pose_without_mutation() {
    let mut sim = game(true);
    let yard = spawn(&mut sim, "fx_yard", v(100, 200));
    fund(&mut sim, 2000);
    assert!(build(&mut sim, yard, "fx_cruiser", None));
    assert!(build(&mut sim, yard, "fx_frigate", None));
    let before = sim.snapshot();
    let digest = sim.digest();
    let first = row(&sim, yard);
    assert_eq!(first.jobs.len(), 2);
    assert!(first.jobs[0].started && first.jobs[0].paid);
    assert!(!first.jobs[1].started);
    assert_eq!(first.jobs[1].progress, Fixed::ZERO);
    let out = super::super::launch::place(
        sim.content().unwrap().unit(key(&sim, "fx_yard")).unwrap(),
        v(100, 200),
        (Fixed::ZERO, Fixed::ONE),
    );
    let site = first.site.unwrap();
    assert_eq!(
        (site.pos, site.heading, site.entity),
        (out.pos, out.heading, None)
    );
    assert_eq!(site.def, key(&sim, "fx_cruiser"));
    assert_eq!(sim.snapshot(), before);
    assert_eq!(sim.digest(), digest);
    for _ in 0..20 {
        sim.step(&[]);
    }
    assert!(row(&sim, yard).jobs[0].progress > first.jobs[0].progress);
    sim.step(&[cmd(
        P0,
        99,
        CommandKind::Order(Order::Stop { units: vec![yard] }),
    )]);
    assert!(sim.production_view().is_empty());
}

#[test]
fn deferred_unpaid_head_has_no_ship_site_until_started_and_paid() {
    let mut sim = game(false);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    assert!(build(&mut sim, yard, "fx_cruiser", None));
    let waiting = row(&sim, yard);
    assert!(!waiting.jobs[0].paid && !waiting.jobs[0].started);
    assert!(waiting.site.is_none());
    fund(&mut sim, 1000);
    let started = row(&sim, yard);
    assert!(started.jobs[0].paid && started.jobs[0].started);
    assert!(started.site.is_some());
}

#[test]
fn confirmed_station_site_survives_travel_welding_snapshot_and_completion() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 2000);
    let at = v(1000, 0);
    assert!(build(&mut sim, rig, "fx_station", Some(at)));
    let pending = row(&sim, rig);
    assert_eq!(pending.site.as_ref().unwrap().pos, at);
    assert_eq!(pending.site.as_ref().unwrap().entity, None);
    assert!(!pending.jobs[0].started);
    let child = loop {
        sim.step(&[]);
        let now = row(&sim, rig);
        if let Some(child) = now.site.unwrap().entity {
            break child;
        }
        assert!(sim.tick < 3000, "rig never reached its site");
    };
    let welding = row(&sim, rig);
    assert!(welding.jobs[0].started && welding.jobs[0].paid);
    assert_eq!(
        welding.site.as_ref().unwrap().pos,
        sim.unit(child).unwrap().pos
    );
    assert_eq!(
        welding.jobs[0].progress,
        sim.unit(child).unwrap().progress.unwrap()
    );
    let restored = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).expect("restore");
    assert_eq!(restored.production_view(), sim.production_view());
    assert_eq!(restored.digest(), sim.digest());
    let mut restored = restored;
    let remaining_bound = sim
        .content()
        .unwrap()
        .unit(key(&sim, "fx_station"))
        .unwrap()
        .build
        .ticks
        + 1;
    for _ in 0..remaining_bound {
        assert_eq!(sim.production_view(), restored.production_view());
        assert_eq!(sim.step(&[]).view.digest, restored.step(&[]).view.digest);
        if sim.production_view().is_empty() {
            break;
        }
    }
    assert!(sim.production_view().is_empty());
    assert!(sim.unit(child).unwrap().progress.is_none());
}

#[test]
fn sorted_independent_yards_reset_heads_and_clear_on_destruction() {
    let mut sim = game(true);
    let a = spawn(&mut sim, "fx_yard", v(0, 0));
    let b = spawn(&mut sim, "fx_yard", v(1000, 0));
    fund(&mut sim, 3000);
    assert!(build(&mut sim, b, "fx_cruiser", None));
    assert!(build(&mut sim, a, "fx_frigate", None));
    assert!(build(&mut sim, a, "fx_cruiser", None));
    assert_eq!(
        sim.production_view()
            .iter()
            .map(|r| r.builder)
            .collect::<Vec<_>>(),
        vec![a, b]
    );
    for _ in 0..450 {
        sim.step(&[]);
    }
    let next = row(&sim, a);
    assert_eq!(next.jobs[0].def, key(&sim, "fx_cruiser"));
    assert!(next.jobs[0].progress < row(&sim, b).jobs[0].progress);
    sim.step(&[cmd(PlayerId::BENCH, 100, CommandKind::Edit(Edit::Kill(a)))]);
    assert_eq!(
        sim.production_view()
            .iter()
            .map(|r| r.builder)
            .collect::<Vec<_>>(),
        vec![b]
    );
}

#[test]
fn welding_site_remains_after_constructor_death_until_the_station_finishes() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 2000);
    assert!(build(&mut sim, rig, "fx_station", Some(v(300, 0))));
    let child = loop {
        let active = row(&sim, rig);
        if let Some(child) = active.site.unwrap().entity {
            break child;
        }
        sim.step(&[]);
        assert!(sim.tick < 3000);
    };
    let before = row(&sim, rig).jobs[0].progress;
    sim.step(&[cmd(
        PlayerId::BENCH,
        101,
        CommandKind::Edit(Edit::Kill(rig)),
    )]);
    assert!(sim.unit(rig).is_none());
    let active = row(&sim, rig);
    assert_eq!(active.site.as_ref().unwrap().entity, Some(child));
    assert!(active.jobs[0].progress > before);
    assert_eq!(
        active.jobs[0].progress,
        sim.unit(child).unwrap().progress.unwrap()
    );
    let bound = sim
        .content()
        .unwrap()
        .unit(key(&sim, "fx_station"))
        .unwrap()
        .build
        .ticks
        + 1;
    for _ in 0..bound {
        sim.step(&[]);
        if sim.production_view().is_empty() {
            break;
        }
    }
    assert!(sim.production_view().is_empty());
    assert!(sim.unit(child).is_some_and(|unit| unit.progress.is_none()));
}
