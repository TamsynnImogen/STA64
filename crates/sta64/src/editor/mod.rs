//! Local map authoring. The live view, camera and simulation stay in place
//! when switching between editing and normal play.

mod document;
mod history;
mod inspector;
mod map_settings;
mod palette;
mod panel;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy::{input::InputSystems, prelude::*};
use sta_sim::source::PlacementEdit;
use sta_sim::{EntityId, Vec3 as SimVec3};

#[cfg(test)]
mod tests;

pub(crate) use document::Document;
use palette::{EntryKind, Palette};

use crate::app::{AppState, GameSet};
use crate::frontend::loading::{LoadInfo, LoadProgress, LoadReport, LoadStage};
use crate::input::orders::{Aim, GroundAt};
use crate::input::select::{Drag, Targetable, unit_under_where};
use crate::map::{MapBrowser, MapSeat};
use crate::sim_link::{CommandQueue, ContentRes, Selection, SelectionChanged, SimLink};
use crate::ui::hud::HudState;
use crate::ui::notices::{Notice, Severity};
use crate::ui::theme::{ButtonClicked, TextField};
use crate::viewer::Mounted;

#[derive(Message)]
pub(crate) struct OpenMap(pub String);

#[derive(Resource)]
pub(crate) struct EditorLoad {
    pub path: String,
    frames: u32,
    requested: bool,
}

impl EditorLoad {
    pub(crate) fn requested(&self) -> bool {
        self.requested
    }
}

#[derive(Resource, Clone)]
pub(crate) struct AuthoredIds(pub BTreeMap<EntityId, usize>);

/// The map picker opens authored placements for sandbox test play.
#[derive(Resource)]
pub(crate) struct AuthoringRun;

#[derive(Default, Resource)]
pub(crate) struct Editing {
    previous_fog: bool,
    palette: Palette,
    directory: Vec<usize>,
    shortcut: Option<Shortcut>,
    drag_origin: Option<(EntityId, Vec2)>,
    last_click: Option<(EntityId, Vec2, f64)>,
    inspected: Option<EntityId>,
    inspector_error: String,
    map_settings: bool,
    history: history::History,
    clipboard: Option<(String, sta_sim::PlayerId)>,
    status: String,
}

// Armada II and Fleet Ops use the authored menu slot at every level (static).
const PALETTE_KEYS: [KeyCode; palette::ENTRY_COUNT] = [
    KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5, KeyCode::F6,
    KeyCode::F7, KeyCode::F8, KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12,
];
const CLICK_RADIUS: f32 = 6.0;
const DOUBLE_CLICK_SECONDS: f64 = 0.4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    Entry(usize),
    Back,
    Cancel,
}

pub(crate) fn play_mode(editing: Option<Res<Editing>>) -> bool {
    editing.is_none()
}

pub(crate) fn canvas_input_free(editing: Option<Res<Editing>>) -> bool {
    editing.is_none_or(|editing| editing.inspected.is_none() && !editing.map_settings)
}

// Copies are editable from this picker regardless of the current mount. Keep
// them out of the playable map catalogues and the installed mod directories.
fn save_folder() -> Option<PathBuf> {
    crate::platform::data_dir().map(|p| p.join("bzn"))
}

pub(crate) fn saved_map_paths() -> Vec<String> {
    let Some(folder) = save_folder() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut paths: Vec<_> = entries.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| supported_name(name))
        .map(|name| format!("saved:{name}"))
        .collect();
    paths.sort();
    paths
}

fn supported_name(name: &str) -> bool {
    name.ends_with(".bzn") || name.ends_with(".sol")
}

pub(crate) fn map_stem(path: &str) -> Option<&str> {
    let file = Path::new(path.strip_prefix("saved:").unwrap_or(path));
    file.file_stem()?.to_str()
}

fn saved_map_file(path: &str) -> Result<PathBuf, String> {
    let name = path.strip_prefix("saved:").ok_or("Invalid saved map name")?;
    if Path::new(name).file_name().and_then(|file| file.to_str()) != Some(name) || name.contains(['/', '\\', ':']) || !supported_name(name) {
        return Err("Invalid saved map name".into());
    }
    let folder = save_folder().ok_or("The user data directory is unavailable")?;
    Ok(folder.join(name))
}

pub(crate) fn read_map(vfs: &sta_vfs::Vfs, path: &str) -> Result<Vec<u8>, String> {
    if path.starts_with("saved:") {
        let file = saved_map_file(path)?;
        if !file.symlink_metadata().is_ok_and(|meta| meta.file_type().is_file()) {
            return Err(format!("{path} is not a saved map"));
        }
        return std::fs::read(file).map_err(|e| e.to_string());
    }
    if !path.starts_with("bzn/") || !supported_name(path) {
        return Err("Choose a BZN or SOL map from the map list".into());
    }
    vfs.read(path).ok_or_else(|| format!("{path} not found"))?.map_err(|e| e.to_string())
}

pub(crate) struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<OpenMap>()
            .add_systems(OnEnter(AppState::FrontEnd), clear)
            .add_systems(PreUpdate, palette_shortcut.after(InputSystems)
                .run_if(resource_exists::<Editing>).run_if(in_state(AppState::InGame)))
            .add_systems(Update, open_map.in_set(GameSet::Ui).after(crate::frontend::screens::on_buttons).run_if(in_state(AppState::FrontEnd)))
            .add_systems(Update, load_map.in_set(GameSet::Ui).after(crate::map::MapRebuild).run_if(in_state(AppState::Loading)))
            .add_systems(Update, toggle_key.in_set(GameSet::Input).before(crate::input::select::select).run_if(in_state(AppState::InGame))
                .run_if(crate::ui::ingame_menu::menu_closed)
                .run_if(crate::cheats::input::game_input_free)
                .run_if(not(crate::ui::presentation::cutscene_running)))
            .add_systems(Update, tools.in_set(GameSet::Input).after(toggle_key).after(crate::input::orders::aim)
                .run_if(resource_exists::<Editing>).run_if(in_state(AppState::InGame))
                .run_if(crate::ui::ingame_menu::menu_closed).run_if(crate::cheats::input::game_input_free))
            .add_systems(Update, (panel::draw, panel::inspector_error).chain().in_set(GameSet::Ui));
    }
}

fn clear(mut commands: Commands) {
    commands.remove_resource::<EditorLoad>();
    commands.remove_resource::<Editing>();
    commands.remove_resource::<Document>();
    commands.remove_resource::<AuthoredIds>();
    commands.remove_resource::<AuthoringRun>();
}

fn open_map(
    mut requests: MessageReader<OpenMap>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppState>>,
    info: Option<ResMut<LoadInfo>>,
    progress: Option<ResMut<LoadProgress>>,
) {
    let Some(OpenMap(path)) = requests.read().last() else { return };
    if let Some(mut info) = info {
        *info = LoadInfo { kicker: "Map editor".into(), title: map_stem(path).unwrap_or(path).into(), ..default() };
    }
    if let Some(mut progress) = progress {
        progress.begin();
    }
    commands.insert_resource(MapSeat(Some(0)));
    commands.insert_resource(AuthoringRun);
    commands.insert_resource(EditorLoad { path: path.clone(), frames: 0, requested: false });
    next.set(AppState::Loading);
}

#[allow(clippy::too_many_arguments)]
fn load_map(
    editing: Option<ResMut<EditorLoad>>,
    browser: Option<ResMut<MapBrowser>>,
    progress: Option<ResMut<LoadProgress>>,
    mut next: ResMut<NextState<AppState>>,
    link: Option<ResMut<SimLink>>,
    document: Option<Res<Document>>,
    mount: Option<Res<Mounted>>,
    mut commands: Commands,
    mut notices: MessageWriter<Notice>,
) {
    let Some(mut request) = editing else { return };
    let Some(mut browser) = browser else {
        next.set(AppState::FrontEnd);
        return;
    };
    request.frames += 1;
    if !request.requested {
        if request.frames >= crate::game::flow::LEAD_FRAMES {
            request.requested = true;
            request.frames = 0;
            browser.dirty = true;
            // Hold tick zero before the map builder publishes the link.
            commands.init_resource::<Editing>();
        }
        return;
    }
    if browser.dirty {
        return;
    }
    let Some(mut link) = link.filter(|_| document.is_some_and(|doc| doc.path == request.path)) else {
        notices.write(Notice::new(Severity::Bad, format!("Cannot edit {}: {}", request.path, browser.info.replace('\n', " "))));
        next.set(AppState::FrontEnd);
        return;
    };
    if let Some(mut p) = progress {
        if !p.is_complete() {
            for stage in LoadStage::ALL {
                p.report(&LoadReport::finished(stage));
            }
        }
        if p.is_active() {
            return;
        }
    }
    commands.insert_resource(begin_editing(&mut link, mount.as_deref()));
    commands.remove_resource::<EditorLoad>();
    next.set(AppState::InGame);
}

fn begin_editing(link: &mut SimLink, mount: Option<&Mounted>) -> Editing {
    let previous_fog = link.fog_lifted();
    link.set_fog_lifted(true);
    Editing { previous_fog, palette: mount.map_or_else(Palette::default, |v| Palette::load(&v.0)), ..default() }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn toggle_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut clicked: MessageReader<ButtonClicked>,
    actions: Query<&panel::Act>,
    editing: Option<Res<Editing>>,
    net: Option<Res<crate::game::net_match::NetGame>>,
    mount: Option<Res<Mounted>>,
    link: Option<ResMut<SimLink>>,
    mut selection: ResMut<Selection>,
    mut changed: MessageWriter<SelectionChanged>,
    mut queue: ResMut<CommandQueue>,
    drag: Option<ResMut<Drag>>,
    hud: Option<ResMut<HudState>>,
    mut commands: Commands,
) {
    let play = clicked.read().any(|ButtonClicked(e)| actions.get(*e).is_ok_and(|a| matches!(a, panel::Act::Play)));
    let toggle = editor_chord(&keys, KeyCode::KeyE);
    if !(toggle || play) || net.is_some() {
        return;
    }
    let Some(mut link) = link else { return };
    if !link.can_edit_placements() {
        return;
    }
    if let Some(playing) = editing {
        link.set_fog_lifted(playing.previous_fog);
        commands.remove_resource::<Editing>();
    } else {
        commands.insert_resource(begin_editing(&mut link, mount.as_deref()));
    }
    selection.0.clear();
    changed.write(SelectionChanged);
    queue.0.clear();
    if let Some(mut drag) = drag {
        *drag = default();
    }
    if let Some(mut hud) = hud {
        hud.placing = None;
        hud.targeting = None;
    }
}

fn editor_chord(keys: &ButtonInput<KeyCode>, key: KeyCode) -> bool {
    keys.just_pressed(key)
        && keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
        && !keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight, KeyCode::ShiftLeft, KeyCode::ShiftRight])
}

fn palette_shortcut(mut keys: ResMut<ButtonInput<KeyCode>>, mut editing: ResMut<Editing>) {
    let plain_keys = !keys.any_pressed([
        KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight,
        KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::ShiftLeft, KeyCode::ShiftRight,
    ]);
    let mut shortcut = None;
    if plain_keys {
        // Reserve bare editor keys before the game's handlers see them
        // (including F10 to leave the match and Tab to toggle strategic icons).
        let dialog = editing.inspected.is_some() || editing.map_settings;
        if keys.clear_just_pressed(KeyCode::Tab) && !dialog {
            shortcut = Some(Shortcut::Back);
        }
        if dialog && keys.clear_just_pressed(KeyCode::Escape) {
            shortcut = Some(Shortcut::Cancel);
        }
        for (slot, key) in PALETTE_KEYS.into_iter().enumerate() {
            if keys.clear_just_pressed(key) && shortcut.is_none() {
                shortcut = Some(Shortcut::Entry(slot));
            }
        }
    }
    if editing.shortcut != shortcut {
        editing.shortcut = shortcut;
    }
}

fn editable(content: &sta_content::ContentDb, key: sta_sim::ContentKey) -> bool {
    content.unit(key).is_some_and(sta_sim::source::editable_placement)
}

fn ground(aim: crate::input::select::AimState, y: f32, bounds: Option<&crate::ui::hud::ghost::PlacementBounds>) -> Option<SimVec3> {
    let p = GroundAt::Ray(aim.ray).on_plane(y)?;
    let pos = SimVec3::from_f32(p.to_array());
    if let Some((lo, hi)) = bounds.and_then(|b| b.0)
        && (pos.x < lo.x || pos.x > hi.x || pos.z < lo.z || pos.z > hi.z)
    {
        return None;
    }
    Some(pos)
}

impl Editing {
    fn entries(&self) -> &[palette::Entry] {
        let mut entries = self.palette.root.as_slice();
        for &index in &self.directory {
            let Some(palette::Entry { kind: EntryKind::Branch(children), .. }) = entries.get(index) else { break };
            entries = children;
        }
        entries
    }

    fn select_entry(&mut self, index: usize) -> Option<(String, bool)> {
        match self.entries().get(index).map(|e| &e.kind) {
            Some(EntryKind::Branch(_)) => {
                self.directory.push(index);
                self.status.clear();
                None
            }
            Some(EntryKind::Leaf { odf, neutral }) => Some((odf.clone(), *neutral)),
            None => None,
        }
    }

    fn finish_click(&mut self, id: EntityId, cursor: Vec2, now: f64) -> bool {
        if self.last_click.is_some_and(|(previous, at, time)| {
            id == previous && cursor.distance(at) <= CLICK_RADIUS && now - time <= DOUBLE_CLICK_SECONDS
        }) {
            self.last_click = None;
            self.inspected = Some(id);
            self.inspector_error.clear();
            return true;
        }
        self.last_click = Some((id, cursor, now));
        false
    }
}

type EditorInputs<'w> = (
    Res<'w, ButtonInput<KeyCode>>,
    Res<'w, ButtonInput<MouseButton>>,
    Res<'w, Time<Real>>,
);
type EditorFields<'w, 's> = (
    Query<'w, 's, (&'static panel::InspectorField, &'static TextField)>,
    Query<'w, 's, (&'static map_settings::Field, &'static TextField)>,
);

#[allow(clippy::too_many_arguments)]
fn tools(
    (keys, buttons, time): EditorInputs<'_>,
    aim: Res<Aim>,
    targets: Targetable,
    content: Option<Res<ContentRes>>,
    bounds: Option<Res<crate::ui::hud::ghost::PlacementBounds>>,
    panel: Query<&bevy::ui::RelativeCursorPosition, With<panel::Root>>,
    (document, ids): (Option<ResMut<Document>>, Option<ResMut<AuthoredIds>>),
    link: Option<ResMut<SimLink>>,
    mut editing: ResMut<Editing>,
    mut clicked: MessageReader<ButtonClicked>,
    actions: Query<&panel::Act>,
    (fields, map_fields): EditorFields<'_, '_>,
    mut names: Option<ResMut<crate::ui::hud::instance_names::InstanceNames>>,
    mut selection: ResMut<Selection>,
    mut changed: MessageWriter<SelectionChanged>,
    mut leave: MessageWriter<crate::game::flow::LeaveMatch>,
) {
    let (Some(mut link), Some(content)) = (link, content) else { return };
    let mut document = document;
    let mut ids = ids;
    let mut rotate = 0;
    let mut delete = false;
    let mut save = editor_chord(&keys, KeyCode::KeyS);
    let back_key = editing.shortcut == Some(Shortcut::Back);
    let mut actions = clicked.read().filter_map(|ButtonClicked(e)| actions.get(*e).ok());
    if editing.map_settings {
        let action = actions.find(|act| matches!(act, panel::Act::CloseStats | panel::Act::ApplyMap));
        if editing.shortcut == Some(Shortcut::Cancel) || matches!(action, Some(panel::Act::CloseStats)) {
            editing.map_settings = false;
        } else if matches!(action, Some(panel::Act::ApplyMap)) && let Some(doc) = document.as_deref_mut() {
            let before = history::Checkpoint::take(&mut link, Some(doc), ids.as_deref(), names.as_deref(), &selection);
            match map_settings::apply(doc, &map_fields) {
                Ok(()) => {
                    editing.history.record(before);
                    editing.map_settings = false;
                    editing.status = "Map updated. Save and reopen to preview.".into();
                }
                Err(error) => editing.inspector_error = error,
            }
        }
        return;
    }
    if let Some(id) = editing.inspected {
        let action = actions.find(|act| matches!(act,
            panel::Act::CloseStats | panel::Act::ApplyStats | panel::Act::RestoreStats));
        if editing.shortcut == Some(Shortcut::Cancel) || matches!(action, Some(panel::Act::CloseStats))
            || link.real_curr().unit(id).is_none() {
            editing.inspected = None;
        } else if let Some(action) = action {
            let restore = matches!(action, panel::Act::RestoreStats);
            let inputs = if restore { BTreeMap::new() } else {
                fields.iter().map(|(field, text)| (field.0, text.text.as_str())).collect()
            };
            let before = history::Checkpoint::take(&mut link, document.as_deref(), ids.as_deref(), names.as_deref(), &selection);
            let result = apply_inspector(id, &mut link, &content, bounds.as_deref(), document.as_deref_mut(),
                ids.as_deref_mut(), names.as_deref_mut(), &inputs, restore);
            match result {
                Ok(updated) => {
                    if updated {
                        editing.history.record(before);
                        editing.status = if restore { "Class values restored".into() } else { "Object updated".into() };
                        changed.write(SelectionChanged);
                    }
                    editing.inspected = None;
                }
                Err(error) => editing.inspector_error = error,
            }
        }
        return;
    }

    let allow_topology = document.as_ref().is_none_or(|d| d.can_change_objects());
    let undo = editor_chord(&keys, KeyCode::KeyZ);
    let redo = editor_chord(&keys, KeyCode::KeyY);
    if undo || redo {
        let result = editing.history.restore(redo, &mut link, &content, document.as_deref_mut(),
            ids.as_deref_mut(), names.as_deref_mut(), &mut selection);
        editing.status = match result {
            Ok(true) => {
                editing.last_click = None;
                editing.drag_origin = None;
                changed.write(SelectionChanged);
                if redo { "Edit redone".into() } else { "Edit undone".into() }
            }
            Ok(false) => if redo { "Nothing to redo".into() } else { "Nothing to undo".into() },
            Err(error) => error,
        };
        return;
    }
    let over_panel = panel.iter().any(|pointer| pointer.cursor_over());
    let world_aim = aim.0.filter(|_| !over_panel);
    let mut back = back_key;
    let mut clicked_entry = None;
    for action in actions {
        match *action {
            panel::Act::Entry(i) => clicked_entry = Some(i),
            panel::Act::Back => back = true,
            panel::Act::Rotate => rotate = 1,
            panel::Act::Delete => delete = true,
            panel::Act::Save => save = true,
            panel::Act::Exit => { leave.write(crate::game::flow::LeaveMatch); }
            panel::Act::MapSettings if document.as_ref().is_some_and(|d| d.can_change_objects()) => {
                editing.map_settings = true;
                editing.inspector_error.clear();
                return;
            }
            panel::Act::Play | panel::Act::CloseStats | panel::Act::ApplyStats
                | panel::Act::RestoreStats | panel::Act::MapSettings | panel::Act::ApplyMap => {}
        }
    }
    let mut product = None;
    if back {
        editing.directory.pop();
        editing.status.clear();
    } else if allow_topology && editor_chord(&keys, KeyCode::KeyV) {
        product = editing.clipboard.clone();
        if product.is_none() {
            editing.status = "Copy an object before pasting".into();
            return;
        }
    } else if allow_topology {
        let keyed_entry = editing.entries().iter().position(|entry| {
            editing.shortcut == Some(Shortcut::Entry(entry.slot))
        });
        if let Some(index) = keyed_entry.or(clicked_entry) {
            editing.drag_origin = None;
            editing.last_click = None;
            if let Some((odf, neutral)) = editing.select_entry(index) {
                if keyed_entry.is_none() {
                    let slot = editing.entries()[index].slot + 1;
                    editing.status = format!("Point at the map and press F{slot}");
                    return;
                }
                let owner = if neutral { sta_sim::world::NEUTRAL } else { link.player() };
                product = Some((odf, owner));
            }
        }
    }
    if let Some((odf, owner)) = product {
        editing.drag_origin = None;
        editing.last_click = None;
        let team = if owner == sta_sim::world::NEUTRAL { 0 } else { i32::from(owner.0) + 1 };
        let before = history::Checkpoint::take(&mut link, document.as_deref(), ids.as_deref(), names.as_deref(), &selection);
        let placed = content.0.key(&odf).filter(|&key| editable(&content.0, key))
            .zip(world_aim.and_then(|aim| ground(aim, 0.0, bounds.as_deref())))
            .and_then(|(def, at)| {
                let edit = PlacementEdit::Place { def, owner, at };
                let Some(doc) = &mut document else { return link.edit_placement(edit) };
                let ids = ids.as_mut()?;
                let index = doc.push(&odf, team, at.to_f32(), [0.0, 1.0])?;
                let Some(id) = link.edit_placement(edit) else {
                    doc.remove(index);
                    return None;
                };
                ids.0.insert(id, index);
                Some(id)
            });
        if let Some(id) = placed {
            editing.history.record(before);
            selection.0 = vec![id];
            changed.write(SelectionChanged);
            editing.status = format!("Placed {odf}");
        } else {
            editing.status = format!("Could not place {odf} here");
        }
        return;
    }
    if save && let Some(doc) = &document {
        let saved = save_folder().ok_or_else(|| "The user data directory is unavailable".to_owned())
            .and_then(|folder| doc.save_copy(&folder));
        editing.status = match saved {
            Ok(p) => format!("Saved {}", p.display()),
            Err(e) => format!("Could not save: {e}"),
        };
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        rotate = 1;
    } else if keys.just_pressed(KeyCode::BracketLeft) {
        rotate = -1;
    }
    delete |= keys.just_pressed(KeyCode::Delete);
    let index_for = |id| ids.as_ref().and_then(|ids| ids.0.get(&id).copied())
        .filter(|&index| document.as_ref().is_some_and(|doc| doc.object(index).is_some()));
    let right = buttons.just_pressed(MouseButton::Right);
    if right {
        editing.last_click = None;
    }
    if buttons.just_pressed(MouseButton::Left) {
        editing.drag_origin = None;
        if let Some(aim) = world_aim {
            let target = unit_under_where(aim.cursor, aim.ray, &targets, |id| {
                link.real_curr().unit(id).is_some_and(|unit| {
                    index_for(id).is_some() || (allow_topology && editable(&content.0, unit.def))
                })
            });
            selection.0 = target.into_iter().collect();
            changed.write(SelectionChanged);
            editing.drag_origin = target.map(|id| (id, aim.cursor));
            editing.status.clear();
        }
        if editing.drag_origin.is_none() {
            editing.last_click = None;
        }
    }

    let mut dragged = false;
    if buttons.just_released(MouseButton::Left) && let Some((id, start)) = editing.drag_origin.take() {
        if let Some(aim) = world_aim.filter(|_| selection.0.first() == Some(&id)) {
            dragged = start.distance(aim.cursor) > CLICK_RADIUS;
            if !dragged && editing.finish_click(id, aim.cursor, time.elapsed_secs_f64()) {
                return;
            }
        }
        if dragged || world_aim.is_none() {
            editing.last_click = None;
        }
    }
    let Some(id) = selection.0.first().copied() else { return };
    let Some(unit) = link.real_curr().unit(id).cloned() else {
        selection.0.clear();
        changed.write(SelectionChanged);
        return;
    };
    let index = index_for(id);
    if !editable(&content.0, unit.def) || (!allow_topology && index.is_none()) {
        return;
    }

    let cut = allow_topology && editor_chord(&keys, KeyCode::KeyX);
    if (cut || editor_chord(&keys, KeyCode::KeyC)) && let Some(odf) = content.0.name(unit.def) {
        editing.clipboard = Some((odf.to_owned(), unit.owner));
        editing.status = format!("Copied {odf}");
        if !cut {
            return;
        }
    }
    if (delete || cut) && allow_topology {
        let before = history::Checkpoint::take(&mut link, document.as_deref(), ids.as_deref(), names.as_deref(), &selection);
        if link.edit_placement(PlacementEdit::Remove(id)).is_some() {
            if let (Some(doc), Some(index), Some(ids)) = (&mut document, index, &mut ids) {
                doc.remove(index);
                ids.0.remove(&id);
            }
            editing.history.record(before);
            selection.0.clear();
            changed.write(SelectionChanged);
            editing.status = if cut { "Object cut".into() } else { "Object deleted".into() };
        }
        return;
    }

    let moved = world_aim.filter(|_| right || dragged)
        .and_then(|a| ground(a, unit.pos.y.to_f32(), bounds.as_deref()));
    if moved.is_none() && rotate == 0 {
        return;
    }
    let before = history::Checkpoint::take(&mut link, document.as_deref(), ids.as_deref(), names.as_deref(), &selection);
    if link.edit_placement(PlacementEdit::Transform { id, at: moved.unwrap_or(unit.pos), quarter_turns: rotate }).is_some() {
        if let (Some(doc), Some(ids)) = (&mut document, &mut ids) {
            if let Some(index) = index {
                if let Some(at) = moved {
                    doc.move_to(index, at.to_f32());
                }
                if rotate != 0 {
                    doc.rotate(index, rotate);
                }
            } else if let Some(unit) = link.real_curr().unit(id)
                && let Some(name) = content.0.name(unit.def)
            {
                // Editing a unit built during play promotes it to a map placement.
                let team = if unit.owner == sta_sim::world::NEUTRAL { 0 } else { i32::from(unit.owner.0) + 1 };
                let heading = [unit.heading.0.to_f32(), unit.heading.1.to_f32()];
                if let Some(index) = doc.push(name, team, unit.pos.to_f32(), heading) {
                    ids.0.insert(id, index);
                }
            }
        }
        editing.history.record(before);
        editing.status = if moved.is_some() { "Object moved" } else { "Object rotated" }.into();
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_inspector(
    id: EntityId,
    link: &mut SimLink,
    content: &ContentRes,
    bounds: Option<&crate::ui::hud::ghost::PlacementBounds>,
    document: Option<&mut Document>,
    ids: Option<&mut AuthoredIds>,
    names: Option<&mut crate::ui::hud::instance_names::InstanceNames>,
    inputs: &BTreeMap<&str, &str>,
    restore: bool,
) -> Result<bool, String> {
    let index = ids.as_ref().and_then(|ids| ids.0.get(&id).copied());
    let object = index.and_then(|index| document.as_ref()?.object(index));
    if document.is_some() && object.is_none() {
        return Err("Move the object on the map first to add it to this document".into());
    }
    let prior = object.and_then(|object| document.as_ref()?.properties(object.entity_id));
    let can_change_identity = document.as_ref().is_none_or(|doc| doc.can_change_objects());
    let plan = inspector::plan(id, link, content, object, prior, can_change_identity, inputs, restore)?;
    if !plan.changed {
        return Ok(false);
    }
    let at = plan.edit.at;
    if let Some((lo, hi)) = bounds.and_then(|bounds| bounds.0)
        && link.real_curr().unit(id).is_some_and(|unit| unit.pos != at)
        && (at.x < lo.x || at.x > hi.x || at.z < lo.z || at.z > hi.z)
    {
        return Err("The position is outside the map".into());
    }
    let name = plan.edit.name.clone();
    link.edit_placement(PlacementEdit::Inspect(Box::new(plan.edit))).ok_or("The object cannot use one or more of these values")?;
    if let Some(names) = names {
        names.set_overrides(id, name.as_deref(), plan.saved.captain.as_deref(), plan.saved.registry.as_deref());
    }
    if let (Some(doc), Some(index), Some(object)) = (document, index, plan.object) {
        doc.update_object(index, object, plan.saved);
    }
    Ok(true)
}
