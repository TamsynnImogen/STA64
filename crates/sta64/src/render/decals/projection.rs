//! CPU projection of an image onto receiver-local hull triangles.
//!
//! The projector looks along its local +Z axis. Its box covers half the
//! authored width/height on either side of XY and half the depth on either
//! side of Z. A receiving face must point toward projector -Z. Image V is
//! zero at projector +Y (the top of a top-down image).

use bevy::math::{Mat4, Vec2, Vec3};

/// One receiver-local triangle. Normals are receiver-local vertex normals.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceTriangle {
    pub positions: [Vec3; 3],
    pub normals: [Vec3; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Projector {
    pub receiver_to_projector: Mat4,
    pub width: f32,
    pub height: f32,
    pub depth: f32,
    /// Minimum cosine of the angle between the face normal and projector -Z.
    pub facing_threshold: f32,
    /// Receiver-local offset along the interpolated vertex normal.
    pub normal_bias: f32,
    pub flip_u: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    InvalidProjector,
}

#[derive(Default, Debug)]
pub struct ProjectedMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Vertex {
    receiver: Vec3,
    projector: Vec3,
    normal: Vec3,
}

impl Vertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            receiver: self.receiver.lerp(other.receiver, t),
            projector: self.projector.lerp(other.projector, t),
            normal: self.normal.lerp(other.normal, t),
        }
    }
}

/// Clip receiver triangles to the finite projector box. A valid projector
/// with no accepted hull area returns an empty mesh, never a fallback quad.
pub fn project_surface(
    triangles: &[SurfaceTriangle],
    projector: &Projector,
) -> Result<ProjectedMesh, ProjectionError> {
    let m = projector.receiver_to_projector;
    if !m.to_cols_array().into_iter().all(f32::is_finite)
        || !projector.width.is_finite()
        || !projector.height.is_finite()
        || !projector.depth.is_finite()
        || projector.width <= 0.0
        || projector.height <= 0.0
        || projector.depth <= 0.0
        || !projector.width.recip().is_finite()
        || !projector.height.recip().is_finite()
        || !projector.facing_threshold.is_finite()
        || !(0.0..=1.0).contains(&projector.facing_threshold)
        || !projector.normal_bias.is_finite()
        || projector.normal_bias < 0.0
    {
        return Err(ProjectionError::InvalidProjector);
    }
    let inverse = m.inverse();
    if !inverse.to_cols_array().into_iter().all(f32::is_finite) {
        return Err(ProjectionError::InvalidProjector);
    }
    let normal_transform = inverse.transpose();
    let half = Vec3::new(
        projector.width * 0.5,
        projector.height * 0.5,
        projector.depth * 0.5,
    );
    if !half.is_finite() || half.min_element() <= 0.0 {
        return Err(ProjectionError::InvalidProjector);
    }

    let mut out = ProjectedMesh::default();
    for triangle in triangles {
        if !triangle
            .positions
            .iter()
            .chain(triangle.normals.iter())
            .all(|v| v.is_finite())
        {
            continue;
        }
        let face = (triangle.positions[1] - triangle.positions[0])
            .cross(triangle.positions[2] - triangle.positions[0]);
        let face_length = face.length();
        if !face_length.is_finite() || face_length <= 1e-12 {
            continue;
        }
        // A transformed edge cross changes sign under reflection. Transform
        // the receiver's face normal instead, so mirrored nodes retain their
        // authored front face.
        let facing = normal_transform.transform_vector3(face / face_length);
        let facing_length = facing.length();
        if !facing_length.is_finite()
            || facing_length <= 1e-12
            || facing.z >= 0.0
            || -facing.z / facing_length < projector.facing_threshold
        {
            continue;
        }
        let mut polygon = Vec::with_capacity(9);
        for i in 0..3 {
            let point = m.transform_point3(triangle.positions[i]);
            if !point.is_finite() || triangle.normals[i].length_squared() <= 1e-24 {
                polygon.clear();
                break;
            }
            polygon.push(Vertex {
                receiver: triangle.positions[i],
                projector: point,
                normal: triangle.normals[i],
            });
        }
        if polygon.len() != 3 {
            continue;
        }
        for (axis, bound, positive) in [
            (0, half.x, true),
            (0, half.x, false),
            (1, half.y, true),
            (1, half.y, false),
            (2, half.z, true),
            (2, half.z, false),
        ] {
            polygon = clip_plane(&polygon, axis, bound, positive);
            if polygon.len() < 3 {
                break;
            }
        }
        if polygon.len() < 3
            || polygon.len() > 9
            || polygon.iter().any(|v| {
                !v.receiver.is_finite() || !v.projector.is_finite() || !v.normal.is_finite()
            })
        {
            continue;
        }
        for i in 1..polygon.len() - 1 {
            let tri = [polygon[0], polygon[i], polygon[i + 1]];
            let area = (tri[1].receiver - tri[0].receiver)
                .cross(tri[2].receiver - tri[0].receiver)
                .length_squared();
            if !area.is_finite() || area <= 1e-24 || out.positions.len() > u32::MAX as usize - 3 {
                continue;
            }
            let base = out.positions.len() as u32;
            if tri.iter().any(|vertex| {
                let normal = vertex.normal.try_normalize().unwrap_or(face / face_length);
                !(vertex.receiver + normal * projector.normal_bias).is_finite()
                    || !(vertex.projector.x / projector.width).is_finite()
                    || !(vertex.projector.y / projector.height).is_finite()
            }) {
                continue;
            }
            for vertex in tri {
                let normal = vertex.normal.try_normalize().unwrap_or(face / face_length);
                let position = vertex.receiver + normal * projector.normal_bias;
                let mut u = vertex.projector.x / projector.width + 0.5;
                if projector.flip_u {
                    u = 1.0 - u;
                }
                let uv = Vec2::new(u, 0.5 - vertex.projector.y / projector.height);
                out.positions.push(position.to_array());
                out.normals.push(normal.to_array());
                out.uvs.push(uv.to_array());
            }
            out.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    }
    Ok(out)
}

fn clip_plane(input: &[Vertex], axis: usize, bound: f32, positive: bool) -> Vec<Vertex> {
    if input.is_empty() {
        return Vec::new();
    }
    let distance = |v: Vertex| {
        let coordinate = v.projector[axis];
        if positive {
            bound - coordinate
        } else {
            bound + coordinate
        }
    };
    let mut output = Vec::with_capacity(input.len() + 1);
    let mut previous = *input.last().expect("nonempty polygon");
    let mut previous_distance = distance(previous);
    for &current in input {
        let current_distance = distance(current);
        let previous_inside = previous_distance >= 0.0;
        let current_inside = current_distance >= 0.0;
        if previous_inside != current_inside {
            let t = previous_distance / (previous_distance - current_distance);
            output.push(previous.lerp(current, t));
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_distance = current_distance;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projector() -> Projector {
        Projector {
            receiver_to_projector: Mat4::IDENTITY,
            width: 2.0,
            height: 2.0,
            depth: 2.0,
            facing_threshold: 0.1,
            normal_bias: 0.0,
            flip_u: false,
        }
    }

    fn triangle(a: Vec3, b: Vec3, c: Vec3) -> SurfaceTriangle {
        SurfaceTriangle {
            positions: [a, b, c],
            normals: [Vec3::NEG_Z; 3],
        }
    }

    fn facing_triangle() -> SurfaceTriangle {
        triangle(
            Vec3::new(-0.5, -0.5, 0.0),
            Vec3::new(-0.5, 0.5, 0.0),
            Vec3::new(0.5, -0.5, 0.0),
        )
    }

    #[test]
    fn clips_each_box_plane_and_keeps_original_winding() {
        for axis in 0..3 {
            for sign in [-1.0, 1.0] {
                let mut t = facing_triangle();
                t.positions[2][axis] = sign * 2.0;
                if (t.positions[1] - t.positions[0])
                    .cross(t.positions[2] - t.positions[0])
                    .z
                    > 0.0
                {
                    t.positions.swap(1, 2);
                }
                let mesh = project_surface(&[t], &projector()).unwrap();
                assert!(!mesh.indices.is_empty(), "axis {axis}, sign {sign}");
                for p in &mesh.positions {
                    assert!(p[axis].abs() <= 1.00001, "axis {axis}, sign {sign}: {p:?}");
                }
                for face in mesh.indices.chunks_exact(3) {
                    let p = |i: u32| Vec3::from_array(mesh.positions[i as usize]);
                    let area = (p(face[1]) - p(face[0])).cross(p(face[2]) - p(face[0]));
                    assert!(
                        area.dot(
                            (t.positions[1] - t.positions[0])
                                .cross(t.positions[2] - t.positions[0])
                        ) > 0.0
                    );
                }
            }
        }
    }

    #[test]
    fn continuous_uvs_across_angled_multi_triangle_hull_and_flip() {
        let a = Vec3::new(-1.0, -1.0, 0.0);
        let b = Vec3::new(-1.0, 1.0, 0.0);
        let c = Vec3::new(0.0, -1.0, 0.4);
        let d = Vec3::new(0.0, 1.0, 0.4);
        let triangles = [triangle(a, b, c), triangle(c, b, d)];
        let plain = project_surface(&triangles, &projector()).unwrap();
        let flipped = project_surface(
            &triangles,
            &Projector {
                flip_u: true,
                ..projector()
            },
        )
        .unwrap();
        assert_eq!(plain.indices.len(), 6);
        for (p, uv) in plain.positions.iter().zip(&plain.uvs) {
            assert!((uv[0] - (p[0] * 0.5 + 0.5)).abs() < 1e-6);
            assert!((uv[1] - (0.5 - p[1] * 0.5)).abs() < 1e-6);
        }
        for (uv, flipped_uv) in plain.uvs.iter().zip(&flipped.uvs) {
            assert!((uv[0] + flipped_uv[0] - 1.0).abs() < 1e-6);
            assert_eq!(uv[1], flipped_uv[1]);
        }
    }

    #[test]
    fn rejects_backfaces_and_surfaces_beyond_depth() {
        let front = facing_triangle();
        let back = triangle(front.positions[0], front.positions[2], front.positions[1]);
        assert!(project_surface(&[back], &projector())
            .unwrap()
            .indices
            .is_empty());
        let far = triangle(
            front.positions[0] + Vec3::Z * 2.0,
            front.positions[1] + Vec3::Z * 2.0,
            front.positions[2] + Vec3::Z * 2.0,
        );
        assert!(project_surface(&[far], &projector())
            .unwrap()
            .indices
            .is_empty());
    }

    #[test]
    fn zero_threshold_still_rejects_exactly_side_on_wall() {
        let mut p = projector();
        p.facing_threshold = 0.0;
        let wall = triangle(
            Vec3::new(0.0, -0.5, -0.5),
            Vec3::new(0.0, 0.5, -0.5),
            Vec3::new(0.0, -0.5, 0.5),
        );
        assert!(project_surface(&[wall], &p).unwrap().indices.is_empty());
        assert!(!project_surface(&[facing_triangle()], &p)
            .unwrap()
            .indices
            .is_empty());
    }

    #[test]
    fn transformed_receiver_and_mirrored_nonuniform_normal() {
        let mut p = projector();
        p.receiver_to_projector = Mat4::from_translation(Vec3::new(0.25, 0.0, 0.0));
        let transformed = project_surface(&[facing_triangle()], &p).unwrap();
        assert!(
            (transformed.uvs[0][0] - (transformed.positions[0][0] + 0.25) * 0.5 - 0.5).abs() < 1e-6
        );
        p.receiver_to_projector = Mat4::from_scale(Vec3::new(-2.0, 0.5, 3.0));
        let mirrored = project_surface(&[facing_triangle()], &p).unwrap();
        assert!(!mirrored.indices.is_empty());
        assert!(mirrored.normals.iter().all(|normal| normal[2] < 0.0));
    }

    #[test]
    fn rejects_invalid_projector_and_skips_degenerate_vertices() {
        let mut p = projector();
        p.width = 0.0;
        assert_eq!(
            project_surface(&[], &p).unwrap_err(),
            ProjectionError::InvalidProjector
        );
        p = projector();
        p.receiver_to_projector.x_axis.x = f32::NAN;
        assert_eq!(
            project_surface(&[], &p).unwrap_err(),
            ProjectionError::InvalidProjector
        );
        p = projector();
        p.receiver_to_projector = Mat4::ZERO;
        assert_eq!(
            project_surface(&[], &p).unwrap_err(),
            ProjectionError::InvalidProjector
        );
        let mut t = facing_triangle();
        t.positions[0].x = f32::NAN;
        assert!(project_surface(&[t], &projector())
            .unwrap()
            .indices
            .is_empty());
        let degenerate = triangle(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        assert!(project_surface(&[degenerate], &projector())
            .unwrap()
            .indices
            .is_empty());
    }
}
