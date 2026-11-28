//! The client's warp offer follows the mission switch and the content's own warp switch (#1102).

use super::*;

#[test]
fn warp_is_offered_only_while_the_mission_switch_and_the_content_allow_it() {
    let db = sta_content::fake::db();
    let off = db.clone().with_rules(db.rules().clone().with_warp(Some(false)));
    let mut view = WorldView::default();
    assert!(warp_allowed(&view, Some(&db)), "content with no key: warp");
    assert!(warp_allowed(&view, None), "a view without content has warp");
    assert!(!warp_allowed(&view, Some(&off)), "content that switches warp off");
    view.attrs.mission_switches.allow_warp = false;
    assert!(!warp_allowed(&view, Some(&db)), "a script switched warp off");
    assert!(!warp_allowed(&view, None));
    // the match option: off in this match, whatever the content and the script say
    let mut view = WorldView::default();
    view.attrs.match_warp_off = true;
    assert!(!warp_allowed(&view, Some(&db)) && !warp_allowed(&view, None), "the match has warp off");
}
