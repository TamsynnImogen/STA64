//! The control-panel slot map of `game_cfg.h` (UI-19, D9 and D11 of ADR 0004).
//!
//! Armada I's control panel is a fixed grid of twelve slots (`controlButton1` to
//! `controlButton12` of the race's GUI cfg). `game_cfg.h` says which command sits in which
//! slot through its `BUTTON_LOCATION_*` keys: a slot number from 1, or -1 for "not shown".
//! Evidence: read from data (the Armada I install's `game_cfg.h`).
//!
//! Following D9 the keys are typed, not a free-form map: [`SlotCommand`] lists every key that
//! is read, a value that is not -1 or 1 to 12 is a [`Warning`] and leaves the key unset, and no
//! slot is invented for a key the header does not state. Following D11 the header is read as
//! the mod chain inherits it ([`crate::layered`]): a mod's `game_cfg.h` may include its own
//! name and state only what it changes. The map is presentation only (it is not sim-visible,
//! so it is not part of the D10 digest).

use std::collections::BTreeMap;

use sta_vfs::Vfs;

use crate::Warning;

/// The file the keys are read from.
pub const FILE: &str = "game_cfg.h";

/// How many slots the control panel has.
pub const SLOTS: u8 = 12;

/// A command with a `BUTTON_LOCATION_*` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotCommand {
    Transport,
    ResearchUpgrade,
    Move,
    Attack,
    Stop,
    Repair,
    Orders,
    StandDown,
    TransportSpecial,
    Harvest,
    Recrew,
    Upgrade,
    Build,
    RallyPoint,
    GreenAlert,
    YellowAlert,
    RedAlert,
    Patrol,
    Scout,
    SearchAndDestroy,
    Guard,
    Decommission,
    SelfDestruct,
    Cancel,
    Back,
}

impl SlotCommand {
    pub const ALL: [SlotCommand; 25] = [
        Self::Transport,
        Self::ResearchUpgrade,
        Self::Move,
        Self::Attack,
        Self::Stop,
        Self::Repair,
        Self::Orders,
        Self::StandDown,
        Self::TransportSpecial,
        Self::Harvest,
        Self::Recrew,
        Self::Upgrade,
        Self::Build,
        Self::RallyPoint,
        Self::GreenAlert,
        Self::YellowAlert,
        Self::RedAlert,
        Self::Patrol,
        Self::Scout,
        Self::SearchAndDestroy,
        Self::Guard,
        Self::Decommission,
        Self::SelfDestruct,
        Self::Cancel,
        Self::Back,
    ];

    /// The header key (the engine's own spelling, `DECOMMISION` included).
    pub const fn key(self) -> &'static str {
        match self {
            Self::Transport => "BUTTON_LOCATION_TRANSPORT",
            Self::ResearchUpgrade => "BUTTON_LOCATION_RESEARCH_UPGRADE",
            Self::Move => "BUTTON_LOCATION_MOVE",
            Self::Attack => "BUTTON_LOCATION_ATTACK",
            Self::Stop => "BUTTON_LOCATION_STOP",
            Self::Repair => "BUTTON_LOCATION_ORDER_REPAIR",
            Self::Orders => "BUTTON_LOCATION_ORDERS",
            Self::StandDown => "BUTTON_LOCATION_ORDER_STAND_DOWN",
            Self::TransportSpecial => "BUTTON_LOCATION_TRANSPORT_SPECIAL",
            Self::Harvest => "BUTTON_LOCATION_HARVEST",
            Self::Recrew => "BUTTON_LOCATION_RECREW",
            Self::Upgrade => "BUTTON_LOCATION_UPGRADE",
            Self::Build => "BUTTON_LOCATION_BUILD",
            Self::RallyPoint => "BUTTON_LOCATION_RALLY_POINT",
            Self::GreenAlert => "BUTTON_LOCATION_ORDER_GREEN_ALERT",
            Self::YellowAlert => "BUTTON_LOCATION_ORDER_YELLOW_ALERT",
            Self::RedAlert => "BUTTON_LOCATION_ORDER_RED_ALERT",
            Self::Patrol => "BUTTON_LOCATION_ORDER_PATROL",
            Self::Scout => "BUTTON_LOCATION_ORDER_SCOUT",
            Self::SearchAndDestroy => "BUTTON_LOCATION_ORDER_SEARCH_AND_DESTROY",
            Self::Guard => "BUTTON_LOCATION_ORDER_GUARD",
            Self::Decommission => "BUTTON_LOCATION_ORDER_DECOMMISION",
            Self::SelfDestruct => "BUTTON_LOCATION_ORDER_SELF_DESTRUCT",
            Self::Cancel => "BUTTON_LOCATION_CANCEL",
            Self::Back => "BUTTON_LOCATION_BACK",
        }
    }

    /// Whether the command lives on the Orders page (the alerts and orders submenu) rather
    /// than on the top page. `ORDER_REPAIR` and `ORDER_STAND_DOWN` are named like the Orders
    /// entries, but in the stock header their slots clash with Orders-page entries' slots and
    /// none of the top page's, so they are top page buttons (a reading of the stock header's
    /// slot assignment; evidence read from data).
    pub const fn on_orders_page(self) -> bool {
        matches!(
            self,
            Self::GreenAlert
                | Self::YellowAlert
                | Self::RedAlert
                | Self::Patrol
                | Self::Scout
                | Self::SearchAndDestroy
                | Self::Guard
                | Self::Decommission
                | Self::SelfDestruct
        )
    }
}

/// The slot of each command: 1 to 12, or hidden (-1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ButtonSlots {
    slots: BTreeMap<SlotCommand, i8>,
}

impl ButtonSlots {
    /// Read the keys of an expanded header text. A later assignment replaces an earlier one.
    pub fn from_header(text: &str) -> (Self, Vec<Warning>) {
        Self::parse_header(text, false)
    }

    /// Armada I's stock values for an A1Compat mod whose A2/FO parent has no
    /// `BUTTON_LOCATION_*` keys. Explicit mod values, including -1 and invalid
    /// values, take precedence. The original Armada I executable reads these
    /// keys and its stock `game_cfg.h` supplies these values (read from data and
    /// the executable, static).
    pub fn a1_compat_from_header(text: &str) -> (Self, Vec<Warning>) {
        Self::parse_header(text, true)
    }

    fn parse_header(text: &str, a1_compat: bool) -> (Self, Vec<Warning>) {
        let mut found: BTreeMap<String, String> = BTreeMap::new();
        for (name, value, _) in crate::layered::scan(text) {
            found.insert(name, value);
        }
        let mut out = Self::default();
        let mut warnings = Vec::new();
        // Ordered to match SlotCommand::ALL. These are the stock Armada I
        // game_cfg.h values, not values copied from any game file at runtime.
        const A1_SLOTS: [i8; 25] = [
            -1, -1, 1, 2, 3, 4, 5, 6, 8, 7, 7, 9, 10, 11, 1, 2, 3, 4, 5, 6, 7, 10, 11, 11, 12,
        ];
        for (command, stock_slot) in SlotCommand::ALL.into_iter().zip(A1_SLOTS) {
            let Some(value) = found.get(command.key()) else {
                if a1_compat {
                    out.slots.insert(command, stock_slot);
                }
                continue;
            };
            match value.trim().parse::<i32>() {
                Ok(n) if n == -1 || (1..=i32::from(SLOTS)).contains(&n) => {
                    out.slots.insert(command, n as i8);
                }
                _ => {
                    if a1_compat {
                        // An authored but invalid value must not silently turn
                        // into a stock button in the compatibility profile.
                        out.slots.insert(command, -1);
                    }
                    warnings.push(Warning::new(
                        FILE,
                        format!("{} is not -1 or a slot from 1 to {SLOTS}; treated as unset", command.key()),
                    ));
                }
            }
        }
        (out, warnings)
    }

    /// Read `game_cfg.h` as the mod chain inherits it (D11). An include cycle is an error.
    pub fn load(vfs: &Vfs) -> Result<(Self, Vec<Warning>), crate::ContentError> {
        Self::load_with(vfs, false)
    }

    /// Load the mounted header with Armada I stock values only for missing
    /// keys. Call this only for an A1Compat mount; native A1 stays data-only.
    pub fn load_a1_compat(vfs: &Vfs) -> Result<(Self, Vec<Warning>), crate::ContentError> {
        Self::load_with(vfs, true)
    }

    fn load_with(vfs: &Vfs, a1_compat: bool) -> Result<(Self, Vec<Warning>), crate::ContentError> {
        let Some(resolved) = crate::layered::resolve(vfs, FILE, 0).map_err(|e| crate::ContentError::IncludeCycle(e.0))? else {
            return Ok((if a1_compat { Self::a1_compat_from_header("").0 } else { Self::default() }, Vec::new()));
        };
        let mut warnings = resolved.warnings;
        let (slots, parsed) = Self::parse_header(&resolved.text, a1_compat);
        warnings.extend(parsed);
        Ok((slots, warnings))
    }

    /// Set a slot (tests and content built in code); `None` for hidden.
    pub fn with(mut self, command: SlotCommand, slot: Option<u8>) -> Self {
        self.slots.insert(command, slot.filter(|s| (1..=SLOTS).contains(s)).map_or(-1, |s| s as i8));
        self
    }

    /// The slot (1 to 12) of a command, `None` when hidden or not stated.
    pub fn slot(&self, command: SlotCommand) -> Option<u8> {
        self.slots.get(&command).copied().filter(|s| *s > 0).map(|s| s as u8)
    }

    /// Whether the header states any key.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The commands of a page, by slot. Several commands may share a slot (Harvest and Recrew
    /// do, as they apply to different units); they come in [`SlotCommand`] order.
    pub fn page(&self, orders: bool) -> Vec<(u8, SlotCommand)> {
        let mut out: Vec<(u8, SlotCommand)> = self
            .slots
            .iter()
            .filter(|(c, s)| **s > 0 && c.on_orders_page() == orders)
            .map(|(c, s)| (*s as u8, *c))
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "// authored for the test\n\
        int BUTTON_LOCATION_MOVE = 6;\n\
        int BUTTON_LOCATION_ATTACK\t\t= 4; // trailing comment\n\
        int BUTTON_LOCATION_UPGRADE = -1;\n\
        int BUTTON_LOCATION_ORDERS = 12;\n\
        int BUTTON_LOCATION_ORDER_SCOUT = 9;\n\
        int BUTTON_LOCATION_ORDER_GUARD = 13;\n\
        int BUTTON_LOCATION_STOP = 7;\n\
        int BUTTON_LOCATION_STOP = 8;\n";

    #[test]
    fn typed_keys_slots_hidden_and_bad_values() {
        let (s, w) = ButtonSlots::from_header(HEADER);
        assert_eq!(s.slot(SlotCommand::Move), Some(6));
        assert_eq!(s.slot(SlotCommand::Attack), Some(4));
        assert_eq!(s.slot(SlotCommand::Upgrade), None, "-1 is hidden");
        assert_eq!(s.slot(SlotCommand::Orders), Some(12));
        assert_eq!(s.slot(SlotCommand::Guard), None, "13 is out of range");
        assert_eq!(s.slot(SlotCommand::Stop), Some(8), "a later assignment wins");
        assert_eq!(s.slot(SlotCommand::Build), None, "a key the header does not state is unset");
        assert_eq!(w.len(), 1);
        assert!(w[0].message.contains("BUTTON_LOCATION_ORDER_GUARD"), "{w:?}");
    }

    #[test]
    fn pages_split_by_command_and_sort_by_slot() {
        let (s, _) = ButtonSlots::from_header(HEADER);
        assert_eq!(
            s.page(false),
            [(4, SlotCommand::Attack), (6, SlotCommand::Move), (8, SlotCommand::Stop), (12, SlotCommand::Orders)]
        );
        assert_eq!(s.page(true), [(9, SlotCommand::Scout)]);
    }

    #[test]
    fn every_key_is_distinct_and_prefixed() {
        let mut keys: Vec<_> = SlotCommand::ALL.iter().map(|c| c.key()).collect();
        assert!(keys.iter().all(|k| k.starts_with("BUTTON_LOCATION_")));
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), SlotCommand::ALL.len());
    }

    #[test]
    fn a_mod_header_inherits_the_layer_below() {
        let d = std::env::temp_dir().join(format!("sta_slots_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (p, text) in [
            ("Data/game_cfg.h", "int BUTTON_LOCATION_MOVE = 6;\nint BUTTON_LOCATION_ATTACK = 4;\n"),
            ("Data/Mods/Top/info.ini", "[mod]\nstandalone=0\n"),
            ("Data/Mods/Top/game_cfg.h", "#include \"game_cfg.h\"\nint BUTTON_LOCATION_ATTACK = 10;\n"),
        ] {
            let p = d.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let vfs = Vfs::mount_install(&d, Some("Top")).unwrap();
        let (s, w) = ButtonSlots::load(&vfs).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((s.slot(SlotCommand::Move), s.slot(SlotCommand::Attack)), (Some(6), Some(10)));
    }

    #[test]
    fn a1_compat_fills_only_absent_keys() {
        let (slots, warnings) = ButtonSlots::a1_compat_from_header(
            "int BUTTON_LOCATION_MOVE = 9;\n\
             int BUTTON_LOCATION_ATTACK = -1;\n\
             int BUTTON_LOCATION_STOP = 99;\n\
             int BUTTON_LOCATION_ORDER_SCOUT = 8;\n",
        );
        assert_eq!(slots.slot(SlotCommand::Move), Some(9), "mod override wins");
        assert_eq!(slots.slot(SlotCommand::Attack), None, "authored -1 stays hidden");
        assert_eq!(slots.slot(SlotCommand::Stop), None, "invalid authored slot does not get a stock button");
        assert_eq!(slots.slot(SlotCommand::Orders), Some(5), "absent key gets the stock slot");
        assert_eq!(slots.slot(SlotCommand::Scout), Some(8), "orders-page override wins");
        assert_eq!(slots.slot(SlotCommand::Back), Some(12));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("BUTTON_LOCATION_STOP"));
        assert_eq!(ButtonSlots::from_header("").0, ButtonSlots::default(), "native A1 still requires its own data");
    }

    #[test]
    fn a1_compat_loads_stock_slots_when_fo_parent_has_no_location_keys() {
        let d = std::env::temp_dir().join(format!("sta_slots_a1compat_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (p, text) in [
            ("Data/game_cfg.h", "int SOME_FO_SETTING = 1;\n"),
            ("Data/Mods/A1Compat/info.ini", "[Mod]\nName=A1Compat\n"),
        ] {
            let p = d.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let vfs = Vfs::mount_install(&d, Some("A1Compat")).unwrap();
        let native = ButtonSlots::load(&vfs).unwrap().0;
        let (compat, warnings) = ButtonSlots::load_a1_compat(&vfs).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(native.is_empty());
        assert_eq!(compat.slot(SlotCommand::Move), Some(1));
        assert_eq!(compat.slot(SlotCommand::Stop), Some(3));
        assert_eq!(compat.slot(SlotCommand::Orders), Some(5));
        assert_eq!(compat.slot(SlotCommand::Back), Some(12));
        assert_eq!(compat.slot(SlotCommand::Transport), None);
    }
}
