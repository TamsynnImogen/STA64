use bevy::prelude::*;

use super::*;

#[test]
fn diagnostics_button_and_escape_keep_the_menu_modal() {
    let mut app = App::new();
    app.init_resource::<InGameMenu>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_message::<ButtonClicked>()
        .add_message::<OpenInGameMenu>()
        .add_message::<LeaveMatch>()
        .add_message::<AppExit>()
        .add_systems(Update, (keys_and_requests, buttons).chain());
    let diagnostics = app.world_mut().spawn(MenuAct::Diagnostics).id();
    let back = app.world_mut().spawn(MenuAct::Back).id();
    let resume = app.world_mut().spawn(MenuAct::Resume).id();

    app.world_mut().write_message(OpenInGameMenu);
    app.update();
    assert_eq!(
        *app.world().resource::<InGameMenu>(),
        InGameMenu {
            open: true,
            page: MenuPage::Main
        }
    );

    app.world_mut().write_message(ButtonClicked(diagnostics));
    app.update();
    assert_eq!(
        *app.world().resource::<InGameMenu>(),
        InGameMenu {
            open: true,
            page: MenuPage::Diagnostics
        }
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert_eq!(
        *app.world().resource::<InGameMenu>(),
        InGameMenu {
            open: true,
            page: MenuPage::Main
        }
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();

    app.world_mut().write_message(ButtonClicked(diagnostics));
    app.update();
    app.world_mut().write_message(ButtonClicked(back));
    app.update();
    assert_eq!(
        *app.world().resource::<InGameMenu>(),
        InGameMenu {
            open: true,
            page: MenuPage::Main
        }
    );
    app.world_mut().write_message(ButtonClicked(resume));
    app.update();
    assert!(!app.world().resource::<InGameMenu>().open);
}

#[test]
fn diagnostics_page_has_a_bounded_preview_and_report_slot() {
    let slots = slots_of(MenuPage::Diagnostics);
    assert!(slots.contains(&MenuSlot::DiagnosticsSummary));
    assert!(slots.contains(&MenuSlot::DiagnosticsWarning4));
    assert!(slots.contains(&MenuSlot::DiagnosticsReport));
    assert!(slots.contains(&MenuSlot::DiagnosticsBack));
}
