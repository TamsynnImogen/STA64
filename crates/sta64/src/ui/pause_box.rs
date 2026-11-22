//! The pause box (UI-48): shown while the player has paused a single-player match
//! ([`PlayerPause`]) and the in-game menu is closed. Laid out from the race's GUI cfg:
//! the `pauseGame` panel area on the cfg's canvas, the text label and the unpause
//! button inside it, with their colour, sprite and string keys. A key the cfg leaves
//! out takes A2 stock's `gui_glob16x12.cfg` value. Rule (the box and its Continue
//! button resume the game) as the stock games show it; nothing measured.

use bevy::prelude::*;

/// Global z of the pause box.
pub const PAUSE_BOX_Z: i32 = 930;

use super::hud::HudConfig;
use super::hud::stock::layout::HudFit;
use super::objectives_panel::panel_fractions;
use super::profile::UiProfile;
use super::i18n::Strings;
use super::ingame_menu::{InGameMenu, PlayerPause};
use super::sprite_font::{SpriteTextStyle, sprite_text_styled};
use super::theme::{ButtonClicked, Theme, UiButton, button, font_role};

#[derive(Component)]
pub(crate) struct PauseBoxRoot;

#[derive(Component)]
pub(crate) struct UnpauseButton;

/// The box's rectangles and keys, see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PauseLayout {
    pub canvas: (f32, f32),
    pub panel: [f32; 4],
    pub text: [f32; 4],
    pub button: [f32; 4],
    pub text_colour: Option<Color>,
    pub label: String,
    pub button_image: Option<String>,
    pub button_text: String,
    pub button_tooltip: Option<String>,
    /// `pauseGameBackgroundPanel`: the art's sprite name and its pieces (`_n`, frame n, at the
    /// piece's rectangle in the panel; `BackgroundPanelSize` of them). Empty where the cfg draws
    /// no background (Roots: size 0, the unpause button's image is the art).
    pub background: Option<String>,
    pub pieces: Vec<(usize, [f32; 4])>,
}

impl PauseLayout {
    pub(crate) fn from_cfg(cfg: Option<&sta_content::GuiLayout>) -> PauseLayout {
        let to = |r: &sta_content::gui_cfg::Rect| [r.x as f32, r.y as f32, r.w as f32, r.h as f32];
        let rect = |key: &str, stock: [f32; 4]| cfg.and_then(|c| c.rects.get(key)).filter(|r| r.w > 0 && r.h > 0).map_or(stock, to);
        let name = |key: &str| cfg.and_then(|c| c.names.get(key)).cloned();
        let pause_panel = cfg.and_then(|c| c.panels.iter().find(|(k, _)| k.eq_ignore_ascii_case("pauseGame")).map(|(_, p)| p));
        let canvas = cfg.map(|c| c.canvas).filter(|c| c.0 > 0 && c.1 > 0).map_or((1600.0, 1200.0), |c| (c.0 as f32, c.1 as f32));
        PauseLayout {
            canvas,
            // `pauseGamePanelArea` is a panel key: the cfg keeps it with the panel.
            panel: cfg
                .and_then(|c| c.panels.iter().find(|(k, _)| k.eq_ignore_ascii_case("pauseGame")).and_then(|(_, p)| p.area))
                .filter(|r| r.w > 0 && r.h > 0)
                .map_or([600.0, 400.0, 384.0, 192.0], |r| to(&r)),
            text: rect("pauseGameTextLabelArea", [78.0, 20.0, 250.0, 30.0]),
            button: rect("pauseGameUnpauseButtonArea", [104.0, 160.0, 176.0, 28.0]),
            text_colour: cfg.and_then(|c| c.colours.get("pauseGameTextColor")).map(|c| Color::srgba(c.r, c.g, c.b, c.a)),
            label: name("pauseGameTextLabel").unwrap_or_else(|| "GUI_POP_PAUSE_TEXT".to_owned()),
            button_image: name("pauseGameUnpauseButtonImage"),
            button_text: name("pauseGameUnpauseButtonText").unwrap_or_else(|| "GUI_POP_PAUSE_CONTINUE".to_owned()),
            button_tooltip: name("pauseGameUnpauseButtonTooltip"),
            background: pause_panel.and_then(|p| p.background.clone()),
            pieces: pause_panel
                .map(|p| {
                    let n = p.background_size.unwrap_or(0).max(0) as u32;
                    p.pieces.iter().filter(|(k, r)| **k < n && r.w > 0 && r.h > 0).map(|(k, r)| (*k as usize, to(r))).collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// What the box was placed on: redrawn when the window's aspect or the HUD fit changes under the stock look.
/// STA64's own HUD ignores both, so they are left out of its key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlaceKey {
    stock: bool,
    aspect_bits: u32,
    fit: Option<HudFit>,
}

impl PlaceKey {
    fn new(stock: bool, aspect: f32, fit: HudFit) -> PlaceKey {
        if stock { PlaceKey { stock, aspect_bits: aspect.to_bits(), fit: Some(fit) } } else { PlaceKey { stock, aspect_bits: 0, fit: None } }
    }
}

/// The box as fractions of the window (`x y w h`). Under the stock look it sits in the HUD rect the way the
/// objectives panel does (ADR 0008 H1/H2, [`panel_fractions`]): inside the sheet's centred area at the sheet's
/// position and size ([`HudFit::Centred`]), or against the whole window ([`HudFit::Stretched`]). STA64's own HUD
/// keeps the cfg canvas as fractions of the window.
pub(crate) fn box_fractions(l: &PauseLayout, stock: bool, aspect: f32, fit: HudFit) -> [f32; 4] {
    if stock {
        panel_fractions(l.canvas, l.panel, aspect, fit)
    } else {
        [l.panel[0] / l.canvas.0, l.panel[1] / l.canvas.1, l.panel[2] / l.canvas.0, l.panel[3] / l.canvas.1]
    }
}

/// A string-table entry, or `fallback` when the table has none.
fn text_of(strings: Option<&Strings>, key: &str, fallback: &str) -> String {
    strings.map(|s| s.game_text(key)).filter(|t| *t != key).unwrap_or(fallback).to_owned()
}

/// Show the box while paused and the menu is closed; rebuild it when the layout changes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    mut commands: Commands,
    theme: Res<Theme>,
    strings: Option<Res<Strings>>,
    hud: Option<Res<HudConfig>>,
    paused: Option<Res<PlayerPause>>,
    menu: Option<Res<InGameMenu>>,
    roots: Query<Entity, With<PauseBoxRoot>>,
    mut shown: Local<Option<(PauseLayout, PlaceKey)>>,
    profile: Option<Res<UiProfile>>,
    window: Option<Single<&Window, With<bevy::window::PrimaryWindow>>>,
) {
    let want = paused.is_some_and(|p| p.0) && !menu.is_some_and(|m| m.open);
    let layout = want.then(|| PauseLayout::from_cfg(hud.as_deref().and_then(|h| h.gui.as_ref())));
    let stock = profile.as_deref().is_some_and(|p| p.is_stock());
    let fit = hud.as_deref().map_or_else(Default::default, |h| h.fit);
    let aspect = window.as_ref().filter(|w| w.height() > 0.0).map(|w| w.width() / w.height()).or_else(|| hud.as_deref().map(|h| h.aspect)).unwrap_or(4.0 / 3.0);
    let key = PlaceKey::new(stock, aspect, fit);
    let current = layout.clone().map(|l| (l, key));
    if *shown == current && !theme.is_changed() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    shown.clone_from(&current);
    let Some(l) = layout else { return };
    let s = strings.as_deref();
    let [_, _, pw, ph] = l.panel;
    let frac = box_fractions(&l, stock, aspect, fit);
    let pct = |v: f32, of: f32| Val::Percent(v / of * 100.0);
    let at = |r: [f32; 4]| Node { position_type: PositionType::Absolute, left: pct(r[0], pw), top: pct(r[1], ph), width: pct(r[2], pw), height: pct(r[3], ph), ..default() };
    // The cfg's art replaces the flat panel: background pieces, or (Roots) an unpause button
    // whose image fills the box.
    let has_art = !l.pieces.is_empty() || l.button_image.is_some();
    let fill = if has_art { Color::NONE } else { theme.panel.with_alpha(0.92) };
    let label = text_of(s, &l.label, "Game paused");
    let continue_text = text_of(s, &l.button_text, "Continue");
    commands
        .spawn((
            PauseBoxRoot,
            Node { position_type: PositionType::Absolute, left: Val::Percent(frac[0] * 100.0), top: Val::Percent(frac[1] * 100.0), width: Val::Percent(frac[2] * 100.0), height: Val::Percent(frac[3] * 100.0), ..default() },
            BackgroundColor(fill),
            GlobalZIndex(PAUSE_BOX_Z),
        ))
        .with_children(|p| {
            if let Some(name) = &l.background {
                for (frame, r) in &l.pieces {
                    p.spawn((at(*r), Pickable::IGNORE, super::gui_sprites::GuiImage::new(name.clone()).with_frame(*frame)));
                }
            }
            // The button first, so the label is drawn over a button that fills the box (Roots).
            p.spawn(at(l.button)).with_children(|b| {
                let mut e = b.spawn((UnpauseButton, button(&theme, &continue_text, l.button[2])));
                if let Some(img) = &l.button_image {
                    // The authored image at the authored size, not the theme's pill.
                    e.insert((
                        super::gui_sprites::GuiButton { names: vec![img.clone()], keep_label: true },
                        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::End, ..default() },
                        BackgroundColor(Color::NONE),
                    ));
                }
                if let Some(tip) = &l.button_tooltip {
                    e.insert(super::notices::Tooltip::new(tip.clone()));
                }
            });
            p.spawn((at(l.text), Pickable::IGNORE, children![sprite_text_styled(label, theme.heading_size, l.text_colour.unwrap_or(theme.text), SpriteTextStyle::role(font_role(&theme, theme.heading_size)).centered().wrap_to_parent())]));
        });
}

/// Continue (or Enter) resumes the match.
pub(crate) fn unpause(
    mut clicks: MessageReader<ButtonClicked>,
    buttons: Query<(), (With<UnpauseButton>, With<UiButton>)>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut paused: ResMut<PlayerPause>,
) {
    let clicked = clicks.read().any(|c| buttons.contains(c.0));
    if paused.0 && (clicked || keys.is_some_and(|k| k.just_pressed(KeyCode::Enter))) {
        paused.0 = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_takes_its_rectangles_and_strings_from_the_cfg() {
        let stock = PauseLayout::from_cfg(None);
        assert_eq!((stock.panel, stock.button, stock.label.as_str()), ([600.0, 400.0, 384.0, 192.0], [104.0, 160.0, 176.0, 28.0], "GUI_POP_PAUSE_TEXT"));
        let text = "screenWidth = 800\nscreenHeight = 600\npauseGamePanelArea = 10 20 300 100\npauseGameTextLabelArea = 5 5 200 20\npauseGameTextColor = 0 1 0\npauseGameTextLabel = \"MY_PAUSE\"\npauseGameUnpauseButtonArea = 0 0 0 0\npauseGameUnpauseButtonImage = \"go\"\n";
        let (cfg, warnings) = sta_content::GuiCfg::parse("p.cfg", text);
        assert!(warnings.is_empty(), "{warnings:?}");
        let l = PauseLayout::from_cfg(Some(&cfg.layout_auto()));
        assert_eq!((l.canvas, l.panel, l.text), ((800.0, 600.0), [10.0, 20.0, 300.0, 100.0], [5.0, 5.0, 200.0, 20.0]));
        assert_eq!(l.button, stock.button, "a zero rectangle keeps the stock place");
        assert_eq!((l.label.as_str(), l.button_image.as_deref()), ("MY_PAUSE", Some("go")));
        assert_eq!(l.text_colour, Some(Color::srgba(0.0, 1.0, 0.0, 1.0)));
        assert!(l.pieces.is_empty() && l.background.is_none());
    }

    #[test]
    fn the_background_pieces_come_from_the_cfg() {
        let text = "screenWidth = 1600\nscreenHeight = 1200\npauseGamePanelArea = 600 400 384 192\npauseGameBackgroundPanel = \"art\"\npauseGameBackgroundPanelArea = 0 0 384 192\npauseGameBackgroundPanelSize = 2\npauseGameBackgroundPanel_0 = 0 0 256 192\npauseGameBackgroundPanel_1 = 256 0 128 192\n";
        let (cfg, _) = sta_content::GuiCfg::parse("p.cfg", text);
        let l = PauseLayout::from_cfg(Some(&cfg.layout_auto()));
        assert_eq!(l.background.as_deref(), Some("art"));
        assert_eq!(l.pieces, vec![(0, [0.0, 0.0, 256.0, 192.0]), (1, [256.0, 0.0, 128.0, 192.0])]);
        // Size 0 (Roots): no pieces even where some are written.
        let (cfg, _) = sta_content::GuiCfg::parse("p.cfg", &text.replace("PanelSize = 2", "PanelSize = 0"));
        assert!(PauseLayout::from_cfg(Some(&cfg.layout_auto())).pieces.is_empty());
    }

    // Authored sheets with invented numbers: a 4:3 sheet and a wider one, each with the pause panel at the
    // right and low in the canvas.
    const SHEETS: [(&str, &str); 2] = [
        ("narrow", "screenWidth = 1280\nscreenHeight = 960\npauseGamePanelArea = 700 300 360 180\n"),
        ("wide", "screenWidth = 1920\nscreenHeight = 1080\npauseGamePanelArea = 1300 500 400 200\n"),
    ];
    const ASPECTS: [f32; 4] = [4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0, 32.0 / 9.0];

    fn sheet(text: &str) -> PauseLayout {
        let (cfg, warnings) = sta_content::GuiCfg::parse("p.cfg", text);
        assert!(warnings.is_empty(), "{warnings:?}");
        PauseLayout::from_cfg(Some(&cfg.layout_auto()))
    }

    /// The HUD rect of a sheet on a window of `aspect`, as fractions of the window.
    fn hud_rect(l: &PauseLayout, aspect: f32) -> [f32; 4] {
        let screen = Vec2::new(aspect * 1000.0, 1000.0);
        let r = crate::ui::hud::stock::layout::fit_canvas(Vec2::new(l.canvas.0, l.canvas.1), screen).1;
        [r.x / screen.x, r.y / screen.y, r.w / screen.x, r.h / screen.y]
    }

    #[test]
    fn centred_the_box_keeps_its_sheet_position_and_size_inside_the_hud_rect() {
        for (name, text) in SHEETS {
            let l = sheet(text);
            for aspect in ASPECTS {
                let f = box_fractions(&l, true, aspect, HudFit::Centred);
                let h = hud_rect(&l, aspect);
                let eps = 2e-3;
                // Inside the HUD rect.
                assert!(f[0] >= h[0] - eps && f[1] >= h[1] - eps, "{name} {aspect}: {f:?} in {h:?}");
                assert!(f[0] + f[2] <= h[0] + h[2] + eps && f[1] + f[3] <= h[1] + h[3] + eps, "{name} {aspect}: {f:?} in {h:?}");
                // Sheet-relative position and size: the same fractions of the HUD rect as of the canvas.
                let want = [l.panel[0] / l.canvas.0, l.panel[1] / l.canvas.1, l.panel[2] / l.canvas.0, l.panel[3] / l.canvas.1];
                let got = [(f[0] - h[0]) / h[2], (f[1] - h[1]) / h[3], f[2] / h[2], f[3] / h[3]];
                for (g, w) in got.iter().zip(want) {
                    assert!((g - w).abs() < eps, "{name} {aspect}: {got:?} vs {want:?}");
                }
                // One scale for both axes: the box keeps the sheet's own aspect on the window.
                let real = (f[2] * aspect) / f[3];
                assert!((real - l.panel[2] / l.panel[3]).abs() < 0.01, "{name} {aspect}: aspect {real}");
            }
        }
    }

    #[test]
    fn stretched_and_the_non_stock_hud_keep_the_percentages_of_the_window() {
        for (name, text) in SHEETS {
            let l = sheet(text);
            let plain = [l.panel[0] / l.canvas.0, l.panel[1] / l.canvas.1, l.panel[2] / l.canvas.0, l.panel[3] / l.canvas.1];
            for aspect in ASPECTS {
                for (stock, fit) in [(true, HudFit::Stretched), (false, HudFit::Stretched), (false, HudFit::Centred)] {
                    let f = box_fractions(&l, stock, aspect, fit);
                    for (g, w) in f.iter().zip(plain) {
                        assert!((g - w).abs() < 1e-5, "{name} {aspect} stock {stock} {fit:?}: {f:?} vs {plain:?}");
                    }
                }
            }
        }
    }

    /// The spawned root's rectangle in percent of the window.
    fn root_rect(app: &mut App) -> Option<[f32; 4]> {
        let n = app.world_mut().query_filtered::<&Node, With<PauseBoxRoot>>().iter(app.world()).next()?;
        let p = |v: Val| match v {
            Val::Percent(p) => p / 100.0,
            other => panic!("not a percentage: {other:?}"),
        };
        Some([p(n.left), p(n.top), p(n.width), p(n.height)])
    }

    fn app_on(text: &str, profile: UiProfile, width: u32, height: u32) -> (App, Entity) {
        let (cfg, _) = sta_content::GuiCfg::parse("p.cfg", text);
        let mut app = App::new();
        app.insert_resource(Theme::default())
            .insert_resource(PlayerPause(true))
            .insert_resource(profile)
            .insert_resource(HudConfig { gui: Some(cfg.layout_auto()), ..default() })
            .add_systems(Update, draw);
        let window = app
            .world_mut()
            .spawn((Window { resolution: bevy::window::WindowResolution::new(width, height), ..default() }, bevy::window::PrimaryWindow))
            .id();
        (app, window)
    }

    fn resize(app: &mut App, window: Entity, width: u32, height: u32) {
        app.world_mut().get_mut::<Window>(window).unwrap().resolution = bevy::window::WindowResolution::new(width, height);
    }

    #[test]
    fn the_drawn_box_carries_the_computed_rect_and_follows_the_window_aspect_and_the_fit() {
        let text = SHEETS[1].1;
        let l = sheet(text);
        let (mut app, window) = app_on(text, UiProfile::Armada2, 1920, 1080);
        app.update();
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(near(root_rect(&mut app).unwrap(), box_fractions(&l, true, 16.0 / 9.0, HudFit::Centred)));
        resize(&mut app, window, 3200, 1000);
        app.update();
        let wide = root_rect(&mut app).unwrap();
        assert!(near(wide, box_fractions(&l, true, 3.2, HudFit::Centred)), "{wide:?}");
        assert_eq!(app.world_mut().query_filtered::<(), With<PauseBoxRoot>>().iter(app.world()).count(), 1);
        app.world_mut().resource_mut::<HudConfig>().fit = HudFit::Stretched;
        app.update();
        let stretched = root_rect(&mut app).unwrap();
        assert!(near(stretched, box_fractions(&l, true, 3.2, HudFit::Stretched)) && !near(stretched, wide), "{stretched:?}");
    }

    #[test]
    fn the_non_stock_hud_draws_the_box_as_percentages_of_the_window() {
        let text = SHEETS[0].1;
        let l = sheet(text);
        let (mut app, window) = app_on(text, UiProfile::Lcars, 1600, 1000);
        app.update();
        let plain = box_fractions(&l, false, 1.6, HudFit::Centred);
        assert_eq!(root_rect(&mut app), Some(plain));
        resize(&mut app, window, 3200, 1000);
        app.update();
        assert_eq!(root_rect(&mut app), Some(plain));
    }
}
