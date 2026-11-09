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
fn button_states_change_colour() {
    let mut app = app();
    let theme = app.world().resource::<Theme>().clone();
    let b = first::<UiButton>(&mut app);
    assert_eq!(bg(&app, b), theme.orange);
    for (state, k) in [(WidgetState::Hovered, theme.hover_boost), (WidgetState::Pressed, theme.press_boost)] {
        *app.world_mut().get_mut::<WidgetState>(b).unwrap() = state;
        app.update();
        assert_eq!(bg(&app, b), scale(theme.orange, k));
    }
    *app.world_mut().get_mut::<WidgetState>(b).unwrap() = WidgetState::Normal;
    app.update();
    assert_eq!(bg(&app, b), theme.orange);
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
fn toggle_off_is_dimmer_than_on() {
    let mut app = app();
    app.update();
    let mut q = app.world_mut().query::<(Entity, &Toggle)>();
    let (on, off): (Vec<_>, Vec<_>) = q.iter(app.world()).map(|(e, t)| (e, t.on)).partition(|(_, on)| *on);
    let lum = |c: Color| c.to_srgba().red;
    assert!(lum(bg(&app, off[0].0)) < lum(bg(&app, on[0].0)));
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
