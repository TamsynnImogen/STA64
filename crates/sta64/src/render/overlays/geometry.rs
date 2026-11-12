use bevy::prelude::*;

/// Scaled local model bounds in Bevy coordinates, independent of click radius.
#[derive(Component, Clone, Copy, Debug)]
pub struct ModelBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl ModelBounds {
    pub fn from_legacy(min: [f32; 3], max: [f32; 3], scale: f32) -> Self {
        Self {
            min: Vec3::new(min[0], min[1], -max[2]) * scale,
            max: Vec3::new(max[0], max[1], -min[2]) * scale,
        }
    }

    pub fn fallback(radius: f32) -> Self {
        Self { min: Vec3::splat(-radius), max: Vec3::splat(radius) }
    }
}

/// Native A2 defaults are pixels, not fractions of an actor's selection sphere.
/// Optional STA64 CFG overrides are read by the stock HUD's resolved layout.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct GaugeStyle {
    pub height: f32,
    pub gap: f32,
}

impl Default for GaugeStyle {
    fn default() -> Self { Self { height: 2.0, gap: 2.0 } }
}

impl GaugeStyle {
    pub fn from_cfg(height: Option<f64>, gap: Option<f64>) -> Self {
        let valid = |value: Option<f64>, fallback: f32| {
            value.filter(|v| v.is_finite() && *v >= 0.0 && *v <= 128.0)
                .map_or(fallback, |v| v as f32)
        };
        Self { height: valid(height, 2.0), gap: valid(gap, 2.0) }
    }
}

/// Project all eight corners. Reject the near plane rather than creating huge
/// gauges when a corner crosses the camera. Offscreen fragments remain clipped
/// by the renderer; wholly offscreen actors have no gauge.
pub(super) fn project(bounds: ModelBounds, world: Mat4, clip: Mat4, size: Vec2) -> Option<(Rect, f32)> {
    if !size.is_finite() || size.min_element() <= 0.0 { return None; }
    let matrix = clip * world;
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut nearest_depth = 0.0_f32;
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                let point = matrix * Vec3::new(x, y, z).extend(1.0);
                if !point.is_finite() || point.w <= 1e-5 || point.z < 0.0 || point.z > point.w { return None; }
                let p = point.truncate() / point.w;
                nearest_depth = nearest_depth.max(p.z);
                let screen = Vec2::new(p.x + 1.0, 1.0 - p.y) * size * 0.5;
                min = min.min(screen);
                max = max.max(screen);
            }
        }
    }
    if max.x < 0.0 || max.y < 0.0 || min.x > size.x || min.y > size.y { return None; }
    // Native gauges are screen overlays. Put our existing mesh quads just in
    // front of the closest bounds corner, not through the actor's centre:
    // otherwise the model can depth-test away a thin gauge at some zoom levels.
    let depth = (nearest_depth * 1.0001).min(1.0 - f32::EPSILON);
    (depth.is_finite()).then_some((Rect { min: min.floor(), max: max.ceil() }, depth))
}

pub(super) fn bar_rect(bounds: Rect, shield: bool, style: GaugeStyle) -> Option<Rect> {
    if style.height <= 0.0 { return None; }
    let top = bounds.min.y - if shield { style.height + style.gap } else { 0.0 };
    Some(Rect::from_corners(Vec2::new(bounds.min.x, top), Vec2::new(bounds.max.x + 1.0, top + style.height)))
}

/// Keep the existing camera-facing mesh path: unproject pixel geometry onto a
/// plane at the actor depth, then reparent to the actor without scaling pixels
/// with its radius. The mesh children use normalized 0..1 fill geometry.
pub(super) fn world_bar(rect: Rect, depth: f32, world_from_clip: Mat4, size: Vec2, rotation: Quat) -> Option<Transform> {
    let world = |p: Vec2| world_from_clip.project_point3(Vec3::new(p.x / size.x * 2.0 - 1.0, 1.0 - p.y / size.y * 2.0, depth));
    let centre = world(rect.center());
    let a = world(rect.min);
    let right = world(Vec2::new(rect.max.x, rect.min.y));
    let bottom = world(Vec2::new(rect.min.x, rect.max.y));
    let scale = Vec3::new(a.distance(right), a.distance(bottom), 1.0);
    (centre.is_finite() && scale.is_finite() && scale.x > 0.0 && scale.y > 0.0)
        .then_some(Transform { translation: centre, rotation, scale })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::CameraProjection;

    #[test]
    fn stock_and_authored_pixel_dimensions_preserve_zero() {
        let bounds = Rect::from_corners(Vec2::new(10.0, 50.0), Vec2::new(110.0, 160.0));
        let hull = bar_rect(bounds, false, GaugeStyle::default()).unwrap();
        let shield = bar_rect(bounds, true, GaugeStyle::default()).unwrap();
        assert_eq!(hull.size(), Vec2::new(101.0, 2.0));
        assert_eq!(shield.max.y + 2.0, hull.min.y);
        assert_eq!(hull.min.y, bounds.min.y);
        assert!(bar_rect(bounds, false, GaugeStyle::from_cfg(Some(0.0), None)).is_none());
        assert_eq!(GaugeStyle::from_cfg(Some(f64::NAN), Some(-1.0)), GaugeStyle::default());
        assert_eq!(GaugeStyle::from_cfg(Some(3.0), Some(4.0)), GaugeStyle { height: 3.0, gap: 4.0 });
    }

    #[test]
    fn legacy_bounds_convert_handedness_and_scale() {
        let b = ModelBounds::from_legacy([-2.0, 3.0, -8.0], [7.0, 9.0, 4.0], 2.0);
        assert_eq!(b.min, Vec3::new(-4.0, 6.0, -8.0));
        assert_eq!(b.max, Vec3::new(14.0, 18.0, 16.0));
    }

    #[test]
    fn gauges_stay_two_pixels_under_zoom_rotation_and_actor_transforms() {
        let size = Vec2::new(1280.0, 720.0);
        let mut projection = PerspectiveProjection::default();
        projection.update(size.x, size.y);
        let bounds = ModelBounds::from_legacy([-40.0, -10.0, -20.0], [40.0, 30.0, 20.0], 1.5);
        for distance in [250.0, 600.0, 1400.0] {
            let camera = Transform::from_xyz(0.0, distance * 0.5, distance).looking_at(Vec3::ZERO, Vec3::Y);
            let clip = projection.get_clip_from_view() * camera.to_matrix().inverse();
            for angle in [0.0, 0.7, 1.8] {
                let actor = Transform::from_xyz(10.0, 15.0, 0.0).with_rotation(Quat::from_rotation_y(angle)).with_scale(Vec3::splat(1.2));
                let (screen, depth) = project(bounds, actor.to_matrix(), clip, size).unwrap();
                let rect = bar_rect(screen, false, GaugeStyle::default()).unwrap();
                let tf = world_bar(rect, depth, clip.inverse(), size, camera.rotation).unwrap();
                let local = GlobalTransform::from(tf).reparented_to(&GlobalTransform::from(actor));
                let to_screen = |p: Vec3| { let p = clip.project_point3(p); Vec2::new(p.x + 1.0, 1.0 - p.y) * size * 0.5 };
                let a = to_screen(tf.transform_point(Vec3::new(-0.5, 0.5, 0.0)));
                let b = to_screen(tf.transform_point(Vec3::new(0.5, -0.5, 0.0)));
                assert!((a - rect.min).length() < 0.01, "{distance} {angle}: {a:?} {rect:?}");
                assert!((b - rect.max).length() < 0.01);
                assert!((b.y - a.y - 2.0).abs() < 0.01);
                let reparented = to_screen(actor.transform_point(local.transform_point(Vec3::new(-0.5, 0.5, 0.0))));
                assert!((reparented - rect.min).length() < 0.01);
            }
        }
        let clip = projection.get_clip_from_view();
        assert!(project(bounds, Mat4::from_translation(Vec3::new(0.0, 0.0, 500.0)), clip, size).is_none());
        assert!(project(bounds, Mat4::from_translation(Vec3::new(10000.0, 0.0, -500.0)), clip, size).is_none());
        assert!(project(bounds, Mat4::IDENTITY, clip, Vec2::ZERO).is_none());
    }
}
