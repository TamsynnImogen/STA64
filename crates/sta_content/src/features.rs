//! Rule profiles and feature switches (S5). Seam types; CON-05 writes the
//! profile detection and the per-profile defaults table.

/// Which rule set the mounted content follows (docs/ARCHITECTURE.md,
/// "Content profiles").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Profile {
    /// STA64's own rules; new features start here.
    #[default]
    Native,
    /// Fleet Operations 4 and the rest of the Roots lineage (STA2 Classic).
    Fo4,
    /// Fleet Operations 3.x (3.2.7) and its mods (Dominion War): the A2
    /// engine under the 3.x `FleetOpsHook.dll` (CON-44, D5).
    Fo3,
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
    /// A shield hit costs the target crew: `(damage - damageThreshold) x
    /// shieldCrewModifier` of the ordnance, a percent of maximum crew, or crew
    /// points on a unit with `saneCrewLossMethod` (SIM-39; measured on stock
    /// A2). Hull hits and system losses are not part of it.
    pub crew_loss_on_hit: bool,
    /// An attack approach stops `0.2 W + 5` inside weapon range, or 80 from
    /// 100 up (the Roots engine, SIM-36; read from the executable, measured).
    /// Off: `min(0.2 W + 5, 50)` (stock).
    pub attack_buffer_roots: bool,
    /// Inside `cfg_MIN_MOVE_WARP_DISTANCE` an attack approach slows to
    /// `combatSpeed` (and so leaves warp) when the target is slower (stock A2,
    /// SIM-36; read from the executable, measured). Off: only once in weapon
    /// range (Roots).
    pub attack_combat_speed: bool,
    /// A craft with an attack order fires at other enemies in range while its
    /// target cannot be hit (Roots, SIM-36; measured). Off: only at its target.
    pub attack_opportunity_fire: bool,
    /// A construction ship holds fire while it flies to a build site and
    /// builds (stock A2, SIM-36; measured). Off: it fires (Roots).
    pub builder_hold_fire: bool,
    /// A craft picks one target for all its weapons: the enemy in range with
    /// the highest `attackPower / current shields` (stock A2, SIM-36; read
    /// from the executable, measured). Off: each weapon takes the nearest.
    pub target_score: bool,
    /// An idle craft attacks whoever hit it or an ally in its sensor range in
    /// the last second, or joins an ally that fired (SIM-36; stock read from
    /// the executable and measured, Roots measured).
    pub idle_assist: bool,
    /// Only crafts at high movement autonomy assist (Roots, SIM-36; measured).
    pub assist_high_autonomy_only: bool,
    /// An ally that fired in the last second (not only one that was hit)
    /// draws an idle craft in (stock A2, SIM-36; read from the executable).
    /// Off: hits only (Roots).
    pub assist_on_ally_fire: bool,
}

impl Features {
    /// Every switch, in declaration order, for the content hash.
    pub(crate) fn bits(&self) -> [bool; 35] {
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
            self.crew_loss_on_hit,
            self.attack_buffer_roots,
            self.attack_combat_speed,
            self.attack_opportunity_fire,
            self.builder_hold_fire,
            self.target_score,
            self.idle_assist,
            self.assist_high_autonomy_only,
            self.assist_on_ally_fire,
        ]
    }
}
