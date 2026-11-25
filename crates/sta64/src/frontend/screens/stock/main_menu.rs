//! The front page's own typography and left-aligned navigation.
//! Its arrangement is authored for STA64; button art comes from the mounted game.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::Vec2;
use bevy::prelude::*;

use super::layout::{Slot, region};
use super::screens::{Placed, cap};
use crate::frontend::screens::view::Ctx;
use crate::frontend::screens::{Act, Screen};
use crate::ui::gui_sprites::{GuiButton, GuiButtonColors, GuiButtonLabel};
use crate::ui::sprite_font::{FontRole, ScalableText, SpriteText, SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::{Focusable, KitFocus, UiButton, WidgetState};

const WHITE: Color = Color::srgb(231.0 / 255.0, 236.0 / 255.0, 242.0 / 255.0);
const GOLD: Color = Color::srgb(244.0 / 255.0, 185.0 / 255.0, 67.0 / 255.0);
const DIM: Color = Color::srgb(0.47, 0.47, 0.46);
pub(super) const BUTTON_BACK: Color = Color::srgba(0.02, 0.03, 0.05, 0.84);
const ART_SHADE: Color = Color::srgba(0.01, 0.02, 0.04, 0.60);

#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct MainMenuButton;

#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct MainMenuLabel;

type Box_<'a> = RelatedSpawnerCommands<'a, ChildOf>;

fn at(p: &mut Box_<'_>, slot: Slot, b: Vec2, content: impl Bundle) {
    let r = region(slot).px(b);
    p.spawn((
        Placed(slot),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x),
            top: Val::Px(r.min.y),
            width: Val::Px(r.width()),
            height: Val::Px(r.height()),
            align_items: AlignItems::Center,
            ..default()
        },
        children![content],
    ));
}

fn caption(text: impl Into<String>, size: f32, color: Color, role: FontRole) -> impl Bundle {
    sprite_text_styled(text, size, color, SpriteTextStyle::role(role))
}

fn title<'a>(ctx: &'a Ctx<'_>) -> &'a str {
    ctx.active_mod.0.as_deref()
        .and_then(|folder| ctx.data.mods.iter().find(|m| m.folder.eq_ignore_ascii_case(folder)))
        .map(|m| m.name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or("STA64")
}

fn title_size(name: &str, scale: f32) -> f32 {
    // Reserve roughly two lines for long mod names inside the title's wide region.
    // Wrapping handles word boundaries; the size also keeps long single words legible.
    let chars = name.chars().count().max(1) as f32;
    (68.0_f32.min(680.0 * 2.0 / (chars * 0.6)).max(24.0)) * scale
}

fn button(p: &mut Box_<'_>, b: Vec2, slot: Slot, text: String, act: Option<Act>) {
    let r = region(slot).px(b);
    let size = 26.0 * b.y / 1080.0;
    let state = if act.is_some() { WidgetState::Normal } else { WidgetState::Disabled };
    let ink = if act.is_some() { WHITE } else { DIM };
    let mut entity = p.spawn((
        Placed(slot), MainMenuButton, UiButton, state, BackgroundColor(BUTTON_BACK),
        // Both the original A2 and Fleet Ops global sprite tables provide this
        // button. The GUI painter resolves it from the mounted install and tints
        // it for hover, press and disabled states.
        GuiButton { names: vec!["comm_generic_button".into()], keep_label: true },
        GuiButtonColors {
            hovered: Color::srgb(1.0, 0.91, 0.70),
            pressed: Color::srgb(0.76, 0.72, 0.62),
            disabled: Color::srgb(0.48, 0.48, 0.48),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x),
            top: Val::Px(r.min.y),
            width: Val::Px(r.width()),
            height: Val::Px(r.height()),
            align_items: AlignItems::Center,
            ..default()
        },
        children![
            (Pickable::IGNORE, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(ART_SHADE)),
            (MainMenuLabel, GuiButtonLabel, caption(text, size, ink, FontRole::Medium)),
        ],
    ));
    if let Some(act) = act {
        entity.insert((act, Focusable));
    }
}

/// Build only the front page. Other stock screens keep their existing widgets.
pub(super) fn spawn(p: &mut Box_<'_>, ctx: &Ctx, b: Vec2) {
    let scale = b.y / 1080.0;
    let name = title(ctx);
    at(
        p,
        Slot::MainTitle,
        b,
        (
            ScalableText,
            sprite_text_styled(
                name.to_owned(), title_size(name, scale), GOLD,
                SpriteTextStyle::role(FontRole::VeryLarge).wrapped(region(Slot::MainTitle).px(b).width()),
            ),
        ),
    );
    let r = region(Slot::MainDivider).px(b);
    at(p, Slot::MainDivider, b, (Node { width: Val::Px(r.width()), height: Val::Px(r.height()), ..default() }, BackgroundColor(GOLD)));

    button(p, b, Slot::MainSinglePlayer, cap(ctx, "single_player", "Single player").to_uppercase(), Some(Act::Goto(Screen::SinglePlayer)));
    button(p, b, Slot::MainInstantAction, cap(ctx, "instant_action", "Instant action").to_uppercase(), Some(Act::Goto(Screen::Setup)));
    button(p, b, Slot::MainMultiplayer, cap(ctx, "multiplayer", "Multiplayer").to_uppercase(), Some(Act::Goto(Screen::Multiplayer)));
    button(p, b, Slot::MainMapEditor, cap(ctx, "map_editor", "Map editor").to_uppercase(), Some(Act::PickEditorMap));
    button(p, b, Slot::MainMods, cap(ctx, "mod_manager", "Mod manager").to_uppercase(), Some(Act::PickMod));
    button(p, b, Slot::MainOptions, cap(ctx, "options", "Options").to_uppercase(), Some(Act::Goto(Screen::Settings)));
    button(p, b, Slot::MainReplayIntro, cap(ctx, "replay_intro", "Replay intro").to_uppercase(), None);
    button(p, b, Slot::MainCredits, cap(ctx, "credits", "Credits").to_uppercase(), None);
    button(p, b, Slot::MainExit, cap(ctx, "exit", "Exit").to_uppercase(), Some(Act::Exit));
}

type PaintedButton = (Entity, &'static WidgetState, &'static Children, &'static mut BackgroundColor, Has<ImageNode>);

/// Restore the dark fallback if the mounted install has no usable button art.
pub(crate) fn paint(
    focus: Res<KitFocus>,
    mut buttons: Query<PaintedButton, With<MainMenuButton>>,
    mut labels: Query<&mut SpriteText, With<MainMenuLabel>>,
) {
    for (entity, state, children, mut background, has_art) in &mut buttons {
        let wanted = if has_art { Color::NONE } else { BUTTON_BACK };
        if background.0 != wanted {
            background.0 = wanted;
        }
        let ink = match state {
            WidgetState::Disabled => DIM,
            WidgetState::Pressed => WHITE,
            WidgetState::Hovered => GOLD,
            WidgetState::Normal if focus.0 == Some(entity) => GOLD,
            WidgetState::Normal => WHITE,
        };
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child)
                && label.color != ink
            {
                label.color = ink;
            }
        }
    }
}
