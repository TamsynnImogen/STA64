//! Cached policies and the mutable per-actor flag. No new public view layout.

use sta_content::{
    ConfinementInitial, ConfinementRules, ConfinementSpatial, ContentDb, Profile, Warning,
};
use sta_core::{ContentKey, EntityId, Vec3};

use super::BoundaryVolume;
use crate::view::UnitFlags;
use crate::world::{Entity, Fnv, Table};

#[derive(Debug, Clone, Copy)]
struct Prepared {
    finished: ConfinementRules,
    construction: ConfinementRules,
    volume: BoundaryVolume,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Confinement {
    active: bool,
    profile: Profile,
    bounds: Option<(Vec3, Vec3)>,
    policies: Vec<Option<Prepared>>,
    flags: Table<bool>,
}

impl Confinement {
    pub(crate) fn prepare(
        db: Option<&ContentDb>,
        bounds: Option<(Vec3, Vec3)>,
    ) -> (Self, Vec<Warning>) {
        let Some(db) = db else {
            return (Self::default(), Vec::new());
        };
        let mut out = Self {
            active: db.has_map_confinement(),
            profile: db.profile(),
            bounds,
            policies: vec![None; db.len()],
            flags: Table::new(),
        };
        let mut warnings = db.map_confinement_warnings().to_vec();
        if out.active {
            for def in db.units() {
                let Some(&finished) = db.map_confinement(def.key).filter(|r| r.is_enabled()) else {
                    continue;
                };
                let construction = *db.construction_confinement(def.key).expect("unit policy");
                let volume = bounds.map(|(min, max)| {
                    BoundaryVolume::new(min, max, finished.inset, finished.upper_epsilon)
                });
                match volume {
                    Some(Ok(volume)) => {
                        out.policies[def.key.0 as usize] = Some(Prepared {
                            finished,
                            construction,
                            volume,
                        })
                    }
                    other => warnings.push(Warning::new(
                        format!("{}.odf", def.odf),
                        format!(
                            "map confinement unavailable: {}",
                            match other {
                                Some(Err(e)) => e.to_string(),
                                _ => "no map bounds".into(),
                            }
                        ),
                    )),
                }
            }
        }
        (out, warnings)
    }

    fn prepared(&self, def: ContentKey) -> Option<Prepared> {
        self.policies.get(def.0 as usize).copied().flatten()
    }

    fn policy(&self, entity: &Entity) -> Option<(ConfinementRules, BoundaryVolume)> {
        let p = self.prepared(entity.def)?;
        Some((
            if entity.progress.is_some() {
                p.construction
            } else {
                p.finished
            },
            p.volume,
        ))
    }

    /// Actual availability is checked even for a false or same-value request.
    pub(crate) fn flag(&self, entity: &Entity) -> Option<bool> {
        if entity.flags.contains(UnitFlags::DYING) {
            return None;
        }
        self.policy(entity)?;
        self.flags.get(entity.id).copied()
    }

    pub(crate) fn set(&mut self, entity: &Entity, enabled: bool) -> bool {
        if self.flag(entity).is_none() {
            return false;
        }
        *self.flags.get_mut(entity.id).expect("validated flag") = enabled;
        true
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.flags.remove(id);
    }

    /// Called at every birth, and again when a weld becomes a finished actor.
    pub(crate) fn initialize(&mut self, entity: &mut Entity) {
        self.flags.remove(entity.id);
        let Some((rules, volume)) = self.policy(entity) else {
            return;
        };
        let enabled = match rules.initial {
            ConfinementInitial::Disabled => false,
            ConfinementInitial::Enabled => true,
            ConfinementInitial::PaddedXz => !volume.check(entity.pos, false).outside_xz,
            ConfinementInitial::RawXz => self.bounds.is_some_and(|(min, max)| {
                entity.pos.x > min.x
                    && entity.pos.x < max.x
                    && entity.pos.z > min.z
                    && entity.pos.z < max.z
            }),
        };
        self.flags.insert(entity.id, enabled);
        let callbacks = match self.profile {
            Profile::Sta1Classic => 2,
            Profile::Native => 1,
            Profile::Sta2Classic | Profile::Fo4 | Profile::Fo3 => 0,
        };
        for _ in 0..callbacks {
            self.position_updated(entity);
        }
    }

    /// Map completion invokes the ordinary SetTransform/update path. Runtime
    /// creation has only the callbacks in `initialize`.
    pub(crate) fn map_loaded(&mut self, entity: &mut Entity) {
        let callbacks = if self.profile == Profile::Sta1Classic {
            2
        } else {
            1
        };
        for _ in 0..callbacks {
            self.position_updated(entity);
        }
    }

    /// A false flag may rearm from XZ entry, without retroactive projection in
    /// that update. Returns a volume only if this update changed the position.
    pub(crate) fn position_updated(&mut self, entity: &mut Entity) -> Option<BoundaryVolume> {
        let (rules, volume) = self.policy(entity)?;
        if rules.spatial == ConfinementSpatial::Terrain {
            return None;
        }
        let flag = self.flags.get_mut(entity.id)?;
        let projected = volume.check(entity.pos, *flag);
        let changed = entity.pos != projected.position;
        entity.pos = projected.position;
        if !*flag && rules.rearm && !projected.outside_xz {
            *flag = true;
        }
        changed.then_some(volume)
    }

    /// Finished Craft dispatch runs even if stationary or engine-disabled.
    /// Native opt-in deliberately follows that shared tick rule.
    pub(crate) fn ticks(&self, entity: &Entity) -> bool {
        entity.progress.is_none()
            && self.policy(entity).is_some_and(|(r, _)| {
                matches!(
                    r.spatial,
                    ConfinementSpatial::Craft | ConfinementSpatial::Native
                )
            })
    }

    pub(crate) fn active_volume(&self, entity: &Entity) -> Option<BoundaryVolume> {
        let (r, volume) = self.policy(entity)?;
        (r.spatial != ConfinementSpatial::Terrain && self.flag(entity) == Some(true))
            .then_some(volume)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn rows(&self) -> Vec<(EntityId, bool)> {
        self.flags.iter().map(|(id, flag)| (id, *flag)).collect()
    }

    /// Restore exact flags, never derive them from a saved position.
    pub(crate) fn restore(
        &mut self,
        units: &crate::world::Entities,
        rows: Vec<(EntityId, bool)>,
    ) -> Result<(), &'static str> {
        let table = Table::from_rows(rows).ok_or("confinement ids out of order")?;
        for (id, _) in table.iter() {
            let entity = units.get(id).ok_or("confinement id does not exist")?;
            if self.policy(entity).is_none() {
                return Err("confinement row has no usable policy");
            }
        }
        for entity in units.iter() {
            if self.policy(entity).is_some() && table.get(entity.id).is_none() {
                return Err("missing confinement row");
            }
        }
        self.flags = table;
        Ok(())
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        if !self.active {
            return;
        }
        h.u64(0x434f_4e46_494e_4501);
        if let Some((min, max)) = self.bounds {
            h.u64(1);
            for f in [min.x, min.y, min.z, max.x, max.y, max.z] {
                h.fixed(f);
            }
        } else {
            h.u64(0);
        }
        h.u64(self.flags.len() as u64);
        for (id, enabled) in self.flags.iter() {
            h.u64(u64::from(id.0));
            h.u64(u64::from(*enabled));
        }
    }
}
