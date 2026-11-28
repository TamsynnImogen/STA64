//! The Save and Load screens (UI-56, ADR 0010 V3's list and V7 in the menus; the player-facing
//! half of #1032). UI-55's [`SaveMatch`] and [`LoadMatch`] do the work; this module is the list, the
//! description field, the questions and the reasons.
//!
//! # What the player does
//!
//! - **Save game** (in-game menu) opens the Save screen: the saves on disk, newest first, a name
//!   field (a new save takes its text as the description; choosing a row puts that save's description
//!   there), and Save, Delete and Back. Saving over an existing save asks first; so does deleting.
//!   Where V7 does not allow a save (a networked match, a mission, an observer, a cutscene, a
//!   computer player that cannot be saved yet, a map without a file ...) the screen says "not
//!   allowed" with UI-55's reason ([`SaveAvailability`]) and Save is off.
//! - **Load game** (in-game menu and the front end's single-player screen) opens the Load screen: the
//!   same list; a save this build cannot load is **greyed out with its reason, never hidden** (another
//!   container version, other content, a changed or missing map, a networked or mission save ...),
//!   and Load is off for it. From a running match, Load asks "leave this match?" first, as the exit
//!   does; the match is left only once UI-55 has checked the whole file. Load is off while a load is
//!   pending (UI-55 R1).
//! - Esc steps back (a question, then the screen).
//!
//! # Where the facts come from
//!
//! The list reads the headers only ([`container::store::list`]); each header is judged by
//! [`Env::header_refusal`](crate::save), which is what a load checks without reading the rest of the
//! file (so the list greys out what a load would refuse for those reasons; damage deeper in a file is
//! found when it is loaded). A map file is read and hashed once per list, however many saves name it.
//!
//! # Captions
//!
//! Read at run time from the mounted `label.map`, never carried in the repository, by the keys
//! `save_game`, `no_save_game`, `load_game`, `previous_menu`, `save_game_disallowed`,
//! `save_game_failure` and `game_type_unloadable`; where a key is missing, STA64's own English through
//! [`tr`]. The key for a save's success is not used: a save closes the screen and says nothing more.
//! Each caption leads UI-55's own reason, which follows it.
//!
//! # Look
//!
//! **Interim, marked:** the layout is arranged ([`Grade::Arranged`]) from the kit's widgets in the
//! theme of the in-game menu, whatever the look. Which parts the original's screens have (a list of
//! saves, a name field, three buttons) is stated in the manual; where they sit, their sizes and colours
//! are not established, so none is claimed. A stock-look variant waits for a measurement.
//!
//! # Questions
//!
//! A question (overwrite, delete, leave the match) is modal: the rows and buttons behind it do not
//! act, and it names the save it means. It keeps that save's file name from the moment it is asked, and
//! the answer acts on that file only (if the file has left the list, nothing is done and the status
//! says so).

use std::path::PathBuf;

use bevy::math::{Rect, Vec2};
use bevy::prelude::*;

use crate::app::{AppState, GameSet};
use crate::frontend::screens::stock::{Grade, Region, StockBox};
use crate::frontend::screens::ScreenData;
use crate::game::flow::ActiveMatch;
use crate::save::container::{self, Header};
use crate::save::{Env, LoadMatch, MapCache, LoadOutcome, PendingLoad, SaveAvailability, SaveMatch, SaveOutcome};
use crate::sim_link::SimLink;
use crate::ui::i18n::{Strings, tr};
use crate::ui::theme::{ButtonClicked, ListSelected, Theme, ThemePlugin, kit_button, kit_text, disabled_button};

/// Global z of the screens: over the in-game menu (960), under notices (1000).
pub const SAVE_Z: i32 = 970;

/// Which screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaveMode {
    Save,
    #[default]
    Load,
}

/// Open the Save or Load screen (the in-game menu writes it; the front end's buttons carry
/// [`OpenSaveButton`]).
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenSaveScreen(pub SaveMode);

/// Put on a button that opens a screen: the front end's single-player Load game button.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenSaveButton(pub SaveMode);

/// What a button of the screen does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAct {
    /// Save (Save screen) or Load (Load screen) the chosen row.
    Primary,
    Delete,
    Back,
    /// "Yes" on the question that is asked.
    Yes,
    /// "No" on it.
    No,
}

/// A question the screen asks before it acts. Each names its save by file name, fixed when asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Save over the existing save in `file`, with `description` (what was in the name field).
    Overwrite { file: String, description: String },
    /// Delete the save in `file`.
    Delete { file: String },
    /// Leave the running match to load `file`.
    Leave { file: String },
}

impl Ask {
    pub fn file(&self) -> &str {
        match self {
            Ask::Overwrite { file, .. } | Ask::Delete { file } | Ask::Leave { file } => file,
        }
    }
}

/// One save on disk, as the list shows it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub file_name: String,
    pub path: PathBuf,
    /// The header, when the file's start could be read.
    pub header: Option<Header>,
    /// Why this build cannot load it; `None` for a save that can be loaded as far as its header says.
    pub refusal: Option<String>,
}

impl Entry {
    pub fn loadable(&self) -> bool {
        self.refusal.is_none()
    }

    /// Whether this row stands for a file of its own that can be deleted (the listing's "?" row for an
    /// unreadable folder entry carries the folder's path).
    pub fn deletable(&self) -> bool {
        self.path.file_name().is_some_and(|n| n == self.file_name.as_str())
    }

    /// The description, else the file name.
    pub fn title(&self) -> String {
        self.header.as_ref().map(|h| h.description.trim()).filter(|d| !d.is_empty()).map_or_else(|| self.file_name.clone(), str::to_owned)
    }
}

/// The screen's state. Closed when `open` is false.
#[derive(Resource, Debug, Clone, Default)]
pub struct SaveScreen {
    pub open: bool,
    pub mode: SaveMode,
    /// The state the screen was opened in: it closes when the state changes.
    pub origin: Option<AppState>,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    pub ask: Option<Ask>,
    /// The last thing that went wrong (or was done), shown under the list.
    pub status: Option<String>,
    /// A load is pending (UI-55) or has been asked for: Load is off.
    pub loading: bool,
    /// The text the name field starts with.
    pub name: String,
}

/// Each part of the screen, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SaveSlot {
    Panel,
    Title,
    List,
    Detail,
    Status,
    NameLabel,
    NameField,
    Primary,
    Delete,
    Back,
    AskPanel,
    AskTitle,
    AskLine1,
    AskLine2,
    Yes,
    No,
}

const fn a(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Arranged }
}

/// The region of `slot` on the 800x600 design screen. All arranged (see the module docs).
pub const fn region(slot: SaveSlot) -> Region {
    match slot {
        SaveSlot::Panel => a(60.0, 30.0, 680.0, 540.0),
        SaveSlot::Title => a(80.0, 42.0, 640.0, 34.0),
        SaveSlot::List => a(80.0, 88.0, 640.0, 270.0),
        SaveSlot::Detail => a(80.0, 366.0, 640.0, 60.0),
        SaveSlot::Status => a(80.0, 430.0, 640.0, 26.0),
        SaveSlot::NameLabel => a(80.0, 464.0, 90.0, 36.0),
        SaveSlot::NameField => a(180.0, 464.0, 540.0, 36.0),
        SaveSlot::Primary => a(80.0, 514.0, 200.0, 36.0),
        SaveSlot::Delete => a(300.0, 514.0, 200.0, 36.0),
        SaveSlot::Back => a(520.0, 514.0, 200.0, 36.0),
        SaveSlot::AskPanel => a(220.0, 210.0, 360.0, 190.0),
        SaveSlot::AskTitle => a(240.0, 222.0, 320.0, 34.0),
        SaveSlot::AskLine1 => a(240.0, 262.0, 320.0, 36.0),
        SaveSlot::AskLine2 => a(240.0, 302.0, 320.0, 44.0),
        SaveSlot::Yes => a(240.0, 352.0, 150.0, 36.0),
        SaveSlot::No => a(410.0, 352.0, 150.0, 36.0),
    }
}

/// The slots a screen shows (the question's, when one is asked, are on top of these).
#[cfg(test)]
pub fn slots_of(mode: SaveMode, asking: bool) -> Vec<SaveSlot> {
    let mut v = vec![SaveSlot::Panel, SaveSlot::Title, SaveSlot::List, SaveSlot::Detail, SaveSlot::Status, SaveSlot::Primary, SaveSlot::Delete, SaveSlot::Back];
    if mode == SaveMode::Save {
        v.extend([SaveSlot::NameLabel, SaveSlot::NameField]);
    }
    if asking {
        v.extend([SaveSlot::AskPanel, SaveSlot::AskTitle, SaveSlot::AskLine1, SaveSlot::AskLine2, SaveSlot::Yes, SaveSlot::No]);
    }
    v
}

/// The root of the screen's nodes.
#[derive(Component)]
pub struct SaveRoot;

/// A node placed in its slot, for tests and layout checks.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavePlaced(pub SaveSlot);

/// The list's node.
#[derive(Component)]
pub struct SaveList;

/// A row of the list; `n` is its index in [`SaveScreen::entries`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveRow(pub usize);

/// On every row: whether its save can be loaded here. A row that cannot is greyed out (dim text).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowLoadable(pub bool);

/// The name field.
#[derive(Component)]
pub struct SaveNameField;

/// The mounted `label.map`'s text for `key`, else the translated `fallback`.
fn caption(strings: Option<&Strings>, key: &str, fallback: &str) -> String {
    strings.and_then(|s| s.native_label(key)).filter(|t| !t.trim().is_empty()).map_or_else(|| tr(fallback), str::to_owned)
}

/// `seconds` as `m:ss`, or `h:mm:ss` from an hour.
pub fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Unix seconds as `YYYY-MM-DD HH:MM` in UTC (days from the civil calendar, proleptic Gregorian).
pub fn date(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rem / 3600, rem / 60 % 60)
}

/// The map's name from a header's path.
fn map_of(h: &Header) -> &str {
    let file = h.map.path.rsplit(['/', '\\']).next().unwrap_or(&h.map.path);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

/// The row's text: date, description, map and play time; a greyed row names its reason instead of the map.
fn row_text(e: &Entry) -> String {
    let Some(h) = &e.header else { return format!("{}: {}", e.file_name, e.refusal.as_deref().unwrap_or("")) };
    // The date ends the row (the side the manual's screens put it on); not a column of its own yet.
    let when = date(h.made_at);
    match &e.refusal {
        None => format!("{}  ({}, {})  {when}", e.title(), map_of(h), clock(h.play_time_secs)),
        Some(why) => format!("{}  -  {why}  {when}", e.title()),
    }
}

/// What the detail area says of the chosen row.
fn detail_text(e: Option<&Entry>, strings: Option<&Strings>) -> String {
    let Some(e) = e else { return tr("Choose a save.") };
    match (&e.header, &e.refusal) {
        (_, Some(why)) => format!("{}: {why}", caption(strings, "game_type_unloadable", "This save cannot be loaded")),
        (Some(h), None) => {
            let races: Vec<&str> = h.seats.iter().map(|s| s.race.as_str()).collect();
            format!("{}: {}.  {}: {}.  {} {}, {} {}.", tr("Map"), map_of(h), tr("Sides"), races.join(", "), tr("Game time"), clock(h.play_time_secs), tr("tick"), h.tick)
        }
        (None, None) => String::new(),
    }
}

/// Reads the saves folder: every file listed, newest first, the ones this build cannot load with their
/// reasons; and what went wrong with the folder itself, if anything.
fn read_entries(env: &Env<'_>, screen: Option<&ScreenData>) -> (Vec<Entry>, Option<String>) {
    let Some(dir) = env.dir() else { return (Vec::new(), None) };
    let (listed, error) = match container::store::list(&dir, None) {
        Ok(l) => (l, None),
        Err(e) => (Vec::new(), Some(format!("{}: {e}", tr("The saves folder cannot be read")))),
    };
    let mut maps = MapCache::new();
    let mut entries: Vec<Entry> = listed
        .into_iter()
        .map(|l| match l.outcome {
            Ok(h) => {
                let refusal = match screen {
                    Some(s) => env.header_refusal(s, &h, &mut maps),
                    None => Some(tr("no game content is loaded")),
                };
                Entry { file_name: l.file_name, path: l.path, header: Some(h), refusal }
            }
            Err(e) => Entry { file_name: l.file_name, path: l.path, header: None, refusal: Some(e.to_string()) },
        })
        .collect();
    entries.sort_by(|x, y| {
        let (a, b) = (x.header.as_ref().map(|h| h.made_at), y.header.as_ref().map(|h| h.made_at));
        b.cmp(&a).then_with(|| x.file_name.cmp(&y.file_name))
    });
    (entries, error)
}

/// A list of rows as the kit's scrolling list draws them, each row in its own colour (a row that
/// cannot be loaded is dim).
fn rows_list(theme: &Theme, rows: &[(String, bool)], selected: Option<usize>, visible: usize) -> impl Bundle {
    use crate::ui::theme::{Focusable, List, ListItem, RowMark, ScrollArea, WidgetState};
    let h = theme.list_row_h;
    let shown = visible.min(rows.len().max(1));
    let items: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(i, (text, loadable))| {
            let colour = if *loadable { theme.text } else { theme.text_dim };
            let row = (
                ListItem(i),
                SaveRow(i),
                WidgetState::Normal,
                Node { height: Val::Px(h), flex_shrink: 0.0, padding: UiRect::axes(Val::Px(theme.pad), Val::Px(2.0)), align_items: AlignItems::Center, overflow: Overflow::clip_x(), ..default() },
                BackgroundColor(if selected == Some(i) { theme.row_sel } else { Color::NONE }),
                children![
                    (
                        RowMark,
                        Pickable::IGNORE,
                        Node { position_type: PositionType::Absolute, left: Val::Px(0.0), width: Val::Px(theme.pad * 0.5), height: Val::Percent(70.0), border_radius: BorderRadius::px(0.0, theme.pad, theme.pad, 0.0), ..default() },
                        BackgroundColor(if selected == Some(i) { theme.accent } else { Color::NONE }),
                    ),
                    kit_text(theme, text.clone(), theme.small_size, colour),
                ],
            );
            (row, RowLoadable(*loadable))
        })
        .collect();
    (
        SaveList,
        List { selected },
        ScrollArea { row_h: h, rows: shown, count: rows.len() },
        ScrollPosition::default(),
        Focusable,
        Node { flex_direction: FlexDirection::Column, flex_shrink: 0.0, width: Val::Percent(100.0), height: Val::Px(h * shown as f32), overflow: Overflow::scroll_y(), ..default() },
        BackgroundColor(theme.track),
        Children::spawn(bevy::ecs::spawn::SpawnIter(items.into_iter())),
    )
}

type Box_<'a> = bevy::ecs::relationship::RelatedSpawnerCommands<'a, ChildOf>;

fn rect(slot: SaveSlot, b: Vec2) -> Rect {
    region(slot).px(b)
}

fn put(p: &mut Box_, slot: SaveSlot, b: Vec2, content: impl Bundle) {
    let r = rect(slot, b);
    p.spawn((
        SavePlaced(slot),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), align_items: AlignItems::Center, ..default() },
        children![content],
    ));
}

fn panel(p: &mut Box_, theme: &Theme, slot: SaveSlot, b: Vec2) {
    let r = rect(slot, b);
    p.spawn((
        SavePlaced(slot),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), border: UiRect::all(Val::Px(2.0)), ..default() },
        BackgroundColor(theme.panel),
        BorderColor::all(theme.accent2),
    ));
}

fn button(p: &mut Box_, theme: &Theme, slot: SaveSlot, b: Vec2, text: &str, act: SaveAct, on: bool) {
    let w = rect(slot, b).width();
    if on {
        put(p, slot, b, (kit_button(theme, text, w), act));
    } else {
        put(p, slot, b, (disabled_button(theme, text, w), act));
    }
}

fn wrapped(p: &mut Box_, theme: &Theme, slot: SaveSlot, b: Vec2, text: &str, colour: Color) {
    use crate::ui::sprite_font::{SpriteTextStyle, sprite_text_styled};
    use crate::ui::theme::font_role;
    let size = theme.small_size;
    put(p, slot, b, sprite_text_styled(text, size, colour, SpriteTextStyle { role: Some(font_role(theme, size)), wrap: Some(rect(slot, b).width()), ..default() }));
}

/// What the screen is made from, gathered so the drawing is a function of values.
struct Parts<'a> {
    theme: &'a Theme,
    strings: Option<&'a Strings>,
    /// Why Save is not allowed now, for the Save screen.
    blocked: Option<String>,
    pending: bool,
}

fn spawn_screen(commands: &mut Commands, s: &SaveScreen, parts: &Parts, b: Vec2) {
    let t = parts.theme;
    let save = s.mode == SaveMode::Save;
    let origin = s.origin.unwrap_or(AppState::FrontEnd);
    commands
        .spawn((
            SaveRoot,
            GlobalZIndex(SAVE_Z),
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        ))
        .with_children(|p| {
            p.spawn(Node { width: Val::Px(b.x), height: Val::Px(b.y), ..default() }).with_children(|p| {
                panel(p, t, SaveSlot::Panel, b);
                let title = if !save {
                    caption(parts.strings, "load_game", "Load game")
                } else if parts.blocked.is_some() {
                    caption(parts.strings, "no_save_game", "Save not allowed")
                } else {
                    caption(parts.strings, "save_game", "Save game")
                };
                put(p, SaveSlot::Title, b, kit_text(t, t.caps(&title, t.caps_titles), t.title_size, t.title));
                let rows: Vec<(String, bool)> = s.entries.iter().map(|e| (row_text(e), e.loadable())).collect();
                let visible = (rect(SaveSlot::List, b).height() / t.list_row_h).floor().max(1.0) as usize;
                if rows.is_empty() {
                    put(p, SaveSlot::List, b, kit_text(t, tr("There are no saves yet."), t.text_size, t.text_dim));
                } else {
                    let r = rect(SaveSlot::List, b);
                    p.spawn((
                        SavePlaced(SaveSlot::List),
                        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), ..default() },
                        children![rows_list(t, &rows, s.selected, visible)],
                    ));
                }
                wrapped(p, t, SaveSlot::Detail, b, &detail_text(s.selected.and_then(|i| s.entries.get(i)), parts.strings), t.text);
                let status = match (&s.status, &parts.blocked, save) {
                    (Some(why), _, _) => why.clone(),
                    (None, Some(why), true) => format!("{}: {why}", caption(parts.strings, "save_game_disallowed", "Save not allowed")),
                    _ => String::new(),
                };
                wrapped(p, t, SaveSlot::Status, b, &status, t.accent);
                let chosen = s.selected.and_then(|i| s.entries.get(i));
                let asking = s.ask.is_some();
                let act_on = !s.loading && !parts.pending && !asking;
                if save {
                    put(p, SaveSlot::NameLabel, b, kit_text(t, tr("Name"), t.text_size, t.text));
                    let r = rect(SaveSlot::NameField, b);
                    put(p, SaveSlot::NameField, b, (crate::ui::theme::text_field(t, &s.name, &tr("Description"), r.width(), 60, crate::ui::theme::TextFilter::Any), SaveNameField));
                    button(p, t, SaveSlot::Primary, b, &caption(parts.strings, "save_game", "Save game"), SaveAct::Primary, parts.blocked.is_none() && !asking);
                } else {
                    let can = chosen.is_some_and(Entry::loadable) && act_on && origin != AppState::Boot;
                    button(p, t, SaveSlot::Primary, b, &caption(parts.strings, "load_game", "Load game"), SaveAct::Primary, can);
                }
                button(p, t, SaveSlot::Delete, b, &tr("Delete game"), SaveAct::Delete, chosen.is_some_and(Entry::deletable) && !s.loading && !asking);
                button(p, t, SaveSlot::Back, b, &caption(parts.strings, "previous_menu", "Previous menu"), SaveAct::Back, true);
                if let Some(ask) = &s.ask {
                    // Nothing behind the question takes a click.
                    p.spawn((Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Px(b.x), height: Val::Px(b.y), ..default() }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.4))));
                    let named = s.entries.iter().find(|e| e.file_name == ask.file()).map_or_else(
                        || ask.file().to_owned(),
                        |e| match &e.header {
                            Some(h) => format!("{} ({})", e.title(), date(h.made_at)),
                            None => e.title(),
                        },
                    );
                    let (head, two) = match ask {
                        Ask::Overwrite { .. } => (tr("Save over this game?"), tr("It is replaced.")),
                        Ask::Delete { .. } => (tr("Delete this game?"), format!("{} {}", tr("The save is removed from the disk."), tr("This cannot be undone."))),
                        // The same question and words as the game menu's Exit.
                        Ask::Leave { .. } => (caption(parts.strings, "load_game", "Load game"), format!("{} {}", tr("Leave this match?"), tr("The match in progress is lost."))),
                    };
                    panel(p, t, SaveSlot::AskPanel, b);
                    put(p, SaveSlot::AskTitle, b, kit_text(t, t.caps(&head, t.caps_titles), t.title_size, t.title));
                    wrapped(p, t, SaveSlot::AskLine1, b, &named, t.text);
                    wrapped(p, t, SaveSlot::AskLine2, b, &two, t.text);
                    button(p, t, SaveSlot::Yes, b, &tr("Yes"), SaveAct::Yes, true);
                    button(p, t, SaveSlot::No, b, &tr("No"), SaveAct::No, true);
                }
            });
        });
}

/// The name a new save is given when the field is empty: the map and the game time.
fn default_name(active: Option<&ActiveMatch>, link: Option<&SimLink>) -> String {
    let map = active.map_or_else(String::new, |a| a.setup.map.name.clone());
    let time = link.map_or(0, |l| l.real_curr().tick / sta_sim::TICK_HZ as u64);
    format!("{map} {}", clock(time)).trim().to_owned()
}

/// A pending load (UI-55) keeps Load off; the flag is cleared by the load's outcome.
fn track_pending(mut screen: ResMut<SaveScreen>, pending: Option<Res<PendingLoad>>) {
    if pending.is_some() && !screen.loading {
        screen.loading = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn open_requests(
    mut requests: MessageReader<OpenSaveScreen>,
    mut screen: ResMut<SaveScreen>,
    state: Option<Res<State<AppState>>>,
    data: Option<Res<ScreenData>>,
    env: Env<'_>,
    active: Option<Res<ActiveMatch>>,
    link: Option<Res<SimLink>>,
    pending: Option<Res<PendingLoad>>,
) {
    let Some(OpenSaveScreen(mode)) = requests.read().last().copied() else { return };
    let (entries, status) = read_entries(&env, data.as_deref());
    let name = if mode == SaveMode::Save { default_name(active.as_deref(), link.as_deref()) } else { String::new() };
    *screen = SaveScreen { open: true, mode, origin: state.map(|s| *s.get()), entries, selected: None, ask: None, status, loading: pending.is_some(), name };
}

/// A refreshed list after a save or a delete; the choice is kept by file name.
fn refresh(screen: &mut SaveScreen, env: &Env<'_>, data: Option<&ScreenData>) {
    let keep = screen.selected.and_then(|i| screen.entries.get(i)).map(|e| e.file_name.clone());
    let (entries, error) = read_entries(env, data);
    screen.entries = entries;
    if error.is_some() {
        screen.status = error;
    }
    screen.selected = keep.and_then(|k| screen.entries.iter().position(|e| e.file_name == k));
}

#[allow(clippy::too_many_arguments)]
fn clicks(
    mut buttons: MessageReader<ButtonClicked>,
    mut rows: MessageReader<ListSelected>,
    mut screen: ResMut<SaveScreen>,
    acts: Query<&SaveAct>,
    opens: Query<&OpenSaveButton>,
    list: Query<Entity, With<SaveList>>,
    fields: Query<&crate::ui::theme::TextField, With<SaveNameField>>,
    blocked: Option<Res<SaveAvailability>>,
    data: Option<Res<ScreenData>>,
    env: Env<'_>,
    active: Option<Res<ActiveMatch>>,
    link: Option<Res<SimLink>>,
    mut open: MessageWriter<OpenSaveScreen>,
    mut save: MessageWriter<SaveMatch>,
    mut load: MessageWriter<LoadMatch>,
) {
    for ev in rows.read() {
        // A question is modal: rows behind it do not take a click.
        if screen.open && list.get(ev.list).is_ok() {
            if screen.ask.is_some() {
                // Drawn again, so the kit's own highlight goes back to the row that is chosen.
                screen.set_changed();
                continue;
            }
            screen.selected = Some(ev.index);
            screen.status = None;
            if screen.mode == SaveMode::Save && let Some(e) = screen.entries.get(ev.index) {
                screen.name = e.title();
            }
        }
    }
    for ButtonClicked(e) in buttons.read() {
        if let Ok(OpenSaveButton(mode)) = opens.get(*e) {
            open.write(OpenSaveScreen(*mode));
            continue;
        }
        let Ok(act) = acts.get(*e) else { continue };
        if !screen.open {
            continue;
        }
        // Behind a question only its own buttons (and Back, which withdraws it) act.
        if screen.ask.is_some() && !matches!(act, SaveAct::Yes | SaveAct::No | SaveAct::Back) {
            continue;
        }
        let chosen = screen.selected.filter(|i| *i < screen.entries.len());
        match act {
            SaveAct::Back => {
                if screen.ask.is_some() {
                    screen.ask = None;
                } else {
                    screen.open = false;
                }
            }
            SaveAct::No => screen.ask = None,
            SaveAct::Delete => {
                if let Some(i) = chosen.filter(|i| screen.entries[*i].deletable()) {
                    screen.ask = Some(Ask::Delete { file: screen.entries[i].file_name.clone() });
                }
            }
            SaveAct::Primary => match screen.mode {
                SaveMode::Load => {
                    if let Some(i) = chosen.filter(|i| screen.entries[*i].loadable()) && !screen.loading {
                        if screen.origin == Some(AppState::InGame) {
                            screen.ask = Some(Ask::Leave { file: screen.entries[i].file_name.clone() });
                        } else {
                            load.write(LoadMatch { path: screen.entries[i].path.clone() });
                            screen.loading = true;
                            screen.status = None;
                        }
                    }
                }
                SaveMode::Save => {
                    if let Some(why) = blocked.as_deref().and_then(|b| b.0.as_ref()) {
                        screen.status = Some(why.to_string());
                        continue;
                    }
                    let text = fields.iter().next().map(|f| f.text.trim().to_owned()).unwrap_or_default();
                    let text = if text.is_empty() { default_name(active.as_deref(), link.as_deref()) } else { text };
                    screen.name = text.clone();
                    // The row chosen with the same description is the save being replaced.
                    match chosen.filter(|i| screen.entries[*i].header.as_ref().is_some_and(|h| h.description.trim() == text)) {
                        Some(i) => screen.ask = Some(Ask::Overwrite { file: screen.entries[i].file_name.clone(), description: text }),
                        None => {
                            save.write(SaveMatch { description: text, overwrite: None });
                        }
                    }
                }
            },
            SaveAct::Yes => {
                let Some(ask) = screen.ask.take() else { continue };
                // The file the question named, found again by name; if it has left the list, nothing is done.
                let Some(entry) = screen.entries.iter().find(|e| e.file_name == ask.file()).cloned() else {
                    screen.status = Some(tr("That save is no longer in the list."));
                    continue;
                };
                match ask {
                    Ask::Delete { .. } => {
                        match std::fs::remove_file(&entry.path) {
                            Ok(()) => screen.status = Some(tr("The save was deleted.")),
                            Err(e) => screen.status = Some(format!("{}: {e}", tr("The save could not be deleted"))),
                        }
                        screen.selected = None;
                        refresh(&mut screen, &env, data.as_deref());
                    }
                    Ask::Overwrite { description, .. } => {
                        save.write(SaveMatch { description, overwrite: Some(entry.file_name.clone()) });
                    }
                    Ask::Leave { .. } => {
                        load.write(LoadMatch { path: entry.path.clone() });
                        screen.loading = true;
                        screen.status = None;
                    }
                }
            }
        }
    }
}

/// UI-55's answers: a save closes the screen, a refusal is shown, a load that took ends the screen and one
/// that was refused frees Load again.
fn outcomes(
    mut saves: MessageReader<SaveOutcome>,
    mut loads: MessageReader<LoadOutcome>,
    mut screen: ResMut<SaveScreen>,
    data: Option<Res<ScreenData>>,
    env: Env<'_>,
    strings: Option<Res<Strings>>,
) {
    for o in saves.read() {
        if !screen.open {
            continue;
        }
        match o {
            SaveOutcome::Saved { .. } => screen.open = false,
            SaveOutcome::NotAllowed(why) => screen.status = Some(format!("{}: {why}", caption(strings.as_deref(), "save_game_disallowed", "Save not allowed"))),
            SaveOutcome::Refused(e) => {
                screen.status = Some(format!("{}: {e}", caption(strings.as_deref(), "save_game_failure", "The save failed")));
                refresh(&mut screen, &env, data.as_deref());
            }
        }
    }
    for o in loads.read() {
        match o {
            LoadOutcome::Loaded { .. } => screen.open = false,
            LoadOutcome::Refused { reason, .. } => {
                screen.loading = false;
                if screen.open {
                    screen.status = Some(reason.clone());
                }
            }
        }
    }
}

/// Esc: one step back.
fn keys(keys: Option<Res<ButtonInput<KeyCode>>>, mut screen: ResMut<SaveScreen>) {
    if screen.open && keys.is_some_and(|k| k.just_pressed(KeyCode::Escape)) {
        if screen.ask.is_some() {
            screen.ask = None;
        } else {
            screen.open = false;
        }
    }
}

/// The screen belongs to the state it was opened in.
fn close_on_leave(state: Option<Res<State<AppState>>>, mut screen: ResMut<SaveScreen>) {
    if screen.open && state.is_some_and(|s| Some(*s.get()) != screen.origin) {
        screen.open = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<SaveRoot>>,
    screen: Res<SaveScreen>,
    theme: Res<Theme>,
    stock_box: Res<StockBox>,
    strings: Option<Res<Strings>>,
    availability: Option<Res<SaveAvailability>>,
) {
    if !(screen.is_changed() || theme.is_changed() || stock_box.is_changed() || strings.as_ref().is_some_and(|s| s.is_changed()) || availability.as_ref().is_some_and(|a| a.is_changed())) {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    if !screen.open {
        return;
    }
    let blocked = availability.as_deref().and_then(|a| a.0.as_ref()).map(ToString::to_string);
    let parts = Parts { theme: &theme, strings: strings.as_deref(), blocked, pending: screen.loading };
    spawn_screen(&mut commands, &screen, &parts, stock_box.0);
}

/// Registers the screens. Add it with the in-game menu.
pub struct SaveMenuPlugin;

impl Plugin for SaveMenuPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        app.init_resource::<SaveScreen>()
            .init_resource::<StockBox>()
            .init_resource::<SaveAvailability>()
            .add_message::<OpenSaveScreen>()
            .add_message::<SaveMatch>()
            .add_message::<LoadMatch>()
            .add_message::<SaveOutcome>()
            .add_message::<LoadOutcome>()
            .add_systems(Update, (track_pending, clicks, keys, open_requests, outcomes, close_on_leave, rebuild).chain().in_set(GameSet::Ui));
    }
}

#[cfg(test)]
mod tests;
