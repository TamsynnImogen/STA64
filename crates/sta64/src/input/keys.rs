//! Rebindable keys: named actions mapped to [`KeyCode`]s.
//!
//! Defaults follow Refit's `qol/README.md` (QOL-1, 4, 5): plain digit recalls,
//! Ctrl+digit replaces, Shift+digit adds, Alt+digit recalls and centres.
//!
//! Systems ask [`KeyBindings`] "is this action pressed" instead of naming a
//! key, so rebinding changes which key triggers the action. Control groups
//! (digits 1..9) and the Shift/Ctrl modifiers are bound the same way.
//!
//! Persistence is a plain text file, one `action = Key` line each (see
//! [`KeyBindings::to_text`] / [`KeyBindings::from_text`], [`KeyBindings::load`] /
//! [`KeyBindings::save`]). The game has no settings path yet, so the resource
//! lives in memory with the defaults; follow-up: the settings package picks the
//! file location and calls `load` at startup and `save` after a rebind.
//! Orders read Stop and Formation from here too.

use std::collections::BTreeMap;
use std::path::Path;

use bevy::prelude::*;
use sta_sim::movement::{FORM_COLUMN, FORM_LINE, FORM_WEDGE};

/// A bindable action. The file names are the [`Action::name`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Held: clicks and group keys add to the selection or group.
    AddModifier,
    /// Held with a digit: assign the selection to that control group.
    GroupModifier,
    /// Held with a digit: select the group and centre the camera at once
    /// (stock's Shift+N, moved to Alt by QOL-4).
    CentreModifier,
    /// Select every own unit of the selected units' class.
    SelectType,
    /// Control group 1..=9 (index 0..=8).
    Group(u8),
    /// Stop the selected units.
    Stop,
    /// Set the selection's formation: one of the `movement::FORM_*` kinds.
    Formation(u8),
    /// Held with `GroupModifier` (Ctrl): write a playtest capture folder (TOOL-14).
    Capture,
}

#[allow(dead_code)] // settings file API, until the settings package wires it
impl Action {
    pub const ALL: [Action; 18] = [
        Action::AddModifier,
        Action::GroupModifier,
        Action::CentreModifier,
        Action::SelectType,
        Action::Group(0),
        Action::Group(1),
        Action::Group(2),
        Action::Group(3),
        Action::Group(4),
        Action::Group(5),
        Action::Group(6),
        Action::Group(7),
        Action::Group(8),
        Action::Stop,
        Action::Formation(FORM_LINE),
        Action::Formation(FORM_COLUMN),
        Action::Formation(FORM_WEDGE),
        Action::Capture,
    ];

    pub fn name(self) -> String {
        match self {
            Action::AddModifier => "add_modifier".into(),
            Action::GroupModifier => "group_modifier".into(),
            Action::CentreModifier => "centre_modifier".into(),
            Action::SelectType => "select_type".into(),
            Action::Group(n) => format!("group{}", n + 1),
            Action::Stop => "stop".into(),
            Action::Formation(FORM_LINE) => "formation_line".into(),
            Action::Formation(FORM_COLUMN) => "formation_column".into(),
            Action::Formation(_) => "formation_wedge".into(),
            Action::Capture => "capture".into(),
        }
    }

    pub fn from_name(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == s)
    }
}

/// Action to keys. An action fires on any of its keys.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct KeyBindings(BTreeMap<Action, Vec<KeyCode>>);

const DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

impl Default for KeyBindings {
    fn default() -> Self {
        let mut m = BTreeMap::new();
        m.insert(Action::AddModifier, vec![KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        m.insert(Action::GroupModifier, vec![KeyCode::ControlLeft, KeyCode::ControlRight]);
        m.insert(Action::CentreModifier, vec![KeyCode::AltLeft, KeyCode::AltRight]);
        m.insert(Action::SelectType, vec![KeyCode::KeyT]);
        m.insert(Action::Stop, vec![KeyCode::KeyS, KeyCode::Space]);
        m.insert(Action::Formation(FORM_LINE), vec![KeyCode::F1]);
        m.insert(Action::Formation(FORM_COLUMN), vec![KeyCode::F2]);
        m.insert(Action::Formation(FORM_WEDGE), vec![KeyCode::F3]);
        m.insert(Action::Capture, vec![KeyCode::F12]);
        for (n, k) in DIGITS.into_iter().enumerate() {
            m.insert(Action::Group(n as u8), vec![k]);
        }
        KeyBindings(m)
    }
}

#[allow(dead_code)] // rebind/load/save: wired by the settings package
impl KeyBindings {
    pub fn keys(&self, a: Action) -> &[KeyCode] {
        self.0.get(&a).map_or(&[], |v| v.as_slice())
    }

    /// Replace all keys of `a` with `key`.
    pub fn rebind(&mut self, a: Action, key: KeyCode) {
        self.0.insert(a, vec![key]);
    }

    pub fn pressed(&self, a: Action, keys: &ButtonInput<KeyCode>) -> bool {
        self.keys(a).iter().any(|k| keys.pressed(*k))
    }

    pub fn just_pressed(&self, a: Action, keys: &ButtonInput<KeyCode>) -> bool {
        self.keys(a).iter().any(|k| keys.just_pressed(*k))
    }

    /// `action = Key, Key` lines.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (a, ks) in &self.0 {
            let ks: Vec<String> = ks.iter().map(|k| format!("{k:?}")).collect();
            s += &format!("{} = {}\n", a.name(), ks.join(", "));
        }
        s
    }

    /// Defaults overridden by whatever `text` binds; unknown keys and actions are skipped.
    pub fn from_text(text: &str) -> KeyBindings {
        let mut b = KeyBindings::default();
        for line in text.lines() {
            let Some((name, keys)) = line.split_once('=') else { continue };
            let Some(a) = Action::from_name(name.trim()) else { continue };
            let ks: Vec<KeyCode> = keys.split(',').filter_map(|k| parse_key(k.trim())).collect();
            if !ks.is_empty() {
                b.0.insert(a, ks);
            }
        }
        b
    }

    /// The file's bindings, or the defaults when it cannot be read.
    pub fn load(path: &Path) -> KeyBindings {
        std::fs::read_to_string(path).map(|t| KeyBindings::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_text())
    }
}

/// A key by its `Debug` name, for the keys worth binding.
#[allow(dead_code)]
fn parse_key(s: &str) -> Option<KeyCode> {
    use KeyCode::*;
    const CANDIDATES: &[KeyCode] = &[
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR,
        KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ, Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6,
        Digit7, Digit8, Digit9, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, ShiftLeft, ShiftRight,
        ControlLeft, ControlRight, AltLeft, AltRight, Space, Tab, Backquote, Home, End, Numpad0, Numpad1, Numpad2,
        Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
    ];
    CANDIDATES.iter().copied().find(|k| format!("{k:?}") == s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_overrides_defaults() {
        let mut b = KeyBindings::default();
        b.rebind(Action::SelectType, KeyCode::KeyQ);
        assert_eq!(KeyBindings::from_text(&b.to_text()), b);
        let partial = KeyBindings::from_text("select_type = KeyZ\nbogus = KeyA\n");
        assert_eq!(partial.keys(Action::SelectType), [KeyCode::KeyZ]);
        assert_eq!(partial.keys(Action::Group(0)), [KeyCode::Digit1]);
    }
}
