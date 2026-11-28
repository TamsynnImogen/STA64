//! Fleet Ops opens a local choice dialog for an owned craft. Its command info bar
//! loads its CFG and walks the craft's weapons in order, filling up to five special
//! buttons (read from the executable, static).

use bevy::{picking::Pickable, prelude::*};
use sta_content::{ContentDb, UnitDef, WeaponDef};
use sta_sim::specials::{SpecialKind, specials_of};
use sta_sim::view::UnitFlags;
use sta_sim::{
    ContentKey, EntityId, Fixed, Tick,
    command::{CommandKind, Order, Target},
};

use super::{
    HudFeed,
    layout::R,
    model::finished,
    panels::HudRoot,
    stock::{StockState, layout::StockLayout},
};
use crate::render::units::LocalPlayer;
use crate::sim_link::CommandQueue;
use crate::ui::{
    gui_sprites::{GuiButton, GuiButtonLabel},
    i18n::tr,
    notices::Tooltip,
    theme::{ButtonClicked, Theme, UiButton, WidgetState, label},
};

#[derive(Component)]
pub(super) struct Dialog;

#[derive(Component, Clone, Copy)]
pub(super) struct Portrait {
    starter: EntityId,
    slot: u8,
    weapon: ContentKey,
}

#[derive(Resource, Default)]
pub(crate) struct ChoiceState {
    current: Option<(EntityId, ContentKey)>,
    geometry: Option<Geometry>,
    submitted: Option<Tick>,
}

impl ChoiceState {
    pub(super) fn is_open(&self) -> bool {
        self.current.is_some()
    }
}

pub(crate) fn closed(state: Option<Res<ChoiceState>>) -> bool {
    state.is_none_or(|s| !s.is_open())
}

#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    panel: R,
    first: R,
    stride: f32,
}

fn authored_geometry(layout: &StockLayout) -> Option<Geometry> {
    let panel = layout.panel("CommandInfoBar")?;
    let origin = Vec2::new(panel.x, panel.y);
    let first = layout.at(origin, "CommandInfoBarButtonRect")?;
    let stride = layout
        .number("CommandInfoBarButtonDistance")
        .map_or(first.w, |n| n as f32 * layout.scale_x);
    Some(Geometry {
        panel,
        first,
        stride,
    })
}

pub(super) fn choice_weapon(def: &UnitDef, kind: SpecialKind, weapon: &WeaponDef) -> bool {
    def.raw.bool("showChoiceDialog").unwrap_or(false)
        && kind == SpecialKind::Replace
        && weapon.base.need_target != Some(true)
}

fn choices<'a>(db: &'a ContentDb, def: &'a UnitDef) -> impl Iterator<Item = (u8, &'a WeaponDef)> {
    specials_of(db, def)
        .enumerate()
        .filter_map(|(slot, (kind, weapon))| {
            if !choice_weapon(def, kind, weapon) {
                return None;
            }
            Some((u8::try_from(slot).ok()?, weapon))
        })
        .take(5)
}

fn starter(feed: &HudFeed) -> Option<(EntityId, ContentKey)> {
    let db = feed.content.as_deref()?;
    feed.view.units.iter().find_map(|unit| {
        if unit.owner != feed.player
            || unit.hull[0] <= Fixed::ZERO
            || unit.flags.contains(UnitFlags::DYING)
            || !finished(unit)
        {
            return None;
        }
        let def = db.unit(unit.def)?;
        def.raw
            .bool("showChoiceDialog")
            .unwrap_or(false)
            .then_some(())?;
        choices(db, def).next().map(|_| (unit.id, unit.def))
    })
}

#[allow(clippy::too_many_arguments)] // Bevy injects the current match and UI state.
pub(super) fn refresh(
    mut commands: Commands,
    root: Query<Entity, With<HudRoot>>,
    dialogs: Query<Entity, With<Dialog>>,
    feed: Res<HudFeed>,
    local: Option<Res<LocalPlayer>>,
    theme: Res<Theme>,
    stock: Option<Res<StockState>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    menu: Option<Res<crate::ui::ingame_menu::InGameMenu>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut state: ResMut<ChoiceState>,
) {
    let available = local.and_then(|local| local.0) == Some(feed.player)
        && scene.is_none()
        && crate::ui::ingame_menu::menu_closed(menu)
        && !crate::ui::presentation::cutscene_running(cine);
    let next = if available && root.single().is_ok() {
        starter(&feed)
    } else {
        None
    };
    let layout = stock.as_ref().and_then(|stock| stock.layout.as_ref());
    let geometry = next.and(layout.and_then(authored_geometry));
    if state.current == next
        && state.geometry == geometry
        && (next.is_none() || !dialogs.is_empty())
    {
        return;
    }
    state.geometry = geometry;
    if state.current != next {
        state.current = next;
        state.submitted = None;
    }
    for dialog in &dialogs {
        commands.entity(dialog).despawn();
    }
    let (Some((starter, def)), Some(db), Ok(root)) = (next, feed.content.as_deref(), root.single())
    else {
        return;
    };
    let Some(def) = db.unit(def) else { return };
    let dialog = commands
        .spawn((
            Dialog,
            ChildOf(root),
            GlobalZIndex(20),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(16.0),
                ..default()
            },
        ))
        .id();
    let parent = if let (Some(geometry), Some(layout)) = (geometry, layout) {
        for piece in layout.pieces("CommandInfoBarBackgroundPanel") {
            let r = piece.rect;
            commands.spawn((
                super::stock::node(R::new(
                    geometry.panel.x + r.x,
                    geometry.panel.y + r.y,
                    r.w,
                    r.h,
                )),
                crate::ui::gui_sprites::GuiImage::new(piece.name).with_frame(piece.frame),
                Pickable::IGNORE,
                ChildOf(dialog),
            ));
        }
        dialog
    } else {
        commands.spawn((
            label(&theme, &tr("Choose an avatar")),
            Pickable::IGNORE,
            ChildOf(dialog),
        ));
        commands
            .spawn((
                Node {
                    max_width: Val::Percent(90.0),
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(18.0),
                    row_gap: Val::Px(18.0),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(dialog),
            ))
            .id()
    };
    for (index, (slot, weapon)) in choices(db, def).enumerate() {
        let caption = weapon.raw.str("tooltip").unwrap_or(&weapon.odf);
        let mut tip = Tooltip::new(caption);
        if let Some(verbose) = weapon
            .raw
            .str("verboseTooltip")
            .or_else(|| weapon.raw.str("verbose"))
        {
            tip = tip.verbose(verbose);
        }
        let node = if let Some(geometry) = geometry {
            let mut r = geometry.first;
            r.x = (r.x + geometry.stride * index as f32).round();
            super::stock::node(r)
        } else {
            Node {
                width: Val::Px(256.0),
                height: Val::Px(256.0),
                ..default()
            }
        };
        commands
            .spawn((
                Portrait {
                    starter,
                    slot,
                    weapon: weapon.key,
                },
                UiButton,
                WidgetState::Normal,
                GuiButton {
                    names: vec![format!("b_{}", weapon.odf)],
                    keep_label: false,
                },
                tip,
                BackgroundColor(theme.button),
                Node {
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..node
                },
                ChildOf(parent),
            ))
            .with_child((GuiButtonLabel, Pickable::IGNORE, label(&theme, caption)));
    }
}

pub(super) fn press(
    mut clicks: MessageReader<ButtonClicked>,
    portraits: Query<&Portrait>,
    feed: Res<HudFeed>,
    mut state: ResMut<ChoiceState>,
    mut queue: ResMut<CommandQueue>,
    cheat: Option<Res<crate::cheats::input::CheatInput>>,
) {
    let selected = clicks
        .read()
        .find_map(|ButtonClicked(entity)| portraits.get(*entity).ok());
    clicks.clear();
    let Some(&Portrait {
        starter,
        slot,
        weapon,
    }) = selected
    else {
        return;
    };
    let Some((source, def)) = state.current else {
        return;
    };
    if starter != source
        || state.submitted == Some(feed.view.tick)
        || cheat.is_some_and(|c| c.open || c.blocked_this_frame)
    {
        return;
    }
    let valid = feed.content.as_deref().is_some_and(|db| {
        db.unit(def)
            .is_some_and(|def| choices(db, def).any(|(s, w)| s == slot && w.key == weapon))
    });
    if valid {
        queue.0.push(CommandKind::Order(Order::Special {
            units: vec![starter],
            slot,
            target: Target::None,
        }));
        state.submitted = Some(feed.view.tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, TeamId, Vec3, WorldSource, command::Command};
    use std::sync::Arc;

    #[test]
    fn installed_fo4_avatar_choices_replace_the_local_starter() {
        let Some(install) = sta_testkit::install(sta_testkit::Kind::ModFamily) else { return };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), Some("FO4")).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (map_db, _) = db.with_named_map_tech_tree(&vfs, "3ys");
        let db = Arc::new(map_db.unwrap_or(db));
        for (source, choices) in [
            ("fed_outposts", ["fed_outpostz", "fed_outposty"]),
            (
                "kli_kahless_stations",
                ["kli_kahless_stationy", "kli_kahless_stationz"],
            ),
            ("rom_starbases", ["rom_starbasey", "rom_starbasez"]),
            ("bor_assi_matrixs", ["bor_assi_matrixy", "bor_assi_matrixz"]),
            ("dom_cloning_facs", ["dom_cloning_facy", "dom_cloning_facz"]),
        ] {
            let source_key = db.key(source).unwrap();
            let player = PlayerId(0);
            let setup = GameSetup {
                content: db.clone(),
                seed: 74,
                map: Arc::new(sta_content::MapDef::empty()),
                players: vec![PlayerSetup {
                    id: player,
                    team: TeamId(0),
                    race: db.unit(source_key).unwrap().race.clone(),
                    control: Control::Human,
                }],
                options: GameOptions::default(),
            };
            for (portrait_index, expected) in choices.into_iter().enumerate() {
                let mut sim = Sim::from_setup(&setup);
                let station = sim
                    .spawn_def(source_key, player, Vec3::ZERO, (Fixed::ONE, Fixed::ZERO))
                    .unwrap();
                let mut app = App::new();
                app.insert_resource(crate::ui::profile::UiProfile::FleetOps.theme())
                    .insert_resource(HudFeed {
                        content: Some(db.clone()),
                        view: sim.view(),
                        player,
                        ..default()
                    })
                    .insert_resource(LocalPlayer(None))
                    .init_resource::<ChoiceState>()
                    .init_resource::<CommandQueue>()
                    .add_message::<ButtonClicked>()
                    .add_systems(Update, (refresh, press).chain());
                app.world_mut().spawn((HudRoot, Node::default()));
                app.update();
                assert!(
                    !app.world().resource::<ChoiceState>().is_open(),
                    "spectating {source}"
                );
                app.world_mut().insert_resource(LocalPlayer(Some(player)));
                app.update();
                assert!(
                    app.world().resource::<ChoiceState>().is_open(),
                    "{source} without a selection"
                );
                let world = app.world_mut();
                let mut query = world.query::<(Entity, &Portrait, &GuiButton, &Node)>();
                let buttons: Vec<_> = query
                    .iter(world)
                    .map(|(entity, portrait, picture, node)| {
                        assert_eq!((node.width, node.height), (Val::Px(256.0), Val::Px(256.0)));
                        assert!(picture.names[0].starts_with("b_avatar_"));
                        assert_eq!(portrait.starter, station);
                        entity
                    })
                    .collect();
                assert_eq!(buttons.len(), 2, "{source}");
                {
                    let mut feed = world.resource_mut::<HudFeed>();
                    feed.selected.push(station);
                    assert!(
                        super::super::commands::shared(&feed).specials.is_empty(),
                        "{source} choice weapons stayed on the command bar"
                    );
                }
                world.write_message(ButtonClicked(buttons[portrait_index]));
                world.write_message(ButtonClicked(buttons[1 - portrait_index]));
                app.update();
                let queued = std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0);
                assert_eq!(queued.len(), 1);
                sim.step(&[Command {
                    player,
                    seq: 1,
                    kind: queued.into_iter().next().unwrap(),
                }]);
                let view = sim.view();
                assert!(view.unit(station).is_none(), "{source} should be replaced");
                assert!(
                    view.units
                        .iter()
                        .any(|u| u.owner == player && db.name(u.def) == Some(expected)),
                    "{source} should become {expected}"
                );
                app.world_mut().resource_mut::<HudFeed>().view = view;
                app.update();
                assert!(
                    !app.world().resource::<ChoiceState>().is_open(),
                    "{source} was chosen"
                );
            }
        }
    }
}
