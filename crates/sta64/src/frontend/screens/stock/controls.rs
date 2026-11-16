//! UI-51: a text editor for the existing `action = Key, Key` bindings grammar.
//! The draft survives page and Options tab rebuilds. Closing Options discards
//! unapplied edits; reopening reads the current bindings (including external changes).

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;

use crate::app::AppState;
use crate::frontend::screens::Screen;
use crate::input::keys::{Action, KeyBindings};
use crate::ui::i18n::{tr, trf};
use crate::ui::theme::{
    ButtonClicked, TextField, TextFilter, Theme, kit_button, label, text_field,
};

const PAGE_SIZE: usize = 6;

#[derive(Resource, Debug, Clone)]
pub struct ControlsDraft {
    pub(crate) text: Vec<String>,
    pub(crate) page: usize,
    pub(crate) error: Option<String>,
    active: bool,
}

/// Changed only when the Controls page needs a new widget tree. Typing into a
/// field updates the draft without destroying the focused field each frame.
#[derive(Resource, Default)]
pub struct ControlsRender(pub u64);

impl Default for ControlsDraft {
    fn default() -> Self {
        Self {
            text: Vec::new(),
            page: 0,
            error: None,
            active: false,
        }
    }
}

impl ControlsDraft {
    fn reload(&mut self, bindings: &KeyBindings) {
        self.text = Action::ALL
            .iter()
            .map(|&a| {
                bindings
                    .keys(a)
                    .iter()
                    .map(|k| format!("{k:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect();
        self.page = 0;
        self.error = None;
        self.active = true;
    }

    fn validate(&mut self, bindings: &KeyBindings) -> Option<KeyBindings> {
        let mut candidate = bindings.clone();
        for (&action, value) in Action::ALL.iter().zip(&self.text) {
            if !value.split(',').any(|key| !key.trim().is_empty()) {
                self.error = Some(format!("{}: no key given", action.name()));
                return None;
            }
            // `bind_from_text` can change its receiver before returning Err.
            if let Err(error) = candidate.bind_from_text(action, value) {
                self.error = Some(error);
                return None;
            }
        }
        self.error = None;
        Some(candidate)
    }
}

fn action_label(action: Action) -> String {
    match action {
        Action::AddModifier => tr("Add to selection"),
        Action::GroupModifier => tr("Assign group modifier"),
        Action::CentreModifier => tr("Centre group modifier"),
        Action::SelectType => tr("Select same type"),
        Action::Group(n) => trf("Group {0}", &[&(n + 1)]),
        Action::Stop => tr("Stop"),
        Action::Formation(sta_sim::movement::FORM_LINE) => tr("Line formation"),
        Action::Formation(sta_sim::movement::FORM_COLUMN) => tr("Column formation"),
        Action::Formation(_) => tr("Wedge formation"),
        Action::Capture => tr("Playtest capture"),
        Action::GameMenu => tr("Game menu"),
        Action::RevealModifier => tr("Reveal modifier"),
        Action::RevealFog => tr("Toggle fog"),
        Action::RevealShroud => tr("Toggle shroud"),
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct BindingField(pub Action);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlsAct {
    Previous,
    Next,
    Apply,
    Revert,
    Reset,
}

/// Read the actual field component before a page switch or Apply. The widget's
/// TextChanged message may be delivered after this click in the same frame.
pub fn update(
    mut clicks: MessageReader<ButtonClicked>,
    acts: Query<&ControlsAct>,
    fields: Query<(&BindingField, &TextField)>,
    screen: Option<Res<State<Screen>>>,
    menu: Option<Res<crate::ui::ingame_menu::InGameMenu>>,
    state: Res<State<AppState>>,
    mut draft: ResMut<ControlsDraft>,
    mut render: ResMut<ControlsRender>,
    mut bindings: ResMut<KeyBindings>,
) {
    let open = match state.get() {
        AppState::FrontEnd => screen
            .as_deref()
            .is_some_and(|s| *s.get() == Screen::Settings),
        AppState::InGame => menu
            .as_deref()
            .is_some_and(|m| m.open && m.page == crate::ui::ingame_menu::MenuPage::Options),
        _ => false,
    };
    if !open {
        draft.active = false;
        clicks.read().for_each(drop);
        return;
    }
    let was_active = draft.active;
    if !was_active {
        draft.reload(&bindings);
        render.0 += 1;
    }
    if was_active {
        for (field, text) in &fields {
            if let Some(i) = Action::ALL.iter().position(|a| *a == field.0)
                && draft.text[i] != text.text
            {
                draft.text[i].clone_from(&text.text);
            }
        }
    }
    for ButtonClicked(entity) in clicks.read() {
        let Ok(act) = acts.get(*entity) else { continue };
        match act {
            ControlsAct::Previous => draft.page = draft.page.saturating_sub(1),
            ControlsAct::Next => {
                draft.page = (draft.page + 1).min((Action::ALL.len() - 1) / PAGE_SIZE)
            }
            ControlsAct::Apply => {
                if let Some(candidate) = draft.validate(&bindings) {
                    *bindings = candidate;
                }
            }
            ControlsAct::Revert => draft.reload(&bindings),
            ControlsAct::Reset => draft.reload(&KeyBindings::default()),
        }
        render.0 += 1;
    }
}

pub fn body(
    p: &mut RelatedSpawnerCommands<ChildOf>,
    theme: &Theme,
    draft: &ControlsDraft,
    width: f32,
) {
    let page = draft.page.min((Action::ALL.len() - 1) / PAGE_SIZE);
    let first = page * PAGE_SIZE;
    let field_width = (width * 0.55).max(170.0);
    p.spawn(label(
        theme,
        &tr("Use KeyCode names separated by commas (for example: KeyS, Space)."),
    ));
    for (i, &action) in Action::ALL.iter().enumerate().skip(first).take(PAGE_SIZE) {
        let caption = action_label(action);
        p.spawn(Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(theme.gap),
            ..default()
        })
        .with_children(|r| {
            r.spawn((
                Node {
                    width: Val::Px(width * 0.32),
                    ..default()
                },
                children![label(theme, &caption)],
            ));
            r.spawn((
                text_field(
                    theme,
                    draft.text.get(i).map_or("", String::as_str),
                    "",
                    field_width,
                    128,
                    TextFilter::Any,
                ),
                BindingField(action),
            ));
        });
    }
    p.spawn(label(
        theme,
        &trf(
            "Page {0} of {1}",
            &[&(page + 1), &Action::ALL.len().div_ceil(PAGE_SIZE)],
        ),
    ));
    if let Some(error) = &draft.error {
        p.spawn(label(theme, &trf("Invalid binding: {0}", &[error])));
    }
    p.spawn(Node {
        column_gap: Val::Px(theme.gap),
        ..default()
    })
    .with_children(|r| {
        for (caption, act) in [
            ("Previous", ControlsAct::Previous),
            ("Next", ControlsAct::Next),
            ("Apply", ControlsAct::Apply),
            ("Revert", ControlsAct::Revert),
            ("Reset keys", ControlsAct::Reset),
        ] {
            r.spawn((
                kit_button(theme, &tr(caption), (width - theme.gap * 4.0) / 5.0),
                act,
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_atomic_and_retains_invalid_text() {
        let mut bindings = KeyBindings::default();
        let before = bindings.clone();
        let mut draft = ControlsDraft::default();
        draft.reload(&bindings);
        draft.text[0] = "KeyQ".into();
        draft.text[1] = "KeyR, NotAKey".into();
        assert!(draft.validate(&bindings).is_none());
        assert_eq!(bindings, before);
        assert_eq!(draft.text[1], "KeyR, NotAKey");
        assert!(draft.error.as_ref().unwrap().contains("NotAKey"));
        draft.text[1] = "KeyR".into();
        bindings = draft.validate(&bindings).unwrap();
        assert_eq!(bindings.keys(Action::AddModifier), [KeyCode::KeyQ]);
        assert_eq!(bindings.keys(Action::GroupModifier), [KeyCode::KeyR]);
        draft.text[1] = ", ,".into();
        assert!(draft.validate(&bindings).is_none());
        assert_eq!(bindings.keys(Action::GroupModifier), [KeyCode::KeyR]);
    }

    #[test]
    fn all_pages_round_trip_and_reset_keys_only() {
        let mut bindings = KeyBindings::default();
        let mut draft = ControlsDraft::default();
        draft.reload(&bindings);
        assert_eq!(draft.text.len(), 22);
        draft.text[0] = "KeyQ".into();
        draft.page = 3;
        draft.text[21] = "KeyH".into();
        bindings = draft.validate(&bindings).unwrap();
        let saved = bindings.to_text();
        assert_eq!(KeyBindings::from_text(&saved), bindings);
        assert_eq!(bindings.keys(Action::RevealShroud), [KeyCode::KeyH]);
        bindings = KeyBindings::default();
        draft.reload(&bindings);
        assert_eq!(draft.page, 0);
        assert_eq!(bindings.keys(Action::RevealShroud), [KeyCode::KeyV]);
    }
}
