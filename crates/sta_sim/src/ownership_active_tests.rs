use super::*;

fn original() -> [Fixed; 4] {
    [100, 200, 300, 400].map(Fixed::from_int)
}

fn spent() -> [Fixed; 4] {
    [90, 180, 270, 360].map(Fixed::from_int)
}

fn active(prepaid: bool, elapsed: usize) -> (Sim, EntityId) {
    let mut sim = game(prepaid);
    let yard = spawn_yard(&mut sim, OLD);
    fund(&mut sim, OLD, original());
    let product = key(&sim, "transfer_four");
    let out = sim.step(&[build(OLD, 0, yard, product)]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::BuildStarted { builder, what } if *builder == yard && *what == product))
    );
    for _ in 0..elapsed {
        sim.step(&[]);
    }
    let item = sim.production.to_rec().queues[0].1[0];
    assert_eq!((item.2, item.3, item.4), (elapsed as u64, true, true));
    assert_eq!(stock(&sim, OLD), spent());
    (sim, yard)
}

fn accepted(events: &[SimEvent]) {
    assert!(
        !events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

#[test]
fn active_zero_and_partial_prepaid_refund_all_four_but_deferred_forfeits() {
    for elapsed in [0, 17] {
        for prepaid in [true, false] {
            let (mut sim, yard) = active(prepaid, elapsed);
            let events = sim.step(&[transfer(0, yard, NEW)]).events;
            accepted(&events);
            assert!(!events.iter().any(|e| matches!(e, SimEvent::Built { .. })));
            assert_eq!(
                stock(&sim, OLD),
                if prepaid { original().to_vec() } else { spent().to_vec() }
            );
            assert!(stock(&sim, NEW).is_empty());
            assert!(sim.production.queue(yard).is_empty());
            assert_eq!(sim.unit(yard).unwrap().owner, NEW);
            for _ in 0..55 {
                assert!(!sim.step(&[]).events.iter().any(|e| matches!(e, SimEvent::Built { .. })));
            }
            assert_eq!(sim.units.len(), 1);
            let product = key(&sim, "transfer_nonpositive");
            assert!(
                sim.step(&[build(OLD, 1, yard, product)])
                    .events
                    .iter()
                    .any(|e| matches!(e, SimEvent::Rejected { .. }))
            );
            accepted(&sim.step(&[build(NEW, 1, yard, product)]).events);
        }
    }
}

#[test]
fn real_three_job_active_queue_refunds_prepaid_jobs_and_forfeits_only_deferred_head() {
    for prepaid in [true, false] {
        let (mut sim, yard) = active(prepaid, 3);
        let product = key(&sim, "transfer_four");
        accepted(
            &sim.step(&[build(OLD, 1, yard, product), build(OLD, 2, yard, product)])
                .events,
        );
        let rec = sim.production.to_rec();
        assert_eq!(rec.queues[0].1.len(), 3);
        assert!(rec.queues[0].1[0].3 && rec.queues[0].1[0].4);
        assert!(
            rec.queues[0].1[1..]
                .iter()
                .all(|item| !item.3 && item.2 == 0 && item.4 == prepaid)
        );
        assert_eq!(
            stock(&sim, OLD),
            if prepaid {
                [70, 140, 210, 280].map(Fixed::from_int)
            } else {
                spent()
            }
        );
        accepted(&sim.step(&[transfer(0, yard, NEW)]).events);
        assert_eq!(stock(&sim, OLD), if prepaid { original() } else { spent() });
        assert!(stock(&sim, NEW).is_empty());
        assert!(sim.production.queue(yard).is_empty());
        for _ in 0..150 {
            assert!(!sim.step(&[]).events.iter().any(|e| matches!(e, SimEvent::Built { .. })));
        }
        assert_eq!(sim.units.len(), 1);
    }
}

#[test]
fn active_head_and_mixed_tails_apply_only_actual_refunds_once() {
    for prepaid in [true, false] {
        let (mut sim, yard) = active(prepaid, 5);
        let product = key(&sim, "transfer_four");
        accepted(
            &sim.step(&[build(OLD, 1, yard, product), build(OLD, 2, yard, product)])
                .events,
        );
        let mut rec = sim.production.to_rec();
        assert_eq!(rec.queues[0].1.len(), 3);
        if !prepaid {
            // A paid receipt on an unstarted tail remains refundable even when
            // the deferred active head is forfeited; the last tail is unpaid.
            rec.queues[0].1[1].4 = true;
        } else {
            rec.queues[0].1[2].4 = false;
        }
        sim.production = Production::from_rec(&rec).unwrap();
        let before = stock(&sim, OLD);
        accepted(
            &sim.step(&[transfer(0, yard, OLD), transfer(1, yard, NEW), transfer(2, yard, OLD)])
                .events,
        );
        let expected = if prepaid {
            // Active head plus one paid tail: two sets of credits.
            before
                .into_iter()
                .zip([20, 40, 60, 80].map(Fixed::from_int))
                .map(|(a, b)| a + b)
                .collect::<Vec<_>>()
        } else {
            // Only the paid unstarted tail credits; active cost is forfeited.
            before
                .into_iter()
                .zip([10, 20, 30, 40].map(Fixed::from_int))
                .map(|(a, b)| a + b)
                .collect::<Vec<_>>()
        };
        assert_eq!(stock(&sim, OLD), expected);
        assert!(stock(&sim, NEW).is_empty());
        assert!(sim.production.queue(yard).is_empty());
        assert_eq!(sim.unit(yard).unwrap().owner, OLD);
        accepted(&sim.step(&[transfer(3, yard, OLD)]).events);
        assert_eq!(stock(&sim, OLD), expected);
    }
}

#[test]
fn capture_before_completion_creates_no_child_but_completed_child_keeps_owner() {
    for prepaid in [true, false] {
        let (mut sim, yard) = active(prepaid, 49);
        let events = sim.step(&[transfer(0, yard, NEW)]).events;
        accepted(&events);
        assert!(!events.iter().any(|e| matches!(e, SimEvent::Built { .. })));
        assert_eq!(sim.units.len(), 1);
        assert_eq!(stock(&sim, OLD), if prepaid { original() } else { spent() });

        let (mut sim, yard) = active(prepaid, 0);
        let mut child = None;
        for _ in 0..50 {
            let events = sim.step(&[]).events;
            if let Some(id) = events.iter().find_map(|e| match e {
                SimEvent::Built { id, .. } => Some(*id),
                _ => None,
            }) {
                child = Some(id);
            }
        }
        let child = child.expect("first product completed");
        assert_eq!(sim.unit(child).unwrap().owner, OLD);
        let product = key(&sim, "transfer_four");
        accepted(&sim.step(&[build(OLD, 1, yard, product)]).events);
        accepted(&sim.step(&[transfer(0, yard, NEW)]).events);
        assert_eq!(sim.unit(child).unwrap().owner, OLD);
        assert_eq!(sim.unit(yard).unwrap().owner, NEW);
        assert_eq!(sim.units.len(), 2);
        assert_eq!(
            stock(&sim, OLD),
            if prepaid {
                spent()
            } else {
                [80, 160, 240, 320].map(Fixed::from_int)
            }
        );
    }
}

#[test]
fn ordinary_stop_refunds_a_deferred_active_head_unlike_capture() {
    let (mut stop, yard) = active(false, 7);
    let (mut capture, captured_yard) = active(false, 7);
    let command = command(OLD, 1, CommandKind::Order(Order::Stop { units: vec![yard] }));
    accepted(&stop.step(&[command]).events);
    accepted(&capture.step(&[transfer(0, captured_yard, NEW)]).events);
    assert_eq!(stock(&stop, OLD), original());
    assert_eq!(stock(&capture, OLD), spent());
    assert!(stop.production.queue(yard).is_empty() && capture.production.queue(captured_yard).is_empty());
}

#[test]
fn malformed_active_records_and_refund_overflow_reject_atomically() {
    for prepaid in [true, false] {
        let base = || {
            let mut sim = game(prepaid);
            let yard = spawn_yard(&mut sim, OLD);
            (sim, yard)
        };
        let (mut sim, yard) = base();
        let four = key(&sim, "transfer_four").0;
        inject(&mut sim, yard, vec![(four, OLD.0, 0, true, false)]);
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(&mut sim, yard, vec![(four, OLD.0, 50, true, true)]);
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(
            &mut sim,
            yard,
            vec![(four, OLD.0, 1, true, true), (four, OLD.0, 0, true, true)],
        );
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(&mut sim, yard, vec![(four, NEW.0, 1, true, true)]);
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(&mut sim, yard, vec![(u32::MAX, OLD.0, 1, true, true)]);
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(&mut sim, yard, vec![(four, OLD.0, 1, true, true)]);
        sim.units.get_mut(yard).unwrap().def = ContentKey::NONE;
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(&mut sim, yard, vec![(four, OLD.0, 1, true, true)]);
        sim.units.get_mut(yard).unwrap().def = ContentKey(four);
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        inject(
            &mut sim,
            yard,
            vec![(four, OLD.0, 1, true, true), (u32::MAX, OLD.0, 0, false, true)],
        );
        atomically_rejected(sim, yard, NEW);

        let (mut sim, yard) = base();
        let fifth = key(&sim, "transfer_fifth").0;
        inject(&mut sim, yard, vec![(fifth, OLD.0, 1, true, true)]);
        atomically_rejected(sim, yard, NEW);

        let (mut sim, _) = base();
        let neutral_yard = spawn_yard(&mut sim, NEUTRAL);
        inject(&mut sim, neutral_yard, vec![(four, NEUTRAL.0, 1, true, true)]);
        atomically_rejected(sim, neutral_yard, NEW);
    }

    let (mut prepaid, yard) = active(true, 1);
    let max = Fixed::from_bits(i64::MAX - 5);
    let cargo = command(PlayerId::BENCH, 0, CommandKind::Edit(Edit::Cargo(OLD, 0, max)));
    accepted(&prepaid.step(&[cargo]).events);
    atomically_rejected(prepaid, yard, NEW);

    let (mut deferred, yard) = active(false, 1);
    let cargo = command(PlayerId::BENCH, 0, CommandKind::Edit(Edit::Cargo(OLD, 0, max)));
    accepted(&deferred.step(&[cargo]).events);
    accepted(&deferred.step(&[transfer(0, yard, NEW)]).events);
    assert_eq!(stock(&deferred, OLD)[0], max);

    let (mut deferred, yard) = active(false, 1);
    let four = key(&deferred, "transfer_four").0;
    inject(
        &mut deferred,
        yard,
        vec![(four, OLD.0, 1, true, true), (four, OLD.0, 0, false, true)],
    );
    deferred.economy.apply_edit(PlayerId::BENCH, &Edit::Cargo(OLD, 0, max));
    atomically_rejected(deferred, yard, NEW);
}

#[test]
fn neutral_nonpositive_active_head_clears_without_bank_and_no_alias_credit() {
    for prepaid in [true, false] {
        let mut sim = game(prepaid);
        let yard = spawn_yard(&mut sim, NEUTRAL);
        let product = key(&sim, "transfer_nonpositive").0;
        inject(&mut sim, yard, vec![(product, NEUTRAL.0, 1, true, true)]);
        accepted(&sim.step(&[transfer(0, yard, NEW)]).events);
        assert!(sim.production.queue(yard).is_empty());
        assert!(sim.economy.to_rec().stock.is_empty());
        assert_eq!(sim.unit(yard).unwrap().owner, NEW);
    }
}

#[test]
fn active_snapshot_and_same_build_replay_continue_in_both_payment_modes() {
    for prepaid in [true, false] {
        let (mut live, yard) = active(prepaid, 11);
        let bytes = live.snapshot();
        let mut loaded = Sim::from_snapshot(&bytes, Some(Arc::clone(live.content().unwrap()))).unwrap();
        assert_eq!(loaded.digest(), live.digest());
        for commands in [vec![transfer(0, yard, NEW)], vec![]] {
            let a = live.step(&commands);
            let b = loaded.step(&commands);
            assert_eq!(a.events, b.events);
            assert_eq!(a.view.digest, b.view.digest);
        }

        let mut map = MapDef::empty();
        map.objects.push(MapObject {
            odf: "transfer_yard".into(),
            owner: Some(OLD.0),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        });
        let setup = setup(prepaid, map);
        let db = Arc::clone(&setup.content);
        let mut recorder = Recorder::with_interval(&setup, 1);
        let yard = recorder.sim().units.iter().next().unwrap().id;
        for (slot, value) in original().into_iter().enumerate() {
            recorder.step(&[command(
                PlayerId::BENCH,
                slot as u32,
                CommandKind::Edit(Edit::Cargo(OLD, slot as u8, value)),
            )]);
        }
        let product = key(recorder.sim(), "transfer_four");
        recorder.step(&[build(OLD, 0, yard, product)]);
        for _ in 0..11 {
            recorder.step(&[]);
        }
        recorder.step(&[transfer(0, yard, NEW)]);
        for _ in 0..55 {
            recorder.step(&[]);
        }
        let expected = recorder.sim().digest();
        let replay = Replay::decode(&recorder.finish().encode()).unwrap();
        let played = replay.play(db).unwrap();
        assert_eq!(played.digest(), expected);
        assert!(played.production.queue(yard).is_empty());
        assert_eq!(played.unit(yard).unwrap().owner, NEW);
    }
}
