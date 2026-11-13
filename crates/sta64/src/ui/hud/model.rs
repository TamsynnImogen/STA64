//! What the HUD shows, as plain data computed from the world (no Bevy UI):
//! the info panel, the command grid with its build page, and the resources
//! strip. The systems in `panels.rs` draw these and rebuild a panel only when
//! its model changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::Resource;
use sta_content::{Availability, ContentDb, Owned, ResourceModel, Unmet, UnitDef};
use sta_sim::command::{CommandKind, Order};
use sta_sim::movement::{FORM_COLUMN, FORM_LINE, FORM_WEDGE};
use sta_sim::{ContentKey, EntityId, Fixed, PlayerId, UnitView, WorldView};

use crate::ui::i18n::{tr, tooltip_key};
use super::layout::{GRID_COLS, GRID_ROWS, InfoHeight};
use super::commands;

#[path = "craft_names.rs"]
pub(crate) mod craft_names;

/// Everything the HUD reads: the view, whose HUD it is, the selection and the
/// content. Filled from the sim link, the selection and the content resource
/// ([`feed_from_link`](super::feed_from_link)); frame runs and tests write it
/// directly.
#[derive(Resource, Clone)]
pub struct HudFeed {
    pub view: Arc<WorldView>,
    pub player: PlayerId,
    pub selected: Vec<EntityId>,
    pub content: Option<Arc<ContentDb>>,
}

impl Default for HudFeed {
    fn default() -> Self {
        HudFeed { view: Arc::default(), player: PlayerId(0), selected: Vec::new(), content: None }
    }
}

impl HudFeed {
    /// The selected units still in the world, in selection order.
    pub fn selected_units(&self) -> Vec<&UnitView> {
        self.selected.iter().filter_map(|&id| self.view.unit(id)).filter(|u| !u.flags.contains(sta_sim::view::UnitFlags::DYING)).collect()
    }

    fn def(&self, u: &UnitView) -> Option<&UnitDef> {
        self.content.as_deref().and_then(|c| c.unit(u.def))
    }

    pub(crate) fn name(&self, u: &UnitView) -> String {
        self.def(u).map_or_else(|| format!("#{}", u.id.0), |d| craft_names::instance(d, u.id))
    }

    /// Authored type caption for the mouseover strip, independent of craft name.
    pub(crate) fn unit_label(&self, u: &UnitView) -> String {
        self.def(u).map_or_else(|| format!("#{}", u.id.0), |d| {
            craft_names::unit_name(d).unwrap_or(&d.class.0).to_owned()
        })
    }

    /// The selected units the player may order.
    pub fn own_selection(&self) -> Vec<EntityId> {
        self.selected_units().into_iter().filter(|u| u.owner == self.player).map(|u| u.id).collect()
    }
}

pub(super) fn finished(u: &UnitView) -> bool {
    u.progress.is_none_or(|p| p >= Fixed::ONE)
}

fn frac(v: [Fixed; 2]) -> f32 {
    let max = v[1].to_f32();
    if max <= 0.0 { 0.0 } else { (v[0].to_f32() / max).clamp(0.0, 1.0) }
}

// ---- the info panel ----------------------------------------------------------

/// One object's card.
#[derive(Debug, Clone, PartialEq)]
pub struct Single {
    pub id: EntityId,
    /// Stable choice from `possibleCraftNames`, then `unitName` or ODF stem.
    pub name: String,
    /// Companion values from the same selected craft-name row, when present.
    pub captain: Option<String>,
    pub registry: Option<String>,
    /// Authored `unitName`, falling back to the definition's class label.
    pub class: String,
    pub own: bool,
    /// Hull as 0..1 and the numbers.
    pub hull: f32,
    pub hull_now: i32,
    pub hull_max: i32,
    /// All four shield facings together, 0..1 (`None`: no shields).
    pub shield: Option<f32>,
    /// Building or being built, 0..1.
    pub progress: Option<f32>,
    /// Has a build list: its panel is the middle height.
    pub builder: bool,
}

/// One tile of a group.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub id: EntityId,
    pub name: String,
    pub hull: f32,
}

/// The info panel's content.
#[derive(Debug, Clone, PartialEq)]
pub enum Info {
    /// Nothing selected: no panel.
    None,
    Single(Single),
    Group(Vec<Tile>),
}

impl Info {
    pub fn height(&self) -> Option<InfoHeight> {
        match self {
            Info::None => None,
            Info::Single(s) if s.builder => Some(InfoHeight::Middle),
            Info::Single(_) => Some(InfoHeight::Low),
            Info::Group(_) => Some(InfoHeight::High),
        }
    }
}

pub fn info(feed: &HudFeed) -> Info {
    let sel = feed.selected_units();
    match sel.as_slice() {
        [] => Info::None,
        [u] => {
            let def = feed.def(u);
            let (name, captain, registry) = def.map_or_else(
                || (format!("#{}", u.id.0), None, None),
                |d| {
                    let identity = craft_names::identity(d, u.id);
                    (identity.name, identity.captain, identity.registry)
                },
            );
            let shields: Vec<[Fixed; 2]> = u.shields.to_vec();
            let (now, max) = shields.iter().fold((0.0, 0.0), |(n, m), s| (n + s[0].to_f32(), m + s[1].to_f32()));
            Info::Single(Single {
                id: u.id,
                name,
                captain,
                registry,
                class: def.map_or_else(String::new, |d| craft_names::unit_name(d).unwrap_or(&d.class.0).to_owned()),
                own: u.owner == feed.player,
                hull: frac(u.hull),
                hull_now: u.hull[0].to_f32().round() as i32,
                hull_max: u.hull[1].to_f32().round() as i32,
                shield: (max > 0.0).then(|| (now / max).clamp(0.0, 1.0)),
                progress: u.progress.map(|p| p.to_f32().clamp(0.0, 1.0)),
                builder: def.is_some_and(|d| !d.build.list.is_empty()) && finished(u),
            })
        }
        many => Info::Group(many.iter().map(|u| Tile { id: u.id, name: feed.name(u), hull: frac(u.hull) }).collect()),
    }
}

// ---- the grid ----------------------------------------------------------------

/// Which page of the grid is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Top,
    Build,
    Formations,
    /// Authored stock command menu; its stable ID comes from loaded ODF metadata.
    Native(u32),
}

/// What pressing a cell does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    OpenBuild,
    OpenFormations,
    OpenNativeMenu(u32),
    Back,
    /// Arm a map target; completed by the HUD's target click system.
    Target(TargetMode),
    /// The sim currently supports cloak as a special slot toggle.
    Special(u8),
    /// UI-02's stop on the own selection.
    Stop,
    /// UI-02's formation on the own selection.
    Formation(u8),
    /// Drop a builder's queue: `Stop` on it (SIM-08's cancel).
    Cancel(EntityId),
    /// `Order::Build`; `site`: the builder is a craft and needs a place, so
    /// the press arms a placement instead.
    Build {
        builder: EntityId,
        what: ContentKey,
        odf: String,
        site: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Move,
    Attack,
    Mine,
    Rally,
}

/// How a button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Ready,
    /// The stock does not cover it; the press still goes to the sim.
    Poor,
    /// Not buildable yet: shown, disabled.
    Locked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    /// 0..15, row by row (the keys Q W E R T / A S D F G / Z X C V B).
    pub cell: usize,
    pub label: String,
    pub act: Act,
    pub look: Look,
    pub tip: String,
    pub verbose: Option<String>,
    /// Sprite IDs in priority order; an empty list leaves the text label visible.
    pub sprite_names: Vec<String>,
    /// Keep the text alongside a resolved icon (formations need distinct labels).
    pub keep_label: bool,
}

/// The cell of "cancel" (key T) and "back" (key G) on every page.
pub const CANCEL_CELL: usize = 4;
pub const BACK_CELL: usize = 9;

/// What the game calls its four stock slots (0 dilithium, 1 latinum, 2 metal,
/// 3 biomatter by id): the content's own words (Fleet Operations: Tritanium
/// for the latinum slot, Supply for the biomatter one), stock's without content.
pub fn resource_model(feed: &HudFeed) -> ResourceModel {
    feed.content.as_deref().map(|db| db.resources().clone()).unwrap_or_default()
}

/// A shipyard's or builder's tech state, from the view: what the player owns
/// finished, its race, and the races of its allies.
fn tech_state(feed: &HudFeed, db: &ContentDb, race: &str) -> Owned {
    let mut state = Owned::new(&race.to_ascii_lowercase());
    let me = feed.view.player(feed.player);
    for u in feed.view.units.iter().filter(|u| finished(u)) {
        let Some(def) = db.unit(u.def) else { continue };
        if u.owner == feed.player {
            state = state.with(&def.odf.to_ascii_lowercase(), 1);
        } else if me.is_some_and(|m| feed.view.player(u.owner).is_some_and(|o| o.team == m.team)) {
            state = state.with_ally(&def.race.to_ascii_lowercase());
        }
    }
    state
}

/// One slot of a builder's list.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildItem {
    pub odf: String,
    pub key: ContentKey,
    pub look: Look,
    pub cost: Vec<f32>,
    /// Why it is locked.
    pub why: Option<String>,
}

fn unmet(u: &Unmet) -> String {
    match u {
        Unmet::Missing(o) => format!("needs {o}"),
        Unmet::Unobtainable(o) => format!("needs {o}, which cannot be built"),
        Unmet::Forbidden(o) => format!("not with {o}"),
        Unmet::NoAlly(r) => format!("needs an ally of {r}"),
        Unmet::WrongRace(r) => format!("only {r} can build it"),
        Unmet::CapacityFull(c) => format!("{c} is full"),
        Unmet::AlreadyResearched => "already researched".into(),
    }
}

/// A builder's build list: through CON-06's tech tree when the content has
/// one (what the sim will accept, SIM-09), else the def's list as it stands.
/// Buttons the tree hides are left out.
pub fn build_items(feed: &HudFeed, builder: &UnitView) -> Vec<BuildItem> {
    let Some(db) = feed.content.as_deref() else { return Vec::new() };
    let Some(def) = db.unit(builder.def) else { return Vec::new() };
    let stock: Vec<f32> = feed.view.player(feed.player).map(|p| p.resources.iter().map(|f| f.to_f32()).collect()).unwrap_or_default();
    let poor = |cost: &[f32]| cost.iter().enumerate().any(|(i, c)| *c > stock.get(i).copied().unwrap_or(0.0));
    let item = |odf: &str, key: ContentKey, avail: Option<&Availability>| -> Option<BuildItem> {
        let u = db.unit(key)?;
        let cost: Vec<f32> = u.build.cost.iter().map(|c| c.to_f32()).collect();
        let (look, why) = match avail {
            None | Some(Availability::Available) => (if poor(&cost) { Look::Poor } else { Look::Ready }, None),
            Some(Availability::Locked { hidden: true, .. }) => return None,
            Some(Availability::Locked { unmet: m, .. }) => (Look::Locked, Some(m.iter().map(unmet).collect::<Vec<_>>().join(", "))),
            Some(Availability::Withheld) => (Look::Locked, Some("cannot be built".into())),
            Some(Availability::NotListed) => (Look::Locked, Some("not in the tech tree".into())),
        };
        Some(BuildItem { odf: odf.to_owned(), key, look, cost, why })
    };
    match db.tech_tree() {
        Some(tree) => {
            let state = tech_state(feed, db, &def.race);
            tree.build_options(db, &def.odf, &state, false).iter().filter_map(|o| item(&o.def.odf, o.def.key, Some(&o.availability))).collect()
        }
        None => def.build.list.iter().filter_map(|d| item(&d.odf, d.key, None)).collect(),
    }
}

fn cost_text(cost: &[f32], names: &ResourceModel) -> String {
    let parts: Vec<String> = cost.iter().enumerate().filter(|(_, c)| **c > 0.0).map(|(i, c)| format!("{} {}", *c as i64, names.name(i))).collect();
    if parts.is_empty() { "free".into() } else { parts.join(", ") }
}

/// A label that fits a cell (it wraps over two lines): the name in capitals, cut short.
pub fn short(name: &str) -> String {
    name.chars().take(12).collect::<String>().to_uppercase()
}

fn button(cell: usize, label: &str, act: Act, tip: &str) -> Button {
    let (sprite_names, keep_label) = match &act {
        Act::OpenBuild => (vec!["b_build".into()], false),
        Act::OpenFormations => (vec!["b_form".into()], false),
        Act::Target(TargetMode::Move) => (vec!["b_move".into()], false),
        Act::Target(TargetMode::Attack) => (vec!["b_attack".into()], false),
        Act::Target(TargetMode::Rally) => (vec!["b_rally".into()], false),
        Act::Stop => (vec!["b_stop".into()], false),
        Act::Back => (vec!["b_back".into()], false),
        Act::Target(TargetMode::Mine) => (vec!["b_harvest".into()], false),
        Act::Cancel(_) => (vec!["b_cancel".into()], false),
        Act::Formation(_) => (vec!["b_form".into()], true),
        // No verified native ID for these actions; use the existing label.
        Act::Special(_) | Act::Build { .. } | Act::OpenNativeMenu(_) => (Vec::new(), false),
    };
    Button { cell, label: label.to_uppercase(), act, look: Look::Ready, tip: tip.to_owned(), verbose: None, sprite_names, keep_label }
}

/// A build icon follows the resolved model basename before the ODF stem.
fn build_sprite_names(model: &str, odf: &str) -> Vec<String> {
    let mut names = Vec::new();
    for source in [model, odf] {
        let basename = source.rsplit(['/', '\\']).next().unwrap_or(source).trim();
        let stem = match basename.rsplit_once('.') {
            Some((stem, ext)) if ["sod", "odf", "glb", "gltf", "m3d"].iter().any(|known| ext.eq_ignore_ascii_case(known)) => stem,
            _ => basename,
        };
        if !stem.is_empty() {
            let name = format!("b_{stem}");
            if !names.iter().any(|old: &String| old.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
    }
    names
}

/// The first selected own unit that can build: the grid's builder.
pub fn builder(feed: &HudFeed) -> Option<&UnitView> {
    feed.selected_units().into_iter().find(|u| u.owner == feed.player && finished(u) && feed.def(u).is_some_and(|d| !d.build.list.is_empty()))
}

/// The cells of the grid for the selection.
///
/// Top page: Build (Q) when a builder is selected; Stop (S) and the three
/// formations (D F G) for own units; nothing for an enemy or nothing selected.
/// Build page: the builder's list fills the cells but cancel (T) and back (G).
pub fn grid(feed: &HudFeed, page: Page) -> Vec<Button> {
    grid_with_capacity(feed, page, GRID_COLS * GRID_ROWS)
}

/// Shared action construction with a presentation-specific build capacity.
/// Stock palettes page the full list; the LCARS wrapper preserves its 15 cells.
pub(super) fn grid_with_capacity(feed: &HudFeed, page: Page, capacity: usize) -> Vec<Button> {
    let own = feed.own_selection();
    if own.is_empty() {
        return Vec::new();
    }
    let caps = commands::shared(feed);
    let builder = caps.build.then(|| builder(feed)).flatten();
    if page == Page::Formations && caps.move_order {
        return vec![
            button(0, &tr("Line"), Act::Formation(FORM_LINE), &tooltip_key("Formation: line")),
            button(1, &tr("Column"), Act::Formation(FORM_COLUMN), &tooltip_key("Formation: column")),
            button(2, &tr("Wedge"), Act::Formation(FORM_WEDGE), &tooltip_key("Formation: wedge")),
            button(BACK_CELL, &tr("Back"), Act::Back, &tooltip_key("Back")),
        ];
    }
    if page == Page::Build
        && let Some(b) = builder
    {
        let items = build_items(feed, b);
        let names = resource_model(feed);
        let site = feed
            .def(b)
            .is_some_and(|d| d.movement.max_speed > Fixed::ZERO);
        let mut out = Vec::new();
        let cells = (0..capacity).filter(|c| *c != CANCEL_CELL && *c != BACK_CELL);
        for (cell, it) in cells.zip(&items) {
            let definition = feed.content.as_deref().and_then(|db| db.unit(it.key));
            let verbose = match &it.why {
                Some(w) => format!("{}. Cost: {}", w, cost_text(&it.cost, &names)),
                None => format!("Cost: {}", cost_text(&it.cost, &names)),
            };
            out.push(Button {
                cell,
                label: short(&it.odf),
                act: Act::Build {
                    builder: b.id,
                    what: it.key,
                    odf: it.odf.clone(),
                    site,
                },
                look: it.look,
                tip: definition.and_then(|d| d.raw.str("tooltip")).map(str::to_owned)
                    .unwrap_or_else(|| format!("Build {}", it.odf)),
                verbose: Some(definition.and_then(|d| d.raw.str("verboseTooltip"))
                    .map_or(verbose, str::to_owned)),
                sprite_names: feed.content.as_deref().and_then(|db| db.unit(it.key))
                    .map_or_else(Vec::new, |def| build_sprite_names(&def.model, &it.odf)),
                keep_label: false,
            });
        }
        out.push(button(CANCEL_CELL, &tr("Cancel"), Act::Cancel(b.id), &tooltip_key("Cancel the queue")));
        out.push(button(BACK_CELL, &tr("Back"), Act::Back, &tooltip_key("Back")));
        out.sort_by_key(|b| b.cell);
        return out;
    }
    let mut out = Vec::new();
    if builder.is_some() {
        out.push(button(0, &tr("Build"), Act::OpenBuild, &tooltip_key("Build")));
    }
    if caps.rally {
        out.push(button(
            1,
            &tr("Rally"),
            Act::Target(TargetMode::Rally),
            &tooltip_key("Set rally point"),
        ));
    }
    if caps.move_order {
        out.push(button(2, &tr("Move"), Act::Target(TargetMode::Move), &tooltip_key("Move")));
        out.push(button(7, &tr("Formations"), Act::OpenFormations, &tooltip_key("Choose formation")));
    }
    if caps.stop {
        out.push(button(6, &tr("Stop"), Act::Stop, &tooltip_key("Stop")));
    }
    if caps.attack {
        out.push(button(
            3,
            &tr("Attack"),
            Act::Target(TargetMode::Attack),
            &tooltip_key("Attack"),
        ));
    }
    if caps.mine {
        out.push(button(4, &tr("Mine"), Act::Target(TargetMode::Mine), &tooltip_key("Mine")));
    }
    for (i, (slot, _)) in caps.specials.iter().enumerate() {
        if let Some(cell) = [5, 10, 11, 12, 13, 14].get(i) {
            out.push(button(*cell, &tr("Cloak"), Act::Special(*slot), &tooltip_key("Toggle cloak")));
        }
    }
    out.sort_by_key(|b| b.cell);
    out
}

/// The command a button's press puts in the queue, with the units it speaks
/// for (for the voice ack). `None` for the page buttons and for a build that
/// needs a site.
pub fn command(feed: &HudFeed, act: &Act) -> Option<(CommandKind, Vec<EntityId>)> {
    let own = feed.own_selection();
    let caps = commands::shared(feed);
    match act {
        Act::OpenBuild | Act::OpenFormations | Act::OpenNativeMenu(_) | Act::Back | Act::Target(_) => None,
        Act::Stop if caps.stop => {
            Some((CommandKind::Order(Order::Stop { units: own.clone() }), own))
        }
        Act::Formation(kind) if caps.move_order => Some((
            CommandKind::Order(Order::Formation {
                units: own.clone(),
                kind: *kind,
            }),
            own,
        )),
        Act::Special(slot) if caps.specials.iter().any(|s| s.0 == *slot) => Some((
            CommandKind::Order(Order::Special {
                units: own.clone(),
                slot: *slot,
                target: sta_sim::command::Target::None,
            }),
            own,
        )),
        Act::Cancel(b) if caps.build && builder(feed).is_some_and(|u| u.id == *b) => Some((
            CommandKind::Order(Order::Stop { units: vec![*b] }),
            vec![*b],
        )),
        Act::Build {
            builder: b,
            what,
            site: false,
            ..
        } if caps.build
            && builder(feed).is_some_and(|u| {
                u.id == *b
                    && feed
                        .def(u)
                        .is_some_and(|d| d.build.list.iter().any(|i| i.key == *what))
            }) =>
        {
            Some((
                CommandKind::Order(Order::Build {
                    builder: *b,
                    what: *what,
                    at: None,
                }),
                vec![*b],
            ))
        }
        _ => None,
    }
}

// ---- the resources strip ----------------------------------------------------

/// One entry of the strip.
#[derive(Debug, Clone, PartialEq)]
pub struct ResEntry {
    /// The game's word for it (`Tritanium`), lower case for a built-in entry.
    pub name: String,
    /// The resource id, for its colour.
    pub id: usize,
    pub text: String,
}

/// The strip: the player's stock in the order dilithium, metal, latinum,
/// biomatter (the order Refit's HUD.md asks for), those the race has, then the
/// officers as "in use/capacity" when it has any.
pub fn resources(feed: &HudFeed, no: [bool; 4]) -> Vec<ResEntry> {
    let Some(p) = feed.view.player(feed.player) else { return Vec::new() };
    let names = resource_model(feed);
    let mut out: Vec<ResEntry> = [0usize, 2, 1, 3]
        .into_iter()
        .filter(|&i| !no[i])
        .map(|i| ResEntry { name: names.name(i).to_owned(), id: i, text: (p.resources.get(i).map_or(0.0, |f| f.to_f32()) as i64).to_string() })
        .collect();
    let [used, cap] = p.officers;
    if cap > Fixed::ZERO {
        out.push(ResEntry { name: "officers".into(), id: 4, text: format!("{}/{}", used.to_f32() as i64, cap.to_f32() as i64) });
    }
    out
}

/// Everything the HUD draws, for change detection.
#[derive(Debug, Clone, PartialEq)]
pub struct Models {
    pub info: Info,
    pub grid: Vec<Button>,
    pub resources: Vec<ResEntry>,
}

pub fn models(feed: &HudFeed, page: Page, no: [bool; 4]) -> Models {
    Models { info: info(feed), grid: grid(feed, page), resources: resources(feed, no) }
}

/// Counts by def name of a player's finished units, for the tests and the
/// bench log.
pub fn owned_counts(feed: &HudFeed) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    if let Some(db) = feed.content.as_deref() {
        for u in feed.view.units.iter().filter(|u| u.owner == feed.player && finished(u)) {
            if let Some(n) = db.name(u.def) {
                *m.entry(n.to_owned()).or_insert(0) += 1;
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::{DefRef, TechTree};
    use sta_sim::view::{PlayerView, UnitFlags};
    use sta_sim::{TeamId, Vec3};

    const ME: PlayerId = PlayerId(1);

    fn unit(db: &ContentDb, id: u32, odf: &str, owner: u8, hull: f64, progress: Option<f64>) -> UnitView {
        let f = Fixed::from_f64;
        let shield = [[f(50.0), f(100.0)]; 4];
        UnitView {
            id: EntityId(id),
            def: db.key(odf).unwrap(),
            owner: PlayerId(owner),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [f(hull), f(100.0)],
            shields: shield,
            crew: None,
            energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE,
            progress: progress.map(f),
        }
    }

    fn player(id: u8, team: u8, stock: [f64; 4], officers: [f64; 2]) -> PlayerView {
        PlayerView { crew: None,
            id: PlayerId(id),
            team: TeamId(team),
            resources: stock.iter().map(|v| Fixed::from_f64(*v)).collect(),
            officers: officers.map(Fixed::from_f64),
            eliminated: false,
            relations: Vec::new(),
        }
    }

    fn feed(db: ContentDb, units: Vec<UnitView>, selected: &[u32]) -> HudFeed {
        let view = WorldView {
            units,
            players: vec![player(1, 1, [3000.0, 20.0, 700.0, 0.0], [3.0, 10.0]), player(2, 2, [0.0; 4], [0.0; 2])],
            ..Default::default()
        };
        HudFeed { view: Arc::new(view), player: ME, selected: selected.iter().map(|&i| EntityId(i)).collect(), content: Some(Arc::new(db)) }
    }

    /// `fake::db()` with a tech tree.
    fn db_with(tree: Option<&str>) -> ContentDb {
        let fake = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            b = b.unit(u.clone());
        }
        if let Some(t) = tree {
            b = b.tech_tree(TechTree::parse("t.tt", t).0);
        }
        b.build().unwrap()
    }

    #[test]
    fn authored_names_survive_ticks_selection_and_snapshot_restore() {
        use sta_sim::{Sim, WorldSource};
        use sta_sim::setup::{GameSetup, GameOptions, PlayerSetup, Control};

        let odf = sta_odf::resolve("ship", &mut |_| Some(
            "classLabel = \"craft\"\nunitName = \"Construction Ship\"\nmaxHealth = 100\npossibleCraftNames =\n \"USS First\" \"USS Second\"\n \"USS Third\"".to_owned()
        )).unwrap();
        let def = sta_content::unit::unit_def("fx_constructor", &odf, None, &mut Vec::new());
        let db = Arc::new(ContentDb::builder().unit(def).build().unwrap());
        let setup = GameSetup {
            seed: 77,
            map: Arc::new(sta_content::MapDef::empty()),
            content: db.clone(),
            players: vec![PlayerSetup { id: ME, team: TeamId(1), race: "fx".into(), control: Control::Human }],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let id = sim.spawn_def(db.key("fx_constructor").unwrap(), ME, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
        let mut f = HudFeed { view: sim.view(), player: ME, selected: vec![id], content: Some(db.clone()) };
        let Info::Single(before) = info(&f) else { panic!("single selection") };
        assert_eq!(before.class, "Construction Ship");
        assert!(["USS First", "USS Second", "USS Third"].contains(&before.name.as_str()));
        let digest = sim.digest();
        for _ in 0..3 {
            f.selected.clear();
            assert_eq!(info(&f), Info::None);
            f.selected.push(id);
            let Info::Single(card) = info(&f) else { panic!("single selection") };
            assert_eq!(card.name, before.name);
        }
        assert_eq!(sim.digest(), digest, "displaying names must not consume sim RNG");
        sim.step(&[]);
        let saved = sim.snapshot();
        let mut restored = Sim::from_snapshot(&saved, Some(db)).unwrap();
        f.view = restored.view();
        let Info::Single(after) = info(&f) else { panic!("restored selection") };
        assert_eq!((after.name, after.class), (before.name.clone(), before.class));
        // Continuation from a save stays identical, including the gameplay RNG.
        assert_eq!(sim.step(&[]).view.digest, restored.step(&[]).view.digest);
        f.view = restored.view();
        let Info::Single(after_tick) = info(&f) else { panic!("selection after tick") };
        assert_eq!(after_tick.name, before.name);
    }

    #[test]
    fn the_info_panel_picks_its_height() {
        let db = sta_content::fake::db();
        let units = vec![
            unit(&db, 1, "fx_cruiser", 1, 80.0, None),
            unit(&db, 2, "fx_yard", 1, 100.0, None),
            unit(&db, 3, "fx_station", 1, 10.0, Some(0.4)),
            unit(&db, 4, "fx_cruiser", 2, 100.0, None),
        ];
        let f = |sel: &[u32]| info(&feed(sta_content::fake::db(), units.clone(), sel));
        assert_eq!(f(&[]), Info::None);
        assert_eq!(f(&[]).height(), None);
        let one = f(&[1]);
        assert_eq!(one.height(), Some(InfoHeight::Low));
        let Info::Single(s) = one else { panic!() };
        assert_eq!((s.name.as_str(), s.class.as_str(), s.own, s.hull_now, s.hull_max), ("fx_cruiser", "craft", true, 80, 100));
        assert_eq!((s.hull, s.shield), (0.8, Some(0.5)));
        // A yard has a build list: the middle height.
        assert_eq!(f(&[2]).height(), Some(InfoHeight::Middle));
        // A station still welding is no builder yet, and shows its progress.
        let Info::Single(s) = f(&[3]) else { panic!() };
        assert_eq!((s.builder, s.progress), (false, Some(0.4)));
        // An inspected enemy gets a card, not marked as ours.
        let Info::Single(s) = f(&[4]) else { panic!() };
        assert!(!s.own);
        // Several: a group, in selection order; the dead are left out.
        let Info::Group(t) = f(&[2, 1, 99]) else { panic!() };
        assert_eq!(t.iter().map(|t| t.id.0).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(f(&[1, 2]).height(), Some(InfoHeight::High));
    }

    #[test]
    fn the_top_page_offers_the_ui02_orders() {
        let db = sta_content::fake::db();
        let units = vec![
            unit(&db, 1, "fx_cruiser", 1, 100.0, None),
            unit(&db, 2, "fx_yard", 1, 100.0, None),
            unit(&db, 4, "fx_cruiser", 2, 100.0, None),
        ];
        let f = feed(db, units, &[1]);
        let g = grid(&f, Page::Top);
        assert_eq!(
            g.iter().map(|b| b.cell).collect::<Vec<_>>(),
            [2, 3, 6, 7],
            "mobile armed craft"
        );
        let stop = g.iter().find(|b| b.act == Act::Stop).unwrap();
        assert_eq!(
            command(&f, &stop.act),
            Some((
                CommandKind::Order(Order::Stop {
                    units: vec![EntityId(1)]
                }),
                vec![EntityId(1)]
            ))
        );
        assert!(g.iter().any(|b| b.act == Act::OpenFormations));
        assert!(!g.iter().any(|b| matches!(b.act, Act::Formation(_))));
        let formations = grid(&f, Page::Formations);
        let wedge = formations
            .iter()
            .find(|b| b.act == Act::Formation(FORM_WEDGE))
            .unwrap();
        assert_eq!(wedge.sprite_names, ["b_form"]);
        assert!(wedge.keep_label);
        assert!(matches!(
            command(&f, &wedge.act),
            Some((
                CommandKind::Order(Order::Formation {
                    kind: FORM_WEDGE,
                    ..
                }),
                _
            ))
        ));
        // A builder adds the opener in the top left.
        let g = grid(
            &feed(
                sta_content::fake::db(),
                vec![unit(&sta_content::fake::db(), 2, "fx_yard", 1, 100.0, None)],
                &[2],
            ),
            Page::Top,
        );
        assert_eq!((g[0].cell, &g[0].act), (0, &Act::OpenBuild));
        assert!(g.iter().any(|b| b.act == Act::Target(TargetMode::Rally)));
        assert!(g.iter().any(|b| b.cell == 6 && b.act == Act::Stop));
        assert!(!g
            .iter()
            .any(|b| matches!(&b.act, Act::Formation(_) | Act::Target(TargetMode::Move))));
        // An enemy or nothing: no orders.
        let db = sta_content::fake::db();
        assert!(
            grid(
                &feed(
                    sta_content::fake::db(),
                    vec![unit(&db, 4, "fx_cruiser", 2, 100.0, None)],
                    &[4]
                ),
                Page::Top
            )
            .is_empty()
        );
        assert!(grid(&feed(sta_content::fake::db(), Vec::new(), &[]), Page::Top).is_empty());
    }

    #[test]
    fn capability_grid_filters_static_mixed_disabled_and_stale_selection() {
        let db = sta_content::fake::db();
        let units = vec![
            unit(&db, 1, "fx_station", 1, 100.0, None),
            unit(&db, 2, "fx_yard", 1, 100.0, None),
            unit(&db, 3, "fx_miner", 1, 100.0, None),
            unit(&db, 4, "fx_cruiser", 1, 100.0, None),
        ];
        let f = feed(db, units, &[1]);
        let station = grid(&f, Page::Top);
        assert!(station.iter().any(|b| b.act == Act::OpenBuild));
        assert!(
            station
                .iter()
                .any(|b| b.act == Act::Target(TargetMode::Rally))
        );
        assert!(
            station
                .iter()
                .any(|b| b.act == Act::Target(TargetMode::Attack))
        );
        assert!(station.iter().any(|b| b.cell == 6 && b.act == Act::Stop));
        assert!(matches!(
            command(&f, &Act::Stop),
            Some((CommandKind::Order(Order::Stop { .. }), _))
        ));
        assert!(!station.iter().any(|b| matches!(
            &b.act,
            Act::Formation(_) | Act::Target(TargetMode::Move)
        )));
        let mut f = f;
        f.selected = vec![EntityId(3)];
        assert!(
            grid(&f, Page::Top)
                .iter()
                .any(|b| b.act == Act::Target(TargetMode::Mine))
        );
        assert_eq!(grid(&f, Page::Top).iter().find(|b| b.act == Act::Target(TargetMode::Mine)).unwrap().sprite_names, ["b_harvest"]);
        f.selected = vec![EntityId(3), EntityId(4)];
        assert!(
            !grid(&f, Page::Top)
                .iter()
                .any(|b| b.act == Act::Target(TargetMode::Mine))
        );
        assert!(
            grid(&f, Page::Top)
                .iter()
                .any(|b| b.act == Act::Target(TargetMode::Move))
        );
        f.selected = vec![EntityId(1), EntityId(2)];
        assert!(grid(&f, Page::Top).iter().any(|b| b.act == Act::OpenBuild));
        assert!(grid(&f, Page::Top).iter().any(|b| b.act == Act::Stop));
        f.selected = vec![EntityId(2), EntityId(4)];
        assert!(!grid(&f, Page::Top).iter().any(|b| b.act == Act::OpenBuild));
        assert_eq!(command(&f, &Act::OpenBuild), None);
        f.selected = vec![EntityId(4)];
        Arc::make_mut(&mut f.view).units[3].flags =
            UnitFlags::SELECTABLE.with(UnitFlags::ENGINES_OFF, true);
        assert!(grid(&f, Page::Top).iter().any(|b| b.cell == 6 && b.act == Act::Stop));
        assert!(matches!(
            command(&f, &Act::Stop),
            Some((CommandKind::Order(Order::Stop { .. }), _))
        ));
        assert!(!grid(&f, Page::Top).iter().any(|b| matches!(
            &b.act,
            Act::Formation(_) | Act::Target(TargetMode::Move)
        )));
        f.selected = vec![EntityId(999)];
        assert!(grid(&f, Page::Top).is_empty());
        assert_eq!(command(&f, &Act::Stop), None);
    }

    #[test]
    fn stop_uses_live_craft_and_station_kind_independent_of_movement() {
        let db = sta_content::fake::db();
        let mut craft = unit(&db, 1, "fx_cruiser", 1, 100.0, None);
        craft.flags = craft.flags.with(UnitFlags::ENGINES_OFF, true);
        let station = unit(&db, 2, "fx_station", 1, 100.0, Some(0.5));
        let resource = unit(&db, 3, "fx_moon", 1, 100.0, None);
        let mut missing_def = unit(&db, 4, "fx_cruiser", 1, 100.0, None);
        missing_def.def = ContentKey::NONE;
        let mut dying = unit(&db, 5, "fx_cruiser", 1, 100.0, None);
        dying.flags = dying.flags.with(UnitFlags::DYING, true);
        let foreign = unit(&db, 6, "fx_cruiser", 2, 100.0, None);
        let mut f = feed(
            db,
            vec![craft, station, resource, missing_def, dying, foreign],
            &[1, 2],
        );
        let caps = commands::shared(&f);
        assert!(caps.stop);
        assert!(!caps.move_order);
        assert!(grid(&f, Page::Top)
            .iter()
            .any(|b| b.cell == 6 && b.act == Act::Stop));
        assert_eq!(
            command(&f, &Act::Stop),
            Some((
                CommandKind::Order(Order::Stop {
                    units: vec![EntityId(1), EntityId(2)]
                }),
                vec![EntityId(1), EntityId(2)]
            ))
        );

        for id in [3, 4, 5, 6, 999] {
            f.selected = vec![EntityId(id)];
            assert!(!commands::shared(&f).stop, "{id} must not gain Stop");
            assert!(!grid(&f, Page::Top).iter().any(|b| b.act == Act::Stop));
            assert_eq!(command(&f, &Act::Stop), None);
        }
        f.selected = vec![EntityId(1), EntityId(3)];
        assert!(
            !commands::shared(&f).stop,
            "the shared command needs every owned unit"
        );
        assert_eq!(command(&f, &Act::Stop), None);
    }

    #[test]
    fn the_build_page_is_the_builders_list_without_a_tree() {
        let db = sta_content::fake::db();
        let f = feed(sta_content::fake::db(), vec![unit(&db, 2, "fx_yard", 1, 100.0, None)], &[2]);
        let g = grid(&f, Page::Build);
        // Two items, cancel on T, back on G.
        assert_eq!(g.iter().map(|b| (b.cell, b.label.as_str())).collect::<Vec<_>>(), [(0, "FX_CRUISER"), (1, "FX_FRIGATE"), (4, "CANCEL"), (9, "BACK")]);
        let cruiser = &g[0];
        assert_eq!(cruiser.look, Look::Ready);
        let what = f.content.as_ref().unwrap().key("fx_cruiser").unwrap();
        assert_eq!(
            command(&f, &cruiser.act),
            Some((CommandKind::Order(Order::Build { builder: EntityId(2), what, at: None }), vec![EntityId(2)]))
        );
        // Cancel stops the yard (SIM-08 drops its queue).
        assert_eq!(command(&f, &g[2].act).unwrap().0, CommandKind::Order(Order::Stop { units: vec![EntityId(2)] }));
        assert_eq!(command(&f, &Act::Back), None);
        // A yard builds in place: no site.
        assert!(matches!(cruiser.act, Act::Build { site: false, .. }));
    }

    #[test]
    fn grid_sprite_metadata_preserves_actions_and_build_order() {
        let fake = sta_content::fake::db();
        let mut db = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { db = db.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_cruiser" { unit.model = "fed_excelsior".into(); }
            db = db.unit(unit);
        }
        let db = db.build().unwrap();
        let f = feed(db, vec![unit(&fake, 2, "fx_yard", 1, 100.0, None)], &[2]);
        let top = grid(&f, Page::Top);
        assert_eq!(top.iter().map(|b| b.cell).collect::<Vec<_>>(), [0, 1, 6]);
        assert_eq!(top[0].sprite_names, ["b_build"]);
        assert_eq!(top[1].sprite_names, ["b_rally"]);
        assert_eq!(top[2].sprite_names, ["b_stop"]);
        let build = grid(&f, Page::Build);
        assert_eq!(build.iter().map(|b| b.cell).collect::<Vec<_>>(), [0, 1, 4, 9]);
        assert_eq!(build[0].sprite_names, ["b_fed_excelsior", "b_fx_cruiser"]);
        assert_eq!(build[0].label, "FX_CRUISER");
        assert!(matches!(build[0].act, Act::Build { .. }));
        assert_eq!(build[2].sprite_names, ["b_cancel"]);
        assert_eq!(build[3].sprite_names, ["b_back"]);
        assert_eq!(command(&f, &build[2].act).unwrap().0, CommandKind::Order(Order::Stop { units: vec![EntityId(2)] }));
        assert_eq!(build_sprite_names("fed_excelsior", "fed_excelsiorA"), ["b_fed_excelsior", "b_fed_excelsiorA"]);
        assert_eq!(build_sprite_names("FX_Cruiser.SOD", "fx_cruiser.odf"), ["b_FX_Cruiser"]);
        assert_eq!(build_sprite_names("models/fed_excelsior.GlB", "fed_excelsiorA"), ["b_fed_excelsior", "b_fed_excelsiorA"]);
    }

    #[test]
    fn build_buttons_preserve_authored_tooltip_keys_with_generated_fallback() {
        let fake = sta_content::fake::db();
        let mut db = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { db = db.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_cruiser" {
                unit.raw = Arc::new(sta_odf::resolve("unit", &mut |_| Some(
                    "tooltip = \"GUI_BUILD_CRUISER\"\nverboseTooltip = \"GUI_BUILD_CRUISER_DETAILS\"".to_owned()
                )).unwrap());
            }
            db = db.unit(unit);
        }
        let f = feed(db.build().unwrap(), vec![unit(&fake, 2, "fx_yard", 1, 100.0, None)], &[2]);
        let buttons = grid(&f, Page::Build);
        assert_eq!(buttons[0].tip, "GUI_BUILD_CRUISER");
        assert_eq!(buttons[0].verbose.as_deref(), Some("GUI_BUILD_CRUISER_DETAILS"));
        assert_eq!(buttons[1].tip, "Build fx_frigate");
        assert!(buttons[1].verbose.as_deref().unwrap().starts_with("Cost:"));
    }

    const TREE: &str = "fx_cruiser 0\nfx_frigate 1 fx_station\nfx_yard 0\nfx_station 0\n";

    #[test]
    fn the_tech_tree_locks_a_button_until_its_prerequisite_is_finished() {
        let db = db_with(Some(TREE));
        let look = |f: &HudFeed, odf: &str| build_items(f, builder(f).unwrap()).into_iter().find(|i| i.odf == odf).map(|i| (i.look, i.why));
        let mut units = vec![unit(&db, 2, "fx_yard", 1, 100.0, None)];
        let f = feed(db_with(Some(TREE)), units.clone(), &[2]);
        // The cruiser has no prerequisite; the frigate wants a station.
        assert_eq!(look(&f, "fx_cruiser"), Some((Look::Ready, None)));
        assert_eq!(look(&f, "fx_frigate"), Some((Look::Locked, Some("needs fx_station".into()))));
        let g = grid(&f, Page::Build);
        assert_eq!(g[1].look, Look::Locked);
        assert!(g[1].verbose.as_deref().unwrap().starts_with("needs fx_station. Cost:"));
        // A station of ours still being welded does not count, nor does the rival's.
        units.push(unit(&db, 3, "fx_station", 1, 100.0, Some(0.5)));
        units.push(unit(&db, 5, "fx_station", 2, 100.0, None));
        let f = feed(db_with(Some(TREE)), units.clone(), &[2]);
        assert_eq!(look(&f, "fx_frigate").unwrap().0, Look::Locked);
        // Finished, it opens: what the sim's SIM-09 would accept.
        units[1].progress = None;
        let f = feed(db_with(Some(TREE)), units, &[2]);
        assert_eq!(look(&f, "fx_frigate"), Some((Look::Ready, None)));
        assert_eq!(owned_counts(&f).get("fx_station"), Some(&1));
    }

    #[test]
    fn poor_buttons_and_costs() {
        let mut db = ContentDb::builder();
        let fake = sta_content::fake::db();
        for w in fake.weapons() {
            db = db.weapon(w.clone());
        }
        for u in fake.units() {
            let mut u = u.clone();
            if u.odf == "fx_cruiser" {
                u.build.cost = vec![Fixed::from_int(600), Fixed::from_int(50)];
            }
            if u.odf == "fx_yard" {
                u.build.list.push(DefRef::new("fx_miner"));
            }
            db = db.unit(u);
        }
        let db = db.profile(fake.profile()).features(fake.features().clone()).build().unwrap();
        let f = feed(db, Vec::new(), &[]);
        let db = f.content.clone().unwrap();
        let f = HudFeed { view: Arc::new(WorldView { units: vec![unit(&db, 2, "fx_yard", 1, 100.0, None)], players: f.view.players.clone(), ..Default::default() }), selected: vec![EntityId(2)], ..f };
        let items = build_items(&f, builder(&f).unwrap());
        let by = |odf: &str| items.iter().find(|i| i.odf == odf).unwrap();
        // Stock is 20 latinum: the cruiser's 50 is out of reach, the frigate's 250 dilithium is not.
        assert_eq!(by("fx_cruiser").look, Look::Poor);
        assert_eq!(by("fx_frigate").look, Look::Ready);
        assert_eq!(cost_text(&by("fx_cruiser").cost, &ResourceModel::default()), "600 Dilithium, 50 Latinum");
        assert_eq!(cost_text(&by("fx_frigate").cost, &ResourceModel::default()), "250 Dilithium");
        assert_eq!(cost_text(&[0.0, 0.0], &ResourceModel::default()), "free");
    }

    #[test]
    fn the_strip_orders_the_stock_and_skips_what_the_race_lacks() {
        let f = feed(sta_content::fake::db(), Vec::new(), &[]);
        let r = resources(&f, [false; 4]);
        assert_eq!(r.iter().map(|e| (e.name.as_str(), e.text.as_str())).collect::<Vec<_>>(), [("Dilithium", "3000"), ("Metal", "700"), ("Latinum", "20"), ("Biomatter", "0"), ("officers", "3/10")]);
        let r = resources(&f, [false, true, false, true]);
        assert_eq!(r.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Dilithium", "Metal", "officers"]);
        // Unknown player: nothing.
        let mut f = f;
        f.player = PlayerId(9);
        assert!(resources(&f, [false; 4]).is_empty());
    }

    #[test]
    fn the_strip_and_costs_use_the_games_words() {
        // Fleet Operations: the latinum slot is the Tritanium, the biomatter slot the Supply,
        // and the metal slot (Collective Connections) is the Borg's, hidden for the rest.
        let db = sta_content::fake::db().with_resources(ResourceModel::for_profile(sta_content::Profile::Fo4));
        let f = feed(db, Vec::new(), &[]);
        let r = resources(&f, [false, false, true, false]);
        assert_eq!(r.iter().map(|e| (e.name.as_str(), e.id, e.text.as_str())).collect::<Vec<_>>(), [("Dilithium", 0, "3000"), ("Tritanium", 1, "20"), ("Supply", 3, "0"), ("officers", 4, "3/10")]);
        let names = resource_model(&f);
        assert_eq!(cost_text(&[261.0, 181.0, 0.0, 10.0], &names), "261 Dilithium, 181 Tritanium, 10 Supply");
    }
}
