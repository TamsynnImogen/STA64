//! The pause box (UI-48): shown while the player has paused a single-player match
//! ([`PlayerPause`]) and the in-game menu is closed. Laid out from the race's GUI cfg:
//! the `pauseGame` panel area on the cfg's canvas, the text label and the unpause
//! button inside it, with their colour, sprite and string keys. A key the cfg leaves
//! out takes A2 stock's `gui_glob16x12.cfg` value. Rule (the box and its Continue
//! button resume the game) as the stock games show it; nothing measured.

use bevy::prelude::*;

use super::hud::HudConfig;
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
    mut shown: Local<Option<PauseLayout>>,
) {
    let want = paused.is_some_and(|p| p.0) && !menu.is_some_and(|m| m.open);
    let layout = want.then(|| PauseLayout::from_cfg(hud.as_deref().and_then(|h| h.gui.as_ref())));
    if *shown == layout && !theme.is_changed() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    shown.clone_from(&layout);
    let Some(l) = layout else { return };
    let s = strings.as_deref();
    let [px, py, pw, ph] = l.panel;
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
            Node { position_type: PositionType::Absolute, left: pct(px, l.canvas.0), top: pct(py, l.canvas.1), width: pct(pw, l.canvas.0), height: pct(ph, l.canvas.1), ..default() },
            BackgroundColor(fill),
            GlobalZIndex(930),
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
                    e.insert(super::notices::Tooltip::new(text_of(s, tip, "")));
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
}
