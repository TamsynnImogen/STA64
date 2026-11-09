//! The widget gallery (`sta64 --ui-gallery`): one of every widget.

use bevy::prelude::*;

use super::{Theme, ThemePlugin, widgets::*};

#[derive(Component)]
pub struct GalleryRoot;

pub struct GalleryPlugin;

impl Plugin for GalleryPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        app.add_systems(Startup, spawn_gallery);
    }
}

pub fn spawn_gallery(mut commands: Commands, theme: Res<Theme>, camera: Query<(), With<Camera2d>>) {
    let t = &*theme;
    if camera.is_empty() {
        commands.spawn((Camera2d, Camera { clear_color: ClearColorConfig::Custom(t.panel), ..default() }));
    }
    let content = (
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(t.gap * 2.0), ..default() },
        children![
            label(t, "Buttons"),
            (
                Node { column_gap: Val::Px(t.gap), ..default() },
                children![
                    button(t, "Engage", 160.0),
                    button(t, "Hold", 160.0),
                    button(t, "Release", 160.0),
                    disabled_button(t, "Disabled", 160.0),
                ],
            ),
            label(t, "Toggles"),
            (
                Node { column_gap: Val::Px(t.gap), ..default() },
                children![toggle(t, "On", true, 160.0), toggle(t, "Off", false, 160.0)],
            ),
            label(t, "Slider"),
            slider(t, 300.0, 0.0, 100.0, 40.0),
            label(t, "Progress"),
            progress_bar(t, 300.0, 0.65),
            label(t, "List"),
            list(t, &["Federation", "Klingon", "Romulan", "Borg"], Some(0)),
        ],
    );
    commands.spawn((
        GalleryRoot,
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), padding: UiRect::all(Val::Px(24.0)), ..default() },
        children![panel(t, "Widget gallery", content)],
    ));
}
