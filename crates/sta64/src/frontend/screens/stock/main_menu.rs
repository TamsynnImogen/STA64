//! The front page's own typography and transparent, left-aligned navigation.
//! Its arrangement is authored for STA64; the installed game's art is not copied.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::Vec2;
use bevy::prelude::*;

use super::layout::{Slot, region};
use super::screens::{Placed, cap};
use crate::frontend::screens::view::Ctx;
use crate::frontend::screens::{Act, Screen};
use crate::ui::sprite_font::{FontRole, ScalableText, SpriteText, SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::{Focusable, KitFocus, UiButton, WidgetState};

const WHITE: Color = Color::srgb(231.0 / 255.0, 236.0 / 255.0, 242.0 / 255.0);
const GOLD: Color = Color::srgb(244.0 / 255.0, 185.0 / 255.0, 67.0 / 255.0);
const DIM: Color = Color::srgb(0.47, 0.47, 0.46);

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

fn button(p: &mut Box_<'_>, b: Vec2, slot: Slot, text: String, act: Option<Act>) {
    let r = region(slot).px(b);
    let size = 26.0 * b.y / 1080.0;
    let state = if act.is_some() { WidgetState::Normal } else { WidgetState::Disabled };
    let ink = if act.is_some() { WHITE } else { DIM };
    let mut entity = p.spawn((
        Placed(slot), MainMenuButton, UiButton, state, BackgroundColor(Color::NONE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x),
            top: Val::Px(r.min.y),
            width: Val::Px(r.width()),
            height: Val::Px(r.height()),
            align_items: AlignItems::Center,
            ..default()
        },
        children![(MainMenuLabel, caption(text, size, ink, FontRole::Medium))],
    ));
    if let Some(act) = act {
        entity.insert((act, Focusable));
    }
}

/// Build only the front page. Other stock screens keep their existing widgets.
pub(super) fn spawn(p: &mut Box_<'_>, ctx: &Ctx, b: Vec2) {
    let scale = b.y / 1080.0;
    at(p, Slot::MainKicker, b, (ScalableText, caption("STAR TREK", 28.0 * scale, WHITE, FontRole::Medium)));
    at(
        p,
        Slot::MainTitle,
        b,
        (
            Node { align_items: AlignItems::FlexStart, ..default() },
            children![
                (ScalableText, caption("ARMADA", 68.0 * scale, GOLD, FontRole::VeryLarge)),
                (
                    Node { margin: UiRect::left(Val::Px(5.0 * scale)), ..default() },
                    children![(ScalableText, caption("64", 34.0 * scale, GOLD, FontRole::Large))]
                )
            ],
        ),
    );
    let r = region(Slot::MainDivider).px(b);
    at(p, Slot::MainDivider, b, (Node { width: Val::Px(r.width()), height: Val::Px(r.height()), ..default() }, BackgroundColor(GOLD)));

    button(p, b, Slot::MainSinglePlayer, cap(ctx, "single_player", "Single player").to_uppercase(), Some(Act::Goto(Screen::SinglePlayer)));
    button(p, b, Slot::MainInstantAction, cap(ctx, "instant_action", "Instant action").to_uppercase(), Some(Act::Goto(Screen::Setup)));
    button(p, b, Slot::MainMultiplayer, cap(ctx, "multiplayer", "Multiplayer").to_uppercase(), Some(Act::Goto(Screen::Multiplayer)));
    button(p, b, Slot::MainMapEditor, cap(ctx, "map_editor", "Map editor").to_uppercase(), None);
    button(p, b, Slot::MainMods, cap(ctx, "mod_manager", "Mod manager").to_uppercase(), Some(Act::PickMod));
    button(p, b, Slot::MainOptions, cap(ctx, "options", "Options").to_uppercase(), Some(Act::Goto(Screen::Settings)));
    button(p, b, Slot::MainReplayIntro, cap(ctx, "replay_intro", "Replay intro").to_uppercase(), None);
    button(p, b, Slot::MainCredits, cap(ctx, "credits", "Credits").to_uppercase(), None);
    button(p, b, Slot::MainExit, cap(ctx, "exit", "Exit").to_uppercase(), Some(Act::Exit));
}

/// Keep the menu text-only after the kit's general button painter runs.
pub(crate) fn paint(
    focus: Res<KitFocus>,
    mut buttons: Query<(Entity, &WidgetState, &Children, &mut BackgroundColor), With<MainMenuButton>>,
    mut labels: Query<&mut SpriteText, With<MainMenuLabel>>,
) {
    for (entity, state, children, mut background) in &mut buttons {
        if background.0 != Color::NONE {
            background.0 = Color::NONE;
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
