//! The widget gallery (`sta64 --ui-gallery`): one of every widget, in the active UI
//! profile; the tabs at the top switch between LCARS and the stock skins (UI-17).

use bevy::ecs::relationship::RelatedSpawner;
use bevy::ecs::spawn::SpawnWith;
use bevy::prelude::*;

use super::{Theme, ThemePlugin, stock, widgets::*};
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::{FontRole, SpriteTextStyle, sprite_text_styled};

#[derive(Component)]
pub struct GalleryRoot;

/// The camera the gallery spawned (its clear colour follows the profile).
#[derive(Component)]
pub struct GalleryCamera;

/// The tab bar that picks the profile.
#[derive(Component)]
pub struct GalleryProfileBar;

/// The profile the gallery on screen was built for.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
struct GalleryBuilt(UiProfile);

const PROFILES: [UiProfile; 3] = [UiProfile::Lcars, UiProfile::Armada2, UiProfile::FleetOps];

pub struct GalleryPlugin;

impl Plugin for GalleryPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        app.init_resource::<UiProfile>()
            .add_systems(Startup, spawn_gallery)
            .add_systems(Update, (switch_profile, rebuild).chain().in_set(crate::app::GameSet::Ui));
    }
}

fn index_of(profile: UiProfile) -> usize {
    PROFILES.iter().position(|p| *p == profile).unwrap_or(0)
}

fn column(t: &Theme) -> Node {
    Node { width: Val::Px(340.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(t.gap * 1.5), ..default() }
}

fn font_sample(t: &Theme, role: FontRole, s: &str) -> impl Bundle {
    (Node::default(), children![sprite_text_styled(s, t.text_size, t.text, SpriteTextStyle::role(role))])
}

/// Tabs with their panels, and the sprite font samples.
fn far_column(t: Theme) -> impl Bundle {
    (
        column(&t),
        Children::spawn(SpawnWith(move |p: &mut RelatedSpawner<ChildOf>| {
            let t = &t;
            p.spawn(label(t, "Tabs"));
            let bar = p.spawn(stock::tab_bar(t, &["Orders", "Fleet", "Notes"], 0)).id();
            for (i, s) in ["Orders: only the selected panel shows.", "Fleet: the tab bar answers Left and Right.", "Notes: Tab moves between widgets."].into_iter().enumerate() {
                p.spawn(stock::tab_panel(bar, i)).with_child(label(t, s));
            }
            p.spawn(label(t, "Text in the mod's sprite font"));
            p.spawn(font_sample(t, FontRole::Small, "FontSmall: the quick brown fox"));
            p.spawn(font_sample(t, FontRole::Medium, "FontMedium: the quick brown fox"));
            p.spawn(font_sample(t, FontRole::Large, "FontLarge: the quick brown fox"));
            p.spawn((
                Node { width: Val::Px(300.0), ..default() },
                children![sprite_text_styled(
                    "A wrapped, centred paragraph is laid out against the width of its box.",
                    t.text_size,
                    t.text,
                    SpriteTextStyle::role(FontRole::Medium).centered().wrapped(300.0),
                )],
            ));
        })),
    )
}

fn build(commands: &mut Commands, t: &Theme, profile: UiProfile) {
    commands.insert_resource(GalleryBuilt(profile));
    let left = (
        column(t),
        children![
            label(t, "Buttons"),
            (
                Node { column_gap: Val::Px(t.gap), ..default() },
                children![button(t, "Engage", 160.0), disabled_button(t, "Disabled", 160.0)],
            ),
            label(t, "Toggles"),
            (
                Node { column_gap: Val::Px(t.gap), ..default() },
                children![toggle(t, "On", true, 160.0), toggle(t, "Off", false, 160.0)],
            ),
            label(t, "Check boxes"),
            stock::checkbox(t, "Show tooltips", true),
            stock::checkbox(t, "Pause on focus loss", false),
            label(t, "Sliders"),
            stock::slider_focusable(t, 300.0, 0.0, 100.0, 40.0, 5.0),
            progress_bar(t, 300.0, 0.65),
            label(t, "Text field"),
            stock::text_field(t, "Cedric", "Player name", 300.0, 16, stock::TextFilter::Name),
        ],
    );
    let right = (
        column(t),
        children![
            label(t, "Dropdown"),
            stock::dropdown(t, &["Federation", "Klingon", "Romulan", "Cardassian", "Borg", "Species 8472", "Dominion"], 0, 300.0),
            label(t, "Scrolling list"),
            stock::scroll_list(
                t,
                &["Alpha Centauri", "Beta Hydri", "Cestus III", "Deep Space 9", "Epsilon Eridani", "Fabrina", "Gamma Hydra", "Hoth"],
                Some(1),
                5
            ),
        ],
    );
    let theme = t.clone();
    let content = (
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(t.gap * 2.0), ..default() },
        Children::spawn(SpawnWith(move |p: &mut RelatedSpawner<ChildOf>| {
            let t = &theme;
            p.spawn(label(t, "UI profile"));
            p.spawn((GalleryProfileBar, stock::tab_bar(t, &["LCARS", "Armada II", "Fleet Operations"], index_of(profile))));
            p.spawn((
                Node { column_gap: Val::Px(t.gap * 4.0), ..default() },
                Children::spawn(SpawnWith(move |p: &mut RelatedSpawner<ChildOf>| {
                    p.spawn(left);
                    p.spawn(right);
                    p.spawn(far_column(theme.clone()));
                })),
            ));
        })),
    );
    commands.spawn((
        GalleryRoot,
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), padding: UiRect::all(Val::Px(24.0)), ..default() },
        children![panel(t, "Widget gallery", content)],
    ));
}

pub fn spawn_gallery(mut commands: Commands, theme: Res<Theme>, profile: Res<UiProfile>, camera: Query<(), With<Camera2d>>) {
    if camera.is_empty() {
        commands.spawn((GalleryCamera, Camera2d, Camera { clear_color: ClearColorConfig::Custom(theme.panel), ..default() }));
    }
    build(&mut commands, &theme, *profile);
}

/// A tab press on the profile bar sets the profile (and with it the theme).
fn switch_profile(
    mut changes: MessageReader<stock::TabChanged>,
    bars: Query<(), With<GalleryProfileBar>>,
    mut profile: ResMut<UiProfile>,
    mut theme: ResMut<Theme>,
) {
    for c in changes.read() {
        if bars.contains(c.bar)
            && let Some(p) = PROFILES.get(c.index).copied()
            && *profile != p
        {
            *profile = p;
            *theme = p.theme();
        }
    }
}

/// Rebuild the gallery when the profile changed since it was spawned.
fn rebuild(
    mut commands: Commands,
    built: Option<Res<GalleryBuilt>>,
    profile: Res<UiProfile>,
    roots: Query<Entity, With<GalleryRoot>>,
    theme: Res<Theme>,
    mut cameras: Query<&mut Camera, With<GalleryCamera>>,
) {
    if built.is_none_or(|b| b.0 == *profile) {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    for mut c in &mut cameras {
        c.clear_color = ClearColorConfig::Custom(theme.panel);
    }
    build(&mut commands, &theme, *profile);
}
