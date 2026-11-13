//! Rule profiles and feature switches (S5). Seam types; CON-05 writes the
//! profile detection and the per-profile defaults table.

/// Which rule set the mounted content follows (docs/ARCHITECTURE.md,
/// "Content profiles").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Profile {
    /// STA64's own rules; new features start here.
    #[default]
    Native,
    /// Fleet Operations 4.
    Fo4,
    /// Stock Armada II.
    Sta2Classic,
    /// Armada I.
    Sta1Classic,
}

/// Rule switches, one per behaviour row of docs/NATIVE_FEATURES.md that the
/// sim implements as a rule. The profile sets the defaults; per-ODF keys
/// switch them per unit. All off by default.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct Features {
    /// Weapons, damage controls, normal weapon tech.
    pub weapon_damage_controls: bool,
    /// Point-defence cycles (numbered firing cycles).
    pub point_defense_cycles: bool,
    /// Weapons fire only inside their hardpoint's arc.
    pub fire_arcs: bool,
    /// Fore/aft/port/starboard shields instead of one pool.
    pub directional_shields: bool,
    /// Energy systems.
    pub energy_systems: bool,
    /// Turrets, linked hull turrets.
    pub turrets: bool,
    /// Team-change weapons.
    pub team_change_weapons: bool,
    /// Resources beyond dilithium (biomatter, latinum, metal).
    pub extra_resources: bool,
    /// Officer capacity per profile.
    pub officer_capacity: bool,
    /// Multi-yard builds and build queues.
    pub multi_yard_builds: bool,
    /// Construction rig: deploy, weld, reveal.
    pub construction_rigs: bool,
    /// Upgrade pods (to level 4), cocoon `EvolverClass`.
    pub upgrade_pods: bool,
    /// Hybrid research station / yard.
    pub hybrid_yards: bool,
    /// Breeders, swarm system, infestation.
    pub breeders: bool,
    /// Collective XP.
    pub collective_xp: bool,
    /// Squadrons.
    pub squadrons: bool,
    /// Station rotation.
    pub station_rotation: bool,
    /// Wreckage.
    pub wreckage: bool,
    /// Craft identity.
    pub craft_identity: bool,
    /// Cheats as sandbox commands.
    pub cheats: bool,
    /// Armada I compatibility: class labels, shields-then-death health model.
    pub a1_compat: bool,
    /// Resources are paid when a build is queued, not when it starts (Refit).
    pub pay_on_queue: bool,
    /// Point defence is a toggled special: off until switched on, it drains
    /// `specialEnergyCost` per second and switches itself off at zero energy
    /// (SIM-34; measured on stock A2 and Roots).
    pub pd_energy: bool,
    /// Point defence fires through one per-weapon timer that may go into debt
    /// (the Roots engine, SIM-34; measured). Off: no rate limit once
    /// `pd_energy` is on.
    pub pd_debt_timer: bool,
    /// Weapons start with a random initial reload and every shot reloads
    /// `shotDelay` plus 0 to 5 % jitter; special orders are refused while
    /// reloading (SIM-35; special launchers measured, regular weapons only
    /// read from the engine's code).
    pub weapon_shot_timing: bool,
    /// No weapon fires at a unit in the first two seconds of mission time
    /// (SIM-53; measured on stock A2 and Roots + STA2 Classic).
    pub start_fire_hold: bool,
}

impl Features {
    /// Every switch, in declaration order, for the content hash.
    pub(crate) fn bits(&self) -> [bool; 26] {
        [
            self.weapon_damage_controls,
            self.point_defense_cycles,
            self.fire_arcs,
            self.directional_shields,
            self.energy_systems,
            self.turrets,
            self.team_change_weapons,
            self.extra_resources,
            self.officer_capacity,
            self.multi_yard_builds,
            self.construction_rigs,
            self.upgrade_pods,
            self.hybrid_yards,
            self.breeders,
            self.collective_xp,
            self.squadrons,
            self.station_rotation,
            self.wreckage,
            self.craft_identity,
            self.cheats,
            self.a1_compat,
            self.pay_on_queue,
            self.pd_energy,
            self.pd_debt_timer,
            self.weapon_shot_timing,
            self.start_fire_hold,
        ]
    }
}
