//! The Armada II window's subject choice and framing (#1167): the shot director's rules
//! ([`crate::render::shot_director`]) fed by the sim's events, in place of the Armada I window's tiers
//! (`action_camera.rs`, unchanged, and used whenever the picture is not Armada II's).
//!
//! Grade: read from the executable, static (see the director's module doc); keys read from data: the class key
//! `hidden` (a class with it is never shown) and `directorRadiusScale` (the framing radius is the unit's bounding
//! radius times it; INTERIM: absent, the scale is one, the original takes the parent class's, not read; and
//! `SelectRadius` stands for the bounding radius). Nothing is measured.
//!
//! - **What qualifies.** Whatever the viewer's team currently sees, whoever owns it (not only the local player's units),
//!   excluding classes with `hidden` (read from the executable, static; the viewer is the local player, INTERIM: a
//!   spectating or allied view is not read).
//! - **What makes a shot** (the events STA64 has; the others, wormholes, docks, a construction cancelled and a
//!   producer's build in progress, are not built: INTERIM): a weapon firing, a unit removed (STA64 has no death
//!   sequence yet, #1100), a construction starting or finishing, a ship finished at a yard.
//! - **Not run** while the picture is hidden (the "Cinematic window" setting, #1364) or while the picture is not the
//!   Armada II one: the gates of `action_camera.rs` apply to these systems too.
//! - **Cut behaviour** between shots and the camera's motion are not read: the pose is set each frame from the shot
//!   (INTERIM, as is the far-plane call of the original, whose receiver is ambiguous in the read: the two candidate
//!   readings are the main view's camera and the window's own; STA64 keeps its present far plane for both).

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::command::Target;
use sta_sim::event::SimEvent;
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::EntityId;

use super::ActionSubject;
use crate::clock::AnimClock;
use crate::render::shot_director::{self as director, Director, Ordnance, Shot, ShotSize};
use crate::render::units::{LocalPlayer, SelectRadius, to_bevy};
use crate::sim_link::{ContentRes, SimEntities, SimLink, SimMsg};

/// The picture is the Armada II window's: a picture exists and the mounted content is Armada II's. Every other
/// content keeps the Armada I window's rules (`action_camera.rs`): Armada I's own window, and any other profile's
/// (its rules are not read: INTERIM).
pub fn a2_active(pictures: Query<(), With<super::ActionPicture>>, content: Option<Res<ContentRes>>) -> bool {
    !pictures.is_empty() && content.is_some_and(|c| c.0.profile() == sta_content::Profile::Sta2Classic)
}

/// Frames a unit stays in the record after it was last seen (for the event that removes it).
const KEEP_FRAMES: u64 = 120;

/// What the director knows of a unit it may show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
    pub pos: Vec3,
    /// Its director radius: bounding radius times the class key `directorRadiusScale`.
    pub radius: f32,
    pub team: u8,
    pub craft: bool,
    pub station: bool,
    /// The frame it was last in sight.
    pub frame: u64,
}

/// The director, what it may show, the shot shown and the pose it gives.
#[derive(Resource, Default)]
pub struct A2State {
    pub director: Director,
    pub shot: Option<Shot>,
    pub seen: BTreeMap<EntityId, Seen>,
    pub frame: u64,
    pub pose: Option<Transform>,
}

/// The default horizontal direction of the camera when two entities give no line of action (INTERIM: STA64's own
/// oblique yaw; the original's default is not read).
fn default_direction() -> Vec3 {
    Quat::from_rotation_y(super::YAW) * Vec3::Z
}

/// Record what the viewer's team sees (classes with `hidden` left out), with each unit's director radius.
#[allow(clippy::too_many_arguments)]
pub fn track(
    link: Res<SimLink>,
    local: Option<Res<LocalPlayer>>,
    content: Option<Res<ContentRes>>,
    entities: Option<Res<SimEntities>>,
    radii: Query<&SelectRadius>,
    mut state: ResMut<A2State>,
) {
    state.frame += 1;
    let frame = state.frame;
    let player = local.and_then(|l| l.0).unwrap_or(link.player());
    let view = link.curr();
    for u in view.visible_to(player) {
        let def = content.as_ref().and_then(|c| c.0.unit(u.def));
        if def.is_some_and(|d| d.raw.bool("hidden").unwrap_or(false)) {
            continue;
        }
        let bounding = entities.as_ref().and_then(|m| m.0.get(&u.id)).and_then(|e| radii.get(*e).ok()).map_or(20.0, |r| r.0);
        let scale = def.and_then(|d| d.raw.f32("directorRadiusScale")).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
        let kind = def.map(EntityKind::of_def);
        let team = view.player(u.owner).map_or(u.owner.0, |p| p.team.0);
        state.seen.insert(
            u.id,
            Seen { pos: to_bevy(u.pos.to_f32()), radius: bounding * scale, team, craft: kind.is_none_or(|k| k == EntityKind::Craft) && !u.flags.contains(UnitFlags::DYING), station: kind == Some(EntityKind::Station), frame },
        );
    }
    state.seen.retain(|_, s| frame - s.frame <= KEEP_FRAMES);
}

/// The ordnance kind of a weapon's class (class labels STA64 reads; INTERIM: the classes of the aimed-ordnance
/// branches are the engine's flight-code classes, the rest take the beam branch).
fn ordnance_of(class: &str) -> Ordnance {
    match class.to_ascii_lowercase().as_str() {
        "pphaser" | "artillery" => Ordnance::Bullet,
        "photontorpedo" | "polerontorpedo" | "shieldremovingtorpedo" | "plasmacannon" | "chainpulsar" => Ordnance::Missile,
        _ => Ordnance::Beam,
    }
}

/// Make shots from this frame's events.
pub fn shots(mut msgs: MessageReader<SimMsg>, clock: Option<Res<AnimClock>>, content: Option<Res<ContentRes>>, mut state: ResMut<A2State>) {
    let now = clock.as_ref().map_or(0.0, |c| c.now() as f32);
    for SimMsg(ev) in msgs.read() {
        let made: Option<((f32, f32, ShotSize), Vec<EntityId>, Vec3)> = match ev {
            SimEvent::Fired { shooter, weapon, target, .. } => state.seen.get(shooter).map(|s| {
                let def = content.as_ref().and_then(|c| c.0.weapon(*weapon));
                // A target entity in sight is aimed at; any other target is free ordnance.
                let aimed = match target {
                    Target::Entity(t) => state.seen.get(t).map(|seen| (*t, *seen)),
                    _ => None,
                };
                let (ordnance, gap, entities) = match aimed {
                    Some((t, tseen)) => (def.map_or(Ordnance::Beam, |d| ordnance_of(&d.class.0)), s.pos.distance(tseen.pos) - s.radius - tseen.radius, vec![*shooter, t]),
                    None => (Ordnance::Free, 0.0, vec![*shooter]),
                };
                let speed = def.map_or(0.0, |d| d.shot_speed.to_f32());
                (director::fire_shot(ordnance, gap, speed), entities, s.pos)
            }),
            SimEvent::Destroyed { id, .. } => state.seen.get(id).map(|s| (director::FIREBALL, vec![*id], s.pos)),
            SimEvent::BuildStarted { builder, .. } => state.seen.get(builder).map(|s| (director::CONSTRUCTION_START, vec![*builder], s.pos)),
            SimEvent::Built { builder, id } => state.seen.get(id).map(|s| {
                let from_yard = s.craft && state.seen.get(builder).is_some_and(|b| b.station);
                (if from_yard { director::SHIP_FINISHED } else { director::CONSTRUCTION_FINISH }, vec![*id], s.pos)
            }),
            _ => None,
        };
        if let Some(((priority, duration, size), entities, at)) = made {
            state.director.add(Shot { priority, duration, size, born: now, entities, at });
        }
    }
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Choose the shot when the director says it is due, and set the subject and the camera's pose from it.
pub fn choose(clock: Option<Res<AnimClock>>, mut state: ResMut<A2State>, mut subject: ResMut<ActionSubject>) {
    let now = clock.as_ref().map_or(0.0, |c| c.now() as f32);
    if state.director.due(now) {
        // A random craft in sight now (a deterministic draw from the clock's seed and tick).
        let current = state.frame;
        let craft: Vec<EntityId> = state.seen.iter().filter(|(_, s)| s.craft && s.frame == current).map(|(id, _)| *id).collect();
        let fallback = (!craft.is_empty()).then(|| {
            let key = clock.as_ref().map_or(0, |c| c.seed() ^ c.tick());
            craft[(splitmix(key) % craft.len() as u64) as usize]
        });
        state.shot = state.director.choose(now, fallback);
    }
    let live: Vec<(EntityId, Seen)> = state.shot.as_ref().map(|s| s.entities.iter().filter_map(|id| state.seen.get(id).map(|v| (*id, *v))).collect()).unwrap_or_default();
    let next = live.first().map(|(id, _)| *id);
    if subject.0 != next {
        subject.0 = next;
    }
    let (Some(shot), Some((_, first))) = (state.shot.as_ref(), live.first()) else { return };
    let radius = director::framing_radius(Some(first.radius), shot.size);
    let direction = live.get(1).and_then(|(_, second)| director::line_of_action(first.pos, second.pos)).unwrap_or_else(default_direction);
    let same = live.iter().filter(|(_, s)| s.team == first.team).count() as f32 / live.len() as f32;
    let offset = director::camera_offset(radius, direction, director::angle_class(same));
    let pose = Transform::from_translation(first.pos + offset).looking_at(first.pos, Vec3::Y);
    if state.pose != Some(pose) {
        state.pose = Some(pose);
    }
}

/// A match ended (its entities are gone): nothing carries into the next one.
pub fn forget(entities: Option<Res<SimEntities>>, mut state: ResMut<A2State>, mut subject: ResMut<ActionSubject>) {
    if entities.is_none_or(|e| e.0.is_empty()) && (state.shot.is_some() || !state.seen.is_empty()) {
        state.director.clear();
        state.shot = None;
        state.seen.clear();
        state.pose = None;
        subject.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn seen(x: f32, z: f32, team: u8, craft: bool) -> Seen {
        Seen { pos: Vec3::new(x, 0.0, z), radius: 30.0, team, craft, station: !craft, frame: 1 }
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(AnimClock::default());
        w.insert_resource(ActionSubject::default());
        w.insert_resource(A2State { frame: 1, ..Default::default() });
        w
    }

    #[test]
    fn the_armada_two_window_follows_the_best_shot_and_frames_it_by_the_rules() {
        let mut w = world();
        {
            let mut s = w.resource_mut::<A2State>();
            s.seen.insert(EntityId(1), seen(0.0, 0.0, 1, true));
            s.seen.insert(EntityId(2), seen(100.0, 0.0, 2, true));
            s.seen.insert(EntityId(3), seen(5000.0, 0.0, 1, true));
            // A weak shot near the first two, a strong one far away.
            s.director.add(Shot { priority: 4.0, duration: 5.0, size: ShotSize::Cu, born: 0.0, entities: vec![EntityId(1), EntityId(2)], at: Vec3::ZERO });
            s.director.add(Shot { priority: 14.0, duration: 5.0, size: ShotSize::Ms, born: 0.0, entities: vec![EntityId(3)], at: Vec3::new(5000.0, 0.0, 0.0) });
        }
        w.run_system_once(choose).unwrap();
        assert_eq!(w.resource::<ActionSubject>().0, Some(EntityId(3)), "the strong shot is shown");
        let pose = w.resource::<A2State>().pose.unwrap();
        // One entity: all of its team, so above; the radius is twice its director radius times the size's scale.
        let radius = 2.0 * 30.0 * ShotSize::Ms.scale();
        let want = Vec3::new(5000.0, 0.0, 0.0) + director::camera_offset(radius, default_direction(), director::Angle::Above);
        assert!(pose.translation.distance(want) < 1e-3, "{:?} vs {want:?}", pose.translation);
        assert!(pose.forward().as_vec3().dot((Vec3::new(5000.0, 0.0, 0.0) - pose.translation).normalize()) > 0.999, "it looks at the subject");
    }

    #[test]
    fn two_opposed_entities_give_a_line_of_action_and_a_level_view() {
        let mut w = world();
        {
            let mut s = w.resource_mut::<A2State>();
            s.seen.insert(EntityId(1), seen(0.0, 0.0, 1, true));
            s.seen.insert(EntityId(2), seen(0.0, -200.0, 2, true));
            s.director.add(Shot { priority: 8.0, duration: 5.0, size: ShotSize::Cu, born: 0.0, entities: vec![EntityId(1), EntityId(2)], at: Vec3::ZERO });
        }
        w.run_system_once(choose).unwrap();
        let pose = w.resource::<A2State>().pose.unwrap();
        // Half the shot's entities are on the first one's team: level, along the line toward the second.
        assert!((pose.translation.y).abs() < 1e-3, "level: {:?}", pose.translation);
        assert!(pose.translation.z < 0.0 && pose.translation.x.abs() < 1e-3, "{:?}", pose.translation);
    }

    #[test]
    fn with_no_event_a_visible_craft_is_shown_and_with_nothing_in_sight_nothing() {
        let mut w = world();
        w.resource_mut::<A2State>().seen.insert(EntityId(5), seen(10.0, 10.0, 1, true));
        w.resource_mut::<A2State>().seen.insert(EntityId(6), seen(10.0, 10.0, 1, false));
        w.run_system_once(choose).unwrap();
        assert_eq!(w.resource::<ActionSubject>().0, Some(EntityId(5)), "the fallback is a craft, not a station");
        let mut w = world();
        w.run_system_once(choose).unwrap();
        assert_eq!(w.resource::<ActionSubject>().0, None);
        assert!(w.resource::<A2State>().pose.is_none(), "no subject: the camera keeps its last pose");
    }

    #[test]
    fn a_match_that_ends_leaves_nothing_behind() {
        let mut w = world();
        w.insert_resource(SimEntities::default());
        w.resource_mut::<A2State>().seen.insert(EntityId(1), seen(0.0, 0.0, 1, true));
        w.resource_mut::<A2State>().director.add(Shot { priority: 8.0, duration: 5.0, size: ShotSize::Cu, born: 0.0, entities: vec![EntityId(1)], at: Vec3::ZERO });
        w.run_system_once(choose).unwrap();
        assert!(w.resource::<ActionSubject>().0.is_some());
        w.run_system_once(forget).unwrap();
        assert!(w.resource::<ActionSubject>().0.is_none() && w.resource::<A2State>().seen.is_empty() && w.resource::<A2State>().pose.is_none());
    }

    #[test]
    fn weapon_classes_pick_their_ordnance_branch() {
        assert_eq!(ordnance_of("pphaser"), Ordnance::Bullet);
        assert_eq!(ordnance_of("PhotonTorpedo"), Ordnance::Missile);
        assert_eq!(ordnance_of("phaser"), Ordnance::Beam);
    }

    fn content(profile: sta_content::Profile) -> ContentRes {
        ContentRes(std::sync::Arc::new(sta_content::ContentBuilder::default().profile(profile).build().unwrap()))
    }

    /// Armada I's window, and any content that is not Armada II's, keeps the present rules; no picture, no director.
    #[test]
    fn the_director_runs_for_armada_twos_picture_only() {
        use sta_content::Profile;
        let mut w = World::new();
        assert!(!w.run_system_once(a2_active).unwrap(), "no picture, no content");
        w.spawn(super::super::ActionPicture);
        for (profile, want) in [(Profile::Sta2Classic, true), (Profile::Sta1Classic, false), (Profile::Fo4, false), (Profile::Native, false)] {
            w.insert_resource(content(profile));
            assert_eq!(w.run_system_once(a2_active).unwrap(), want, "{profile:?}");
        }
        let mut none = World::new();
        none.insert_resource(content(Profile::Sta2Classic));
        assert!(!none.run_system_once(a2_active).unwrap(), "Armada II content without a picture (hidden or absent): nothing runs");
    }
}
