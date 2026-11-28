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
use crate::frontend::screens::stock::ScreenLayout;
use crate::frontend::screens::{Act, Screen, ScreensConfig};
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
    assert!(matches!(screen(&a).ask, Some(Ask::Leave { .. })), "leaving a running match is asked about");
    click_where(&mut a, SaveAct::Yes);
    until(&mut a, 400, |a| state(a) == AppState::InGame && !screen(a).open && a.world().get_resource::<PendingLoad>().is_none());
    settle(&mut a, 3);
    assert_eq!((tick(&a), digest(&a)), (10, saved_digest), "the match continues from the save");

    // From the front end, in each look: a fresh app, the single-player screen's Load game button.
    for layout in [ScreenLayout::Stock, ScreenLayout::Plain] {
        let mut f = menu_app(&dir, true);
        f.insert_resource(ScreensConfig { own_camera: false, fit_scale: false, layout, ..default() });
        settle(&mut f, 4);
        let go = f.world_mut().query::<(Entity, &Act)>().iter(f.world()).find(|(_, act)| **act == Act::Goto(Screen::SinglePlayer)).unwrap_or_else(|| panic!("{layout:?}: no way to single player")).0;
        click(f.world_mut(), go);
        settle(&mut f, 3);
        let opens: Vec<Entity> = f.world_mut().query::<(Entity, &OpenSaveButton)>().iter(f.world()).map(|(e, _)| e).collect();
        assert_eq!(opens.len(), 1, "{layout:?}: one Load game button");
        click(f.world_mut(), opens[0]);
        settle(&mut f, 3);
        assert!(screen(&f).open && screen(&f).origin == Some(AppState::FrontEnd), "{layout:?}");
        assert_eq!(screen(&f).entries.len(), 1);
        select_row(&mut f, 0);
        click_where(&mut f, SaveAct::Primary);
        assert!(screen(&f).ask.is_none(), "nothing to leave: it loads at once");
        until(&mut f, 400, |a| state(a) == AppState::InGame && !screen(a).open);
        settle(&mut f, 3);
        assert_eq!((tick(&f), digest(&f)), (10, saved_digest), "{layout:?}");
    }
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
    click_where(&mut a, MenuAct::Load);
    select_row(&mut a, 0);
    assert_eq!(primary_state(&mut a), WidgetState::Normal);
    // Leaving the match is answered Yes: from this frame the load is pending and the screen is still up.
    click_where(&mut a, SaveAct::Primary);
    assert!(matches!(screen(&a).ask, Some(Ask::Leave { .. })));
    let yes = a.world_mut().query::<(Entity, &SaveAct)>().iter(a.world()).find(|(_, act)| **act == SaveAct::Yes).unwrap().0;
    click(a.world_mut(), yes);
    a.update();
    assert!(screen(&a).open && screen(&a).loading, "the answer makes the load pending");
    assert_eq!(primary_state(&mut a), WidgetState::Disabled, "Load is off until the load is over");
    // A press that reaches the screen anyway (the button is off, but the guard is the rule) starts nothing.
    let primary = a.world_mut().query::<(Entity, &SaveAct)>().iter(a.world()).find(|(_, act)| **act == SaveAct::Primary).unwrap().0;
    a.world_mut().write_message(ButtonClicked(primary));
    a.update();
    assert!(screen(&a).ask.is_none(), "no second question, no second load");
    assert!(screen(&a).loading);
    // UI-55 refusing the load frees Load again.
    a.world_mut().write_message(LoadOutcome::Refused { path: PathBuf::from("x"), reason: "no".into() });
    a.update();
    assert!(!screen(&a).loading);
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
    assert!(matches!(&screen(&a).ask, Some(Ask::Overwrite { description, .. }) if description == "slot"));
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
    assert!(matches!(screen(&a).ask, Some(Ask::Delete { .. })));
    click_where(&mut a, SaveAct::No);
    assert_eq!(container::store::list(&dir, None).unwrap().len(), 1);
    click_where(&mut a, SaveAct::Delete);
    click_where(&mut a, SaveAct::Yes);
    assert!(container::store::list(&dir, None).unwrap().is_empty());
    assert!(screen(&a).entries.is_empty());
    assert!(drain::<SaveOutcome>(&mut a).iter().all(|o| matches!(o, SaveOutcome::Saved { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two saves of different descriptions; the screen's rows are looked up by title.
fn two_saves(name: &str) -> (App, PathBuf) {
    let dir = temp_dir(name);
    let data = armed();
    let mut a = menu_app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 3);
    save_from_the_menu(&mut a, "alpha");
    run(&mut a, 3);
    click_where(&mut a, MenuAct::Save);
    set_name(&mut a, "beta");
    click_where(&mut a, SaveAct::Primary);
    assert_eq!(container::store::list(&dir, None).unwrap().len(), 2);
    (a, dir)
}

fn row_of(app: &App, title: &str) -> usize {
    screen(app).entries.iter().position(|e| e.title() == title).unwrap_or_else(|| panic!("no row {title}"))
}

fn file_of(app: &App, title: &str) -> String {
    screen(app).entries[row_of(app, title)].file_name.clone()
}

fn descriptions(dir: &PathBuf) -> Vec<(String, String, u64)> {
    let mut v: Vec<_> = container::store::list(dir, None).unwrap().into_iter().map(|l| {
        let h = l.outcome.unwrap();
        (l.file_name, h.description, h.tick)
    }).collect();
    v.sort();
    v
}

#[test]
fn a_question_is_modal_and_acts_on_the_save_it_named() {
    let (mut a, dir) = two_saves("modal-delete");
    click_where(&mut a, MenuAct::Load);
    let (ia, ib) = (row_of(&a, "alpha"), row_of(&a, "beta"));
    let (fa, fb) = (file_of(&a, "alpha"), file_of(&a, "beta"));
    select_row(&mut a, ia);
    click_where(&mut a, SaveAct::Delete);
    assert_eq!(screen(&a).ask, Some(Ask::Delete { file: fa.clone() }));
    // The question says which save it means.
    assert!(texts(&mut a).iter().any(|t| t.contains("alpha (20")), "the question names the save and its date");
    // Nothing behind it takes a click: another row, the buttons.
    click_where(&mut a, SaveRow(ib));
    assert_eq!(screen(&a).selected, Some(ia), "the choice did not move");
    assert_eq!(screen(&a).ask, Some(Ask::Delete { file: fa.clone() }));
    let behind = a.world_mut().query::<(Entity, &SaveAct, &WidgetState)>().iter(a.world()).filter(|(_, act, _)| matches!(act, SaveAct::Primary | SaveAct::Delete)).map(|(_, _, st)| *st).collect::<Vec<_>>();
    assert_eq!(behind, vec![WidgetState::Disabled; 2], "the buttons behind the question are off");
    // A press that reaches the screen anyway does not change the question.
    let primary = a.world_mut().query::<(Entity, &SaveAct)>().iter(a.world()).find(|(_, act)| **act == SaveAct::Primary).unwrap().0;
    a.world_mut().write_message(ButtonClicked(primary));
    a.update();
    assert_eq!(screen(&a).ask, Some(Ask::Delete { file: fa.clone() }));
    // Yes removes the save that was asked about, and only it.
    click_where(&mut a, SaveAct::Yes);
    let left: Vec<String> = descriptions(&dir).into_iter().map(|d| d.0).collect();
    assert_eq!(left, vec![fb.clone()]);

    // The file the question named is gone from the list by the time it is answered: nothing is done.
    let mut s = screen(&a);
    s.ask = Some(Ask::Delete { file: fa });
    *a.world_mut().resource_mut::<SaveScreen>() = s;
    settle(&mut a, 2);
    click_where(&mut a, SaveAct::Yes);
    assert_eq!(descriptions(&dir).len(), 1, "the other save is untouched");
    assert!(screen(&a).status.as_deref().is_some_and(|t| t.contains("no longer")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_overwrite_question_replaces_the_save_it_named_with_the_description_it_was_asked_with() {
    let (mut a, dir) = two_saves("modal-overwrite");
    let (fa, fb) = (file_of_disk(&dir, "alpha"), file_of_disk(&dir, "beta"));
    run(&mut a, 4);
    click_where(&mut a, MenuAct::Save);
    let (ia, ib) = (row_of(&a, "alpha"), row_of(&a, "beta"));
    select_row(&mut a, ia);
    set_name(&mut a, "alpha");
    click_where(&mut a, SaveAct::Primary);
    assert!(matches!(&screen(&a).ask, Some(Ask::Overwrite { file, description }) if *file == fa && description == "alpha"));
    // Choosing the other row now does nothing: neither the choice nor the name field follows it.
    click_where(&mut a, SaveRow(ib));
    assert_eq!(screen(&a).selected, Some(ia));
    assert_eq!(screen(&a).name, "alpha");
    click_where(&mut a, SaveAct::Yes);
    let now = descriptions(&dir);
    let alpha = now.iter().find(|d| d.0 == fa).unwrap();
    let beta = now.iter().find(|d| d.0 == fb).unwrap();
    assert_eq!((alpha.1.as_str(), beta.1.as_str()), ("alpha", "beta"), "each file keeps its own description");
    assert!(alpha.2 > beta.2, "and the asked file is the one that was written again");
    let _ = std::fs::remove_dir_all(&dir);
}

fn file_of_disk(dir: &PathBuf, description: &str) -> String {
    descriptions(dir).into_iter().find(|d| d.1 == description).unwrap().0
}

/// The screen alone, on a window and the in-game state, with what UI-55 would say set by hand.
fn bare(width: u32, height: u32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    // A folder that does not exist: an empty list, and the user's own saves are never read.
    app.insert_resource(crate::save::SaveDir(Some(std::env::temp_dir().join("sta64-ui56-no-such-saves")))).insert_state(AppState::InGame).add_plugins((InGameMenuPlugin, SaveMenuPlugin));
    app.world_mut().spawn((Window { resolution: WindowResolution::new(width, height), ..default() }, PrimaryWindow));
    settle(&mut app, 3);
    app
}

#[test]
fn save_is_not_allowed_in_a_networked_match_and_in_a_replay_and_load_is_off_in_a_networked_match() {
    use crate::save::SaveBlock;
    use crate::ui::ingame_menu::NetworkedMatch;
    for (block, word) in [(SaveBlock::Networked, "networked"), (SaveBlock::NoJournal, "replay")] {
        let mut a = bare(1280, 720);
        a.world_mut().insert_resource(SaveAvailability(Some(block.clone())));
        a.world_mut().write_message(OpenSaveScreen(SaveMode::Save));
        settle(&mut a, 3);
        let reason = block.to_string();
        assert!(reason.contains(word), "{reason}");
        assert!(texts(&mut a).iter().any(|t| t.contains(&reason)), "the screen gives the reason for {block:?}");
        assert_eq!(primary_state(&mut a), WidgetState::Disabled);
    }

    // The in-game menu's Load entry is a live button in a single-player match and an off one in a networked match.
    for networked in [false, true] {
        let mut a = bare(1280, 720);
        if networked {
            a.world_mut().insert_resource(NetworkedMatch);
        }
        open_menu(&mut a);
        let live = a.world_mut().query::<&MenuAct>().iter(a.world()).any(|act| *act == MenuAct::Load);
        assert_eq!(live, !networked, "networked: {networked}");
        assert!(texts(&mut a).iter().any(|t| t.eq_ignore_ascii_case("load game")), "the entry is shown either way");
        if networked {
            let off = a.world_mut().query::<&WidgetState>().iter(a.world()).filter(|st| **st == WidgetState::Disabled).count();
            assert!(off >= 1, "an off button stands in its place");
        }
    }
}

#[test]
fn an_unreadable_folder_says_so_and_an_unreadable_entry_cannot_be_deleted() {
    let dir = temp_dir("unreadable");
    // A file where the folder should be: reading it as a folder is an error, not an empty folder.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::write(&dir, b"not a folder").unwrap();
    let mut a = menu_app(&dir, true);
    settle(&mut a, 4);
    a.world_mut().write_message(OpenSaveScreen(SaveMode::Load));
    settle(&mut a, 3);
    let s = screen(&a);
    assert!(s.entries.is_empty());
    assert!(s.status.as_deref().is_some_and(|t| t.contains("cannot be read")), "{:?}", s.status);
    assert!(texts(&mut a).iter().any(|t| t.contains("cannot be read")));
    let _ = std::fs::remove_file(&dir);

    // The listing's row for an entry it could not read has the folder's path and the name "?".
    let folder = PathBuf::from("/saves");
    let unreadable = Entry { file_name: "?".into(), path: folder, header: None, refusal: Some("cannot read".into()) };
    assert!(!unreadable.deletable());
    assert!(entry("a.stasave", None).deletable());
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
    let mut shapes: Vec<(&str, Vec2, f32)> = Vec::new();
    for (name, w, h) in WINDOWS {
        let mut app = bare(w, h);
        let b = fit_box(Vec2::new(w as f32, h as f32));
        assert_eq!(app.world().resource::<StockBox>().0, b, "{name}: the box");
        let theme = app.world().resource::<Theme>().clone();
        let many: Vec<Entry> = (0..40).map(|i| entry(&format!("s{i}.stasave"), (i % 3 == 0).then_some("other content"))).collect();
        for (mode, entries, ask) in [
            (SaveMode::Load, many.clone(), None),
            (SaveMode::Save, many.clone(), None),
            (SaveMode::Load, Vec::new(), None),
            (SaveMode::Save, many.clone(), Some(Ask::Overwrite { file: "s0.stasave".into(), description: "s0".into() })),
            (SaveMode::Load, many.clone(), Some(Ask::Leave { file: "s1.stasave".into() })),
        ] {
            let ctx = format!("{name} {mode:?} {} rows, ask {ask:?}", entries.len());
            *app.world_mut().resource_mut::<SaveScreen>() = SaveScreen { open: true, mode, origin: Some(AppState::InGame), entries, selected: Some(0), ask: ask.clone(), status: Some("a status".into()), ..default() };
            settle(&mut app, 3);
            let placed: Vec<(SaveSlot, Entity)> = app.world_mut().query::<(Entity, &SavePlaced)>().iter(app.world()).map(|(e, p)| (p.0, e)).collect();
            for slot in slots_of(mode, ask.is_some()) {
                assert!(placed.iter().any(|(s, _)| *s == slot), "{ctx}: no {slot:?}");
            }
            let is_ask = |s: SaveSlot| matches!(s, SaveSlot::AskPanel | SaveSlot::AskTitle | SaveSlot::AskLine1 | SaveSlot::AskLine2 | SaveSlot::Yes | SaveSlot::No);
            let mut main_rects = Vec::new();
            let mut ask_rects = Vec::new();
            let all_texts: Vec<(Entity, String)> = app.world_mut().query::<(Entity, &SpriteText)>().iter(app.world()).map(|(e, t)| (e, t.text.clone())).collect();
            let under = |root: Entity| -> Vec<String> {
                all_texts
                    .iter()
                    .filter(|(start, _)| {
                        let mut e = *start;
                        while e != root {
                            match app.world().get::<ChildOf>(e) {
                                Some(c) => e = c.parent(),
                                None => return false,
                            }
                        }
                        true
                    })
                    .map(|(_, t)| t.clone())
                    .collect()
            };
            for (slot, e) in &placed {
                let r = node_rect(&app, *e);
                // A one-line text fits its slot (its width estimated from the size of its letters).
                let size = match slot {
                    SaveSlot::Title | SaveSlot::AskTitle => Some(theme.title_size),
                    SaveSlot::Primary | SaveSlot::Delete | SaveSlot::Back | SaveSlot::Yes | SaveSlot::No => Some(theme.text_size),
                    _ => None,
                };
                if let Some(size) = size {
                    let words = under(*e);
                    assert!(!words.is_empty(), "{ctx}: {slot:?} has no text");
                    for text in words {
                        let width = text.chars().count() as f32 * size * 0.55;
                        assert!(width <= r.width() + 0.01, "{ctx}: {slot:?} text {text:?} (about {width}px) is wider than its {}px slot", r.width());
                    }
                }
                if *slot == SaveSlot::Primary && ctx.ends_with("ask None") && mode == SaveMode::Load && !screen(&app).entries.is_empty() {
                    shapes.push((name, b, r.width() / r.height()));
                }
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
    // The three windows are three different shapes, and the screen follows each.
    assert_eq!(shapes.len(), 3);
    for (i, (na, ba, _)) in shapes.iter().enumerate() {
        for (nb, bb, _) in &shapes[i + 1..] {
            assert!(ba != bb, "{na} and {nb} gave the same box {ba:?}");
        }
    }
    // The box follows the window's shape up to the widest the layouts allow (16:9): 4:3 and 16:9 differ,
    // and 21:9 is held to the widest box rather than stretched.
    assert!((shapes[0].2 - shapes[1].2).abs() > 0.01, "4:3 and 16:9 gave the same shape");
    assert!((shapes[1].2 - shapes[2].2).abs() < 0.01, "21:9 is held to the 16:9 shape");
}

#[test]
fn the_installs_caption_keys_win_over_the_screens_own_text() {
    let dir = temp_dir("labels");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("label.map"),
        "+ save_game_disallowed {Authored: not now}\n+ save_game_failure {Authored: failed}\n+ game_type_unloadable {Authored: unloadable}\n+ load_game {Authored load}\n",
    )
    .unwrap();
    let mut vfs = sta_vfs::Vfs::default();
    vfs.push_dir("authored".into(), dir.clone()).unwrap();
    let strings = Strings::load(&vfs, "english");
    assert_eq!(caption(Some(&strings), "load_game", "Load game"), "Authored load");
    assert_eq!(caption(Some(&strings), "no_such_key", "Own text"), "Own text", "the screen's own text where the install has no key");
    let refused = entry("a.stasave", Some("another version"));
    assert!(detail_text(Some(&refused), Some(&strings)).starts_with("Authored: unloadable: another version"));
    assert!(detail_text(Some(&refused), None).starts_with("This save cannot be loaded"));

    // On the screen: the not-allowed line and the refused save's line lead with the install's text.
    let mut a = bare(1280, 720);
    a.world_mut().insert_resource(strings);
    a.world_mut().insert_resource(SaveAvailability(Some(crate::save::SaveBlock::Networked)));
    a.world_mut().write_message(OpenSaveScreen(SaveMode::Save));
    settle(&mut a, 3);
    assert!(texts(&mut a).iter().any(|t| t.starts_with("Authored: not now: ")), "{:?}", texts(&mut a));
    a.world_mut().write_message(SaveOutcome::Refused(container::SaveError::Invalid("x".into())));
    settle(&mut a, 3);
    assert!(texts(&mut a).iter().any(|t| t.starts_with("Authored: failed: ")), "{:?}", texts(&mut a));
    let _ = std::fs::remove_dir_all(&dir);
}
