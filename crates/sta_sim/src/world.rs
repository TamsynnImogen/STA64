//! The entity model (SIM-01): every thing in the world, in one store ordered
//! by id.
//!
//! - [`Entities`] keeps them sorted by ascending [`EntityId`]. Ids are handed
//!   out in order and never reused, so iteration order is spawn order and the
//!   same on every peer.
//! - [`Entity`] holds what every kind shares and what views show: kind,
//!   definition, owner, kinematics, hull, shields, flags, build progress. All
//!   of it is [`Fixed`] or integers.
//! - State only some entities have (weapon cooldowns, cargo, build queues,
//!   waypoints, cloak timers) goes in a [`Table`] owned by the package that
//!   needs it, held beside `Sim::units` and cleared in `Sim::despawn`, so
//!   this struct does not grow a field per system.

use sta_content::{ClassLabel, ContentKey, Features, MoveStats, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use crate::view::{UnitFlags, UnitView};

/// Owner of things no player owns (moons, props). Not a player slot, so
/// [`PlayerId::is_player`] is false; nobody may order them.
pub const NEUTRAL: PlayerId = PlayerId(253);

/// What sort of thing an entity is. Decides which systems touch it; the
/// definition says the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum EntityKind {
    /// Ships: anything that moves under orders.
    Craft = 0,
    /// Starbases, yards, research and mining stations: built, never move.
    Station = 1,
    /// Moons and other things that are mined.
    Resource = 2,
    /// Torpedoes and other shots in flight (SIM-04 spawns them).
    Projectile = 3,
    /// Neutral fixtures the map places that are none of the above
    /// (background objects, wormholes, scrap).
    Prop = 4,
}

impl EntityKind {
    /// Shared mining admission. Background planets have no resource interface;
    /// producer planets require an explicitly resolved resource. Other nodes
    /// retain their existing SIM-18 policy.
    pub fn mineable_def(def: &UnitDef) -> bool {
        Self::of_def(def) == Self::Resource
            && match def.planet_role {
                Some(sta_content::PlanetRole::Background) => false,
                Some(sta_content::PlanetRole::Producer) => def.resource.is_some(),
                None => true,
                _ => false,
            }
    }

    /// Stationary production is independent of a planet's body/resource kind.
    pub fn producer_def(def: &UnitDef) -> bool {
        Self::of_def(def) == Self::Station
            || def.planet_role == Some(sta_content::PlanetRole::Producer)
    }

    /// The kind of a definition: by [`EntityKind::of`], except that a fixture of an unlisted
    /// class that carries a `resource` is a resource node (Fleet Ops' moons are class
    /// `scrap`, which also covers plain wrecks). A craft or station with a `resource` stays
    /// what its class says (SIM-18).
    pub fn of_def(def: &sta_content::UnitDef) -> EntityKind {
        match EntityKind::of(&def.class, &def.movement) {
            EntityKind::Prop if def.resource.is_some() => EntityKind::Resource,
            kind => kind,
        }
    }

    /// Whether the player never sees `def`: its ODF says `hidden = 1`.
    ///
    /// Fleet Operations and Dominion War mark their engine helpers this way:
    /// the start pods and faction objects a race's `standardUnits` place
    /// (`all_ai_pod`, `all_start`, `fed_effectsS`, ...: `classLabel = "pod"`,
    /// `basename = "effect_empty"`, `mapicon = "mapicon_empty"`), the pods
    /// their start weapons spawn (`unitcap_pod`, `ai_pod_*`) and map triggers
    /// (`trigger_derelictcreation`). Such an object exists in the world
    /// (scripts, weapons and the tech tree see it) but is not the player's to
    /// pick, so the view leaves it out of every visible set
    /// ([`crate::Sim`]'s view). Their lifecycle (expiry, replacement) is
    /// `start_effects.rs`. Some `hidden = 1` effects do have models (FO's
    /// `effect_mstorm`); none reach the sim yet.
    pub fn hidden(def: &sta_content::UnitDef) -> bool {
        def.hidden
    }

    /// The kind for a definition, from its ODF class (docs/data/odf_classes.md).
    /// Unlisted classes that can move count as craft, the rest as props.
    pub fn of(class: &ClassLabel, movement: &MoveStats) -> EntityKind {
        match class.0.as_str() {
            "craft" | "freighter" | "constructionrig" | "wingman" | "repairship" | "cargo_ship"
            | "evolver" | "scavenger" | "fighter" => EntityKind::Craft,
            "starbase" | "shipyard" | "buildyard" | "research" | "mining" | "p_mining"
            | "sensor" | "colony" | "trading_station" => EntityKind::Station,
            "planet" | "planet_obj" | "moon" | "latinum" | "asteroid" => EntityKind::Resource,
            _ if movement.max_speed > Fixed::ZERO => EntityKind::Craft,
            _ => EntityKind::Prop,
        }
    }
}

/// Shield strength now and at most. With directional shields each facing
/// (fore, aft, port, starboard) is its own pool of `max`; without, only
/// `now[0]` is used and views show it on all four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shields {
    pub now: [Fixed; 4],
    pub max: Fixed,
    pub directional: bool,
}

impl Shields {
    /// Charged to `max`.
    pub fn full(max: Fixed, directional: bool) -> Shields {
        let now = if directional {
            [max; 4]
        } else {
            [max, Fixed::ZERO, Fixed::ZERO, Fixed::ZERO]
        };
        Shields {
            now,
            max,
            directional,
        }
    }

    /// `[now, max]` per facing, as [`UnitView::shields`] has it.
    pub fn view(&self) -> [[Fixed; 2]; 4] {
        if self.directional {
            self.now.map(|n| [n, self.max])
        } else {
            [[self.now[0], self.max]; 4]
        }
    }
}

/// One thing in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// Set by [`Entities::spawn`].
    pub id: EntityId,
    pub kind: EntityKind,
    /// [`ContentKey::NONE`] for entities not made from content (the legacy
    /// `Sim::spawn`).
    pub def: ContentKey,
    pub owner: PlayerId,
    pub pos: Vec3,
    /// Unit vector in the XZ plane: `(x, z)`.
    pub heading: (Fixed, Fixed),
    /// Units per second along the heading.
    pub speed: Fixed,
    /// Current turn rate, radians per second; positive turns from +X toward +Z.
    pub omega: Fixed,
    /// Where it is moving to.
    pub target: Option<Vec3>,
    /// Copied from the definition at spawn, so movement never looks it up.
    pub stats: MoveStats,
    /// `[now, max]`.
    pub hull: [Fixed; 2],
    pub shields: Shields,
    /// Current and maximum crew; unknown for bare or legacy entities.
    pub crew: Option<[Fixed; 2]>,
    pub flags: UnitFlags,
    /// Building or under construction, 0..1.
    pub progress: Option<Fixed>,
}

/// Normalise an XZ heading; a zero vector faces +Z.
pub fn unit_heading(h: (Fixed, Fixed)) -> (Fixed, Fixed) {
    let len = (h.0 * h.0 + h.1 * h.1).sqrt();
    if len == Fixed::ZERO {
        (Fixed::ZERO, Fixed::ONE)
    } else {
        (h.0 / len, h.1 / len)
    }
}

impl Entity {
    /// At rest, no hull or shields, not yet given an id.
    pub fn new(
        kind: EntityKind,
        def: ContentKey,
        owner: PlayerId,
        pos: Vec3,
        heading: (Fixed, Fixed),
        stats: MoveStats,
    ) -> Entity {
        Entity {
            id: EntityId::NONE,
            kind,
            def,
            owner,
            pos,
            heading: unit_heading(heading),
            speed: Fixed::ZERO,
            omega: Fixed::ZERO,
            target: None,
            stats,
            hull: [Fixed::ZERO; 2],
            shields: Shields::default(),
            crew: None,
            // Shots in flight are the only thing a player cannot pick.
            flags: if kind == EntityKind::Projectile {
                UnitFlags::default()
            } else {
                UnitFlags::SELECTABLE
            },
            progress: None,
        }
    }

    /// Made from `def`, at its authored starting hull and shields (full if absent).
    pub fn from_def(
        def: &UnitDef,
        features: &Features,
        owner: PlayerId,
        pos: Vec3,
        heading: (Fixed, Fixed),
    ) -> Entity {
        let kind = EntityKind::of_def(def);
        // Generic resources stay neutral (SIM-18). Typed planets have their
        // own ownership policy, independent of their body/resource kind.
        let owner = if def.planet_role.is_none()
            && (kind == EntityKind::Resource || def.resource.is_some()) {
            NEUTRAL
        } else {
            owner
        };
        let mut e = Entity::new(kind, def.key, owner, pos, heading, def.movement);
        e.hull = [def.initial_hull.unwrap_or(def.hull), def.hull];
        e.shields = Shields::full(def.shields.max, features.directional_shields);
        let strength = def.initial_shields.unwrap_or(def.shields.max);
        e.shields.now = if e.shields.directional {
            [strength; 4]
        } else {
            [strength, Fixed::ZERO, Fixed::ZERO, Fixed::ZERO]
        };
        if matches!(kind, EntityKind::Craft | EntityKind::Station) {
            e.crew = Some([
                def.crew.max(Fixed::ZERO),
                def.maximum_crew.unwrap_or(def.crew).max(Fixed::ZERO),
            ]);
        }
        e
    }

    /// Can take movement orders. Stop also applies to immobile craft/stations.
    pub fn orderable(&self) -> bool {
        self.kind == EntityKind::Craft && self.stats.max_speed > Fixed::ZERO
    }

    /// Steps under movement each tick.
    pub fn mobile(&self) -> bool {
        self.stats.max_speed > Fixed::ZERO && !self.flags.contains(UnitFlags::ENGINES_OFF)
    }

    pub fn view(&self) -> UnitView {
        UnitView {
            id: self.id,
            def: self.def,
            owner: self.owner,
            pos: self.pos,
            heading: self.heading,
            speed: self.speed,
            hull: self.hull,
            shields: self.shields.view(),
            flags: self.flags,
            progress: self.progress,
            crew: self.crew,
            energy: None,
        }
    }

    /// Feed every field into the digest, in declaration order.
    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.id.0 as u64);
        h.u64(self.kind as u64);
        h.u64(self.def.0 as u64);
        h.u64(self.owner.0 as u64);
        for f in [
            self.pos.x,
            self.pos.y,
            self.pos.z,
            self.heading.0,
            self.heading.1,
            self.speed,
            self.omega,
        ] {
            h.fixed(f);
        }
        match self.target {
            Some(t) => {
                h.u64(1);
                for f in [t.x, t.y, t.z] {
                    h.fixed(f);
                }
            }
            None => h.u64(0),
        }
        let s = &self.stats;
        for f in [
            s.max_speed,
            s.forward_accel,
            s.turn_omega,
            s.turn_alpha,
            s.turn_at_rest,
        ] {
            h.fixed(f);
        }
        // Smooth-physics keys (SIM-15) only when set, so sims without them
        // hash as before.
        if s.has_smooth_keys() {
            for f in s.smooth_fields() {
                h.fixed(f);
            }
        }
        for f in self
            .hull
            .into_iter()
            .chain(self.shields.now)
            .chain([self.shields.max])
        {
            h.fixed(f);
        }
        h.u64(self.shields.directional as u64);
        h.u64(self.flags.0 as u64);
        match self.progress {
            Some(p) => {
                h.u64(1);
                h.fixed(p);
            }
            None => h.u64(0),
        }
    }
}

/// Every entity, ascending id. Removal keeps the order; ids are never reused.
#[derive(Debug, Clone)]
pub struct Entities {
    list: Vec<Entity>,
    next: u32,
}

impl Default for Entities {
    fn default() -> Self {
        Entities {
            list: Vec::new(),
            next: 1,
        }
    }
}

impl Entities {
    pub fn new() -> Entities {
        Entities::default()
    }

    /// Rebuild from a snapshot: `list` ascending by id, every id below
    /// `next`. `None` if either does not hold.
    pub(crate) fn from_parts(list: Vec<Entity>, next: u32) -> Option<Entities> {
        let ordered = list.windows(2).all(|w| w[0].id < w[1].id);
        (ordered && list.last().is_none_or(|e| e.id.0 < next)).then_some(Entities { list, next })
    }

    /// Give `e` the next id and add it; ids only grow, so the list stays sorted.
    pub fn spawn(&mut self, mut e: Entity) -> EntityId {
        let id = EntityId(self.next);
        self.next = self.next.checked_add(1).expect("entity ids exhausted");
        e.id = id;
        self.list.push(e);
        id
    }

    /// Take an entity out of the world. Its id stays used.
    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        self.index(id).map(|i| self.list.remove(i))
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.index(id).map(|i| &self.list[i])
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.index(id).map(|i| &mut self.list[i])
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.index(id).is_some()
    }

    /// The id the next spawn gets.
    pub fn next_id(&self) -> EntityId {
        EntityId(self.next)
    }

    /// Ascending id.
    pub fn iter(&self) -> std::slice::Iter<'_, Entity> {
        self.list.iter()
    }

    /// Ascending id. Fields may change; ids may not.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Entity> {
        self.list.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn index(&self, id: EntityId) -> Option<usize> {
        self.list.binary_search_by_key(&id, |e| e.id).ok()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.next as u64);
        h.u64(self.list.len() as u64);
        for e in &self.list {
            e.hash_into(h);
        }
    }
}

impl<'a> IntoIterator for &'a Entities {
    type Item = &'a Entity;
    type IntoIter = std::slice::Iter<'a, Entity>;
    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl<'a> IntoIterator for &'a mut Entities {
    type Item = &'a mut Entity;
    type IntoIter = std::slice::IterMut<'a, Entity>;
    fn into_iter(self) -> Self::IntoIter {
        self.list.iter_mut()
    }
}

/// Per-entity state that only some entities have, ascending id: the place
/// for a system's own components. Rows for despawned entities must go with
/// them (`Sim::despawn`).
#[derive(Debug, Clone, PartialEq)]
pub struct Table<T> {
    rows: Vec<(EntityId, T)>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Table { rows: Vec::new() }
    }
}

impl<T> Table<T> {
    pub fn new() -> Table<T> {
        Table::default()
    }

    /// Rebuild from a snapshot: rows must ascend by id, else `None`.
    pub(crate) fn from_rows(rows: Vec<(EntityId, T)>) -> Option<Table<T>> {
        rows.windows(2)
            .all(|w| w[0].0 < w[1].0)
            .then_some(Table { rows })
    }

    /// Set the row for `id`, returning the one it replaces.
    pub fn insert(&mut self, id: EntityId, v: T) -> Option<T> {
        match self.rows.binary_search_by_key(&id, |r| r.0) {
            Ok(i) => Some(std::mem::replace(&mut self.rows[i].1, v)),
            Err(i) => {
                self.rows.insert(i, (id, v));
                None
            }
        }
    }

    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        self.rows
            .binary_search_by_key(&id, |r| r.0)
            .ok()
            .map(|i| self.rows.remove(i).1)
    }

    pub fn get(&self, id: EntityId) -> Option<&T> {
        self.rows
            .binary_search_by_key(&id, |r| r.0)
            .ok()
            .map(|i| &self.rows[i].1)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        self.rows
            .binary_search_by_key(&id, |r| r.0)
            .ok()
            .map(|i| &mut self.rows[i].1)
    }

    /// Ascending id.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.rows.iter().map(|(id, v)| (*id, v))
    }

    /// Ascending id.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.rows.iter_mut().map(|(id, v)| (*id, v))
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// FNV-1a over little-endian words, as `ContentDb::hash` uses: the hash of
/// [`Sim::digest`](crate::Sim::digest).
pub(crate) struct Fnv(pub(crate) u64);

impl Fnv {
    pub(crate) fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }

    pub(crate) fn fixed(&mut self, v: Fixed) {
        self.u64(v.to_bits() as u64);
    }

    /// Length first, so adjacent strings cannot run together.
    pub(crate) fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        for b in s.bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn craft(owner: u8) -> Entity {
        Entity::new(
            EntityKind::Craft,
            ContentKey::NONE,
            PlayerId(owner),
            Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
            MoveStats::default(),
        )
    }

    #[test]
    fn ids_ascend_and_are_never_reused() {
        let mut w = Entities::new();
        assert_eq!(w.next_id(), EntityId(1), "0 is none and never handed out");
        let ids: Vec<_> = (0..4).map(|i| w.spawn(craft(i))).collect();
        assert_eq!(ids, [1, 2, 3, 4].map(EntityId));
        assert_eq!(w.remove(EntityId(2)).unwrap().owner, PlayerId(1));
        assert!(w.remove(EntityId(2)).is_none());
        assert_eq!(w.remove(EntityId(4)).unwrap().id, EntityId(4));
        // The removed ids stay used: the next spawn is 5, not 2 or 4.
        assert_eq!(w.spawn(craft(7)), EntityId(5));
        let left: Vec<_> = w.iter().map(|e| e.id).collect();
        assert_eq!(left, [1, 3, 5].map(EntityId));
        assert!(w.get(EntityId(3)).is_some() && w.get(EntityId(4)).is_none());
        assert_eq!((&w).into_iter().count(), 3);
    }

    #[test]
    fn kinds_follow_the_class() {
        let db = sta_content::fake::db();
        let kind = |n: &str| {
            let d = db.unit(db.key(n).unwrap()).unwrap();
            EntityKind::of_def(d)
        };
        assert_eq!(kind("fx_cruiser"), EntityKind::Craft);
        assert_eq!(kind("fx_miner"), EntityKind::Craft);
        assert_eq!(kind("fx_station"), EntityKind::Station);
        assert_eq!(kind("fx_yard"), EntityKind::Station);
        assert_eq!(kind("fx_moon"), EntityKind::Resource);
        let unknown = ClassLabel::new("background_obj");
        assert_eq!(
            EntityKind::of(&unknown, &MoveStats::STATIC),
            EntityKind::Prop
        );
        assert_eq!(
            EntityKind::of(&unknown, &MoveStats::default()),
            EntityKind::Craft
        );
    }

    /// Fleet Ops' moons are class `scrap`, which is also plain wreckage: the `resource` decides.
    #[test]
    fn a_scrap_class_node_with_a_resource_is_a_resource() {
        let mut moon = sta_content::UnitDef::new("map_moon", "scrap");
        moon.resource = Some(0);
        let wreck = sta_content::UnitDef::new("wreck", "scrap");
        assert_eq!(EntityKind::of_def(&moon), EntityKind::Resource);
        assert_eq!(EntityKind::of_def(&wreck), EntityKind::Prop);
        let e = Entity::from_def(&moon, &Features::default(), PlayerId(0), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE));
        assert_eq!((e.kind, e.owner), (EntityKind::Resource, NEUTRAL));
    }

    #[test]
    fn entities_from_defs_start_full() {
        let db = sta_content::fake::db();
        let cruiser = db.unit(db.key("fx_cruiser").unwrap()).unwrap();
        let e = Entity::from_def(
            cruiser,
            db.features(),
            PlayerId(0),
            Vec3::ZERO,
            (Fixed::ZERO, Fixed::from_int(2)),
        );
        assert_eq!(e.hull, [Fixed::from_int(800); 2]);
        assert_eq!(e.shields.view(), [[Fixed::from_int(400); 2]; 4]);
        assert_eq!(e.heading, (Fixed::ZERO, Fixed::ONE), "normalised");
        assert!(e.orderable() && e.flags.contains(UnitFlags::SELECTABLE));
        // One pool: shown on all four facings.
        let s = Shields::full(Fixed::from_int(50), false);
        assert_eq!(s.now[1], Fixed::ZERO);
        assert_eq!(s.view(), [[Fixed::from_int(50); 2]; 4]);
    }

    #[test]
    fn tables_stay_sorted() {
        let mut t = Table::new();
        assert_eq!(t.insert(EntityId(5), 'a'), None);
        assert_eq!(t.insert(EntityId(2), 'b'), None);
        assert_eq!(t.insert(EntityId(9), 'c'), None);
        assert_eq!(t.insert(EntityId(5), 'd'), Some('a'));
        assert_eq!(
            t.iter().map(|(id, v)| (id.0, *v)).collect::<Vec<_>>(),
            [(2, 'b'), (5, 'd'), (9, 'c')]
        );
        *t.get_mut(EntityId(9)).unwrap() = 'e';
        assert_eq!(t.remove(EntityId(2)), Some('b'));
        assert_eq!(t.get(EntityId(9)), Some(&'e'));
        assert_eq!(t.len(), 2);
    }
}
