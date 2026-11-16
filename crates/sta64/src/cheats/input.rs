//! In-match cheat text entry. Input is Bevy's keyboard message stream; no OS input is touched.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseWheel};
use bevy::input::{ButtonState, InputSystems};
use bevy::prelude::*;

use crate::frontend::scene::MenuScene;
use crate::sim_link::SimLink;
use crate::ui::ingame_menu::InGameMenu;
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{KitFocus, Theme, kit_text};

const MAX_CHARS: usize = 128;

#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct CheatSubmitted(pub String);

/// `blocked_this_frame` also covers the frame that opens or closes entry.
#[derive(Resource, Default, Debug)]
pub struct CheatInput {
    pub open: bool,
    pub text: String,
    pub blocked_this_frame: bool,
    /// A key used to type must be released before its repeats become a game hotkey.
    quarantined: Vec<KeyCode>,
}

/// Use on game input systems that read held buttons or accumulated scroll.
pub fn game_input_free(input: Option<Res<CheatInput>>) -> bool {
    input.is_none_or(|input| !input.open && !input.blocked_this_frame)
}

#[derive(Component)]
struct CheatOverlay;

#[derive(Component)]
struct CheatOverlayText;

pub struct CheatInputPlugin;

impl Plugin for CheatInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Theme>()
            .init_resource::<CheatInput>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<KeyboardInput>()
            .add_message::<CheatSubmitted>()
            .add_systems(PreUpdate, handle_input.after(InputSystems))
            .add_systems(Update, sync_overlay);
    }
}

#[allow(clippy::too_many_arguments)] // Bevy system inputs; bounded below its 16-parameter limit.
fn handle_input(
    mut input: ResMut<CheatInput>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut submitted: MessageWriter<CheatSubmitted>,
    link: Option<Res<SimLink>>,
    scene: Option<Res<MenuScene>>,
    menu: Option<Res<InGameMenu>>,
    focus: Option<Res<KitFocus>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    scroll: Option<ResMut<AccumulatedMouseScroll>>,
    wheel: Option<ResMut<Messages<MouseWheel>>>,
    flow: Option<Res<crate::game::flow::Flow>>,
    state: Option<Res<State<crate::app::AppState>>>,
    lifecycle: Option<Res<crate::platform::LifecycleGate>>,
) {
    input.blocked_this_frame = false;
    let in_match = link.is_some()
        && scene.is_none()
        && (flow.is_none() || state.is_some_and(|s| *s.get() == crate::app::AppState::InGame))
        && lifecycle.is_none_or(|gate| gate.is_foreground());
    if !in_match || menu.is_some_and(|menu| menu.open) {
        input.quarantined.clear();
        if input.open {
            input.open = false;
            input.text.clear();
            input.blocked_this_frame = true;
        }
        // Advance this reader even outside a match, so stale keys cannot open entry later.
        keyboard.read().for_each(drop);
    } else {
        for event in keyboard.read() {
            if event.state != ButtonState::Pressed {
                input.quarantined.retain(|key| *key != event.key_code);
                continue;
            }
            if (input.open || input.blocked_this_frame)
                && !input.quarantined.contains(&event.key_code)
            {
                input.quarantined.push(event.key_code);
            }
            if event.repeat && matches!(event.key_code, KeyCode::Enter | KeyCode::NumpadEnter) {
                continue;
            }
            if !input.open {
                if input.blocked_this_frame {
                    continue;
                }
                if matches!(event.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
                    && !focus.as_ref().is_some_and(|focus| focus.0.is_some())
                {
                    input.open = true;
                    if !input.quarantined.contains(&event.key_code) {
                        input.quarantined.push(event.key_code);
                    }
                    input.text.clear();
                    input.blocked_this_frame = true;
                }
                continue;
            }
            input.blocked_this_frame = true;
            match event.key_code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    submitted.write(CheatSubmitted(std::mem::take(&mut input.text)));
                    input.open = false;
                }
                KeyCode::Escape => {
                    input.text.clear();
                    input.open = false;
                }
                KeyCode::Backspace => {
                    input.text.pop();
                }
                _ => {
                    let text = event
                        .text
                        .as_deref()
                        .or(matches!(event.logical_key, Key::Space).then_some(" "));
                    if let Some(text) = text {
                        let remaining = MAX_CHARS.saturating_sub(input.text.chars().count());
                        input
                            .text
                            .extend(text.chars().filter(|c| !c.is_control()).take(remaining));
                    }
                }
            }
        }
    }
    if input.open || input.blocked_this_frame {
        keys.reset_all();
        mouse.reset_all();
        if let Some(mut scroll) = scroll {
            scroll.delta = Vec2::ZERO;
        }
        if let Some(mut wheel) = wheel {
            wheel.clear();
        }
    } else {
        for key in &input.quarantined {
            keys.reset(*key);
        }
    }
}

fn sync_overlay(
    mut commands: Commands,
    input: Res<CheatInput>,
    theme: Res<Theme>,
    root: Query<Entity, With<CheatOverlay>>,
    mut caption: Query<&mut SpriteText, With<CheatOverlayText>>,
) {
    if !input.open {
        for entity in &root {
            commands.entity(entity).despawn();
        }
        return;
    }
    let label = format!("Cheat: {}|    Enter: submit   Esc: cancel", input.text);
    if root.is_empty() {
        commands.spawn((
            CheatOverlay,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(20.0),
                bottom: Val::Px(28.0),
                width: Val::Percent(60.0),
                padding: UiRect::all(Val::Px(theme.pad)),
                ..default()
            },
            BackgroundColor(theme.panel),
            GlobalZIndex(2000),
            children![(
                CheatOverlayText,
                kit_text(&theme, label, theme.text_size, theme.text)
            )],
        ));
    } else if input.is_changed() {
        for mut text in &mut caption {
            text.text = label.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use sta_sim::command::{Command, LocalRouter};
    use sta_sim::{PlayerId, TickOutput, WorldSource, WorldView};

    struct Frozen(Arc<WorldView>);

    impl WorldSource for Frozen {
        fn step(&mut self, _: &[Command]) -> TickOutput {
            TickOutput {
                view: self.0.clone(),
                events: vec![],
            }
        }

        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
    }

    fn app() -> (App, Entity) {
        let mut app = App::new();
        let window = app.world_mut().spawn_empty().id();
        app.init_resource::<CheatInput>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<KeyboardInput>()
            .add_message::<CheatSubmitted>()
            .add_systems(PreUpdate, handle_input)
            .insert_resource(SimLink::new(
                Box::new(Frozen(Arc::new(WorldView::default()))),
                Box::new(LocalRouter::new()),
                PlayerId(0),
            ));
        (app, window)
    }

    fn press(app: &mut App, window: Entity, key_code: KeyCode, text: Option<&str>) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: text.map_or(
                Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                |t| Key::Character(t.into()),
            ),
            state: ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window,
        });
        app.update();
    }

    #[test]
    fn entry_edit_submit_and_frame_isolation() {
        let (mut app, window) = app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyA);
        press(&mut app, window, KeyCode::Enter, None);
        assert!(app.world().resource::<CheatInput>().open);
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyA)
        );
        assert!(app.world().resource::<CheatInput>().blocked_this_frame);
        press(&mut app, window, KeyCode::KeyX, Some("é🙂"));
        press(&mut app, window, KeyCode::Backspace, None);
        assert_eq!(app.world().resource::<CheatInput>().text, "é");
        press(&mut app, window, KeyCode::Enter, None);
        let mut cursor = app
            .world()
            .resource::<Messages<CheatSubmitted>>()
            .get_cursor();
        let messages = app.world().resource::<Messages<CheatSubmitted>>();
        assert_eq!(
            cursor.read(messages).next(),
            Some(&CheatSubmitted("é".into()))
        );
        assert!(!app.world().resource::<CheatInput>().open);
        assert!(app.world().resource::<CheatInput>().blocked_this_frame);
    }

    #[test]
    fn escape_and_link_loss_discard_text() {
        let (mut app, window) = app();
        press(&mut app, window, KeyCode::Enter, None);
        press(&mut app, window, KeyCode::KeyX, Some("abc"));
        press(&mut app, window, KeyCode::Escape, None);
        assert_eq!(app.world().resource::<CheatInput>().text, "");
        press(&mut app, window, KeyCode::Enter, None);
        press(&mut app, window, KeyCode::KeyX, Some("again"));
        app.world_mut().remove_resource::<SimLink>();
        app.update();
        let input = app.world().resource::<CheatInput>();
        assert!(!input.open && input.text.is_empty());
    }

    #[test]
    fn typed_keys_stay_quarantined_after_submit_and_text_is_bounded() {
        let (mut app, window) = app();
        press(&mut app, window, KeyCode::Enter, None);
        press(&mut app, window, KeyCode::KeyS, Some(&"s".repeat(200)));
        assert_eq!(app.world().resource::<CheatInput>().text.len(), MAX_CHARS);
        press(&mut app, window, KeyCode::Enter, None);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        press(&mut app, window, KeyCode::KeyS, Some("s"));
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyS)
        );
    }

    #[test]
    fn captured_tab_is_discarded_instead_of_being_replayed_into_the_widget_kit() {
        use crate::ui::theme::{TextField, TextFilter, ThemePlugin, text_field};
        let (mut app, window) = app();
        app.add_plugins(MinimalPlugins).add_plugins(ThemePlugin);
        crate::app::configure_sets(&mut app);
        let theme = app.world().resource::<Theme>().clone();
        let field = app
            .world_mut()
            .spawn(text_field(&theme, "", "test", 200.0, 128, TextFilter::Any))
            .id();
        press(&mut app, window, KeyCode::Enter, None);
        press(&mut app, window, KeyCode::Tab, None);
        press(&mut app, window, KeyCode::Escape, None);
        app.update();
        assert_eq!(app.world().resource::<KitFocus>().0, None);
        app.world_mut().resource_mut::<KitFocus>().0 = Some(field);
        press(&mut app, window, KeyCode::KeyZ, Some("z"));
        assert_eq!(app.world().get::<TextField>(field).unwrap().text, "z");
        assert!(!app.world().resource::<CheatInput>().open);
    }
}
