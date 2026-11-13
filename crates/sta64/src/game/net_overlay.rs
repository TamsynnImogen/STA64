//! What a networked match tells the player, in plain words (INT-M5): who the game is
//! waiting for and for how long, who dropped, a desync and where its report is, that the
//! match is over. A small panel over the map, outside `ui/hud/` (the HUD is not touched); it
//! reads the [`NetGame`] the duties keep and writes nothing back.

use std::time::Instant;

use bevy::prelude::*;

use super::net_match::{DesyncStage, NetGame, NetShared};
use crate::app::GameSet;

/// The lines the panel shows, newest state first. Kept as a resource so tests (and a future
/// HUD widget) read the words, not the layout.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct NetStatus(pub Vec<String>);

/// Marks the panel's text.
#[derive(Component)]
pub struct NetOverlayText;

/// Marks the panel's root.
#[derive(Component)]
pub struct NetOverlayRoot;

/// The words for the state of `s` at `now`: a desync, an ending, a stall, and the seats that
/// dropped. Empty while the match runs and nobody has dropped.
pub fn lines(s: &NetShared, now: Instant) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(end) = &s.ending {
        out.push(end.text());
        if let Some(p) = &s.replay_path {
            out.push(format!("The game was saved as {}.", p.display()));
        }
    }
    if let Some(d) = &s.desync {
        out.push(format!("The game went out of step at tick {}.", d.tick));
        match &d.stage {
            DesyncStage::Repairing => out.push("The match is stopped while the host's state is loaded.".into()),
            DesyncStage::Repaired => out.push("It was repaired from the host's state and plays on.".into()),
            DesyncStage::Stopped(why) => out.push(format!("The match is stopped for good: {why}. Press F10 to leave.")),
        }
        match (&d.report, &d.error) {
            (Some(p), _) => out.push(format!("Report: {} (compare two players' with `sta-bench desync <a.star> <b.star>`).", p.display())),
            (_, Some(e)) => out.push(format!("The report could not be written: {e}.")),
            _ => {}
        }
    }
    if let Some((since, waiting)) = &s.stall
        && s.ending.is_none()
        && s.desync.is_none()
        && now.saturating_duration_since(*since) >= s.opts_stall_after()
    {
        let secs = now.saturating_duration_since(*since).as_secs();
        let who: Vec<String> = waiting.iter().map(|p| seat_state(s, *p, now)).collect();
        out.push(format!("Waiting for {} ({secs} s).", who.join(", ")));
    }
    if !s.dropped.is_empty() {
        let who: Vec<String> = s.dropped.keys().map(|p| s.display_name(*p)).collect();
        out.push(format!(
            "Dropped out: {}. Their ships stay where they are and take no orders; what happens to a dropped player's forces is not decided yet.",
            who.join(", ")
        ));
    }
    out
}

/// A waiting seat, with what is known of it: a lost connection and the time left for a rejoin.
fn seat_state(s: &NetShared, p: sta_core::PlayerId, now: Instant) -> String {
    let name = s.display_name(p);
    if s.rejoining.contains(&p) {
        return format!("{name} (rejoining)");
    }
    match s.departed.get(&p) {
        Some(since) => {
            let left = s.opts_drop_grace().saturating_sub(now.saturating_duration_since(*since)).as_secs();
            format!("{name} (connection lost, held for a rejoin for {left} more seconds)")
        }
        None => name,
    }
}

fn update(mut commands: Commands, game: Option<Res<NetGame>>, mut status: ResMut<NetStatus>, root: Query<Entity, With<NetOverlayRoot>>, mut texts: Query<&mut Text, With<NetOverlayText>>) {
    let now = game.as_ref().map(|g| g.read(|s| lines(s, Instant::now()))).unwrap_or_default();
    if status.0 != now {
        status.0.clone_from(&now);
    }
    if now.is_empty() {
        for e in &root {
            commands.entity(e).despawn();
        }
        return;
    }
    let text = now.join("\n");
    if let Ok(mut t) = texts.single_mut() {
        if t.0 != text {
            t.0 = text;
        }
        return;
    }
    commands
        .spawn((
            NetOverlayRoot,
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), top: Val::Percent(12.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() },
            GlobalZIndex(60),
        ))
        .with_children(|p| {
            p.spawn((
                NetOverlayText,
                Text::new(text),
                TextFont { font_size: bevy::text::FontSize::Px(20.0), ..default() },
                TextColor(Color::srgb(0.98, 0.9, 0.55)),
                TextLayout::justify(Justify::Center),
                Node { max_width: Val::Percent(70.0), ..default() },
            ));
        });
}

pub struct NetOverlayPlugin;

impl Plugin for NetOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetStatus>().add_systems(Update, update.in_set(GameSet::Ui).after(super::net_match::net_duties));
    }
}
