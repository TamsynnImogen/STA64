//! Convert SOL placements to a menu scene and resolve Lua routes against their names.

mod lua;

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::{Vec3, warn};
use sta_bzn::sol::{SolObject, parse};
use sta_content::ContentDb;
use sta_sim::Fixed;
use sta_vfs::Vfs;

use super::{MenuMap, PlanetDef, ShipDef, ViewDef};

const CAMERA: &str = "menu_camera_";
const TARGET: &str = "menu_target_";
const POINT: &str = "menu_point_";
const MAX_OBJECTS: usize = 512;
const MAX_SOL_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_SECONDS: f32 = 10.0;

/// A route resolved at load time. Each leg gets an equal share of the duration.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Path {
    points: Vec<Vec3>,
    seconds: f32,
    looped: bool,
}

impl Path {
    pub(super) fn position(&self, time: f32) -> Vec3 {
        let legs = self.points.len() - 1;
        if legs == 0 {
            return self.points[0];
        }
        let fraction = if self.looped {
            time.rem_euclid(self.seconds) / self.seconds
        } else {
            (time / self.seconds).clamp(0.0, 1.0)
        };
        let at = fraction * legs as f32;
        let leg = (at as usize).min(legs - 1);
        self.points[leg].lerp(self.points[leg + 1], at - leg as f32)
    }
}

pub(super) fn load(
    vfs: &Vfs,
    layer: usize,
    stem: &str,
    bytes: &[u8],
    content: &ContentDb,
) -> Result<MenuMap, String> {
    if bytes.len() > MAX_SOL_BYTES {
        return Err("menu SOL exceeds 8 MiB".into());
    }
    let sol = parse(bytes).map_err(|error| error.to_string())?;
    if sol.objects.len() > MAX_OBJECTS {
        return Err("menu SOL has more than 512 objects".into());
    }
    let sidecar = format!("{stem}.lua");
    let script = match vfs
        .resolve_all(&sidecar)
        .into_iter()
        .find(|(name, _)| *name == vfs.layers()[layer].name())
    {
        Some((_, source)) => {
            let bytes = vfs
                .read_from(layer, source)
                .map_err(|error| format!("{sidecar}: {error}"))?;
            lua::parse(&bytes).map_err(|error| format!("{sidecar}: {error}"))?
        }
        None => lua::Script::default(),
    };

    let mut objects = BTreeMap::new();
    let mut map = MenuMap::parse("");
    map.sky = script.sky.unwrap_or_default();
    map.stars = script.stars.unwrap_or(1_000);
    map.blend = script.blend.unwrap_or(map.blend);
    for object in &sol.objects {
        let name = if object.instance_name.trim().is_empty() {
            format!("object_{}", object.id)
        } else {
            label(&object.instance_name)
        };
        let at = position(object);
        if !at.length_squared().is_finite() {
            return Err(format!("{name}: invalid position"));
        }
        if objects.insert(name.clone(), object).is_some() {
            return Err(format!("duplicate object name {name:?}"));
        }
        if [CAMERA, TARGET, POINT]
            .into_iter()
            .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let Some(def) = content.key(&object.odf).and_then(|key| content.unit(key)) else {
            warn!("menu scene: {name} has no definition for {:?}", object.odf);
            continue;
        };
        if def.planet_role.is_some() || matches!(def.class.0.as_str(), "planet" | "planet_obj") {
            if map.planet.is_some() {
                return Err("a menu scene can have one planet".into());
            }
            let up = Vec3::from_slice(&object.transform[3..6]);
            map.planet = Some(PlanetDef {
                class: def.odf.clone(),
                radius: def
                    .raw
                    .f32("ScaleSOD")
                    .filter(|r| r.is_finite() && *r > 0.0)
                    .unwrap_or(3000.0),
                at,
                axis: up.normalize_or(Vec3::Y),
                spin: 0.0,
                clouds: 0.0,
                orbit: 0.0,
            });
            continue;
        }
        let front = forward(object);
        map.ships.push(ShipDef {
            name,
            odf: def.odf.clone(),
            group: 0,
            nth: map.ships.len(),
            at,
            yaw: front.x.atan2(front.z),
            bob: 0.0,
            phase: 0.0,
            warp: def.movement.max_speed > Fixed::ZERO,
            locked_at: None,
            locked_yaw: None,
            path: None,
        });
    }

    let path = |names: &[String], seconds: Option<f32>, looped: bool| -> Result<Path, String> {
        let mut points = names
            .iter()
            .map(|name| {
                objects
                    .get(&label(name))
                    .map(|object| position(object))
                    .ok_or_else(|| format!("unknown waypoint {name:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let first = *points.first().ok_or("path has no waypoints")?;
        if looped && points.last() != Some(&first) {
            points.push(first);
        }
        Ok(Path {
            points,
            seconds: seconds.unwrap_or(DEFAULT_SECONDS),
            looped,
        })
    };

    for ship in script.ships {
        let name = label(&ship.name);
        let Some(actor) = map.ships.iter_mut().find(|s| s.name == name) else {
            return Err(format!("unknown ship {:?}", ship.name));
        };
        if actor.path.is_some() {
            return Err(format!("duplicate path for ship {:?}", ship.name));
        }
        actor.path = Some(path(&ship.path, ship.seconds, ship.looped)?);
    }

    if let Some(views) = script.views {
        let mut names = BTreeSet::new();
        for view in views {
            let name = label(&view.name);
            if !names.insert(name.clone()) {
                return Err(format!("duplicate view {name:?}"));
            }
            let eye = path(&view.camera, view.seconds, view.looped)?;
            let target = view
                .target
                .as_ref()
                .map(|points| path(points, view.seconds, view.looped))
                .transpose()?;
            let look = eye.points[0] + forward(objects[&label(&view.camera[0])]) * 100.0;
            let mut def = view_def(
                name,
                eye.points[0],
                target.as_ref().map_or(look, |path| path.points[0]),
                view.fov.unwrap_or(40.0),
            );
            def.eye_path = Some(eye);
            def.target_path = target;
            map.views.push(def);
        }
    } else {
        for (key, object) in &objects {
            let Some(name) = key.strip_prefix(CAMERA).filter(|name| !name.is_empty()) else {
                continue;
            };
            let eye = position(object);
            let look = objects
                .get(&format!("{TARGET}{name}"))
                .map_or_else(|| eye + forward(object) * 100.0, |object| position(object));
            map.views.push(view_def(name.to_owned(), eye, look, 40.0));
        }
    }
    if map.views.is_empty() {
        return Err("no menu camera views".into());
    }
    Ok(map)
}

fn label(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn position(object: &SolObject) -> Vec3 {
    Vec3::from_slice(&object.transform[9..12])
}

fn forward(object: &SolObject) -> Vec3 {
    Vec3::from_slice(&object.transform[6..9]).normalize_or(Vec3::Z)
}

fn view_def(name: String, eye: Vec3, target: Vec3, fov: f32) -> ViewDef {
    ViewDef {
        name,
        group: 0,
        eye,
        target,
        fov,
        sway: Vec3::ZERO,
        sway_period: 40.0,
        orbit: 0.0,
        warp_eye: None,
        warp_target: None,
        eye_path: None,
        target_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::scene::Director;
    use std::sync::Arc;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/menu-sol");

    fn content() -> ContentDb {
        let mut scout = sta_content::UnitDef::new("fscout", "craft");
        scout.hull = Fixed::from_int(100);
        scout.movement.max_speed = Fixed::from_int(100);
        ContentDb::builder().unit(scout).build().unwrap()
    }

    #[test]
    fn authored_sol_and_lua_place_a_ship_and_move_the_camera_and_ship() {
        let mut vfs = Vfs::default();
        vfs.push_dir("fixture".into(), FIXTURE.into()).unwrap();
        let content = Arc::new(content());
        let map = Arc::new(MenuMap::from_vfs(&vfs, &content));
        assert_eq!(
            map.ships.len(),
            1,
            "camera and waypoint markers are never spawned"
        );
        assert_eq!(map.ships[0].name, "escort");
        assert_eq!(map.ships[0].odf, "fscout");
        let built = super::super::build(&map, &content, &sta_content::MapDef::empty());
        assert_eq!(built.slots.len(), 1);
        let mut director = Director::new(map.clone(), "main");
        let start = director.frame(0.0);
        assert_eq!(start.cam.eye, Vec3::new(0.0, 80.0, 400.0));
        assert_eq!(start.cam.target, Vec3::ZERO);
        assert_eq!(start.ships[0].pos, Vec3::ZERO);
        let mid = director.frame(5.0);
        assert_eq!(mid.cam.eye, Vec3::new(90.0, 85.0, 390.0));
        assert_eq!(mid.ships[0].pos, Vec3::new(50.0, 0.0, 0.0));
        let end = director.frame(12.0);
        assert_eq!(end.cam.eye, Vec3::new(180.0, 90.0, 380.0));
        assert_eq!(end.ships[0].pos, Vec3::new(100.0, 0.0, 0.0));

        let mut departure = Director::new(map, "main");
        departure.frame(5.0);
        departure.depart(5.0);
        let leaving = departure.frame(6.5);
        assert_eq!(leaving.ships[0].pos.x, 50.0);
        assert_eq!(leaving.cam.target.x, 50.0);
    }

    #[test]
    fn bad_sol_sidecar_falls_back_to_its_layer_ini_and_never_runs_as_a_mission() {
        let root =
            std::env::temp_dir().join(format!("sta64-sol-menu-fallback-{}", std::process::id()));
        let dir = root.join("Menu");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(
            std::path::Path::new(FIXTURE).join("Menu/menu.sol"),
            dir.join("menu.sol"),
        )
        .unwrap();
        std::fs::write(
            dir.join("menu.lua"),
            "return { views = {{ name='bad', camera={'missing'} }} }",
        )
        .unwrap();
        std::fs::write(dir.join("map.ini"), "[View.ini_fallback]\nEye=0,50,300\n").unwrap();
        let mut vfs = Vfs::default();
        vfs.push_dir("mod".into(), root.clone()).unwrap();
        vfs.push_dir("fixture".into(), FIXTURE.into()).unwrap();
        let map = MenuMap::from_vfs(&vfs, &content());
        assert_eq!(map.view_names(), ["ini_fallback"]);
        std::fs::remove_dir_all(root).unwrap();
        assert!(lua::parse(b"while true do end").is_err());
        assert!(lua::parse(b"return { sky = io.open('menu.sol') }").is_err());
    }
}
