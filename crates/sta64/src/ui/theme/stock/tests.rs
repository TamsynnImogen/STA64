//! Windowless tests of the widget kit: the dropdown, keyboard and mouse paths of
//! every widget, and the gallery in each UI profile. Clicks go through `kit_click` /
//! `click`, presses through `press`, keys through `kit_key`: the same functions the
//! pointer observers and the keyboard system call.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::backend::HitData;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;

use super::*;
use crate::app::configure_sets;
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{GalleryPlugin, GalleryRoot, ListItem, ListSelected, Slider, SliderChanged, ThemePlugin, UiButton, click, label};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.add_plugins(ThemePlugin);
    app.update();
    app
}

fn msgs<M: Message>(app: &App) -> usize {
    app.world().resource::<Messages<M>>().len()
}

fn open_dropdown(app: &mut App, options: &[&str]) -> Entity {
    let theme = app.world().resource::<Theme>().clone();
    let e = app.world_mut().spawn(dropdown(&theme, options, 0, 200.0)).id();
    app.update();
    e
}

fn popup_rows(app: &mut App) -> Vec<(usize, Entity)> {
    let mut rows: Vec<_> = app.world_mut().query::<(Entity, &DropdownOption)>().iter(app.world()).map(|(e, o)| (o.0, e)).collect();
    rows.sort();
    rows
}

fn label_of(app: &App, e: Entity) -> String {
    let kids = app.world().get::<Children>(e).unwrap();
    kids.iter().find_map(|k| app.world().get::<SpriteText>(k)).map(|t| t.text.clone()).unwrap()
}

#[test]
fn dropdown_opens_selects_and_closes() {
    let mut app = app();
    let d = open_dropdown(&mut app, &["Federation", "Klingon", "Romulan"]);
    assert!(popup_rows(&mut app).is_empty());
    kit_click(app.world_mut(), d);
    app.update();
    let rows = popup_rows(&mut app);
    assert_eq!(rows.len(), 3);
    assert!(app.world().get::<Dropdown>(d).unwrap().open);
    // The list is a child of the field, drawn above everything.
    let popup = app.world_mut().query_filtered::<Entity, With<DropdownPopup>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<ChildOf>(popup).unwrap().parent(), d);
    assert!(app.world().get::<GlobalZIndex>(popup).is_some());

    kit_click(app.world_mut(), rows[2].1);
    app.update();
    let dd = app.world().get::<Dropdown>(d).unwrap();
    assert_eq!((dd.selected, dd.open), (2, false));
    assert!(popup_rows(&mut app).is_empty(), "the list closes on a choice");
    assert_eq!(msgs::<DropdownChanged>(&app), 1);
    assert_eq!(label_of(&app, d), "Romulan");
}

#[test]
fn clicking_the_field_again_closes_without_a_change() {
    let mut app = app();
    let d = open_dropdown(&mut app, &["A", "B"]);
    kit_click(app.world_mut(), d);
    app.update();
    kit_click(app.world_mut(), d);
    app.update();
    assert!(!app.world().get::<Dropdown>(d).unwrap().open);
    assert!(popup_rows(&mut app).is_empty());
    assert_eq!(msgs::<DropdownChanged>(&app), 0);
}

#[test]
fn dropdown_closes_on_a_press_outside_and_stays_on_a_press_inside() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let other = app.world_mut().spawn(label(&theme, "elsewhere")).id();
    let d = open_dropdown(&mut app, &["A", "B"]);
    kit_click(app.world_mut(), d);
    app.update();
    let row = popup_rows(&mut app)[1].1;
    press(app.world_mut(), &[row]);
    assert!(app.world().get::<Dropdown>(d).unwrap().open, "a press on its own row is inside");
    press(app.world_mut(), &[other]);
    app.update();
    assert!(!app.world().get::<Dropdown>(d).unwrap().open);
    assert!(popup_rows(&mut app).is_empty());
    assert_eq!(app.world().get::<Dropdown>(d).unwrap().selected, 0);
}

#[test]
fn outside_press_through_the_pointer_system() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let other = app.world_mut().spawn(label(&theme, "elsewhere")).id();
    let d = open_dropdown(&mut app, &["A", "B"]);
    kit_click(app.world_mut(), d);
    app.update();
    // The mouse is down over `other`.
    let camera = app.world_mut().spawn_empty().id();
    app.world_mut().resource_mut::<HoverMap>().insert(PointerId::Mouse, [(other, HitData::new(camera, 0.0, None, None))].into_iter().collect());
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    app.update();
    assert!(!app.world().get::<Dropdown>(d).unwrap().open);
    assert_eq!(app.world().resource::<KitFocus>().0, None, "a press over nothing focusable clears focus");
}

#[test]
fn dropdown_keyboard() {
    let mut app = app();
    let d = open_dropdown(&mut app, &["A", "B", "C", "D"]);
    press(app.world_mut(), &[d]);
    assert_eq!(app.world().resource::<KitFocus>().0, Some(d));
    let key = |app: &mut App, k: KeyCode| {
        kit_key(app.world_mut(), KitKey::code(k));
        app.update();
    };
    // Closed: arrows change the choice.
    key(&mut app, KeyCode::ArrowDown);
    assert_eq!(app.world().get::<Dropdown>(d).unwrap().selected, 1);
    key(&mut app, KeyCode::ArrowUp);
    key(&mut app, KeyCode::ArrowUp);
    assert_eq!(app.world().get::<Dropdown>(d).unwrap().selected, 0);
    assert_eq!(msgs::<DropdownChanged>(&app), 2);
    // Enter opens; arrows move the highlight only; Escape closes unchanged.
    key(&mut app, KeyCode::Enter);
    assert!(app.world().get::<Dropdown>(d).unwrap().open);
    key(&mut app, KeyCode::ArrowDown);
    key(&mut app, KeyCode::ArrowDown);
    assert_eq!(app.world().get::<Dropdown>(d).unwrap().highlight, 2);
    assert_eq!(app.world().get::<Dropdown>(d).unwrap().selected, 0);
    key(&mut app, KeyCode::Escape);
    assert!(!app.world().get::<Dropdown>(d).unwrap().open);
    assert_eq!(app.world().get::<Dropdown>(d).unwrap().selected, 0);
    // Open, End, Enter chooses the last row.
    key(&mut app, KeyCode::Space);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Enter);
    let dd = app.world().get::<Dropdown>(d).unwrap();
    assert_eq!((dd.selected, dd.open), (3, false));
    assert_eq!(label_of(&app, d), "D");
}

#[test]
fn a_long_dropdown_scrolls_to_keep_the_highlight_in_view() {
    let mut app = app();
    let names: Vec<String> = (0..20).map(|i| format!("Row {i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let d = open_dropdown(&mut app, &refs);
    press(app.world_mut(), &[d]);
    kit_key(app.world_mut(), KitKey::code(KeyCode::Enter));
    kit_key(app.world_mut(), KitKey::code(KeyCode::End));
    app.update();
    let (area, pos) = app.world_mut().query::<(&ScrollArea, &ScrollPosition)>().single(app.world()).unwrap();
    assert_eq!(area.rows, MAX_ROWS, "an open list shows at most eight rows");
    assert_eq!(pos.0.y, area.max_offset());
    assert!(pos.0.y > 0.0);
}

#[test]
fn tab_walks_focus_in_tree_order_and_back() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let (c, d, t) = {
        let root = app.world_mut().spawn(Node::default()).id();
        let c = app.world_mut().spawn((checkbox(&theme, "c", false), ChildOf(root))).id();
        let d = app.world_mut().spawn((dropdown(&theme, &["a"], 0, 100.0), ChildOf(root))).id();
        let t = app.world_mut().spawn((text_field(&theme, "", "", 100.0, 8, TextFilter::Any), ChildOf(root))).id();
        (c, d, t)
    };
    app.update();
    let focus = |app: &App| app.world().resource::<KitFocus>().0;
    kit_key(app.world_mut(), KitKey::code(KeyCode::Tab));
    assert_eq!(focus(&app), Some(c));
    kit_key(app.world_mut(), KitKey::code(KeyCode::Tab));
    assert_eq!(focus(&app), Some(d));
    kit_key(app.world_mut(), KitKey::code(KeyCode::Tab));
    assert_eq!(focus(&app), Some(t));
    kit_key(app.world_mut(), KitKey::code(KeyCode::Tab));
    assert_eq!(focus(&app), Some(c), "wraps");
    kit_key(app.world_mut(), KitKey { code: KeyCode::Tab, text: None, shift: true });
    assert_eq!(focus(&app), Some(t), "Shift+Tab goes back");
    // The focus ring follows.
    app.update();
    assert!(app.world().get::<Outline>(t).is_some());
    assert!(app.world().get::<Outline>(c).is_none());
}

#[test]
fn tab_closes_an_open_dropdown() {
    let mut app = app();
    let d = open_dropdown(&mut app, &["A", "B"]);
    press(app.world_mut(), &[d]);
    kit_click(app.world_mut(), d);
    kit_key(app.world_mut(), KitKey::code(KeyCode::Tab));
    assert!(!app.world().get::<Dropdown>(d).unwrap().open);
}

#[test]
fn text_field_edits_with_the_keyboard() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let f = app.world_mut().spawn(text_field(&theme, "", "Name", 200.0, 5, TextFilter::Any)).id();
    app.update();
    assert_eq!(label_of(&app, f), "Name", "an empty unfocused field shows its placeholder");
    press(app.world_mut(), &[f]);
    for s in ["a", "b"] {
        kit_key(app.world_mut(), KitKey::typed(s));
    }
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "ab");
    app.update();
    assert_eq!(label_of(&app, f), "ab|", "the focused field shows its caret");
    kit_key(app.world_mut(), KitKey::code(KeyCode::Backspace));
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowLeft));
    kit_key(app.world_mut(), KitKey::typed("X"));
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "Xa");
    kit_key(app.world_mut(), KitKey::code(KeyCode::End));
    kit_key(app.world_mut(), KitKey::code(KeyCode::Home));
    kit_key(app.world_mut(), KitKey::code(KeyCode::Delete));
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "a");
    // The length limit holds.
    kit_key(app.world_mut(), KitKey::typed("bcdefgh"));
    assert_eq!(app.world().get::<TextField>(f).unwrap().text.chars().count(), 5);
    assert_eq!(msgs::<TextChanged>(&app), 6);
    // Enter submits and leaves the field.
    kit_key(app.world_mut(), KitKey::code(KeyCode::Enter));
    assert_eq!(msgs::<TextSubmitted>(&app), 1);
    assert_eq!(app.world().resource::<KitFocus>().0, None);
    app.update();
    assert!(!label_of(&app, f).contains('|'));
}

#[test]
fn text_field_filters_what_it_takes() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let f = app.world_mut().spawn(text_field(&theme, "12ab", "", 200.0, 8, TextFilter::Digits)).id();
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "12", "the initial text is filtered too");
    press(app.world_mut(), &[f]);
    kit_key(app.world_mut(), KitKey::typed("x7y"));
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "127");
    kit_key(app.world_mut(), KitKey::code(KeyCode::Escape));
    assert_eq!(app.world().resource::<KitFocus>().0, None);
    assert_eq!(msgs::<TextSubmitted>(&app), 0);
}

#[test]
fn text_field_handles_multibyte_characters() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let f = app.world_mut().spawn(text_field(&theme, "", "", 200.0, 8, TextFilter::Any)).id();
    press(app.world_mut(), &[f]);
    kit_key(app.world_mut(), KitKey::typed("\u{e9}\u{e8}"));
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowLeft));
    kit_key(app.world_mut(), KitKey::code(KeyCode::Backspace));
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "\u{e8}");
}

#[test]
fn checkbox_toggles_by_click_and_by_space() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let c = app.world_mut().spawn(checkbox(&theme, "Sound", false)).id();
    app.update();
    kit_click(app.world_mut(), c);
    assert!(app.world().get::<Checkbox>(c).unwrap().on);
    press(app.world_mut(), &[c]);
    kit_key(app.world_mut(), KitKey::code(KeyCode::Space));
    assert!(!app.world().get::<Checkbox>(c).unwrap().on);
    assert_eq!(msgs::<CheckboxChanged>(&app), 2);
    // The mark follows the value.
    kit_click(app.world_mut(), c);
    app.update();
    let frame = app.world().get::<Children>(c).unwrap()[0];
    let mark = app.world().get::<Children>(frame).unwrap()[0];
    assert_eq!(app.world().get::<BackgroundColor>(mark).unwrap().0, theme.accent);
}

#[test]
fn a_disabled_checkbox_does_not_toggle() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let c = app.world_mut().spawn(checkbox(&theme, "Locked", false)).id();
    app.world_mut().entity_mut(c).insert(WidgetState::Disabled);
    kit_click(app.world_mut(), c);
    assert!(!app.world().get::<Checkbox>(c).unwrap().on);
}

#[test]
fn tabs_show_one_panel_and_answer_clicks_and_arrows() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let bar = app.world_mut().spawn(tab_bar(&theme, &["A", "B", "C"], 0)).id();
    let panels: Vec<Entity> = (0..3).map(|i| app.world_mut().spawn(tab_panel(bar, i)).id()).collect();
    app.update();
    let shown = |app: &App| panels.iter().map(|p| app.world().get::<Node>(*p).unwrap().display == Display::Flex).collect::<Vec<_>>();
    assert_eq!(shown(&app), [true, false, false]);
    let tabs: Vec<Entity> = app.world().get::<Children>(bar).unwrap().iter().collect();
    kit_click(app.world_mut(), tabs[2]);
    app.update();
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 2);
    assert_eq!(shown(&app), [false, false, true]);
    press(app.world_mut(), &[bar]);
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowLeft));
    app.update();
    assert_eq!(shown(&app), [false, true, false]);
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowLeft));
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowLeft));
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 0);
    assert_eq!(msgs::<TabChanged>(&app), 3);
    // The selected tab is in the "on" colour.
    app.update();
    assert_eq!(app.world().get::<BackgroundColor>(tabs[0]).unwrap().0, theme.button_on);
    assert_eq!(app.world().get::<BackgroundColor>(tabs[1]).unwrap().0, theme.button);
}

#[test]
fn scroll_list_keyboard_selects_and_scrolls() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let names: Vec<String> = (0..12).map(|i| format!("Map {i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let l = app.world_mut().spawn(scroll_list(&theme, &refs, None, 4)).id();
    app.update();
    press(app.world_mut(), &[l]);
    let key = |app: &mut App, k: KeyCode| kit_key(app.world_mut(), KitKey::code(k));
    key(&mut app, KeyCode::ArrowDown);
    assert_eq!(app.world().get::<crate::ui::theme::List>(l).unwrap().selected, Some(0));
    for _ in 0..5 {
        key(&mut app, KeyCode::ArrowDown);
    }
    assert_eq!(app.world().get::<crate::ui::theme::List>(l).unwrap().selected, Some(5));
    let row_h = theme.list_row_h;
    // Row 5 is the last of a 4-row view that starts at row 2.
    assert_eq!(app.world().get::<ScrollPosition>(l).unwrap().0.y, 2.0 * row_h);
    key(&mut app, KeyCode::End);
    assert_eq!(app.world().get::<crate::ui::theme::List>(l).unwrap().selected, Some(11));
    assert_eq!(app.world().get::<ScrollPosition>(l).unwrap().0.y, 8.0 * row_h);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.world().get::<ScrollPosition>(l).unwrap().0.y, 0.0);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.world().get::<crate::ui::theme::List>(l).unwrap().selected, Some(4));
    assert_eq!(msgs::<ListSelected>(&app), 9);
    // Clicking a row selects it through the LCARS kit's click.
    let rows: Vec<Entity> = app.world().get::<Children>(l).unwrap().iter().collect();
    click(app.world_mut(), rows[9]);
    assert_eq!(app.world().get::<crate::ui::theme::List>(l).unwrap().selected, Some(9));
}

#[test]
fn the_wheel_scrolls_the_list_under_the_pointer() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let names: Vec<String> = (0..10).map(|i| format!("Map {i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let l = app.world_mut().spawn(scroll_list(&theme, &refs, None, 3)).id();
    app.update();
    let row = app.world().get::<Children>(l).unwrap()[0];
    let camera = app.world_mut().spawn_empty().id();
    app.world_mut().resource_mut::<HoverMap>().insert(PointerId::Mouse, [(row, HitData::new(camera, 0.0, None, None))].into_iter().collect());
    let window = app.world_mut().spawn_empty().id();
    app.world_mut().write_message(MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y: -1.0, window, phase: bevy::input::touch::TouchPhase::Moved });
    app.update();
    assert_eq!(app.world().get::<ScrollPosition>(l).unwrap().0.y, 3.0 * theme.list_row_h, "one wheel line is three rows");
    // Never past the end.
    for _ in 0..10 {
        app.world_mut().write_message(MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y: -1.0, window, phase: bevy::input::touch::TouchPhase::Moved });
        app.update();
    }
    assert_eq!(app.world().get::<ScrollPosition>(l).unwrap().0.y, 7.0 * theme.list_row_h);
}

#[test]
fn focusable_sliders_and_buttons_answer_the_keyboard() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let s = app.world_mut().spawn(slider_focusable(&theme, 200.0, 0.0, 100.0, 50.0, 10.0)).id();
    let b = app.world_mut().spawn(focusable(crate::ui::theme::button(&theme, "Go", 100.0))).id();
    press(app.world_mut(), &[s]);
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowRight));
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowRight));
    assert_eq!(app.world().get::<Slider>(s).unwrap().value, 70.0);
    kit_key(app.world_mut(), KitKey::code(KeyCode::Home));
    assert_eq!(app.world().get::<Slider>(s).unwrap().value, 0.0);
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowLeft));
    assert_eq!(app.world().get::<Slider>(s).unwrap().value, 0.0, "clamped");
    assert_eq!(msgs::<SliderChanged>(&app), 3);
    press(app.world_mut(), &[b]);
    kit_key(app.world_mut(), KitKey::code(KeyCode::Enter));
    assert_eq!(msgs::<crate::ui::theme::ButtonClicked>(&app), 1);
    // A disabled button is skipped by Tab and ignores keys.
    let off = app.world_mut().spawn(focusable(crate::ui::theme::disabled_button(&theme, "No", 100.0))).id();
    assert!(!focus_order(app.world_mut()).contains(&off));
    let _ = UiButton;
}

#[test]
fn font_roles_follow_the_theme_sizes() {
    use crate::ui::sprite_font::FontRole::*;
    let t = Theme::default();
    assert_eq!(font_role(&t, t.micro_size), Small);
    assert_eq!(font_role(&t, t.small_size), Small);
    assert_eq!(font_role(&t, t.text_size), Medium);
    assert_eq!(font_role(&t, t.title_size), Large);
    assert_eq!(font_role(&t, t.display_size), VeryLarge);
}

fn gallery(profile: UiProfile) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.insert_resource(profile).insert_resource(profile.theme());
    app.add_plugins(GalleryPlugin);
    app.update();
    app
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query_filtered::<(), With<C>>().iter(app.world()).count()
}

#[test]
fn the_gallery_shows_every_widget_in_every_profile() {
    for profile in [UiProfile::Lcars, UiProfile::Armada2, UiProfile::FleetOps] {
        let mut app = gallery(profile);
        assert_eq!(count::<GalleryRoot>(&mut app), 1, "{profile:?}");
        assert_eq!(count::<Dropdown>(&mut app), 1, "{profile:?}");
        assert_eq!(count::<Checkbox>(&mut app), 2, "{profile:?}");
        assert_eq!(count::<TextField>(&mut app), 1, "{profile:?}");
        assert_eq!(count::<TabBar>(&mut app), 2, "{profile:?}");
        assert_eq!(count::<TabPanel>(&mut app), 3, "{profile:?}");
        assert_eq!(count::<ScrollArea>(&mut app), 1, "{profile:?}");
        assert_eq!(count::<Slider>(&mut app), 1, "{profile:?}");
        assert_eq!(count::<UiButton>(&mut app), 2, "{profile:?}");
        // The widgets wear the profile's theme.
        let theme = app.world().resource::<Theme>().clone();
        let field = app.world_mut().query_filtered::<&BackgroundColor, With<TextField>>().single(app.world()).unwrap().0;
        assert_eq!(field, theme.track, "{profile:?}");
        let dd = app.world_mut().query_filtered::<&BackgroundColor, With<Dropdown>>().single(app.world()).unwrap().0;
        assert_eq!(dd, theme.button, "{profile:?}");
    }
    assert_ne!(UiProfile::Lcars.theme().track, UiProfile::Armada2.theme().track);
}

#[test]
fn the_profile_tabs_rebuild_the_gallery_in_the_other_skin() {
    let mut app = gallery(UiProfile::Lcars);
    let lcars_track = app.world().resource::<Theme>().track;
    let bar = app.world_mut().query_filtered::<Entity, With<crate::ui::theme::GalleryProfileBar>>().single(app.world()).unwrap();
    let tabs: Vec<Entity> = app.world().get::<Children>(bar).unwrap().iter().collect();
    kit_click(app.world_mut(), tabs[1]);
    app.update();
    app.update();
    assert_eq!(*app.world().resource::<UiProfile>(), UiProfile::Armada2);
    assert_ne!(app.world().resource::<Theme>().track, lcars_track);
    assert_eq!(count::<GalleryRoot>(&mut app), 1, "the old gallery is replaced, not added to");
    let field = app.world_mut().query_filtered::<&BackgroundColor, With<TextField>>().single(app.world()).unwrap().0;
    assert_eq!(field, UiProfile::Armada2.theme().track);
    // The new bar shows the new profile as selected.
    let bar = app.world_mut().query_filtered::<&TabBar, With<crate::ui::theme::GalleryProfileBar>>().single(app.world()).unwrap();
    assert_eq!(bar.selected, 1);
}

#[test]
fn stock_labels_are_light_on_the_dark_pills() {
    for profile in [UiProfile::Armada2, UiProfile::FleetOps] {
        let t = profile.theme();
        let lum = |c: Color| {
            let c = c.to_srgba();
            0.2126 * c.red + 0.7152 * c.green + 0.0722 * c.blue
        };
        assert!(lum(t.text_on) - lum(t.button) > 0.4, "{profile:?}: pill label contrast");
        assert!(lum(t.text) - lum(t.track) > 0.4, "{profile:?}: field text contrast");
    }
}

#[test]
fn a_real_click_on_a_row_chooses_and_does_not_reopen_the_list() {
    use bevy::picking::events::PointerClick;
    use bevy::picking::pointer::{Location, PointerButton};
    let mut app = app();
    let d = open_dropdown(&mut app, &["A", "B", "C"]);
    kit_click(app.world_mut(), d);
    app.update();
    let row = popup_rows(&mut app)[1].1;
    // The event bubbles row -> list -> field, as the picking backend sends it.
    let camera = app.world_mut().spawn_empty().id();
    app.world_mut().trigger(PointerClick {
        entity: row,
        pointer: bevy::picking::events::Pointer::new(
            PointerId::Mouse,
            Location { target: bevy::camera::NormalizedRenderTarget::None { width: 1, height: 1 }, position: Vec2::ZERO },
        ),
        button: PointerButton::Primary,
        hit: HitData::new(camera, 0.0, None, None),
        duration: std::time::Duration::ZERO,
        count: 1,
    });
    app.update();
    let dd = app.world().get::<Dropdown>(d).unwrap();
    assert_eq!((dd.selected, dd.open), (1, false), "chosen, and still closed");
    assert!(popup_rows(&mut app).is_empty());
}

#[test]
fn focus_on_a_hidden_panel_is_dropped_and_the_next_tab_is_not_lost() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let bar = app.world_mut().spawn(tab_bar(&theme, &["A", "B"], 0)).id();
    let panel = app.world_mut().spawn(tab_panel(bar, 0)).id();
    let f = app.world_mut().spawn((text_field(&theme, "", "", 100.0, 8, TextFilter::Any), ChildOf(panel))).id();
    let other = app.world_mut().spawn(checkbox(&theme, "x", false)).id();
    app.update();
    press(app.world_mut(), &[f]);
    assert_eq!(app.world().resource::<KitFocus>().0, Some(f));
    let tabs: Vec<Entity> = app.world().get::<Children>(bar).unwrap().iter().collect();
    kit_click(app.world_mut(), tabs[1]);
    app.update();
    // The panel is hidden: typing goes nowhere and Tab moves on at once.
    kit_key(app.world_mut(), KitKey::typed("z"));
    assert_eq!(app.world().get::<TextField>(f).unwrap().text, "");
    assert_eq!(app.world().resource::<KitFocus>().0, None);
    kit_key(app.world_mut(), KitKey::code(KeyCode::Tab));
    assert!(app.world().resource::<KitFocus>().0.is_some());
    let _ = other;
}

#[test]
fn a_disabled_widget_does_not_take_focus_and_scopes_confine_tab() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let off = app.world_mut().spawn(checkbox(&theme, "off", false)).id();
    app.world_mut().entity_mut(off).insert(WidgetState::Disabled);
    press(app.world_mut(), &[off]);
    assert_eq!(app.world().resource::<KitFocus>().0, None);
    // A modal scope: Tab walks only inside it.
    let behind = app.world_mut().spawn(checkbox(&theme, "behind", false)).id();
    let modal = app.world_mut().spawn((Node::default(), FocusScope)).id();
    let inside = app.world_mut().spawn((checkbox(&theme, "inside", false), ChildOf(modal))).id();
    app.update();
    assert_eq!(focus_order(app.world_mut()), vec![inside]);
    let _ = behind;
}

#[test]
fn the_kit_reports_when_it_wants_the_keyboard_or_the_wheel() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let f = app.world_mut().spawn(text_field(&theme, "", "", 100.0, 8, TextFilter::Any)).id();
    app.update();
    assert_eq!(*app.world().resource::<KitCapture>(), KitCapture::default());
    press(app.world_mut(), &[f]);
    app.update();
    assert!(app.world().resource::<KitCapture>().keyboard);
    let l = app.world_mut().spawn(scroll_list(&theme, &["a", "b"], None, 2)).id();
    let camera = app.world_mut().spawn_empty().id();
    app.world_mut().resource_mut::<HoverMap>().insert(PointerId::Mouse, [(l, HitData::new(camera, 0.0, None, None))].into_iter().collect());
    app.update();
    assert!(app.world().resource::<KitCapture>().pointer);
}

#[test]
fn an_open_list_is_rebuilt_when_the_option_texts_change() {
    let mut app = app();
    let d = open_dropdown(&mut app, &["A", "B"]);
    kit_click(app.world_mut(), d);
    app.update();
    app.world_mut().get_mut::<Dropdown>(d).unwrap().options = vec!["X".into(), "Y".into()];
    app.update();
    app.update();
    let row = popup_rows(&mut app)[0].1;
    let kid = app.world().get::<Children>(row).unwrap().iter().find_map(|k| app.world().get::<SpriteText>(k)).unwrap();
    assert_eq!(kid.text, "X");
}

#[test]
fn a_locked_hud_style_label_still_dims() {
    // Labels that are plain `Text` (the HUD's command cells) are dimmed by the shared painter.
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let cell = app.world_mut().spawn((WidgetState::Disabled, BackgroundColor(Color::NONE), Node::default())).id();
    let lab = app.world_mut().spawn((crate::ui::theme::ButtonText, Text::new("X"), TextColor(theme.text_on), ChildOf(cell))).id();
    app.update();
    assert_eq!(app.world().get::<TextColor>(lab).unwrap().0, theme.text_off);
}
