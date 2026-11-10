//! The building ghost: while a build waits for its site ([`HudState::placing`])
//! a translucent copy of the building's model follows the cursor on the
//! ground at the builder's height, green where the site is clear and red where
//! it is not (stock Armada's blueprint, which the refit notes describe as green
//! when clear and red when blocked).
//!
//! Whether the site is clear is [`sta_sim::production::site::check_site`], the
//! rule the sim applies to the order, fed with the same view, so the ghost and
//! the sim agree; [`site_status`] is the one entry the click uses too.

use bevy::prelude::*;
use sta_sim::production::site::{self, SiteError};
use sta_sim::{ContentKey, Vec3 as SimVec3};

use super::model::HudFeed;
use super::{Armed, HudState, ground_point};
use crate::input::orders::Aim;
use crate::render::units::{NeedsModel, TeamColour, to_bevy};

/// The map's playable volume, for build sites (`None`: unbounded). Set when a
/// map loads, from the same bounds the sim was given.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct PlacementBounds(pub Option<(SimVec3, SimVec3)>);

/// Tint of a ghost over a clear site.
pub const CLEAR: Color = Color::srgba(0.35, 1.0, 0.4, 0.6);
/// Tint of a ghost over a blocked site.
pub const BLOCKED: Color = Color::srgba(1.0, 0.25, 0.2, 0.6);

/// The ghost entity; `status` is whether the site under the cursor is clear.
#[derive(Component, Debug)]
pub struct Ghost {
    pub def: ContentKey,
    pub status: Option<Result<(), SiteError>>,
    painted: Option<Result<(), SiteError>>,
    own: Vec<Handle<StandardMaterial>>,
}

/// Whether `armed` may be placed at `at` now: `check_site` over the feed's
/// view with the builder left out.
pub fn site_status(feed: &HudFeed, bounds: Option<&(SimVec3, SimVec3)>, armed: &Armed, at: SimVec3) -> Result<(), SiteError> {
    let Some(db) = feed.content.as_deref() else { return Ok(()) };
    site::check_site(db, bounds, armed.what, at, site::view_obstacles(db, &feed.view, Some(armed.builder)))
}

/// Whether the player can pay for `armed` now (the sim refuses a rig's order otherwise).
pub fn affordable(feed: &HudFeed, armed: &Armed) -> bool {
    let (Some(db), Some(p)) = (feed.content.as_deref(), feed.view.player(feed.player)) else { return true };
    let Some(def) = db.unit(armed.what) else { return true };
    def.build.cost.iter().enumerate().all(|(i, &c)| c <= sta_sim::Fixed::ZERO || p.resources.get(i).is_some_and(|&h| h >= c))
}

fn paint(color: Color) -> impl Fn(&mut StandardMaterial) {
    move |m| {
        m.base_color = color;
        m.alpha_mode = AlphaMode::Blend;
        // Glows in its tint whatever the lighting, so it reads against dark space.
        m.emissive = color.to_linear() * 1.5;
        m.perceptual_roughness = 1.0;
    }
}

/// Keep one ghost while a placement is armed: spawn it, move it to the
/// cursor, set its status, remove it when the placement ends.
#[allow(clippy::too_many_arguments)]
pub fn update_ghost(
    mut commands: Commands,
    state: Res<HudState>,
    feed: Res<HudFeed>,
    aim: Option<Res<Aim>>,
    bounds: Option<Res<PlacementBounds>>,
    mut ghosts: Query<(Entity, &mut Ghost, &mut Transform, &mut Visibility)>,
) {
    let armed = state.placing.as_ref().filter(|a| feed.view.unit(a.builder).is_some());
    let Some(armed) = armed else {
        for (e, ..) in &ghosts {
            commands.entity(e).despawn();
        }
        return;
    };
    let mut have = false;
    for (e, mut g, mut t, mut vis) in &mut ghosts {
        if have || g.def != armed.what {
            commands.entity(e).despawn();
            continue;
        }
        have = true;
        let y = feed.view.unit(armed.builder).map_or(0.0, |b| b.pos.y.to_f32());
        let at = aim.as_ref().and_then(|a| a.0).and_then(|a| ground_point(a.ray, y));
        match at {
            Some(at) => {
                t.translation = to_bevy(at.to_f32());
                g.status = Some(site_status(&feed, bounds.as_deref().and_then(|b| b.0.as_ref()), armed, at));
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    if !have {
        commands.spawn((
            Name::new("building ghost"),
            Ghost { def: armed.what, status: None, painted: None, own: Vec::new() },
            NeedsModel(armed.what),
            TeamColour(Color::WHITE),
            // Faces the way the finished building will (heading +z).
            Transform::IDENTITY.looking_to(Vec3::NEG_Z, Vec3::Y),
            Visibility::Hidden,
        ));
    }
}

/// Give the ghost's model translucent materials of its own (the model's parts
/// share theirs with real units) and keep them tinted by the site's status.
pub fn tint_ghost(
    mut ghosts: Query<(Entity, &mut Ghost)>,
    children: Query<&Children>,
    mut handles: Query<&mut MeshMaterial3d<StandardMaterial>>,
    assets: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let Some(mut assets) = assets else { return };
    for (root, mut g) in &mut ghosts {
        let Some(status) = g.status else { continue };
        if g.own.is_empty() {
            for d in children.iter_descendants(root) {
                let Ok(mut h) = handles.get_mut(d) else { continue };
                let Some(copy) = assets.get(&h.0).cloned() else { continue };
                let own = assets.add(copy);
                h.0 = own.clone();
                g.own.push(own);
            }
        }
        if g.painted == Some(status) && !g.own.is_empty() {
            continue;
        }
        let color = if status.is_ok() { CLEAR } else { BLOCKED };
        let apply = paint(color);
        for h in &g.own {
            if let Some(mut m) = assets.get_mut(h) {
                apply(&mut m);
            }
        }
        if !g.own.is_empty() {
            g.painted = Some(status);
        }
    }
}

/// Why a click on a blocked site is refused, as the notice says it.
pub fn refusal(e: SiteError) -> &'static str {
    match e {
        SiteError::OffMap => "Cannot build here: off the map",
        SiteError::Blocked => "Cannot build here: the site is blocked",
    }
}
