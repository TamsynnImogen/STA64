//! Select a camera when the front-end screen or campaign changes.

use bevy::prelude::*;

use crate::clock::AnimClock;
use crate::frontend::campaign::Campaigns;
use crate::frontend::screens::{MapReturn, Screen};

use super::{MenuMap, MenuScene, MenuSceneConfig};

fn key(screen: Screen) -> &'static str {
    match screen {
        Screen::Main => "main",
        Screen::SinglePlayer => "single_player",
        Screen::Setup => "setup",
        Screen::Advanced => "advanced",
        Screen::MapSelect => "map_select",
        Screen::Settings => "settings",
        Screen::ModSelect => "mods",
        Screen::Missions => "missions",
        Screen::Briefing => "briefing",
        Screen::Debrief => "debrief",
        Screen::Multiplayer => "multiplayer",
        Screen::HostGame => "host_game",
        Screen::JoinGame => "join_game",
        Screen::Lobby => "lobby",
    }
}

fn named<'a>(map: &'a MenuMap, id: &str) -> Option<&'a str> {
    map.view_index(id).map(|i| map.views[i].name.as_str())
}

fn screen_view(map: &MenuMap, screen: Screen) -> Option<&str> {
    let key = key(screen);
    map.routing
        .screens
        .get(key)
        .map(String::as_str)
        .or_else(|| named(map, key))
}

fn parent(screen: Screen, map_return: Screen) -> Option<Screen> {
    match screen {
        Screen::MapSelect if map_return == Screen::Lobby => Some(Screen::Lobby),
        Screen::Advanced | Screen::MapSelect => Some(Screen::Setup),
        Screen::Briefing | Screen::Debrief => Some(Screen::Missions),
        Screen::HostGame | Screen::JoinGame | Screen::Lobby => Some(Screen::Multiplayer),
        _ => None,
    }
}

pub(super) fn follow(
    screen: Option<Res<State<Screen>>>,
    map_return: Option<Res<MapReturn>>,
    campaigns: Option<Res<Campaigns>>,
    config: Res<MenuSceneConfig>,
    mut scene: ResMut<MenuScene>,
    clock: Res<AnimClock>,
) {
    let Some(screen) = screen else { return };
    if !screen.is_changed()
        && !config.is_changed()
        && !map_return
            .as_ref()
            .is_some_and(|origin| origin.is_changed())
        && !campaigns
            .as_ref()
            .is_some_and(|campaigns| campaigns.is_changed())
    {
        return;
    }
    let screen = *screen.get();
    let map_return = map_return.map_or(Screen::Setup, |origin| origin.0);
    let campaign = campaigns.as_deref().and_then(|campaigns| match screen {
        Screen::Missions | Screen::Briefing => campaigns.campaign.as_deref(),
        Screen::Debrief => campaigns
            .debrief
            .as_ref()
            .map(|result| result.campaign.as_str()),
        _ => None,
    });
    let map = &config.map;
    let view = campaign
        .and_then(|id| {
            map.routing
                .campaigns
                .get(&id.to_ascii_lowercase())
                .map(String::as_str)
        })
        .or_else(|| screen_view(map, screen))
        .or_else(|| campaign.and_then(|id| named(map, id)))
        .or_else(|| {
            std::iter::successors(parent(screen, map_return), |screen| {
                parent(*screen, map_return)
            })
            .find_map(|screen| screen_view(map, screen))
        })
        .unwrap_or(&config.view);
    if scene.view() != view {
        let time = scene.time(clock.now());
        scene.director.request_view(view, time);
    }
}
