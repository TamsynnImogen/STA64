//! Armada II's player-controlled tactical view. The stock `toggle_tactical`
//! action calls CameraManager::SwapCameras (0x004de380). It passes the current
//! interest point to the tactical camera, then looks for the nearest visible
//! entity within cfgTACTICAL_SNAP_DISTANCE (0x004de460). The overview camera
//! takes back the tactical interest point when the player switches back.
//!
//! The original ShotDirector can compose shots from several entities and tracks.
//! This camera follows one subject using the recovered viewing distance.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use sta_sim::view::UnitFlags;
use sta_sim::{EntityId, PlayerId, UnitView, WorldView};

use super::camera_cinematic::{self, Pose};
use super::keys::{Action, KeyBindings};
use super::select::CameraFocus;
use crate::map::{CameraLimits, MainCamera};
use crate::render::units::{LocalPlayer, to_bevy};
use crate::sim_link::{ContentRes, SimLink};
use crate::viewer::Orbit;

// TacticalCamera::ResetToDefault, with the π/2 constant set by 0x004e4300.
const DEFAULT_PITCH: f32 = 0.275 * std::f32::consts::FRAC_PI_2;

#[derive(Clone, Copy)]
pub(crate) struct TacticalConfig {
    snap: f32,
    zoom: f32,
}

impl TacticalConfig {
    pub(crate) fn from_cfg(cfg: Option<&sta_assets::cfg_h::CfgH>) -> Self {
        let value = |key| cfg.and_then(|c| c.f32(key)).filter(|v| v.is_finite());
        Self {
            snap: value("cfgTACTICAL_SNAP_DISTANCE").unwrap_or(800.0).max(0.0),
            zoom: value("cfg_DEFAULT_TACTICAL_ZOOM")
                .unwrap_or(10.0)
                .clamp(1.0, 20.0),
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct TacticalView {
    shot: Option<Shot>,
    framing: Option<Framing>,
    toggle_requested: bool,
}

impl TacticalView {
    pub(crate) fn is_active(&self) -> bool {
        self.shot.is_some()
    }

    pub(crate) fn request_toggle(&mut self) {
        self.toggle_requested = true;
    }

    /// Whether a swap was requested since the last call (the tests' read of the request).
    #[cfg(test)]
    pub(crate) fn take_toggle_request(&mut self) -> bool {
        std::mem::take(&mut self.toggle_requested)
    }
}

pub(crate) fn overview_active(view: Option<Res<TacticalView>>) -> bool {
    !view.is_some_and(|view| view.is_active())
}

#[derive(Clone, Copy)]
struct Framing {
    yaw: f32,
    pitch: f32,
    zoom: f32,
}

struct Shot {
    focus: Vec3,
    subject: Option<EntityId>,
    framing: Framing,
}

impl Shot {
    fn enter(focus: Vec3, framing: Framing) -> Self {
        Self {
            focus,
            subject: None,
            framing,
        }
    }

    fn aim(&mut self, unit: Option<&UnitView>, point: Vec3) {
        self.focus = unit.map_or(point, |u| to_bevy(u.pos.to_f32()));
        self.subject = unit.map(|u| u.id);
    }

    fn pose(&self) -> Pose {
        // TacticalCamera::UpdateCamera (0x004e4985) calls the yaw/pitch/zoom
        // overload of GetCameraDescription (0x004e0770), which scales zoom by 100.
        Pose {
            focus: self.focus,
            yaw: self.framing.yaw,
            pitch: self.framing.pitch,
            dist: self.framing.zoom * 100.0,
        }
    }
}

fn visible(view: &WorldView, player: Option<PlayerId>, id: EntityId) -> bool {
    player.is_none_or(|p| {
        view.visibility.is_empty()
            || view
                .visibility
                .binary_search_by_key(&p, |(owner, _)| *owner)
                .is_ok_and(|index| view.visibility[index].1.binary_search(&id).is_ok())
    })
}

fn can_follow(db: &sta_content::ContentDb, unit: &UnitView) -> bool {
    db.unit(unit.def).is_some_and(|def| {
        use sta_sim::world::EntityKind;
        matches!(
            EntityKind::of_def(def),
            EntityKind::Craft | EntityKind::Station
        )
    }) && unit.flags.contains(UnitFlags::SELECTABLE)
        && !unit.flags.contains(UnitFlags::DYING)
        && unit.hull[0] > sta_sim::Fixed::ZERO
}

fn nearest<'a>(
    view: &'a WorldView,
    db: &sta_content::ContentDb,
    player: Option<PlayerId>,
    at: Vec3,
    snap: f32,
) -> Option<&'a UnitView> {
    let at = at.xz();
    view.units
        .iter()
        .filter_map(|u| {
            let distance = to_bevy(u.pos.to_f32()).xz().distance_squared(at);
            (distance < snap * snap && can_follow(db, u) && visible(view, player, u.id))
                .then_some((distance, u))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)))
        .map(|(_, u)| u)
}

fn behind(u: &UnitView) -> f32 {
    (-u.heading.0.to_f32()).atan2(u.heading.1.to_f32())
}

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct TacticalInput<'w> {
    keys: Option<Res<'w, ButtonInput<KeyCode>>>,
    bindings: Option<Res<'w, KeyBindings>>,
    buttons: Option<Res<'w, ButtonInput<MouseButton>>>,
    motion: Option<Res<'w, AccumulatedMouseMotion>>,
    scroll: Option<Res<'w, AccumulatedMouseScroll>>,
    time: Res<'w, Time>,
    clock: Option<Res<'w, crate::clock::AnimClock>>,
}

impl TacticalInput<'_> {
    fn held(&self, code: KeyCode) -> bool {
        self.keys.as_ref().is_some_and(|keys| keys.pressed(code))
    }

    fn pressed(&self, code: KeyCode) -> bool {
        self.keys
            .as_ref()
            .is_some_and(|keys| keys.just_pressed(code))
    }

    fn modifier(&self, pair: [KeyCode; 2]) -> bool {
        pair.into_iter().any(|key| self.held(key))
    }

    fn control(&self) -> bool {
        self.modifier([KeyCode::ControlLeft, KeyCode::ControlRight])
    }

    fn plain(&self) -> bool {
        !self.control()
            && !self.modifier([KeyCode::ShiftLeft, KeyCode::ShiftRight])
            && !self.modifier([KeyCode::AltLeft, KeyCode::AltRight])
    }

    fn toggle(&self) -> bool {
        self.plain()
            && self.keys.as_ref().is_some_and(|keys| {
                self.bindings.as_ref().map_or_else(
                    || keys.just_pressed(KeyCode::CapsLock),
                    |b| b.just_pressed(Action::ToggleTactical, keys),
                )
            })
    }

    fn place(&self, shot: &mut Shot, transform: &mut Transform, limits: &CameraLimits) {
        let dt = self.time.delta_secs().min(0.1);
        let side =
            f32::from(self.held(KeyCode::ArrowRight)) - f32::from(self.held(KeyCode::ArrowLeft));
        let fore =
            f32::from(self.held(KeyCode::ArrowUp)) - f32::from(self.held(KeyCode::ArrowDown));
        if self.control() {
            shot.framing.yaw -= side * dt * 1.5;
            shot.framing.zoom *= (-fore * dt * 1.5).exp();
        } else if self.plain() && (side != 0.0 || fore != 0.0) {
            shot.subject = None;
            let yaw = Quat::from_rotation_y(shot.framing.yaw);
            let movement = yaw * Vec3::new(side, 0.0, -fore);
            shot.focus =
                limits.clamp_focus(shot.focus + movement * shot.pose().dist.max(50.0) * 0.8 * dt);
        }
        if self
            .buttons
            .as_ref()
            .is_some_and(|buttons| buttons.pressed(MouseButton::Middle))
            && let Some(delta) = self.motion.as_ref().map(|motion| motion.delta)
        {
            shot.framing.yaw -= delta.x * 0.008;
            shot.framing.pitch = (shot.framing.pitch + delta.y * 0.008).clamp(-1.45, 1.45);
        }
        if let Some(scroll) = self.scroll.as_ref() {
            let lines = if scroll.unit == MouseScrollUnit::Line {
                scroll.delta.y
            } else {
                scroll.delta.y / 40.0
            };
            shot.framing.zoom *= 0.88_f32.powf(lines);
        }
        shot.framing.zoom = shot.framing.zoom.clamp(1.0, 20.0);
        let pose = shot.pose();
        transform.translation = camera_cinematic::camera_translation(&pose);
        transform.look_at(pose.focus, Vec3::Y);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    mut mode: ResMut<TacticalView>,
    input: TacticalInput,
    limits: Res<CameraLimits>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<LocalPlayer>>,
    script: Option<Res<camera_cinematic::CinematicCamera>>,
    editing: Option<Res<crate::editor::Editing>>,
    mut focus_events: MessageReader<CameraFocus>,
    mut radar_events: MessageReader<crate::ui::radar::RadarClick>,
    mut camera: Query<(&mut Orbit, &mut Transform), MainCamera>,
) {
    let radar_focus = radar_events
        .read()
        .filter_map(crate::ui::radar::camera_point)
        .last()
        .map(|p| limits.clamp_focus(Vec3::new(p.x, 0.0, -p.y)));
    let requested_focus = focus_events.read().last().map(|msg| msg.0).or(radar_focus);
    let clicked = std::mem::take(&mut mode.toggle_requested);
    let (Some(link), Some(db), Ok((mut orbit, mut transform))) =
        (link, content, camera.single_mut())
    else {
        *mode = TacticalView::default();
        return;
    };
    if editing.is_some() {
        *mode = TacticalView::default();
        return;
    }
    if script.is_some_and(|c| c.locked) {
        return;
    }
    let config = limits.tactical;
    let player = local.as_ref().and_then(|local| local.0);
    let view = link.curr();
    if clicked || input.toggle() {
        if let Some(shot) = mode.shot.take() {
            mode.framing = Some(shot.framing);
            let focus = limits.clamp_focus(shot.focus);
            orbit.focus.x = focus.x;
            orbit.focus.z = focus.z;
            return;
        }
        let point = requested_focus.unwrap_or(orbit.focus);
        let nearest = nearest(view, &db.0, player, point, config.snap);
        let framing = mode.framing.unwrap_or_else(|| Framing {
            yaw: nearest.map_or(orbit.yaw, behind),
            pitch: DEFAULT_PITCH,
            zoom: config.zoom,
        });
        let mut shot = Shot::enter(point, framing);
        shot.aim(nearest, point);
        mode.shot = Some(shot);
    } else if let (Some(shot), Some(point)) = (mode.shot.as_mut(), requested_focus) {
        shot.aim(nearest(view, &db.0, player, point, config.snap), point);
    }
    let Some(shot) = mode.shot.as_mut() else {
        return;
    };
    if let Some(id) = shot.subject {
        let alpha = input.clock.as_ref().map_or(1.0, |clock| clock.alpha());
        if visible(view, player, id)
            && view.unit(id).is_some_and(|unit| can_follow(&db.0, unit))
            && let Some((pos, _)) = link.interpolate(id, alpha)
        {
            shot.focus = to_bevy(pos);
        } else {
            shot.subject = None;
        }
    }
    if input.control() && input.pressed(KeyCode::KeyV) {
        shot.framing.zoom = config.zoom;
        shot.framing.pitch = DEFAULT_PITCH;
        if let Some(u) = shot.subject.and_then(|id| view.unit(id)) {
            shot.framing.yaw = behind(u);
        }
    }
    input.place(shot, &mut transform, &limits);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::command::LocalRouter;
    use sta_sim::{ContentKey, Fixed, Sim};

    fn unit(id: u32, def: ContentKey, x: i32) -> UnitView {
        UnitView {
            id: EntityId(id),
            def,
            owner: PlayerId(0),
            pos: sta_sim::Vec3::from_f32([x as f32, 20.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: UnitFlags::SELECTABLE,
            progress: None,
        }
    }

    #[test]
    fn tactical_view_snaps_follows_and_returns_interest_without_changing_overview_zoom() {
        let db = sta_content::fake::db();
        let def = db.key("fx_cruiser").unwrap();
        let view = WorldView {
            units: vec![unit(1, def, 20), unit(2, def, 180), unit(3, def, 350)],
            visibility: vec![(PlayerId(0), vec![EntityId(2), EntityId(3)])],
            ..default()
        };
        let mut link = SimLink::new(
            Box::new(Sim::new(1)),
            Box::new(LocalRouter::new()),
            PlayerId(0),
        );
        link.feed(std::sync::Arc::new(view.clone()));

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(link)
            .insert_resource(ContentRes(std::sync::Arc::new(db)))
            .insert_resource(LocalPlayer(Some(PlayerId(0))))
            .insert_resource(CameraLimits::from_cfg(None))
            .init_resource::<TacticalView>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_message::<CameraFocus>()
            .add_message::<crate::ui::radar::RadarClick>()
            .add_systems(
                Update,
                (update, crate::viewer::orbit_camera.run_if(overview_active)).chain(),
            );
        let cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Orbit {
                    focus: Vec3::ZERO,
                    yaw: 0.5,
                    pitch: 1.0,
                    dist: 2400.0,
                    home: (Vec3::ZERO, 2400.0),
                },
            ))
            .id();

        let toggle = |app: &mut App| {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.reset(KeyCode::CapsLock);
            keys.press(KeyCode::CapsLock);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset(KeyCode::CapsLock);
        };
        toggle(&mut app);
        let shot = app
            .world()
            .resource::<TacticalView>()
            .shot
            .as_ref()
            .unwrap();
        assert_eq!(
            shot.subject,
            Some(EntityId(2)),
            "the closer unit is outside sensor sight"
        );
        assert_eq!(shot.focus, Vec3::new(180.0, 20.0, 0.0));
        assert_eq!(shot.pose().dist, 1000.0);
        assert_eq!(app.world().get::<Orbit>(cam).unwrap().dist, 2400.0);

        let mut moved = view;
        moved.units[1].pos.x = Fixed::from_int(480);
        app.world_mut()
            .resource_mut::<SimLink>()
            .feed(std::sync::Arc::new(moved.clone()));
        app.update();
        let shot = app
            .world()
            .resource::<TacticalView>()
            .shot
            .as_ref()
            .unwrap();
        assert_eq!(shot.focus.x, 480.0);
        assert!((app.world().get::<Transform>(cam).unwrap().translation.x - 480.0).abs() < 0.01);
        moved.visibility[0].1 = vec![EntityId(3)];
        moved.units[1].pos.x = Fixed::from_int(850);
        app.world_mut()
            .resource_mut::<SimLink>()
            .feed(std::sync::Arc::new(moved));
        app.update();
        let shot = app
            .world()
            .resource::<TacticalView>()
            .shot
            .as_ref()
            .unwrap();
        assert_eq!(shot.subject, None);
        assert_eq!(
            shot.focus.x, 480.0,
            "keep the last seen interest when a subject leaves sight"
        );

        toggle(&mut app);
        assert!(!app.world().resource::<TacticalView>().is_active());
        let orbit = app.world().get::<Orbit>(cam).unwrap();
        assert_eq!(
            (orbit.focus, orbit.yaw, orbit.pitch, orbit.dist),
            (Vec3::new(480.0, 0.0, 0.0), 0.5, 1.0, 2400.0)
        );

        let mut scripted = camera_cinematic::CinematicCamera::default();
        scripted.locked = true;
        app.insert_resource(scripted);
        app.world_mut()
            .resource_mut::<TacticalView>()
            .request_toggle();
        app.update();
        assert!(
            !app.world().resource::<TacticalView>().is_active(),
            "mission camera has priority"
        );

        app.world_mut()
            .remove_resource::<camera_cinematic::CinematicCamera>();
        app.world_mut()
            .resource_mut::<TacticalView>()
            .request_toggle();
        app.update();
        assert!(
            app.world().resource::<TacticalView>().is_active(),
            "HUD requests use the same camera"
        );
    }
}
