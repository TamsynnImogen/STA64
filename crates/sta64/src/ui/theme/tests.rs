//! Windowless: spawn the gallery in a bare `App`, assert every widget type is
//! there, and drive state transitions without a pointer.

use bevy::prelude::*;

use super::*;
use crate::app::configure_sets;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.add_plugins(GalleryPlugin);
    app.update();
    app
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query::<&C>().iter(app.world()).count()
}

fn first<C: Component>(app: &mut App) -> Entity {
    app.world_mut().query_filtered::<Entity, With<C>>().iter(app.world()).next().unwrap()
}

fn bg(app: &App, e: Entity) -> Color {
    app.world().get::<BackgroundColor>(e).unwrap().0
}

#[test]
fn gallery_has_every_widget_type() {
    let mut app = app();
    assert_eq!(count::<GalleryRoot>(&mut app), 1);
    assert_eq!(count::<Panel>(&mut app), 1);
    assert_eq!(count::<UiButton>(&mut app), 4);
    assert_eq!(count::<Toggle>(&mut app), 2);
    assert_eq!(count::<Slider>(&mut app), 1);
    assert_eq!(count::<ProgressBar>(&mut app), 1);
    assert_eq!(count::<List>(&mut app), 1);
    assert_eq!(count::<ListItem>(&mut app), 4);
    assert!(count::<UiLabel>(&mut app) >= 5);
}

#[test]
fn button_states_use_the_refit_roles() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let b = first::<UiButton>(&mut app);
    assert_eq!(bg(&app, b), theme.button);
    for (state, want) in [(WidgetState::Hovered, theme.button_hot), (WidgetState::Pressed, theme.button_down)] {
        *app.world_mut().get_mut::<WidgetState>(b).unwrap() = state;
        app.update();
        assert_eq!(bg(&app, b), want);
    }
    *app.world_mut().get_mut::<WidgetState>(b).unwrap() = WidgetState::Normal;
    app.update();
    assert_eq!(bg(&app, b), theme.button);
}

#[test]
fn disabled_button_is_dark_with_dim_text() {
    let mut app = app();
    app.update();
    let theme = app.world().resource::<Theme>().clone();
    let d = app.world_mut().query::<(Entity, &WidgetState)>().iter(app.world()).find(|(_, s)| **s == WidgetState::Disabled).unwrap().0;
    assert_eq!(bg(&app, d), theme.button_off);
    let kid = app.world().get::<Children>(d).unwrap()[0];
    assert_eq!(app.world().get::<TextColor>(kid).unwrap().0, theme.text_off);
    // An enabled pill's label is black.
    let b = first::<UiButton>(&mut app);
    let kid = app.world().get::<Children>(b).unwrap()[0];
    assert_eq!(app.world().get::<TextColor>(kid).unwrap().0, theme.text_on);
}

#[test]
fn toggle_on_is_gold_and_off_is_lavender() {
    let mut app = app();
    app.update();
    let theme = app.world().resource::<Theme>().clone();
    let mut q = app.world_mut().query::<(Entity, &Toggle)>();
    let (on, off): (Vec<_>, Vec<_>) = q.iter(app.world()).map(|(e, t)| (e, t.on)).partition(|(_, on)| *on);
    assert_eq!(bg(&app, on[0].0), theme.button_on);
    assert_eq!(bg(&app, off[0].0), theme.button);
}

#[test]
fn tokens_are_refits_lcars_form() {
    let t = Theme::default();
    assert_eq!(t.accent2, Color::srgb_u8(0x9c, 0x9c, 0xde));
    assert_eq!(t.accent, Color::srgb_u8(0xf4, 0xb8, 0x4c));
    assert_eq!((t.text_size, t.title_size, t.pad, t.gap, t.button_h, t.corners), (20.0, 26.0, 12.0, 8.0, 40.0, 30.0));
    assert_eq!(t.caps("ok", true), "OK");
}

#[test]
fn selected_list_row_has_the_gold_mark() {
    let mut app = app();
    app.update();
    let theme = app.world().resource::<Theme>().clone();
    let row = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 0).unwrap().0;
    assert_eq!(bg(&app, row), theme.row_sel);
    let mark = app.world().get::<Children>(row).unwrap()[0];
    assert_eq!(bg(&app, mark), theme.accent);
    let other = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 2).unwrap().0;
    let mark = app.world().get::<Children>(other).unwrap()[0];
    assert_eq!(bg(&app, mark), Color::NONE);
}

#[test]
fn disabled_button_ignores_clicks_and_pointer() {
    let mut app = app();
    let disabled = app
        .world_mut()
        .query::<(Entity, &WidgetState)>()
        .iter(app.world())
        .find(|(_, s)| **s == WidgetState::Disabled)
        .unwrap()
        .0;
    click(app.world_mut(), disabled);
    assert_eq!(app.world().resource::<Messages<ButtonClicked>>().len(), 0);
    assert_eq!(*app.world().get::<WidgetState>(disabled).unwrap(), WidgetState::Disabled);
}

#[test]
fn click_sends_messages_and_flips_toggle_and_list() {
    let mut app = app();
    let b = first::<UiButton>(&mut app);
    click(app.world_mut(), b);
    assert_eq!(app.world().resource::<Messages<ButtonClicked>>().len(), 1);

    let t = first::<Toggle>(&mut app);
    let before = app.world().get::<Toggle>(t).unwrap().on;
    click(app.world_mut(), t);
    assert_eq!(app.world().get::<Toggle>(t).unwrap().on, !before);

    let item = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 2).unwrap().0;
    click(app.world_mut(), item);
    let list = first::<List>(&mut app);
    assert_eq!(app.world().get::<List>(list).unwrap().selected, Some(2));
}

#[test]
fn progress_and_slider_fills_follow_value() {
    let mut app = app();
    let p = first::<ProgressBar>(&mut app);
    app.world_mut().get_mut::<ProgressBar>(p).unwrap().0 = 0.25;
    let s = first::<Slider>(&mut app);
    app.world_mut().get_mut::<Slider>(s).unwrap().value = 100.0;
    app.update();
    let width_of = |app: &App, e: Entity| {
        let kids = app.world().get::<Children>(e).unwrap();
        app.world().get::<Node>(kids[0]).unwrap().width
    };
    assert_eq!(width_of(&app, p), Val::Percent(25.0));
    assert_eq!(width_of(&app, s), Val::Percent(100.0));
}
