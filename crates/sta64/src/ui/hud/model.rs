//! What the HUD shows, as plain data computed from the world (no Bevy UI):
//! the info panel, the command grid with its build page, and the resources
//! strip. The systems in `panels.rs` draw these and rebuild a panel only when
//! its model changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::Resource;
use sta_content::{Availability, ContentDb, Owned, ResourceModel, Unmet, UnitDef};
use sta_sim::command::{CommandKind, Order};
use sta_sim::world::EntityKind;
use sta_sim::movement::{FORM_COLUMN, FORM_LINE, FORM_WEDGE};
use sta_sim::production::view::{ProductionView, ResearchBlock};
use sta_sim::{ContentKey, EntityId, Fixed, PlayerId, UnitView, WorldView};

use crate::ui::i18n::{tr, trf, tooltip_key};
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
    /// Queues and production eligibility from the same tick as `view`.
    pub production: Vec<ProductionView>,
    /// The names the map gave its objects (host-only, see [`super::instance_names`]).
    pub names: super::instance_names::InstanceNames,
}

impl Default for HudFeed {
    fn default() -> Self {
        HudFeed { view: Arc::default(), player: PlayerId(0), selected: Vec::new(), content: None, production: Vec::new(), names: Default::default() }
    }
}

impl HudFeed {
    /// The selected units still in the world, in selection order.
    pub fn selected_units(&self) -> Vec<&UnitView> {
        self.selected.iter().filter_map(|&id| self.view.unit(id)).filter(|u| !u.flags.contains(sta_sim::view::UnitFlags::DYING)).collect()
    }

    pub(super) fn def(&self, u: &UnitView) -> Option<&UnitDef> {
        self.content.as_deref().and_then(|c| c.unit(u.def))
    }

    pub(super) fn can_trade(&self, unit: &UnitView) -> bool {
        self.content.as_deref().is_some_and(|db| {
            let Some(def) = db.unit(unit.def) else { return false };
            let race = self.view.attrs.races.binary_search_by_key(&unit.id, |row| row.0)
                .ok().map_or(def.race.as_str(), |i| self.view.attrs.races[i].1.as_str());
            db.can_trade(race)
        })
    }

    pub(super) fn cargo_ship(&self, unit: &UnitView) -> bool {
        unit.owner == self.player && unit.hull[0] > Fixed::ZERO && finished(unit)
            && self.can_trade(unit)
            && self.def(unit).is_some_and(|def| {
                def.trade_cargo_capacity.is_some_and(|max| max > Fixed::ZERO)
                    && def.raw.bool("cargo_ship") != Some(false)
            })
    }


    /// The name a script gave the unit (`WorldView::attrs::names`), else the one its map gave
    /// it, as written; the ODF caption and craft-name pool apply when neither exists.
    pub(crate) fn given_name(&self, u: &UnitView) -> Option<&str> {
        let set = &self.view.attrs.names;
        set.binary_search_by_key(&u.id, |n| n.0)
            .ok()
            .map(|i| set[i].1.as_str())
            .filter(|n| !n.trim().is_empty())
            .or_else(|| self.names.get(u.id))
    }

    pub(crate) fn name(&self, u: &UnitView) -> String {
        if let Some(n) = self.given_name(u) {
            return n.to_owned();
        }
        self.def(u).map_or_else(|| format!("#{}", u.id.0), |d| craft_names::instance(d, u.id))
    }

    /// The mouseover strip's name: the unit's given name (script-set, else the map's) when it has
    /// one, else the authored type caption (independent of craft name). Whether the original's
    /// strip shows a given name or the type caption is not established.
    pub(crate) fn unit_label(&self, u: &UnitView) -> String {
        if let Some(n) = self.given_name(u) {
            return n.to_owned();
        }
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

pub(super) fn frac(v: [Fixed; 2]) -> f32 {
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
    /// A completed unit with a build list, used by the alternate HUDs.
    pub builder: bool,
    /// Shipyards and starbases use Armada II's tall builder display.
    pub a2_yard: bool,
}

/// One tile of a group.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub id: EntityId,
    pub name: String,
    pub hull: f32,
    /// The shields' share over all facings; `None` without shields.
    pub shield: Option<f32>,
}

/// A unit's shields as one share over all facings, `None` without shields.
pub(super) fn shield_frac(u: &UnitView) -> Option<f32> {
    let (now, max) = u.shields.iter().fold((0.0, 0.0), |(n, m), s| (n + s[0].to_f32(), m + s[1].to_f32()));
    (max > 0.0).then(|| (now / max).clamp(0.0, 1.0))
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
            Info::Single(Single {
                id: u.id,
                name: feed.given_name(u).map_or(name, str::to_owned),
                captain,
                registry,
                class: def.map_or_else(String::new, |d| craft_names::unit_name(d).unwrap_or(&d.class.0).to_owned()),
                own: u.owner == feed.player,
                hull: frac(u.hull),
                hull_now: u.hull[0].to_f32().round() as i32,
                hull_max: u.hull[1].to_f32().round() as i32,
                shield: shield_frac(u),
                progress: u.progress.map(|p| p.to_f32().clamp(0.0, 1.0)),
                builder: def.is_some_and(|d| !d.build.list.is_empty()) && finished(u),
                a2_yard: def.is_some_and(|d| matches!(d.class.0.as_str(), "shipyard" | "starbase" | "buildyard")),
            })
        }
        many => Info::Group(many.iter().map(|u| Tile { id: u.id, name: feed.name(u), hull: frac(u.hull), shield: shield_frac(u) }).collect()),
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
    /// Armada I's Orders page: the alerts and orders submenu behind the Orders button.
    Orders,
}

/// What pressing a cell does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    OpenBuild,
    OpenFormations,
    OpenNativeMenu(u32),
    /// Armada I's Orders button: open the Orders page.
    OpenOrders,
    /// One of Armada I's Orders-page orders the sim takes.
    Order(UnitOrder),
    Back,
    /// Arm a map target; completed by the HUD's target click system.
    Target(TargetMode),
    /// An immediate supported special slot (including untargeted replace weapons).
    Special(u8),
    /// UI-02's stop on the own selection.
    Stop,
    /// Station crew recovery from the player's pool.
    Recrew,
    /// UI-02's formation on the own selection.
    Formation(u8),
    /// A loaded stock command ODF whose formation parameter has no verified sim mapping.
    NativeFormation(String),
    /// An immediate command identified by its authored stock ODF.
    NativeOrder(String),
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

/// The Armada I Orders-page orders that need no target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitOrder {
    SearchAndDestroy,
    Scout,
    /// A slot the stock grid has and the sim cannot take an order for yet: drawn locked.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Move,
    Attack,
    Mine,
    Rally,
    /// The stock COLONIZE command: pick a planet for the colony ships.
    Colonize,
    /// The stock TRANSPORT command: pick a ship or station to beam crew to or board.
    Transport,
    /// Roots TRANSPORT_ATTACK: the stock capture command uses eligible ready transporters.
    TransportAttack,
    /// The stock REPAIR command: pick a repair yard, or anywhere for the best one (SIM-40).
    Repair,
    /// The stock DECOMMISSION command: pick a yard, or anywhere for the best one or in place (SIM-45).
    Decommission,
    /// Select a trading station as a cargo ship's trade source.
    TradeSource,
    /// Select a trading station as a cargo ship's trade destination.
    TradeDestination,
    /// A replace weapon whose authored `needTarget` requires a map pick.
    Special(u8),
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

/// Live state of the shared point-defence slot. A mixed selection keeps its
/// per-unit toggle behaviour: the button reports the mixture rather than
/// claiming every unit is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointDefenceState {
    Off,
    On,
    Mixed,
}

/// Whether this selection can send the point-defence toggle, and what it shows.
/// The sim initializes an absent energy row to the unit's authored pool before
/// handling a special order, so use that same initial value here.
fn point_defence_state(feed: &HudFeed, slot: u8) -> Option<(PointDefenceState, bool)> {
    let db = feed.content.as_deref()?;
    let mut on = 0;
    let mut total = 0;
    let mut ready = false;
    for unit in feed.selected_units().into_iter().filter(|u| u.owner == feed.player) {
        let def = db.unit(unit.def)?;
        let (kind, weapon) = sta_sim::specials::specials_of(db, def).nth(slot as usize)?;
        if kind != sta_sim::specials::SpecialKind::PointDefence { return None; }
        let active = feed.view.point_defence_on(unit.id, slot);
        on += usize::from(active);
        total += 1;
        let energy = unit.energy.map(|pool| pool[0])
            .or_else(|| sta_sim::specials::energy_def(def).map(|pool| pool.max));
        ready |= active || sta_sim::specials::point_defence_has_energy(db, weapon, energy);
    }
    if total == 0 { return None; }
    let state = if on == 0 { PointDefenceState::Off }
        else if on == total { PointDefenceState::On }
        else { PointDefenceState::Mixed };
    Some((state, ready))
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
    /// Keep the text alongside a resolved icon when explicitly requested.
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
pub(super) fn tech_state(feed: &HudFeed, db: &ContentDb, race: &str, player: PlayerId) -> Owned {
    let mut state = Owned::new(&race.to_ascii_lowercase());
    let me = feed.view.player(player);
    for u in feed.view.units.iter().filter(|u| finished(u)) {
        let Some(def) = db.unit(u.def) else { continue };
        if u.owner == player {
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
    /// The build cell it sits in, counted over the cells the page offers (the cancel and back
    /// cells are not counted). Items follow each other, except that in a text tech tree an item
    /// that cannot be built (withheld or not listed) leaves its own cell empty and the others
    /// keep their positions (read from the executable, static, Armada I and II; the cell position
    /// is the item's place in the builder's list, not compared with the original: interim).
    /// Without a tree, with an XML tree, and for an item a requirement hides, items are packed
    /// (as before; not compared with the original).
    pub slot: usize,
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
    build_layout(feed, builder).0
}

/// The build cells with no item in a text tree (the cells of unbuildable items), below the last
/// cell the builder's list reaches. Empty without a text tree.
pub(super) fn build_holes(feed: &HudFeed, builder: &UnitView) -> Vec<usize> {
    let (items, cells) = build_layout(feed, builder);
    (0..cells).filter(|c| !items.iter().any(|i| i.slot == *c)).collect()
}

/// The items and the number of cells the list reaches (items plus the empty cells of a text tree).
fn build_layout(feed: &HudFeed, builder: &UnitView) -> (Vec<BuildItem>, usize) {
    let Some(db) = feed.content.as_deref() else { return (Vec::new(), 0) };
    let Some(def) = db.unit(builder.def) else { return (Vec::new(), 0) };
    let stock: Vec<f32> = feed.view.player(feed.player).map(|p| p.resources.iter().map(|f| f.to_f32()).collect()).unwrap_or_default();
    let poor = |cost: &[f32]| cost.iter().enumerate().any(|(i, c)| *c > stock.get(i).copied().unwrap_or(0.0));
    let research = feed.production.iter().find(|row| row.builder == builder.id).and_then(|row| row.research.as_ref());
    let item = |odf: &str, key: ContentKey, avail: Option<&Availability>, slot: usize| -> Option<BuildItem> {
        let u = db.unit(key)?;
        let cost: Vec<f32> = u.build.cost.iter().map(|c| c.to_f32()).collect();
        let (mut look, mut why) = match avail {
            None | Some(Availability::Available) => (if poor(&cost) { Look::Poor } else { Look::Ready }, None),
            Some(Availability::Locked { hidden: true, .. }) => return None,
            Some(Availability::Locked { unmet: m, .. }) => (Look::Locked, Some(m.iter().map(unmet).collect::<Vec<_>>().join(", "))),
            Some(Availability::Withheld) => (Look::Locked, Some("cannot be built".into())),
            Some(Availability::NotListed) => (Look::Locked, Some("not in the tech tree".into())),
        };
        if let Some(block) = research.and_then(|r| r.unavailable.iter().find(|(what, _)| *what == key).map(|(_, block)| block)) {
            let reason = match block {
                ResearchBlock::AlreadyResearched => "already researched",
                ResearchBlock::Queued => "research already queued",
                ResearchBlock::NoSlot => "no research slot available",
            };
            look = Look::Locked;
            why = Some(why.map_or_else(|| reason.to_owned(), |existing| format!("{existing}, {reason}")));
        }
        Some(BuildItem { odf: odf.to_owned(), key, look, cost, why, slot })
    };
    match db.tech_tree() {
        Some(tree) => {
            let state = tech_state(feed, db, &def.race, feed.player);
            let mut out: Vec<BuildItem> = Vec::new();
            let mut cells = 0;
            for o in tree.build_options(db, &def.odf, &state, false).iter() {
                if matches!(o.availability, Availability::Locked { hidden: true, .. }) {
                    continue;
                }
                // A text tree: what cannot be built keeps its cell, empty; the others keep theirs.
                if tree.is_text() && matches!(o.availability, Availability::Withheld | Availability::NotListed) {
                    cells += 1;
                    continue;
                }
                if let Some(b) = item(&o.def.odf, o.def.key, Some(&o.availability), cells) {
                    out.push(b);
                    cells += 1;
                }
            }
            (out, cells)
        }
        None => {
            let out: Vec<BuildItem> = def.build.list.iter().filter_map(|d| item(&d.odf, d.key, None, 0)).enumerate().map(|(i, mut b)| {
                b.slot = i;
                b
            }).collect();
            let n = out.len();
            (out, n)
        }
    }
}

/// The build buttons of a builder with the build cell (`BuildItem::slot`) each sits in; the
/// button's own `cell` is not set. The stock HUDs place them by that cell.
pub(super) fn build_item_buttons(feed: &HudFeed, b: &UnitView) -> Vec<(usize, Button)> {
    let items = build_items(feed, b);
    let names = resource_model(feed);
    let builder = feed.def(b);
    items
        .iter()
        .map(|it| {
            let definition = feed.content.as_deref().and_then(|db| db.unit(it.key));
            let site = builder.zip(definition).is_some_and(|(b, product)| EntityKind::build_requires_site(b, product));
            let (crew, officers) = definition.map_or((0.0, 0.0), |d| (d.crew.to_f32(), d.build.officers.to_f32()));
            let cost = build_cost_text(&it.cost, &names, crew, officers);
            let verbose = match &it.why {
                Some(w) => format!("{}. Cost: {cost}", w),
                None => format!("Cost: {cost}"),
            };
            let btn = Button {
                cell: 0,
                label: short(&it.odf),
                act: Act::Build { builder: b.id, what: it.key, odf: it.odf.clone(), site },
                look: it.look,
                tip: definition.and_then(|d| d.raw.str("tooltip")).map(str::to_owned).unwrap_or_else(|| format!("Build {}", it.odf)),
                verbose: Some(definition.and_then(|d| d.raw.str("verboseTooltip").or_else(|| d.raw.str("verbose"))).map_or(verbose, str::to_owned)),
                sprite_names: definition.map_or_else(Vec::new, |def| build_sprite_names(&def.model, &it.odf)),
                keep_label: false,
            };
            (it.slot, btn)
        })
        .collect()
}

/// What the Armada II stock tooltip of a build item is made of: the class's `tooltip`, `verboseTooltip`
/// (else `verbose`) and `hotkeyLabel` keys and its costs in the fixed order dilithium, metal, latinum, biomatter, crew,
/// officers (keys `GUI_CP_*_RES`; read from the executable, static; the composition measured at one window
/// size). The officer word is the builder's race's (interim: the builder's race stands in for the player's).
pub(super) fn build_tip_parts(feed: &HudFeed, builder: EntityId, what: ContentKey) -> Option<crate::ui::hud::stock::tooltip_text::BuildTipParts> {
    use crate::ui::hud::stock::tooltip_text::{BuildTipParts, CostPart};
    let db = feed.content.as_deref()?;
    let def = db.unit(what)?;
    let names = resource_model(feed);
    let key = |k: &str| def.raw.str(k).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
    let race = feed.view.unit(builder).and_then(|u| db.unit(u.def)).map_or_else(String::new, |d| d.race.to_ascii_lowercase());
    let officers = if race.contains("borg") {
        "GUI_CP_BORG_OFF_RES"
    } else if race.contains("rom") {
        "GUI_CP_ROM_OFF_RES"
    } else if race.contains("8472") {
        "GUI_CP_8472_OFF_RES"
    } else {
        "GUI_CP_OFFICERS_RES"
    };
    let slot = |slot: usize, key: &'static str| CostPart {
        key,
        fallback: names.name(slot).to_lowercase(),
        amount: def.build.cost.get(slot).map_or(0.0, |c| c.to_f32()),
    };
    Some(BuildTipParts {
        tooltip: key("tooltip").unwrap_or_else(|| format!("Build {}", def.odf)),
        verbose: key("verboseTooltip").or_else(|| key("verbose")),
        hotkey: key("hotkeyLabel"),
        cost: vec![
            slot(0, "GUI_CP_DILITHIUM_RES"),
            slot(2, "GUI_CP_METAL_RES"),
            slot(1, "GUI_CP_LATINUM_RES"),
            slot(3, "GUI_CP_BIOMATTER_RES"),
            CostPart { key: "GUI_CP_CREW_RES", fallback: "crew".into(), amount: def.crew.to_f32() },
            CostPart { key: officers, fallback: "officers".into() /* STA64's own fallback word */, amount: def.build.officers.to_f32() },
        ],
    })
}

fn cost_text(cost: &[f32], names: &ResourceModel) -> String {
    let parts: Vec<String> = cost.iter().enumerate().filter(|(_, c)| **c > 0.0).map(|(i, c)| format!("{} {}", *c as i64, names.name(i))).collect();
    if parts.is_empty() { "free".into() } else { parts.join(", ") }
}

fn build_cost_text(cost: &[f32], names: &ResourceModel, crew: f32, officers: f32) -> String {
    let resources = cost_text(cost, names);
    let mut parts = Vec::new();
    if resources != "free" {
        parts.push(resources);
    }
    if crew > 0.0 {
        parts.push(format!("{} crew", crew as i64));
    }
    if officers > 0.0 {
        parts.push(format!("{} officers", officers as i64));
    }
    if parts.is_empty() { "free".into() } else { parts.join(", ") }
}

/// A label that fits a cell (it wraps over two lines): the name in capitals, cut short.
pub fn short(name: &str) -> String {
    name.chars().take(12).collect::<String>().to_uppercase()
}

pub(super) fn button(cell: usize, label: &str, act: Act, tip: &str) -> Button {
    let (sprite_names, keep_label) = match &act {
        Act::OpenBuild => (vec!["b_build".into()], false),
        Act::OpenFormations => (vec!["b_form".into()], false),
        Act::Target(TargetMode::Move) => (vec!["b_move".into()], false),
        Act::Target(TargetMode::Attack) => (vec!["b_attack".into()], false),
        Act::Target(TargetMode::Rally) => (vec!["b_rally".into()], false),
        Act::Stop => (vec!["b_stop".into()], false),
        Act::Recrew => (vec!["b_recrew".into()], false),
        Act::Back => (vec!["b_back".into()], false),
        Act::Target(TargetMode::Mine) => (vec!["b_harvest".into()], false),
        Act::Target(TargetMode::Colonize) => (vec!["b_colonize".into()], false),
        Act::Target(TargetMode::Transport) => (vec!["b_transport".into()], false),
        Act::Target(TargetMode::TransportAttack) => (vec!["b_capture".into()], false),
        Act::Target(TargetMode::Repair) => (vec!["b_repair".into()], false),
        Act::Target(TargetMode::TradeSource) => (vec!["b_strade".into()], false),
        Act::Target(TargetMode::TradeDestination) => (vec!["b_etrade".into()], false),
        Act::Target(TargetMode::Decommission) => (vec!["b_decom".into()], false),
        Act::Target(TargetMode::Special(_)) => (Vec::new(), false),
        Act::Cancel(_) => (vec!["b_cancel".into()], false),
        Act::Formation(_) => (Vec::new(), false),
        // No verified native ID for these actions; use the existing label.
        Act::Special(_) | Act::Build { .. } | Act::OpenNativeMenu(_) | Act::NativeFormation(_) | Act::OpenOrders | Act::Order(_) | Act::NativeOrder(_) => (Vec::new(), false),
    };
    Button { cell, label: label.to_uppercase(), act, look: Look::Ready, tip: tip.to_owned(), verbose: None, sprite_names, keep_label }
}

/// A build icon follows the resolved model basename before the ODF stem.
pub(super) fn build_sprite_names(model: &str, odf: &str) -> Vec<String> {
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

/// The build buttons of the selected builder with the build cell each sits in (`BuildItem::slot`),
/// the buttons' own `cell` unset: the stock HUDs place them themselves.
pub(super) fn build_buttons_by_slot(feed: &HudFeed) -> Vec<(usize, Button)> {
    if feed.own_selection().is_empty() {
        return Vec::new();
    }
    let builder = commands::shared(feed).build.then(|| builder(feed)).flatten();
    builder.map(|b| build_item_buttons(feed, b)).unwrap_or_default()
}

/// The empty build cells of the selected builder's page (a text tree's unbuildable items).
pub(super) fn build_holes_of(feed: &HudFeed) -> Vec<usize> {
    if feed.own_selection().is_empty() {
        return Vec::new();
    }
    let builder = commands::shared(feed).build.then(|| builder(feed)).flatten();
    builder.map(|b| build_holes(feed, b)).unwrap_or_default()
}

/// How many empty build cells lie below a build button's own cell, given the empty cells
/// (`holes`): a stock palette that packs its buttons adds this to the button's place in the list.
/// A build button is found by its cell in the full-capacity grid. Zero for any other button.
pub(super) fn holes_before(button: &Button, holes: &[usize]) -> usize {
    let Act::Build { .. } = button.act else { return 0 };
    let slot = button.cell - usize::from(button.cell > CANCEL_CELL) - usize::from(button.cell > BACK_CELL);
    holes.iter().filter(|h| **h < slot).count()
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
        let mut out = Vec::new();
        // The capacity may be `usize::MAX` (the stock palettes), so the cells are walked, never collected.
        for (slot, mut btn) in build_item_buttons(feed, b) {
            let Some(cell) = (0..capacity).filter(|c| *c != CANCEL_CELL && *c != BACK_CELL).nth(slot) else { continue };
            btn.cell = cell;
            out.push(btn);
        }
        out.push(button(CANCEL_CELL, &tr("Cancel"), Act::Cancel(b.id), &tooltip_key("Cancel the queue")));
        out.push(button(BACK_CELL, &tr("Back"), Act::Back, &tooltip_key("Back")));
        out.sort_by_key(|b| b.cell);
        return out;
    }
    let mut out = Vec::new();
    let mut evolve_extra = false;
    if let Some(builder) = builder {
        // A class with the research key gets the Research menu button, while its
        // contents still use the build page.
        let key = |k: &str| feed.def(builder).is_some_and(|def| def.raw.bool(k) == Some(true));
        let embryo = feed.def(builder).is_some_and(UnitDef::is_evolver);
        let research = !embryo && key("research");
        // The evolve key gives an Evolve button by the same mechanism (own sprite name and
        // tooltip keys; it opens the same item page). Read from data: the keys; that they choose
        // the button is read from the executable, static; no frame of the original yet.
        // An evolve-only class has it in the opener's place; a class with a builder key keeps
        // Build and gets Evolve in the first free cell (below). INTERIM: the position of a second
        // menu button is STA64's choice, not determined for the original.
        // INTERIM: a class with the research key and the evolve key shows Research only; the
        // static reading says both would show, so this is STA64's choice.
        let evolve = embryo || (key("evolve") && !research);
        let builds = !embryo && (key("builder_ship") || key("builder_facility"));
        evolve_extra = evolve && builds;
        let caption = if research { "Research" } else if evolve && !builds { "Evolve" } else { "Build" };
        let mut opener = button(0, &tr(caption), Act::OpenBuild, &tooltip_key(if research { "Research menu" } else if caption == "Evolve" { "Evolve menu" } else { "Build" }));
        if research {
            opener.sprite_names = vec!["b_research".into()];
            opener.verbose = Some(tooltip_key("Open the research menu."));
        } else if caption == "Evolve" {
            opener = evolve_button(0);
        }
        out.push(opener);
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
    if caps.colonize {
        out.push(button(8, &tr("Colonize"), Act::Target(TargetMode::Colonize), &tooltip_key("Colonize")));
    }
    if caps.transport {
        let mut b = button(14, &tr("Transport"), Act::Target(TargetMode::Transport), &tooltip_key("Transport"));
        // Shown in its stock place but greyed when the sim would refuse (no transporter crew, Armada I).
        if !caps.transport_ready { b.look = Look::Locked; }
        out.push(b);
    }
    if caps.repair {
        // Cell 1 is the producers' Rally; a craft that can move is never one.
        out.push(button(1, &tr("Repair"), Act::Target(TargetMode::Repair), &tooltip_key("Repair at a shipyard")));
    }
    if caps.decommission {
        out.push(button(13, &tr("Decommission"), Act::Target(TargetMode::Decommission), &tooltip_key("Decommission")));
    }
    if caps.recrew {
        let mut b = button(9, &tr("Recrew"), Act::Recrew, &tooltip_key("Recrew"));
        if !caps.recrew_ready { b.look = Look::Locked; }
        out.push(b);
    }
    for (i, (slot, kind)) in caps.specials.iter().enumerate() {
        if let Some(cell) = [5, 10, 11, 12, 13, 14].get(i) {
            let weapon = selected_special_weapon(feed, *slot);
            let targeted = *kind == sta_sim::specials::SpecialKind::Replace
                && special_targeted(feed, *slot) == Some(true);
            if *kind == sta_sim::specials::SpecialKind::Replace && special_targeted(feed, *slot).is_none() { continue; }
            let act = if targeted { Act::Target(TargetMode::Special(*slot)) } else { Act::Special(*slot) };
            let mut b = if *kind == sta_sim::specials::SpecialKind::PointDefence {
                let Some((state, ready)) = point_defence_state(feed, *slot) else { continue };
                let status = match state {
                    PointDefenceState::Off => tr("Off"),
                    PointDefenceState::On => tr("On"),
                    PointDefenceState::Mixed => tr("Mixed"),
                };
                let label = trf("PD: {0}", &[&status]);
                let tip = trf("Point defence: {0}", &[&status]);
                let mut b = button(*cell, &label, act, &tip);
                if !ready { b.look = Look::Locked; }
                b
            } else if *kind == sta_sim::specials::SpecialKind::Cloak {
                button(*cell, &tr("Cloak"), act, &tooltip_key("Toggle cloak"))
            } else {
                let label = weapon.map_or("Replace", |w| w.odf.as_str());
                button(*cell, &tr(label), act, &tooltip_key(label))
            };
            if let Some(weapon) = weapon {
                if *kind == sta_sim::specials::SpecialKind::Replace {
                    let stem = weapon.odf.rsplit(['/', '\\']).next().unwrap_or(&weapon.odf).trim_end_matches(".odf");
                    b.sprite_names = vec![format!("b_{stem}"), stem.to_owned()];
                }
                if let Some(tip) = weapon.raw.str("tooltip") {
                    b.tip = tip.to_owned();
                }
                b.verbose = weapon.raw.str("verboseTooltip").or_else(|| weapon.raw.str("verbose")).map(str::to_owned);
            }
            out.push(b);
        }
    }
    if evolve_extra {
        let cell = (0..).find(|c| !out.iter().any(|b| b.cell == *c)).unwrap_or(0);
        out.push(evolve_button(cell));
    }
    out.sort_by_key(|b| b.cell);
    out
}

/// The Evolve menu button: it opens the same item page as Build (`Act::OpenBuild`).
fn evolve_button(cell: usize) -> Button {
    let mut b = button(cell, &tr("Evolve"), Act::OpenBuild, &tooltip_key("Evolve menu"));
    b.sprite_names = vec!["b_evolve".into()];
    b.verbose = Some(tooltip_key("Open the evolve menu."));
    b
}

/// A shared tooltip describes the slot when all selected owners use the same weapon.
fn selected_special_weapon(feed: &HudFeed, slot: u8) -> Option<&sta_content::WeaponDef> {
    let db = feed.content.as_deref()?;
    let mut weapons = feed.selected_units().into_iter()
        .filter(|u| u.owner == feed.player)
        .map(|u| {
            let def = feed.def(u)?;
            let (_, weapon) = sta_sim::specials::specials_of(db, def).nth(slot as usize)?;
            Some(weapon)
        });
    let weapon = weapons.next()??;
    weapons.all(|other| other.is_some_and(|w| w.key == weapon.key)).then_some(weapon)
}

/// A shared replace slot must agree on whether its weapons require a map pick.
pub(super) fn special_targeted(feed: &HudFeed, slot: u8) -> Option<bool> {
    let db = feed.content.as_deref()?;
    let mut units = feed.selected_units().into_iter().filter(|u| u.owner == feed.player);
    let first = units.next()?;
    let targeted = sta_sim::specials::specials_of(db, feed.def(first)?)
        .nth(slot as usize).filter(|(kind, _)| *kind == sta_sim::specials::SpecialKind::Replace)?
        .1.base.need_target == Some(true);
    units.all(|u| feed.def(u).and_then(|def| sta_sim::specials::specials_of(db, def).nth(slot as usize))
        .is_some_and(|(kind, weapon)| kind == sta_sim::specials::SpecialKind::Replace
            && (weapon.base.need_target == Some(true)) == targeted))
        .then_some(targeted)
}

/// The command a button's press puts in the queue, with the units it speaks
/// for (for the voice ack). `None` for the page buttons and for a build that
/// needs a site.
pub fn command(feed: &HudFeed, act: &Act) -> Option<(CommandKind, Vec<EntityId>)> {
    let own = feed.own_selection();
    let caps = commands::shared(feed);
    match act {
        Act::Order(UnitOrder::SearchAndDestroy) if caps.attack && caps.move_order => {
            Some((CommandKind::Order(Order::SearchAndDestroy { units: own.clone() }), own))
        }
        Act::Order(UnitOrder::Scout) if caps.move_order => Some((CommandKind::Order(Order::Scout { units: own.clone() }), own)),
        Act::OpenBuild | Act::OpenFormations | Act::OpenNativeMenu(_) | Act::OpenOrders | Act::Order(_) | Act::Back | Act::Target(_) | Act::NativeFormation(_) | Act::NativeOrder(_) => None,
        Act::Recrew => {
            let units = commands::recrew_stations(feed, &own);
            (!units.is_empty()).then(|| (CommandKind::Order(Order::Recrew { units: units.clone(), on: None }), units))
        }
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
        Act::Special(slot) if caps.specials.iter().any(|s| s.0 == *slot
            && (s.1 != sta_sim::specials::SpecialKind::PointDefence
                || point_defence_state(feed, *slot).is_some_and(|(_, ready)| ready))
            && (s.1 != sta_sim::specials::SpecialKind::Replace
                || special_targeted(feed, *slot) == Some(false))) => Some((
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
                    && feed.content.as_deref().is_some_and(|db| {
                        if !db.a1_officers() || db.unit(*what).is_none_or(|d| d.a1_officer_upgrade_gain.is_none()) {
                            return true;
                        }
                        db.unit(u.def).and_then(|d| d.a1_base_officers.as_ref())
                            .is_some_and(|policy| policy.upgrade == Some(*what))
                            && feed.production.iter().find(|row| row.builder == *b)
                                .and_then(|row| row.officer_upgrades.as_ref())
                                .is_some_and(|state| state.remaining > 0)
                    })
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
/// crew when initialized, then officers as "in use/capacity", or A1's
/// "available/capacity", subject to the race's authored hide flags.
pub fn resources(feed: &HudFeed, no: [bool; 4], no_personnel: [bool; 2]) -> Vec<ResEntry> {
    let Some(p) = feed.view.player(feed.player) else { return Vec::new() };
    let names = resource_model(feed);
    let mut out: Vec<ResEntry> = [0usize, 2, 1, 3]
        .into_iter()
        .filter(|&i| !no[i])
        .map(|i| ResEntry { name: names.name(i).to_owned(), id: i, text: (p.resources.get(i).map_or(0.0, |f| f.to_f32()) as i64).to_string() })
        .collect();
    if !no_personnel[0] && let Some(crew) = p.crew {
        out.push(ResEntry { name: "crew".into(), id: 5, text: (crew.to_bits() / Fixed::ONE.to_bits()).to_string() });
    }
    if !no_personnel[1] && (p.officers[1] > Fixed::ZERO || feed.content.as_deref().is_some_and(ContentDb::a1_officers)) {
        out.push(ResEntry { name: "officers".into(), id: 4, text: officer_text(feed, p.officers) });
    }
    out
}

pub(super) fn officer_text(feed: &HudFeed, [used, cap]: [Fixed; 2]) -> String {
    let shown = if feed.content.as_deref().is_some_and(ContentDb::a1_officers) {
        Fixed::from_bits(cap.to_bits().saturating_sub(used.to_bits()).max(0))
    } else {
        used
    };
    let whole = |value: Fixed| value.to_bits() / Fixed::ONE.to_bits();
    format!("{}/{}", whole(shown), whole(cap))
}

/// Everything the HUD draws, for change detection.
#[derive(Debug, Clone, PartialEq)]
pub struct Models {
    pub info: Info,
    pub grid: Vec<Button>,
    pub resources: Vec<ResEntry>,
}

pub fn models(feed: &HudFeed, page: Page, no: [bool; 4], no_personnel: [bool; 2]) -> Models {
    Models { info: info(feed), grid: grid(feed, page), resources: resources(feed, no, no_personnel) }
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
        HudFeed { view: Arc::new(view), player: ME, selected: selected.iter().map(|&i| EntityId(i)).collect(), content: Some(Arc::new(db)), ..Default::default() }
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
        let mut f = HudFeed { view: sim.view(), player: ME, selected: vec![id], content: Some(db.clone()), ..Default::default() };
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

    /// A name the map gave a unit shows on the selection card and the hover/tile name; a name a
    /// script set wins over it; both are shown as written (non-ASCII included, no panic) and a
    /// blank one is ignored. Invented names only.
    #[test]
    fn a_map_instance_name_is_preferred_to_the_caption_and_a_script_name_to_both() {
        use super::super::instance_names::InstanceNames;
        let db = sta_content::fake::db();
        let units = vec![unit(&db, 1, "fx_cruiser", 1, 80.0, None), unit(&db, 2, "fx_cruiser", 1, 80.0, None), unit(&db, 3, "fx_cruiser", 1, 80.0, None)];
        let mut f = feed(db, units, &[1]);
        let map = sta_bzn::Map {
            objects: ["Aurelia Test-7", "Ångström Ω-2 \u{1F680}", "   "]
                .iter()
                .enumerate()
                .map(|(i, n)| sta_bzn::MapObject { odf: "fx_cruiser".into(), label: format!("obj_{i}"), instance_name: (*n).into(), ..Default::default() })
                .collect(),
            ..Default::default()
        };
        f.names = InstanceNames::of(&map, [(EntityId(1), "obj_0"), (EntityId(2), "obj_1"), (EntityId(3), "obj_2")]);
        let one = |f: &HudFeed| match info(f) {
            Info::Single(c) => c.name,
            other => panic!("{other:?}"),
        };
        let u = |f: &HudFeed, id: u32| f.view.unit(EntityId(id)).unwrap().clone();
        assert_eq!(one(&f), "Aurelia Test-7");
        assert_eq!(f.unit_label(&u(&f, 1)), "Aurelia Test-7");
        assert_eq!(f.name(&u(&f, 2)), "Ångström Ω-2 \u{1F680}");
        // A blank map name leaves the ODF caption.
        assert_eq!(f.name(&u(&f, 3)), f.def(&u(&f, 3)).map(|d| craft_names::identity(d, EntityId(3)).name).unwrap());
        // A script-set name replaces the map's; an empty one does not hide it.
        Arc::make_mut(&mut f.view).attrs.names = vec![(EntityId(1), "Script Name".into()), (EntityId(2), "".into())];
        assert_eq!(one(&f), "Script Name");
        assert_eq!(f.unit_label(&u(&f, 1)), "Script Name");
        assert_eq!(f.name(&u(&f, 2)), "Ångström Ω-2 \u{1F680}");
        // The group tile uses the same name.
        f.selected = vec![EntityId(1), EntityId(2)];
        let Info::Group(tiles) = info(&f) else { panic!("group") };
        assert_eq!(tiles[0].name, "Script Name");
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
            [1, 2, 3, 6, 7, 13],
            "mobile armed craft (1: Repair, SIM-40; 13: Decommission, SIM-45)"
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
        assert!(wedge.sprite_names.is_empty());
        assert!(!wedge.keep_label);
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
    fn authored_research_menu_uses_research_sprite_and_keeps_build_routing() {
        let fake = sta_content::fake::db();
        let mut db = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { db = db.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_station" {
                unit.raw = Arc::new(sta_odf::resolve("unit", &mut |_| Some("research = 1".into())).unwrap());
            }
            db = db.unit(unit);
        }
        let mut planet = UnitDef::new("fx_planet", "planet");
        planet.build.list = vec![DefRef::new("fx_cruiser")];
        planet.raw = Arc::new(sta_odf::resolve("unit", &mut |_| Some("research = 1".into())).unwrap());
        db = db.unit(planet);
        let db = db.build().unwrap();
        for (odf, expected_label, expected_sprite) in [
            ("fx_station", "RESEARCH", "b_research"),
            ("fx_planet", "RESEARCH", "b_research"),
            ("fx_yard", "BUILD", "b_build"),
        ] {
            let f = feed(db.clone(), vec![unit(&db, 2, odf, 1, 100.0, None)], &[2]);
            let opener = grid(&f, Page::Top).into_iter().find(|b| b.act == Act::OpenBuild).unwrap();
            assert_eq!(opener.label, expected_label, "{odf}");
            assert_eq!(opener.tip, if expected_label == "RESEARCH" { "Research menu" } else { "Build" }, "{odf}");
            assert_eq!(opener.sprite_names, [expected_sprite], "{odf}");
            if expected_label == "RESEARCH" {
                let strings = crate::ui::i18n::Strings::from_game("english", r#""GUI_CP_RESEARCH_TOOLTIP", "Native short", "GUI_CP_RESEARCH_VTOOLTIP", "Native verbose","#);
                assert_eq!(strings.tooltip_text(&opener.tip), "Native short");
                assert_eq!(strings.tooltip_text(opener.verbose.as_deref().unwrap()), "Native verbose");
            }
            assert!(grid(&f, Page::Build).iter().any(|b| matches!(b.act, Act::Build { .. })), "{odf}");
        }
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
        // Mine shows when any selected unit can mine; the order goes to the miners only.
        assert!(
            grid(&f, Page::Top)
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

    /// The fake content with the yard and station given a positive speed (authored data),
    /// plus an authored construction rig.
    fn moving_stations_db() -> ContentDb {
        let fake = sta_content::fake::db();
        let mut db = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { db = db.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if matches!(unit.odf.as_str(), "fx_yard" | "fx_station") {
                unit.movement.max_speed = Fixed::from_int(20);
            }
            db = db.unit(unit);
        }
        let mut rig = UnitDef::new("fx_rig", "constructionrig");
        rig.movement.max_speed = Fixed::from_int(20);
        rig.build.list = vec![sta_content::DefRef::new("fx_station")];
        db.unit(rig).build().unwrap()
    }

    #[test]
    fn a_producer_station_with_a_physics_speed_queues_its_ships_in_place() {
        let db = moving_stations_db();
        for odf in ["fx_yard", "fx_station"] {
            let item = if odf == "fx_yard" { "fx_cruiser" } else { "fx_miner" };
            let f = feed(moving_stations_db(), vec![unit(&db, 2, odf, 1, 100.0, None)], &[2]);
            let what = f.content.as_ref().unwrap().key(item).unwrap();
            let button = grid(&f, Page::Build).into_iter().find(|b| matches!(b.act, Act::Build { .. })).unwrap();
            assert!(matches!(button.act, Act::Build { site: false, .. }), "{odf} must not arm a placement");
            assert_eq!(
                command(&f, &button.act),
                Some((CommandKind::Order(Order::Build { builder: EntityId(2), what, at: None }), vec![EntityId(2)]))
            );
            // The rally point stays offered on the top page.
            assert!(commands::shared(&f).rally, "{odf}");
            assert!(grid(&f, Page::Top).iter().any(|b| matches!(b.act, Act::Target(TargetMode::Rally))));
        }
    }

    #[test]
    fn a_construction_rig_still_arms_a_placement_for_its_stations() {
        let db = moving_stations_db();
        let f = feed(moving_stations_db(), vec![unit(&db, 3, "fx_rig", 1, 100.0, None)], &[3]);
        let button = grid(&f, Page::Build).into_iter().find(|b| matches!(b.act, Act::Build { .. })).unwrap();
        assert!(matches!(button.act, Act::Build { site: true, .. }));
        // A site press sends no order until the map is clicked.
        assert_eq!(command(&f, &button.act), None);
        assert!(!commands::shared(&f).rally);
    }

    /// Install-gated (`STA64_TEST_INSTALL`, optional `STA64_TEST_MOD`; skipped when unset), read
    /// from data: whatever the install, a builder whose list holds only craft queues in place and a
    /// builder whose list holds only stations places them on the map, judged by the listed items'
    /// kinds, not by class labels or speed.
    #[test]
    fn installed_builders_split_by_what_they_build() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let active = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
        let Ok(vfs) = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active.as_deref()) else { return };
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let Ok((db, _)) = ContentDb::load(&vfs, &index) else { return };
        let (mut queues, mut places) = (0, 0);
        for u in db.units().filter(|u| !u.build.list.is_empty() && !u.is_evolver()) {
            let kinds: Vec<EntityKind> = u.build.list.iter().filter_map(|r| db.unit(r.key)).map(EntityKind::of_def).collect();
            let crafts = kinds.iter().any(|k| *k == EntityKind::Craft);
            let stations = kinds.iter().any(|k| *k == EntityKind::Station);
            if crafts && !stations {
                assert!(u.build.list.iter().filter_map(|item| db.unit(item.key)).all(|product| !EntityKind::build_requires_site(u, product)), "{}: builds only craft, must queue in place", u.odf);
                queues += 1;
            } else if stations && !crafts && EntityKind::of_def(u) == EntityKind::Craft {
                assert!(u.build.list.iter().filter_map(|item| db.unit(item.key)).all(|product| EntityKind::build_requires_site(u, product)), "{}: a craft that builds only stations places them", u.odf);
                places += 1;
            }
        }
        assert!(queues > 0 && places > 0, "the install has queueing builders ({queues}) and placing builders ({places})");
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
        assert_eq!(top.iter().map(|b| b.cell).collect::<Vec<_>>(), [0, 1, 6, 13]);
        assert_eq!(top[0].sprite_names, ["b_build"]);
        assert_eq!(top[1].sprite_names, ["b_rally"]);
        assert_eq!(top[2].sprite_names, ["b_stop"]);
        assert_eq!(top[3].sprite_names, ["b_decom"]);
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
    fn research_refusals_lock_visible_build_items_without_moving_their_cells() {
        use sta_sim::production::view::ResearchView;

        let db = db_with(None);
        let mut f = feed(db.clone(), vec![unit(&db, 2, "fx_yard", 1, 100.0, None)], &[2]);
        let cruiser = db.units().find(|u| u.odf == "fx_cruiser").unwrap().key;
        for (block, reason) in [
            (ResearchBlock::AlreadyResearched, "already researched"),
            (ResearchBlock::Queued, "research already queued"),
            (ResearchBlock::NoSlot, "no research slot available"),
        ] {
            f.production = vec![ProductionView {
                builder: EntityId(2),
                jobs: Vec::new(),
                site: None,
                portal: None,
                research: Some(ResearchView { unavailable: vec![(cruiser, block)], pods: Vec::new(), active_axes: None }),
                officer_upgrades: None,
            }];
            let buttons = grid(&f, Page::Build);
            assert_eq!(buttons[0].cell, 0);
            assert_eq!(buttons[0].look, Look::Locked);
            assert!(buttons[0].verbose.as_deref().unwrap().contains(reason));
            assert_eq!(buttons[1].cell, 1);
            assert_eq!(buttons[1].look, Look::Ready);
        }
    }

    /// A text tree that leaves an item out empties that item's cell; the other cells keep their
    /// positions (see `BuildItem::slot`).
    #[test]
    fn a_text_tree_leaves_the_cell_of_an_unbuildable_item_empty() {
        let db = db_with(Some(TREE));
        let units = vec![unit(&db, 2, "fx_yard", 1, 100.0, None)];
        let omitting = "fx_yard 0\nfx_frigate 0\n";
        let f = feed(db_with(Some(omitting)), units.clone(), &[2]);
        let g = grid(&f, Page::Build);
        assert!(!g.iter().any(|b| b.cell == 0 && matches!(b.act, Act::Build { .. })), "the omitted cruiser's cell is empty");
        assert!(g.iter().any(|b| b.cell == 1 && matches!(b.act, Act::Build { .. })), "the frigate keeps its cell");
        let withheld = "fx_yard 0\nfx_frigate 0\nfx_cruiser -1\n";
        let f = feed(db_with(Some(withheld)), units.clone(), &[2]);
        let g = grid(&f, Page::Build);
        assert!(!g.iter().any(|b| b.cell == 0 && matches!(b.act, Act::Build { .. })));
        // The unrestricted tree is unchanged: both cells are there.
        let f = feed(db_with(Some(TREE)), units, &[2]);
        let g = grid(&f, Page::Build);
        assert!(g.iter().any(|b| b.cell == 0 && matches!(b.act, Act::Build { .. })));
        assert!(g.iter().any(|b| b.cell == 1 && matches!(b.act, Act::Build { .. })));
    }

    /// `db_with` with a yard that builds three things (cruiser, frigate, miner).
    fn wide_db(tree: &str) -> ContentDb {
        wide_db_of(tree, &["fx_cruiser", "fx_frigate", "fx_miner"])
    }

    /// `db_with` with a yard that builds the given things.
    fn wide_db_of(tree: &str, list: &[&str]) -> ContentDb {
        let fake = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            let mut u = u.clone();
            if u.odf == "fx_yard" {
                u.build.list = list.iter().map(|d| DefRef::new(*d)).collect();
            }
            b = b.unit(u);
        }
        b.tech_tree(TechTree::parse("t.tt", tree).0).build().unwrap()
    }

    const WIDE_ALL: &str = "fx_yard 0\nfx_cruiser 0\nfx_frigate 0\nfx_miner 0\n";
    /// The frigate, the middle item, is left out.
    const WIDE_HOLE: &str = "fx_yard 0\nfx_cruiser 0\nfx_miner 0\n";

    fn wide_feed(tree: &str) -> HudFeed {
        feed_of(wide_db(tree))
    }

    fn feed_of(db: ContentDb) -> HudFeed {
        let units = vec![unit(&db, 2, "fx_yard", 1, 100.0, None)];
        feed(db, units, &[2])
    }

    fn cells_of(buttons: &[Button]) -> Vec<(String, usize)> {
        buttons.iter().map(|b| (b.label.clone(), b.cell)).collect()
    }

    /// The build page with an unlimited capacity (what the stock palettes ask for) answers at once
    /// and keeps the hole rule: no allocation sized by the capacity.
    #[test]
    fn the_build_page_with_unlimited_capacity_keeps_the_hole() {
        let g = grid_with_capacity(&wide_feed(WIDE_HOLE), Page::Build, usize::MAX);
        let items: Vec<usize> = g.iter().filter(|b| matches!(b.act, Act::Build { .. })).map(|b| b.cell).collect();
        assert_eq!(items, vec![0, 2], "the frigate's cell 1 stays empty");
        let g = grid_with_capacity(&wide_feed(WIDE_ALL), Page::Build, usize::MAX);
        let items: Vec<usize> = g.iter().filter(|b| matches!(b.act, Act::Build { .. })).map(|b| b.cell).collect();
        assert_eq!(items, vec![0, 1, 2]);
    }

    /// Armada I's stock HUD places build buttons by their build cell: the omitted item's slot stays
    /// empty, the others keep theirs; an unrestricted tree fills the slots in order as before.
    #[test]
    fn the_armada_one_hud_leaves_the_omitted_items_slot_empty() {
        use crate::ui::hud::stock::a1::control;
        let slots = sta_content::ButtonSlots::from_header("int BUTTON_LOCATION_CANCEL = 4;\nint BUTTON_LOCATION_BACK = 5;\n").0;
        let build = |tree: &str| -> Vec<(String, usize)> {
            let b = control::buttons(&wide_feed(tree), Page::Build, &slots, None);
            cells_of(&b.iter().filter(|b| matches!(b.act, Act::Build { .. })).cloned().collect::<Vec<_>>())
        };
        // Free slots are 1, 2, 3, 6, ... (cancel 4, back 5): cells 0, 1, 2, 5.
        assert_eq!(build(WIDE_ALL).iter().map(|c| c.1).collect::<Vec<_>>(), vec![0, 1, 2]);
        let held = build(WIDE_HOLE);
        assert_eq!(held.iter().map(|c| c.1).collect::<Vec<_>>(), vec![0, 2], "slot of the frigate empty, miner keeps its slot");
        let all = build(WIDE_ALL);
        assert_eq!(held[0], all[0]);
        assert_eq!(held[1].1, all[2].1);
        // Cancel and back stay where the slot map puts them.
        let b = control::buttons(&wide_feed(WIDE_HOLE), Page::Build, &slots, None);
        assert!(b.iter().any(|b| b.cell == 3 && matches!(b.act, Act::Cancel(_))));
        assert!(b.iter().any(|b| b.cell == 4 && b.act == Act::Back));
    }

    /// The palette HUD (Armada II and Fleet Operations looks) packs its buttons in list order but
    /// leaves the omitted item's cell empty; unrestricted it packs exactly as before.
    #[test]
    fn the_palette_hud_leaves_the_omitted_items_cell_empty() {
        use crate::ui::hud::stock::{buttons, place_buttons};
        let native = crate::ui::hud::stock::commands::NativeCommands::default();
        let place = |tree: &str| -> Vec<(usize, String)> {
            let f = wide_feed(tree);
            let next = buttons(&f, Page::Build, &native);
            let holes = build_holes_of(&f);
            let p = place_buttons(&next, 20, 0, &holes);
            assert!(!p.paged);
            p.shown.iter().map(|(i, b)| (*i, b.label.clone())).collect()
        };
        let all = place(WIDE_ALL);
        // Unchanged from the packed order: three items, then cancel, then back.
        assert_eq!(all.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        let held = place(WIDE_HOLE);
        assert_eq!(held.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 2, 3, 4], "cell 1 is empty, the miner stays third");
        assert_eq!(held[0].1, all[0].1);
        assert_eq!(held[1].1, all[2].1);
    }

    /// The palette placement of the build page with the given command metadata: the list in
    /// order with the cell index each button takes, checked for the invariants that hold for every
    /// sort order (indices strictly increase along the list, so no two buttons share a cell).
    fn palette_placement(f: &HudFeed, native: &crate::ui::hud::stock::commands::NativeCommands, capacity: usize, page: usize) -> (Vec<(usize, Act)>, usize) {
        use crate::ui::hud::stock::{buttons, place_buttons};
        let next = buttons(f, Page::Build, native);
        let p = place_buttons(&next, capacity, page, &build_holes_of(f));
        let mut seen = std::collections::BTreeSet::new();
        for (i, _) in &p.shown {
            assert!(seen.insert(*i), "two buttons share cell {i}");
        }
        let pos: Vec<usize> = p.shown.iter().filter(|(_, b)| !p.paged || !matches!(b.act, Act::Back | Act::Cancel(_))).map(|(i, _)| *i).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "indices strictly increase along the list: {pos:?}");
        (p.shown.iter().map(|(i, b)| (*i, b.act.clone())).collect(), p.pages)
    }

    fn cell_of(placed: &[(usize, Act)], pick: impl Fn(&Act) -> bool) -> usize {
        placed.iter().find(|(_, a)| pick(a)).map(|(i, _)| *i).expect("button placed")
    }

    fn is_item(odf: &'static str) -> impl Fn(&Act) -> bool {
        move |a| matches!(a, Act::Build { odf: o, .. } if o == odf)
    }

    /// Command metadata that gives Cancel a high priority and a position, as an install's does:
    /// Cancel then sorts before the build buttons.
    fn native_with_cancel() -> crate::ui::hud::stock::commands::NativeCommands {
        use crate::ui::hud::stock::commands::{NativeCommand, NativeCommands};
        let mut native = NativeCommands::default();
        native.commands.insert("cancelbuild".into(), NativeCommand {
            odf: "cancelbuild.odf".into(), command: "NONE".into(), button: "b_cancel".into(),
            preferred: Some([0, 3]), menu: None, source: Vec::new(), param: None, tooltip: None, verbose: None,
            normal: None, hovered: None, pressed: None, disabled: None, active: None,
        });
        native
    }

    /// Cancel carries command metadata and sorts before the build buttons: with a hole, Cancel and
    /// a build item never share a cell, the hole is where the omitted item was, and the
    /// unrestricted list places as the plain list order.
    #[test]
    fn the_palette_with_cancel_metadata_keeps_cells_apart_and_the_hole_in_place() {
        let native = native_with_cancel();
        let (all, _) = palette_placement(&wide_feed(WIDE_ALL), &native, 20, 0);
        assert_eq!(all.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert!(matches!(all[0].1, Act::Cancel(_)), "Cancel sorts first with this metadata");
        let (held, _) = palette_placement(&wide_feed(WIDE_HOLE), &native, 20, 0);
        let (cancel, cruiser, miner) = (cell_of(&held, |a| matches!(a, Act::Cancel(_))), cell_of(&held, is_item("fx_cruiser")), cell_of(&held, is_item("fx_miner")));
        assert_eq!((cancel, cruiser, miner), (0, 1, 3), "the frigate's cell 2 is empty");
        assert_eq!(cell_of(&held, |a| *a == Act::Back), 4);
        assert!(!held.iter().any(|(i, _)| *i == 2));
    }

    /// A hole and paging together: the pages are counted over the cells with the hole, the hole
    /// stays on its page and the pinned Back and Cancel are on every page.
    #[test]
    fn a_hole_and_paging_work_together() {
        let list = ["fx_cruiser", "fx_frigate", "fx_miner", "fx_station", "fx_yard", "fx_moon"];
        // The frigate is left out: items on cells 0, 2, 3, 4, 5; capacity 7 leaves 3 cells a page.
        let f = feed_of(wide_db_of("fx_yard 0\nfx_cruiser 0\nfx_miner 0\nfx_station 0\nfx_moon 0\n", &list));
        let native = crate::ui::hud::stock::commands::NativeCommands::default();
        let (p0, pages) = palette_placement(&f, &native, 7, 0);
        assert_eq!(pages, 2);
        let items = |p: &[(usize, Act)]| p.iter().filter(|(_, a)| matches!(a, Act::Build { .. })).map(|(i, _)| *i).collect::<Vec<_>>();
        let pinned = |p: &[(usize, Act)]| p.iter().filter(|(_, a)| matches!(a, Act::Back | Act::Cancel(_))).map(|(i, _)| *i).collect::<Vec<_>>();
        assert_eq!(items(&p0), vec![0, 2], "page one: the cruiser, the empty cell 1, the miner");
        assert_eq!(pinned(&p0), vec![3, 4]);
        let (p1, _) = palette_placement(&f, &native, 7, 1);
        assert_eq!(items(&p1), vec![0, 1, 2], "page two: the next three items");
        assert_eq!(pinned(&p1), vec![3, 4]);
    }

    /// An omitted item after the last offered one pushes nothing: Cancel and Back follow the last
    /// offered item as if the list ended there (interim: not compared with the original).
    #[test]
    fn a_trailing_omitted_item_does_not_push_cancel_or_back() {
        let native = crate::ui::hud::stock::commands::NativeCommands::default();
        let (trailing, _) = palette_placement(&wide_feed("fx_yard 0\nfx_cruiser 0\nfx_frigate 0\n"), &native, 20, 0);
        assert_eq!(trailing.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        let a1 = sta_content::ButtonSlots::from_header("int BUTTON_LOCATION_CANCEL = 4;\nint BUTTON_LOCATION_BACK = 5;\n").0;
        let b = crate::ui::hud::stock::a1::control::buttons(&wide_feed("fx_yard 0\nfx_cruiser 0\nfx_frigate 0\n"), Page::Build, &a1, None);
        assert!(b.iter().any(|b| b.cell == 3 && matches!(b.act, Act::Cancel(_))) && b.iter().any(|b| b.cell == 4 && b.act == Act::Back));
    }

    /// Install-gated: the palette HUD's build page with the real command metadata of each install
    /// named by `STA64_TEST_A1_INSTALL`, `STA64_TEST_A2_STOCK_INSTALL`, `STA64_TEST_ROOTS_INSTALL`
    /// or `STA64_TEST_INSTALL` (read at run time, nothing committed). Only invariants are
    /// asserted: no two buttons share a cell, the unrestricted list places as the plain list
    /// order, and with a restricted tree the omitted item's cell is empty.
    #[test]
    fn installed_command_metadata_keeps_the_palette_cells_apart() {
        let vars = ["STA64_TEST_A1_INSTALL", "STA64_TEST_A2_STOCK_INSTALL", "STA64_TEST_ROOTS_INSTALL", "STA64_TEST_INSTALL"];
        let mut ran = 0;
        for var in vars {
            let Some(root) = std::env::var_os(var) else { continue };
            let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).unwrap();
            let native = crate::ui::hud::stock::commands::NativeCommands::load(&vfs);
            ran += 1;
            let (all, _) = palette_placement(&wide_feed(WIDE_ALL), &native, 40, 0);
            assert_eq!(all.iter().map(|c| c.0).collect::<Vec<_>>(), (0..all.len()).collect::<Vec<_>>(), "{var}: unrestricted is the plain list order");
            let (held, _) = palette_placement(&wide_feed(WIDE_HOLE), &native, 40, 0);
            let (cruiser, miner) = (cell_of(&held, is_item("fx_cruiser")), cell_of(&held, is_item("fx_miner")));
            let between = held.iter().filter(|(i, _)| *i > cruiser && *i < miner).count();
            // The omitted frigate's cell lies between them and holds nothing.
            assert_eq!(miner - cruiser - 1, between + 1, "{var}: exactly one empty cell between the cruiser and the miner");
            assert_eq!(held.len(), all.len() - 1, "{var}: the omitted item is gone, nothing else");
        }
        if ran == 0 {
            eprintln!("SKIP: set STA64_TEST_A1_INSTALL, STA64_TEST_A2_STOCK_INSTALL, STA64_TEST_ROOTS_INSTALL or STA64_TEST_INSTALL");
        }
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
        assert_eq!(build_cost_text(&[0.0, 0.0], &ResourceModel::default(), 37.0, 4.0), "37 crew, 4 officers");
        assert_eq!(build_cost_text(&[0.0, 0.0], &ResourceModel::default(), 0.0, 0.0), "free");
    }

    #[test]
    fn the_strip_orders_the_stock_and_skips_what_the_race_lacks() {
        let f = feed(sta_content::fake::db(), Vec::new(), &[]);
        let r = resources(&f, [false; 4], [false; 2]);
        assert_eq!(r.iter().map(|e| (e.name.as_str(), e.text.as_str())).collect::<Vec<_>>(), [("Dilithium", "3000"), ("Metal", "700"), ("Latinum", "20"), ("Biomatter", "0"), ("officers", "3/10")]);
        let r = resources(&f, [false, true, false, true], [false; 2]);
        assert_eq!(r.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Dilithium", "Metal", "officers"]);
        // Unknown player: nothing.
        let mut f = f;
        f.player = PlayerId(9);
        assert!(resources(&f, [false; 4], [false; 2]).is_empty());
    }

    #[test]
    fn the_strip_shows_live_personnel_and_honours_authored_hide_flags() {
        let f = feed(sta_content::fake::db(), Vec::new(), &[]);
        let mut view = (*f.view).clone();
        view.players[0].crew = Some(Fixed::from_int(16_777_217));
        view.players[0].officers = [Fixed::from_int(3), Fixed::from_int(10)];
        let f = HudFeed { view: Arc::new(view), ..f };
        let personnel = |hide| resources(&f, [false; 4], hide).into_iter()
            .filter(|e| e.id >= 4).map(|e| (e.id, e.name, e.text)).collect::<Vec<_>>();
        assert_eq!(personnel([false; 2]), [(5, "crew".into(), "16777217".into()), (4, "officers".into(), "3/10".into())]);
        assert_eq!(personnel([true, false]), [(4, "officers".into(), "3/10".into())]);
        assert_eq!(personnel([false, true]), [(5, "crew".into(), "16777217".into())]);
        assert!(personnel([true; 2]).is_empty());
        let mut view = (*f.view).clone();
        view.players[0].crew = None;
        let f = HudFeed { view: Arc::new(view), ..f };
        assert!(resources(&f, [false; 4], [false; 2]).iter().all(|e| e.id != 5));
    }

    #[test]
    fn a1_officer_strip_shows_available_capacity_without_changing_player_view() {
        let db = ContentDb::builder().profile(sta_content::Profile::Sta1Classic).build().unwrap();
        let f = feed(db, Vec::new(), &[]);
        assert_eq!(f.view.players[0].officers, [Fixed::from_int(3), Fixed::from_int(10)]);
        assert_eq!(resources(&f, [false; 4], [false; 2]).last().unwrap().text, "7/10");
        let mut view = (*f.view).clone();
        view.players[0].officers = [Fixed::from_int(12), Fixed::from_int(10)];
        let f = HudFeed { view: Arc::new(view), ..f };
        assert_eq!(resources(&f, [false; 4], [false; 2]).last().unwrap().text, "0/10");
        let mut view = (*f.view).clone();
        view.players[0].officers = [Fixed::ZERO, Fixed::ZERO];
        let f = HudFeed { view: Arc::new(view), ..f };
        assert_eq!(resources(&f, [false; 4], [false; 2]).last().unwrap().text, "0/0");
    }

    #[test]
    fn the_strip_and_costs_use_the_games_words() {
        // Fleet Operations: the latinum slot is the Tritanium, the biomatter slot the Supply,
        // and the metal slot (Collective Connections) is the Borg's, hidden for the rest.
        let db = sta_content::fake::db().with_resources(ResourceModel::for_profile(sta_content::Profile::Fo4));
        let f = feed(db, Vec::new(), &[]);
        let r = resources(&f, [false, false, true, false], [false; 2]);
        assert_eq!(r.iter().map(|e| (e.name.as_str(), e.id, e.text.as_str())).collect::<Vec<_>>(), [("Dilithium", 0, "3000"), ("Tritanium", 1, "20"), ("Supply", 3, "0"), ("officers", 4, "3/10")]);
        let names = resource_model(&f);
        assert_eq!(cost_text(&[261.0, 181.0, 0.0, 10.0], &names), "261 Dilithium, 181 Tritanium, 10 Supply");
    }

    /// A def with an item list and the given raw keys (authored data).
    fn menu_def(odf: &str, class: &str, keys: &str) -> UnitDef {
        let mut def = UnitDef::new(odf, class);
        def.build.list = vec![DefRef::new("fx_cruiser")];
        def.raw = Arc::new(sta_odf::resolve(odf, &mut |_| Some(keys.to_owned())).unwrap());
        def
    }

    fn menu_feed(units: Vec<UnitDef>, mobile_miner: bool) -> HudFeed {
        let fake = sta_content::fake::db();
        let mut db = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { db = db.weapon(weapon.clone()); }
        for unit in fake.units() {
            if !(mobile_miner && unit.odf == "fx_miner") { db = db.unit(unit.clone()); }
        }
        let first = units[0].odf.clone();
        for u in units { db = db.unit(u); }
        let db = db.build().unwrap();
        feed(db.clone(), vec![unit(&db, 2, &first, 1, 100.0, None)], &[2])
    }

    fn openers(f: &HudFeed) -> Vec<(usize, Vec<String>)> {
        grid(f, Page::Top).into_iter().filter(|b| b.act == Act::OpenBuild).map(|b| (b.cell, b.sprite_names)).collect()
    }

    #[test]
    fn the_evolve_key_gives_an_evolve_menu_button() {
        // Evolve-only: the Evolve button takes the opener's cell, same action as Build.
        let f = menu_feed(vec![menu_def("m_evolve", "station", "evolve = 1\n")], false);
        assert_eq!(openers(&f), [(0, vec!["b_evolve".to_string()])]);
        let b = grid(&f, Page::Top).into_iter().find(|b| b.act == Act::OpenBuild).unwrap();
        assert_eq!((b.label.as_str(), b.tip.as_str(), b.verbose.as_deref()), ("EVOLVE", "Evolve menu", Some("Open the evolve menu.")));
        let strings = crate::ui::i18n::Strings::from_game("english", r#""GUI_CP_EVOLVE_TOOLTIP", "s", "GUI_CP_EVOLVE_VTOOLTIP", "v","#);
        assert_eq!((strings.tooltip_text(&b.tip), strings.tooltip_text(b.verbose.as_deref().unwrap())), ("s", "v"));
        assert!(grid(&f, Page::Build).iter().any(|b| matches!(b.act, Act::Build { .. })), "the same item page");
        // Research keeps winning (one button).
        let f = menu_feed(vec![menu_def("m_evolve", "station", "evolve = 1\nresearch = 1\n")], false);
        assert_eq!(openers(&f), [(0, vec!["b_research".to_string()])]);
    }

    /// INTERIM placement: a builder with the evolve key keeps Build in the opener's cell and gets
    /// Evolve in the first free cell, never one a command already holds (a mobile builder that mines).
    #[test]
    fn a_builder_with_the_evolve_key_shows_both_buttons_in_distinct_cells() {
        let mut miner = sta_content::fake::db().units().find(|u| u.odf == "fx_miner").unwrap().clone();
        miner.build.list = vec![DefRef::new("fx_cruiser")];
        miner.raw = Arc::new(sta_odf::resolve("fx_miner", &mut |_| Some("builder_facility = 1\nevolve = 1\n".into())).unwrap());
        let f = menu_feed(vec![miner], true);
        let g = grid(&f, Page::Top);
        let mine = g.iter().find(|b| b.act == Act::Target(TargetMode::Mine)).expect("it mines").cell;
        let o = openers(&f);
        assert_eq!(o.len(), 2);
        assert_eq!(o[0], (0, vec!["b_build".to_string()]));
        assert_eq!(o[1].1, ["b_evolve"]);
        assert_ne!(o[1].0, mine);
        let mut cells: Vec<usize> = g.iter().map(|b| b.cell).collect();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells.len(), g.len(), "no two buttons share a cell");
        let others: Vec<usize> = g.iter().filter(|b| b.sprite_names != ["b_evolve"]).map(|b| b.cell).collect();
        assert_eq!(Some(o[1].0), (0..).find(|c| !others.contains(c)), "the first free cell");
    }

    /// Install-gated (`STA64_TEST_A2_STOCK_INSTALL`, read at run time, structure only): every class
    /// with an item list shows the menu button(s) its keys give, found by sprite name; research and
    /// planet classes, builder-ship and builder-facility classes and evolve classes exist, and the
    /// three sprite names resolve in a mounted GUI sprite table.
    #[test]
    fn installed_classes_show_the_menu_button_their_keys_give() {
        let Some(root) = std::env::var_os("STA64_TEST_A2_STOCK_INSTALL") else {
            eprintln!("SKIP: set STA64_TEST_A2_STOCK_INSTALL");
            return;
        };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (mut research, mut planet, mut ship, mut facility, mut evolve) = (0, 0, 0, 0, 0);
        for def in db.units().filter(|d| !d.build.list.is_empty()) {
            let key = |k: &str| def.raw.bool(k) == Some(true);
            let builds = key("builder_ship") || key("builder_facility");
            let expected: Vec<&str> = if key("research") {
                vec!["b_research"]
            } else if key("evolve") && builds {
                vec!["b_build", "b_evolve"]
            } else if key("evolve") {
                vec!["b_evolve"]
            } else {
                vec!["b_build"]
            };
            let f = feed(db.clone(), vec![unit(&db, 2, &def.odf, 1, 100.0, None)], &[2]);
            let got: Vec<String> = openers(&f).into_iter().flat_map(|(_, s)| s).collect();
            assert_eq!(got, expected, "{}", def.odf);
            research += usize::from(key("research") && def.planet_role.is_none());
            planet += usize::from(key("research") && def.planet_role.is_some());
            ship += usize::from(key("builder_ship") && !key("research"));
            facility += usize::from(key("builder_facility") && !key("research"));
            evolve += usize::from(key("evolve") && !key("research"));
        }
        assert!(research > 0 && planet > 0 && ship > 0 && facility > 0 && evolve > 0, "{research} {planet} {ship} {facility} {evolve}");
        let tables: Vec<_> = vfs
            .list("sprites")
            .into_iter()
            .filter(|p| p.rsplit('/').next().is_some_and(|f| f.starts_with("gui_") && f.ends_with(".spr")))
            .map(|p| crate::ui::gui_sprites::read_table(&vfs, &p))
            .collect();
        for name in ["b_build", "b_research", "b_evolve"] {
            assert!(tables.iter().any(|t| t.sprite(name).is_some() || t.sprite(&format!("{name}.0")).is_some()), "{name} resolves");
        }
    }
}
