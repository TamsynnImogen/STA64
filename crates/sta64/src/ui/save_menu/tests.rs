//! The Save and Load screens in a bare `App`, on the front end's flow and UI-55's headless match (a
//! stand-in for the map view that builds the sim and arms the journal): the in-game menu's entries, the
//! list with its greyed rows and reasons, the questions, Load off while a load is pending, the not-allowed
//! reason, and the layout in three window shapes.

use std::sync::Arc;

use bevy::state::app::StatesPlugin;
use bevy::window::{PrimaryWindow, WindowResolution};
use sta_sim::setup::Control;

use super::*;
use crate::app::configure_sets;
use crate::clock::MatchSpeed;
use crate::frontend::screens::{Act, Screen};
use crate::save::tests::{app, armed, drain, run, setup, settle, start, state, temp_dir};
use crate::ui::ingame_menu::{InGameMenu, InGameMenuPlugin, MenuAct, OpenInGameMenu};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{TextField, WidgetState, click};

fn menu_app(dir: &PathBuf, map_file: bool) -> App {
    let mut a = app(dir, map_file);
    a.add_plugins((InGameMenuPlugin, SaveMenuPlugin));
    a
}

/// Clicks the one entity that carries `want`.
fn click_where<C: Component + PartialEq + std::fmt::Debug>(app: &mut App, want: C) {
    let e = app.world_mut().query::<(Entity, &C)>().iter(app.world()).find(|(_, c)| **c == want).unwrap_or_else(|| panic!("nothing carries {want:?}")).0;
    click(app.world_mut(), e);
    settle(app, 3);
}

fn screen(app: &App) -> SaveScreen {
    app.world().resource::<SaveScreen>().clone()
}

fn texts(app: &mut App) -> Vec<String> {
    app.world_mut().query::<&SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect()
}

fn until(app: &mut App, max: usize, done: impl Fn(&App) -> bool) {
    for _ in 0..max {
        if done(app) {
            return;
        }
        app.update();
    }
    panic!("not reached in {max} frames (state {:?})", state(app));
}

fn open_menu(app: &mut App) {
    app.world_mut().write_message(OpenInGameMenu);
    settle(app, 3);
    assert!(app.world().resource::<InGameMenu>().open);
}

fn select_row(app: &mut App, n: usize) {
    click_where(app, SaveRow(n));
    assert_eq!(screen(app).selected, Some(n));
}

fn set_name(app: &mut App, text: &str) {
    let mut q = app.world_mut().query_filtered::<&mut TextField, With<SaveNameField>>();
    q.single_mut(app.world_mut()).unwrap().text = text.to_owned();
}

fn primary_state(app: &mut App) -> WidgetState {
    let mut q = app.world_mut().query::<(&SaveAct, &WidgetState)>();
    *q.iter(app.world()).find(|(a, _)| **a == SaveAct::Primary).expect("a primary button").1
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimLink>().real_curr().tick
}

fn digest(app: &App) -> u64 {
    app.world().resource::<SimLink>().real_curr().digest
}

/// The Save screen from the in-game menu, a description typed, saved.
fn save_from_the_menu(app: &mut App, name: &str) {
    open_menu(app);
    click_where(app, MenuAct::Save);
    assert!(screen(app).open && screen(app).mode == SaveMode::Save);
    set_name(app, name);
    click_where(app, SaveAct::Primary);
}

#[test]
fn a_save_made_from_the_in_game_menu_is_listed_with_its_description_and_loads_from_the_menu_and_from_the_front_end() {
    let dir = temp_dir("menu");
    let data = armed();
    let mut a = menu_app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 10);
    let saved_digest = digest(&a);
    save_from_the_menu(&mut a, "first save");
    assert!(!screen(&a).open, "a save closes the screen");
    assert!(a.world().resource::<InGameMenu>().open, "and leaves the menu as it was");

    // It is on disk, with its description, and in the Load list.
    let on_disk = container::store::list(&dir, None).unwrap();
    assert_eq!(on_disk.len(), 1);
    assert_eq!(on_disk[0].outcome.as_ref().unwrap().description, "first save");
    click_where(&mut a, MenuAct::Load);
    let s = screen(&a);
    assert!(s.open && s.mode == SaveMode::Load);
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.entries[0].title(), "first save");
    assert!(s.entries[0].loadable());
    assert!(texts(&mut a).iter().any(|t| t.contains("first save")), "the row says it");

    // The match goes on; loading from the in-game menu asks first, then takes the match back to the save.
    run(&mut a, 5);
    assert_eq!(tick(&a), 15);
    select_row(&mut a, 0);
    click_where(&mut a, SaveAct::Primary);
    assert_eq!(screen(&a).ask, Some(Ask::Leave(0)), "leaving a running match is asked about");
    click_where(&mut a, SaveAct::Yes);
    until(&mut a, 400, |a| state(a) == AppState::InGame && !screen(a).open && a.world().get_resource::<PendingLoad>().is_none());
    settle(&mut a, 3);
    assert_eq!((tick(&a), digest(&a)), (10, saved_digest), "the match continues from the save");

    // From the front end: a fresh app, the single-player screen's Load game button.
    let mut f = menu_app(&dir, true);
    settle(&mut f, 4);
    let go = f.world_mut().query::<(Entity, &Act)>().iter(f.world()).find(|(_, act)| **act == Act::Goto(Screen::SinglePlayer)).unwrap().0;
    click(f.world_mut(), go);
    settle(&mut f, 3);
    let open = f.world_mut().query::<(Entity, &OpenSaveButton)>().iter(f.world()).next().expect("a Load game button").0;
    click(f.world_mut(), open);
    settle(&mut f, 3);
    assert!(screen(&f).open && screen(&f).origin == Some(AppState::FrontEnd));
    assert_eq!(screen(&f).entries.len(), 1);
    select_row(&mut f, 0);
    click_where(&mut f, SaveAct::Primary);
    assert!(screen(&f).ask.is_none(), "nothing to leave: it loads at once");
    until(&mut f, 400, |a| state(a) == AppState::InGame && !screen(a).open);
    settle(&mut f, 3);
    assert_eq!((tick(&f), digest(&f)), (10, saved_digest));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_of_another_version_and_one_of_other_content_are_listed_greyed_with_their_reasons_and_cannot_be_loaded() {
    let dir = temp_dir("grey");
    let other_dir = temp_dir("grey-other");
    let data = armed();
    let mut a = menu_app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 4);
    save_from_the_menu(&mut a, "good");
    let good = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();

    // Another container version: the number at its fixed place.
    let mut bytes = std::fs::read(&good).unwrap();
    bytes[4..6].copy_from_slice(&9u16.to_le_bytes());
    std::fs::write(dir.join("version.stasave"), &bytes).unwrap();
    // Other content: a save made in a game that ran on other content.
    let mut data2 = armed();
    data2.content = Arc::new(sta_content::fake::db_with_formations());
    let mut b = menu_app(&other_dir, true);
    start(&mut b, setup(&data2, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut b, 4);
    save_from_the_menu(&mut b, "elsewhere");
    let theirs = std::fs::read_dir(&other_dir).unwrap().next().unwrap().unwrap().path();
    std::fs::copy(&theirs, dir.join("content.stasave")).unwrap();

    click_where(&mut a, MenuAct::Load);
    let s = screen(&a);
    assert_eq!(s.entries.len(), 3, "refused files are listed, not hidden");
    let by = |name: &str| s.entries.iter().position(|e| e.file_name == name).unwrap();
    let (v, c) = (by("version.stasave"), by("content.stasave"));
    assert!(s.entries[v].refusal.as_deref().is_some_and(|r| r.contains("version 9") && r.contains("reads version")), "{:?}", s.entries[v].refusal);
    assert!(s.entries[c].refusal.as_deref().is_some_and(|r| r.contains("other content")), "{:?}", s.entries[c].refusal);
    assert_eq!(s.entries.iter().filter(|e| e.loadable()).count(), 1);
    let greyed = a.world_mut().query::<(&SaveRow, &RowLoadable)>().iter(a.world()).filter(|(_, l)| !l.0).map(|(r, _)| r.0).collect::<Vec<_>>();
    assert_eq!(greyed.len(), 2);
    assert!(greyed.contains(&v) && greyed.contains(&c));
    assert!(texts(&mut a).iter().any(|t| t.contains("other content")), "the reason is on the screen");

    // A greyed row can be chosen (to delete it) but not loaded.
    for n in [v, c] {
        select_row(&mut a, n);
        assert_eq!(primary_state(&mut a), WidgetState::Disabled);
        let e = a.world_mut().query::<(Entity, &SaveAct)>().iter(a.world()).find(|(_, act)| **act == SaveAct::Primary).unwrap().0;
        click(a.world_mut(), e);
        settle(&mut a, 2);
        assert!(!screen(&a).loading && screen(&a).ask.is_none(), "nothing was asked or loaded");
    }
    assert_eq!(state(&a), AppState::InGame);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other_dir);
}

#[test]
fn save_says_it_is_not_allowed_with_the_reason_and_cannot_be_pressed() {
    let dir = temp_dir("blocked");
    let data = armed();
    // A match whose map has no file.
    let mut a = menu_app(&dir, false);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    open_menu(&mut a);
    assert!(texts(&mut a).iter().any(|t| t.to_lowercase().contains("not allowed")), "the menu entry says so");
    click_where(&mut a, MenuAct::Save);
    let reason = a.world().resource::<SaveAvailability>().0.clone().expect("not allowed").to_string();
    assert!(reason.contains("map file"));
    assert!(texts(&mut a).iter().any(|t| t.contains(&reason)), "the screen gives UI-55's reason");
    assert_eq!(primary_state(&mut a), WidgetState::Disabled);
    assert!(container::store::list(&dir, None).unwrap().is_empty());

    // A cutscene is another reason.
    let mut b = menu_app(&dir, true);
    start(&mut b, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    b.world_mut().insert_resource(crate::ui::presentation::CineractiveState { on: true, skippable: false });
    open_menu(&mut b);
    click_where(&mut b, MenuAct::Save);
    assert!(texts(&mut b).iter().any(|t| t.contains("cutscene")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_is_off_while_a_load_is_pending() {
    let dir = temp_dir("pending");
    let data = armed();
    let mut a = menu_app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 3);
    save_from_the_menu(&mut a, "one");
    let mut f = menu_app(&dir, true);
    settle(&mut f, 4);
    f.world_mut().write_message(OpenSaveScreen(SaveMode::Load));
    settle(&mut f, 3);
    select_row(&mut f, 0);
    assert_eq!(primary_state(&mut f), WidgetState::Normal);
    let e = f.world_mut().query::<(Entity, &SaveAct)>().iter(f.world()).find(|(_, act)| **act == SaveAct::Primary).unwrap().0;
    click(f.world_mut(), e);
    f.update();
    assert!(screen(&f).loading, "the click makes the load pending");
    settle(&mut f, 1);
    if screen(&f).open {
        assert_eq!(primary_state(&mut f), WidgetState::Disabled, "Load is off until the load is over");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_over_a_save_and_deleting_one_ask_first() {
    let dir = temp_dir("asks");
    let data = armed();
    let mut a = menu_app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 3);
    save_from_the_menu(&mut a, "slot");
    assert_eq!(container::store::list(&dir, None).unwrap().len(), 1);
    run(&mut a, 7);

    // The same description over the chosen row: a question; No changes nothing.
    click_where(&mut a, MenuAct::Save);
    select_row(&mut a, 0);
    assert_eq!(screen(&a).name, "slot", "choosing a row puts its description in the name field");
    set_name(&mut a, "slot");
    click_where(&mut a, SaveAct::Primary);
    assert_eq!(screen(&a).ask, Some(Ask::Overwrite(0)));
    click_where(&mut a, SaveAct::No);
    assert!(screen(&a).ask.is_none());
    assert_eq!(container::store::list(&dir, None).unwrap()[0].outcome.as_ref().unwrap().tick, 3, "still the first save");
    // Yes replaces it.
    click_where(&mut a, SaveAct::Primary);
    click_where(&mut a, SaveAct::Yes);
    let listed = container::store::list(&dir, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].outcome.as_ref().unwrap().tick, 10, "the file was replaced, not added");

    // Delete asks, and removes the file once it is said yes.
    click_where(&mut a, MenuAct::Load);
    select_row(&mut a, 0);
    click_where(&mut a, SaveAct::Delete);
    assert_eq!(screen(&a).ask, Some(Ask::Delete(0)));
    click_where(&mut a, SaveAct::No);
    assert_eq!(container::store::list(&dir, None).unwrap().len(), 1);
    click_where(&mut a, SaveAct::Delete);
    click_where(&mut a, SaveAct::Yes);
    assert!(container::store::list(&dir, None).unwrap().is_empty());
    assert!(screen(&a).entries.is_empty());
    assert!(drain::<SaveOutcome>(&mut a).iter().all(|o| matches!(o, SaveOutcome::Saved { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dates_and_times_read_as_a_person_reads_them() {
    assert_eq!(date(0), "1970-01-01 00:00");
    assert_eq!(date(951_827_640), "2000-02-29 12:34");
    assert_eq!(date(1_795_900_000), "2026-11-28 21:06");
    assert_eq!(clock(59), "0:59");
    assert_eq!(clock(3725), "1:02:05");
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

const WINDOWS: [(&str, u32, u32); 3] = [("4:3", 1600, 1200), ("16:9", 1920, 1080), ("21:9", 3440, 1440)];

fn overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x - 0.01 && b.min.x < a.max.x - 0.01 && a.min.y < b.max.y - 0.01 && b.min.y < a.max.y - 0.01
}

fn node_rect(app: &App, e: Entity) -> Rect {
    let n = app.world().get::<Node>(e).unwrap();
    let px = |v: Val| match v {
        Val::Px(p) => p,
        other => panic!("not px: {other:?}"),
    };
    Rect::new(px(n.left), px(n.top), px(n.left) + px(n.width), px(n.top) + px(n.height))
}

fn entry(name: &str, refusal: Option<&str>) -> Entry {
    Entry { file_name: name.into(), path: PathBuf::from(name), header: None, refusal: refusal.map(str::to_owned) }
}

#[test]
fn every_screen_fits_three_window_shapes() {
    use crate::frontend::screens::stock::layout::fit_box;
    for (name, w, h) in WINDOWS {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        configure_sets(&mut app);
        app.insert_state(AppState::InGame).add_plugins((InGameMenuPlugin, SaveMenuPlugin));
        app.world_mut().spawn((Window { resolution: WindowResolution::new(w, h), ..default() }, PrimaryWindow));
        settle(&mut app, 3);
        let b = fit_box(Vec2::new(w as f32, h as f32));
        assert_eq!(app.world().resource::<StockBox>().0, b, "{name}: the box");
        let theme = app.world().resource::<Theme>().clone();
        let many: Vec<Entry> = (0..40).map(|i| entry(&format!("s{i}.stasave"), (i % 3 == 0).then_some("other content"))).collect();
        for (mode, entries, ask) in [
            (SaveMode::Load, many.clone(), None),
            (SaveMode::Save, many.clone(), None),
            (SaveMode::Load, Vec::new(), None),
            (SaveMode::Save, many.clone(), Some(Ask::Overwrite(0))),
            (SaveMode::Load, many.clone(), Some(Ask::Leave(1))),
        ] {
            let ctx = format!("{name} {mode:?} {} rows, ask {ask:?}", entries.len());
            *app.world_mut().resource_mut::<SaveScreen>() = SaveScreen { open: true, mode, origin: Some(AppState::InGame), entries, selected: Some(0), ask, status: Some("a status".into()), ..default() };
            settle(&mut app, 3);
            let placed: Vec<(SaveSlot, Entity)> = app.world_mut().query::<(Entity, &SavePlaced)>().iter(app.world()).map(|(e, p)| (p.0, e)).collect();
            for slot in slots_of(mode, ask.is_some()) {
                assert!(placed.iter().any(|(s, _)| *s == slot), "{ctx}: no {slot:?}");
            }
            let is_ask = |s: SaveSlot| matches!(s, SaveSlot::AskPanel | SaveSlot::AskTitle | SaveSlot::AskLine1 | SaveSlot::AskLine2 | SaveSlot::Yes | SaveSlot::No);
            let mut main_rects = Vec::new();
            let mut ask_rects = Vec::new();
            for (slot, e) in &placed {
                let r = node_rect(&app, *e);
                let want = region(*slot).px(b);
                for (got, exp) in [(r.min.x, want.min.x), (r.min.y, want.min.y), (r.max.x, want.max.x), (r.max.y, want.max.y)] {
                    assert!((got - exp).abs() < 0.01, "{ctx}: {slot:?} {r:?} is not its region {want:?}");
                }
                assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= b.x + 0.01 && r.max.y <= b.y + 0.01, "{ctx}: {slot:?} leaves the box");
                if matches!(slot, SaveSlot::Primary | SaveSlot::Delete | SaveSlot::Back | SaveSlot::Yes | SaveSlot::No) {
                    assert!(r.height() + 0.01 >= theme.button_h, "{ctx}: {slot:?} is {}px high for a {}px button", r.height(), theme.button_h);
                }
                if matches!(slot, SaveSlot::Panel | SaveSlot::AskPanel) {
                    continue;
                }
                if is_ask(*slot) { ask_rects.push((*slot, r)) } else { main_rects.push((*slot, r)) }
            }
            for group in [&main_rects, &ask_rects] {
                for (i, (sa, ra)) in group.iter().enumerate() {
                    for (sb, rb) in &group[i + 1..] {
                        assert!(!overlap(*ra, *rb), "{ctx}: {sa:?} overlaps {sb:?}");
                    }
                }
            }
            // The list shows whole rows and scrolls past them.
            if !screen(&app).entries.is_empty() {
                let list = app.world_mut().query_filtered::<&Node, With<SaveList>>().single(app.world()).unwrap().clone();
                let Val::Px(list_h) = list.height else { panic!("{ctx}: list height") };
                let slot_h = region(SaveSlot::List).px(b).height();
                let rows = list_h / theme.list_row_h;
                assert!(list_h <= slot_h + 0.01 && (rows - rows.round()).abs() < 1e-3, "{ctx}: the list is {list_h}px in a {slot_h}px slot");
            }
        }
    }
}
