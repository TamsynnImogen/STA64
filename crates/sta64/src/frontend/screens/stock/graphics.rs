//! Graphics options shared by the front-end and the in-game options panel.
//! The values in `GraphicsInfo` describe the active renderer; only controls
//! backed by `Settings` are editable.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;

#[cfg(not(target_os = "android"))]
use crate::frontend::screens::WindowSize;
use crate::frontend::screens::view::{self, SettingSlider, SettingToggle};
use crate::frontend::screens::{SettingKey, Settings};
use crate::ui::i18n::{tr, trf};
use crate::ui::theme::{Theme, checkbox, dropdown, slider_focusable};
use crate::ui::sprite_font::{FontRole, SpriteTextStyle, sprite_text};

/// Live renderer information supplied by the options screen.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GraphicsInfo {
    pub adapter: String,
    pub renderer: String,
    pub monitor: String,
    pub msaa: u8,
    pub quality_active: bool,
    pub post_active: bool,
    pub shadows: bool,
    pub bloom: bool,
    pub post_aa: Option<String>,
}

impl Default for GraphicsInfo {
    fn default() -> Self {
        Self {
            adapter: tr("Unknown display device"),
            renderer: tr("Unknown renderer"),
            monitor: tr("Current display"),
            msaa: 4,
            quality_active: false,
            post_active: false,
            shadows: true,
            bloom: false,
            post_aa: None,
        }
    }
}

/// Major regions, useful to inspect the same layout in both menu contexts.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsPart {
    Body,
    DisplayDevice,
    Resolution,
    AntiAliasing,
    Renderer,
    Monitor,
    Toggles,
    Detail,
    Gamma,
    Footer,
}

type Box_<'a> = RelatedSpawnerCommands<'a, ChildOf>;

fn caption(t: &Theme, text: &str) -> impl Bundle {
    (sprite_text(text, t.small_size, t.text_dim), SpriteTextStyle::role(FontRole::Small).wrap_to_parent(), Node { width: Val::Percent(100.0), min_width: Val::Px(0.0), ..default() })
}

fn value(t: &Theme, text: &str) -> impl Bundle {
    (sprite_text(text, t.small_size, t.text), SpriteTextStyle::role(FontRole::Small).wrap_to_parent(), Node { width: Val::Percent(100.0), min_width: Val::Px(0.0), ..default() })
}

fn column(gap: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(gap),
        flex_grow: 1.0,
        flex_basis: Val::Px(0.0),
        min_width: Val::Px(0.0),
        ..default()
    }
}

fn info_row(c: &mut Box_, t: &Theme, part: GraphicsPart, name: &str, shown: &str) {
    c.spawn((
        part,
        Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            min_height: Val::Px(t.list_row_h),
            align_items: AlignItems::Center,
            column_gap: Val::Px(t.gap),
            padding: UiRect::horizontal(Val::Px(t.pad)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(t.track),
        BorderColor::all(t.accent2),
    ))
    .with_children(|r| {
        r.spawn((
            Node {
                width: Val::Percent(28.0),
                min_width: Val::Px(0.0),
                ..default()
            },
            children![caption(t, name)],
        ));
        r.spawn((
            Node {
                flex_grow: 1.0,
                min_width: Val::Px(0.0),
                ..default()
            },
            children![value(t, shown)],
        ));
    });
}

/// Populate a Graphics tab. The caller owns its tabs and outer panel.
pub(super) fn body(c: &mut Box_, t: &Theme, s: &Settings, info: &GraphicsInfo, width: f32) {
    let half = ((width - t.gap).max(0.0) / 2.0).max(1.0);
    let wide = width >= 900.0;
    #[cfg(not(target_os = "android"))]
    let selector_w = if wide {
        ((width - 2.0 * t.gap) / 4.0).max(1.0)
    } else {
        half
    };
    #[cfg(target_os = "android")]
    let selector_w = half;
    #[cfg(not(target_os = "android"))]
    let device_w = (width - 2.0 * t.gap - 2.0 * selector_w).max(1.0);
    #[cfg(target_os = "android")]
    let device_w = (width - t.gap - selector_w).max(1.0);

    c.spawn((
        GraphicsPart::Body,
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(if wide { (width / 70.0).clamp(t.gap, 24.0) } else { t.gap * 0.5 }),
            ..default()
        },
    ))
    .with_children(|c| {
        if !wide {
            info_row(c, t, GraphicsPart::DisplayDevice, &tr("Display device"), &info.adapter);
        }
        c.spawn(Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            column_gap: Val::Px(t.gap),
            ..default()
        })
        .with_children(|r| {
            if wide {
                r.spawn((
                    GraphicsPart::DisplayDevice,
                    Node { width: Val::Px(device_w), min_width: Val::Px(0.0), flex_direction: FlexDirection::Column, ..default() },
                ))
                .with_children(|f| {
                    f.spawn(caption(t, &tr("Display device")));
                    f.spawn((
                        Node { min_height: Val::Px(t.button_h), align_items: AlignItems::Center, min_width: Val::Px(0.0), padding: UiRect::horizontal(Val::Px(t.pad)), border: UiRect::all(Val::Px(1.0)), ..default() },
                        BackgroundColor(t.track),
                        BorderColor::all(t.accent2),
                        children![value(t, &info.adapter)],
                    ));
                });
            }
            #[cfg(not(target_os = "android"))]
            r.spawn((
                GraphicsPart::Resolution,
                Node {
                    width: Val::Px(selector_w),
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ))
            .with_children(|f| {
                f.spawn(caption(t, &tr("Resolution")));
                let sizes = WindowSize::ALL.map(|size| tr(size.label()));
                let size_refs: Vec<_> = sizes.iter().map(String::as_str).collect();
                let picked = WindowSize::ALL
                    .iter()
                    .position(|&size| size == s.window_size)
                    .unwrap_or(0);
                f.spawn((dropdown(t, &size_refs, picked, selector_w), view::WindowSizeSelect));
            });
            r.spawn((
                GraphicsPart::AntiAliasing,
                Node {
                    width: Val::Px(selector_w),
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ))
            .with_children(|f| {
                f.spawn(caption(t, &tr("Anti-aliasing")));
                let picked = match s.msaa {
                    None => 0,
                    Some(1) => 1,
                    Some(2) => 2,
                    Some(4) => 3,
                    _ => 0,
                };
                let choices = [tr("Automatic"), tr("Off"), tr("MSAA 2x"), tr("MSAA 4x")];
                let refs: Vec<_> = choices.iter().map(String::as_str).collect();
                f.spawn((dropdown(t, &refs, picked, selector_w), view::MsaaSelect));
            });
        });

        c.spawn(Node { width: Val::Px(selector_w), min_width: Val::Px(0.0), flex_direction: FlexDirection::Column, ..default() })
            .with_children(|f| {
                f.spawn(caption(t, &tr("HUD layout")));
                let fits = crate::ui::hud::stock::layout::HudFit::ALL.map(|fit| tr(fit.label()));
                let refs: Vec<_> = fits.iter().map(String::as_str).collect();
                let picked = crate::ui::hud::stock::layout::HudFit::ALL.iter().position(|&fit| fit == s.hud_fit).unwrap_or(0);
                f.spawn((dropdown(t, &refs, picked, selector_w), view::HudFitSelect));
            });
        #[cfg(not(target_os = "android"))]
        c.spawn(caption(t, &tr("Windowed resolution; fullscreen uses the current monitor.")));
        if let Some(post_aa) = &info.post_aa {
            c.spawn(caption(t, &trf("{0} post-processing AA overrides MSAA.", &[post_aa])));
        }
        info_row(c, t, GraphicsPart::Renderer, &tr("Renderer"), &info.renderer);
        info_row(c, t, GraphicsPart::Monitor, &tr("Game monitor"), &info.monitor);

        c.spawn((
            GraphicsPart::Toggles,
            Node {
                width: Val::Percent(100.0),
                min_width: Val::Px(0.0),
                column_gap: Val::Px(t.gap),
                ..default()
            },
        ))
        .with_children(|r| {
            r.spawn(column(if wide { t.gap } else { 0.0 })).with_children(|col| {
                #[cfg(not(target_os = "android"))]
                col.spawn((
                    checkbox(t, &tr("Windowed mode"), !s.fullscreen),
                    view::WindowedMode,
                ));
                col.spawn((
                    checkbox(t, &tr("Vsync"), s.vsync),
                    SettingToggle(SettingKey::Vsync),
                ));
                col.spawn((
                    checkbox(t, &tr("Show FPS"), s.show_fps),
                    SettingToggle(SettingKey::ShowFps),
                ));
            });
            r.spawn(column(if wide { t.gap } else { 0.0 })).with_children(|col| {
                col.spawn((
                    checkbox(t, &tr("Shadows"), if info.quality_active { info.shadows } else { s.shadows.unwrap_or(info.shadows) }),
                    SettingToggle(SettingKey::Shadows),
                ));
                col.spawn((
                    checkbox(t, &tr("Bloom"), if info.post_active { info.bloom } else { s.bloom.unwrap_or(info.bloom) }),
                    SettingToggle(SettingKey::Bloom),
                ));
                col.spawn((
                    checkbox(t, &tr("LCARS interface"), s.lcars_ui),
                    SettingToggle(SettingKey::LcarsUi),
                ));
            });
        });

        c.spawn(Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            column_gap: Val::Px(t.gap),
            ..default()
        })
        .with_children(|r| {
            r.spawn((
                GraphicsPart::Detail,
                Node {
                    width: Val::Px(half),
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ))
            .with_children(|col| {
                col.spawn(caption(t, &tr("Graphics detail (nebula)")));
                col.spawn((
                    slider_focusable(
                        t,
                        half,
                        0.0,
                        2.0,
                        f32::from(s.graphics_detail.unwrap_or(1)),
                        1.0,
                    ),
                    SettingSlider(SettingKey::GraphicsDetail),
                ));
                col.spawn(caption(t, &tr("Low / Default / High")));
            });
            r.spawn((
                GraphicsPart::Gamma,
                Node {
                    width: Val::Px(half),
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ))
            .with_children(|col| {
                col.spawn(caption(t, &tr("Gamma correction")));
                col.spawn(value(t, &tr("Uses system display settings")));
            });
        });
        c.spawn((
            GraphicsPart::Footer,
            caption(t, &tr("Changes save and apply automatically.")),
        ));
    });
}
