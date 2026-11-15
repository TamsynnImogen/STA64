//! The payment test a client shares with the sim (`afford::first_short`): the refactor of
//! `can_pay` answers as before, and `build_shortfall` names what is short. Authored numbers only.

use std::sync::Arc;

use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId};

use super::afford::{Shortfall, build_shortfall, first_short};
use super::can_pay;
use crate::economy::{Economy, RESOURCES};
use crate::view::PlayerView;

const P: PlayerId = PlayerId(3);

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

/// The fake content with `fx_station` costing `cost` and `officers`.
fn db(cost: [f64; 4], officers: f64) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_station" {
            u.build.cost = cost.iter().map(|c| fx(*c)).collect();
            u.build.officers = fx(officers);
        }
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

/// The payment test as `can_pay` read before it shared `first_short`.
fn old_can_pay(have: &[Fixed], cost: &[Fixed]) -> bool {
    !cost.iter().enumerate().any(|(i, &c)| c > Fixed::ZERO && have.get(i).is_none_or(|&h| h < c))
}

#[test]
fn can_pay_answers_as_before_over_a_matrix_of_stock_and_cost() {
    let stocks = [[0.0; 4], [10.0, 0.0, 0.0, 0.0], [10.0, 5.0, 5.0, 5.0], [9.5, 40.0, 0.0, 1.0], [100.0, 100.0, 100.0, 100.0]];
    let costs = [[0.0; 4], [10.0, 0.0, 0.0, 0.0], [10.5, 0.0, 0.0, 0.0], [0.0, 6.0, 0.0, 0.0], [9.5, 40.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.5]];
    for stock in stocks {
        for cost in costs {
            let db = db(cost, 0.0);
            let mut eco = Economy::default();
            let mut s = [Fixed::ZERO; RESOURCES];
            for (i, v) in stock.iter().enumerate() {
                s[i] = fx(*v);
            }
            eco.set_stock(P, s);
            let key = db.key("fx_station").unwrap();
            let have = eco.stock_of(P);
            let scaled = eco.scaled_cost(P, &db.unit(key).unwrap().build.cost);
            assert_eq!(can_pay(&eco, &db, P, key), old_can_pay(&have, &scaled), "stock {stock:?} cost {cost:?}");
        }
    }
}

#[test]
fn first_short_is_the_first_resource_above_the_stock_and_ignores_zero_costs() {
    let have = [fx(10.0), fx(0.0)];
    assert_eq!(first_short(&have, &[fx(10.0), fx(0.0)]), None);
    assert_eq!(first_short(&have, &[fx(11.0), fx(1.0)]), Some(0));
    assert_eq!(first_short(&have, &[fx(0.0), fx(1.0)]), Some(1));
    assert_eq!(first_short(&have, &[fx(0.0), fx(0.0), fx(1.0)]), Some(2), "a slot the stock does not hold is short");
    assert_eq!(first_short(&have, &[fx(0.0), fx(0.0), fx(0.0)]), None);
}

fn view(have: [f64; 4], officers: [f64; 2]) -> PlayerView {
    PlayerView {
        crew: None,
        id: P,
        team: sta_core::TeamId(1),
        resources: have.iter().map(|v| fx(*v)).collect(),
        officers: officers.map(fx),
        eliminated: false,
        relations: Vec::new(),
    }
}

#[test]
fn build_shortfall_names_resources_first_then_officers() {
    let db = db([20.0, 0.0, 3.0, 0.0], 4.0);
    let key = db.key("fx_station").unwrap();
    assert_eq!(build_shortfall(&db, &view([20.0, 0.0, 3.0, 0.0], [0.0, 0.0]), key), None);
    assert_eq!(build_shortfall(&db, &view([19.0, 0.0, 0.0, 0.0], [0.0, 0.0]), key), Some(Shortfall::Resource(0)));
    assert_eq!(build_shortfall(&db, &view([20.0, 0.0, 2.0, 0.0], [0.0, 0.0]), key), Some(Shortfall::Resource(2)));
    assert_eq!(build_shortfall(&db, &view([20.0, 0.0, 3.0, 0.0], [2.0, 5.0]), key), Some(Shortfall::Officers), "2 + 4 is over 5");
    assert_eq!(build_shortfall(&db, &view([20.0, 0.0, 3.0, 0.0], [1.0, 5.0]), key), None, "1 + 4 fits 5");
    assert_eq!(build_shortfall(&db, &view([20.0, 0.0, 3.0, 0.0], [9.0, 0.0]), key), None, "a limit of 0 is no cap");
}
