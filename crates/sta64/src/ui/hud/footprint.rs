//! Presentation mesh for the station placement grid. Its outer border follows
//! the supplied physical bounds; only interior cell spacing is configurable.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::{Mesh, Vec2};

const LINE_WIDTH: f32 = 0.025;
const FILL_ALPHA: f32 = 0.12;
const FILL_Y: f32 = -0.001;

/// A centered square with one translucent fill and one ribbon per grid line.
#[cfg(test)]
pub(super) fn mesh(cells: u32) -> Mesh {
    rectangular_mesh(cells, cells)
}

/// Rectangular footprint, retaining square cells on both axes.
#[cfg(test)]
pub(super) fn rectangular_mesh(columns: u32, rows: u32) -> Mesh {
    sized_mesh(Vec2::new(columns as f32 * 0.5, rows as f32 * 0.5), 1.0)
}

/// Grid with exact world-space half extents. The final cell on each axis may
/// be shorter than `spacing`, so the border never overstates the site's size.
pub(super) fn sized_mesh(half: Vec2, spacing: f32) -> Mesh {
    assert!(half.x.is_finite() && half.y.is_finite() && half.x > 0.0 && half.y > 0.0);
    assert!(spacing.is_finite() && spacing > 0.0);
    let (left, right) = (-half.x, half.x);
    let (bottom, top) = (-half.y, half.y);
    let columns = (half.x * 2.0 / spacing).ceil() as u32;
    let rows = (half.y * 2.0 / spacing).ceil() as u32;
    let quads = 1 + columns as usize + rows as usize + 2;
    let line_width = (spacing * LINE_WIDTH).min(half.min_element() * 2.0);
    let mut positions = Vec::with_capacity(quads * 4);
    let mut normals = Vec::with_capacity(quads * 4);
    let mut uvs = Vec::with_capacity(quads * 4);
    let mut colors = Vec::with_capacity(quads * 4);
    let mut indices = Vec::with_capacity(quads * 6);
    let mut quad = |x0: f32, z0: f32, x1: f32, z1: f32, y: f32, alpha: f32| {
        let base = positions.len() as u32;
        positions.extend_from_slice(&[[x0, y, z0], [x0, y, z1], [x1, y, z1], [x1, y, z0]]);
        normals.extend_from_slice(&[[0.0, 1.0, 0.0]; 4]);
        uvs.extend_from_slice(&[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);
        colors.extend_from_slice(&[[1.0, 1.0, 1.0, alpha]; 4]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    // Recess the fill slightly so the white ribbons remain visible over it.
    quad(left, bottom, right, top, FILL_Y * spacing, FILL_ALPHA);
    for line in 0..=columns.max(rows) {
        if line <= columns {
            let at = if line == columns {
                right
            } else {
                left + line as f32 * spacing
            };
            quad(
                (at - line_width * 0.5).max(left),
                bottom,
                (at + line_width * 0.5).min(right),
                top,
                0.0,
                1.0,
            );
        }
        if line <= rows {
            let at = if line == rows {
                top
            } else {
                bottom + line as f32 * spacing
            };
            quad(
                left,
                (at - line_width * 0.5).max(bottom),
                right,
                (at + line_width * 0.5).min(top),
                0.0,
                1.0,
            );
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn positions(mesh: &Mesh) -> &[[f32; 3]] {
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("grid positions");
        };
        positions
    }

    #[test]
    fn grid_is_centered_square_with_exact_unit_spaced_boundaries() {
        let cells = 4;
        let grid = mesh(cells);
        let positions = positions(&grid);
        let quads = 1 + 2 * (cells as usize + 1);
        assert_eq!(positions.len(), quads * 4);
        let Some(Indices::U32(indices)) = grid.indices() else {
            panic!("grid indices");
        };
        assert_eq!(indices.len(), quads * 6);
        for axis in [0, 2] {
            assert_eq!(
                positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min),
                -2.0
            );
            assert_eq!(
                positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max),
                2.0
            );
        }
        for line in 0..=cells as usize {
            let at = -2.0 + line as f32;
            let vertical = &positions[(1 + line * 2) * 4..(2 + line * 2) * 4];
            let horizontal = &positions[(2 + line * 2) * 4..(3 + line * 2) * 4];
            let start = (at - LINE_WIDTH * 0.5).max(-2.0);
            let end = (at + LINE_WIDTH * 0.5).min(2.0);
            assert_eq!((vertical[0][0], vertical[2][0]), (start, end));
            assert_eq!((horizontal[0][2], horizontal[2][2]), (start, end));
            assert!(vertical.iter().all(|p| p[1] == 0.0));
            assert!(horizontal.iter().all(|p| p[1] == 0.0));
        }
    }

    #[test]
    fn rectangular_regions_retain_square_cell_spacing() {
        let grid = rectangular_mesh(6, 2);
        let positions = positions(&grid);
        assert_eq!(positions.len(), (1 + 7 + 3) * 4);
        for (axis, half) in [(0, 3.0), (2, 1.0)] {
            assert_eq!(
                positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min),
                -half
            );
            assert_eq!(
                positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max),
                half
            );
        }
    }

    #[test]
    fn nonmultiple_square_has_exact_border_and_a_clipped_final_cell() {
        let grid = sized_mesh(Vec2::splat(37.5), 50.0);
        let positions = positions(&grid);
        assert_eq!(positions.len(), (1 + 3 + 3) * 4);
        for point in positions {
            assert!((-37.5..=37.5).contains(&point[0]));
            assert!((-37.5..=37.5).contains(&point[2]));
        }
        assert_eq!((positions[0][0], positions[2][0]), (-37.5, 37.5));
        assert_eq!((positions[0][2], positions[2][2]), (-37.5, 37.5));
        // Quads are fill, then vertical and horizontal strips per boundary.
        let vertical = |line: usize| &positions[(1 + line * 2) * 4..(2 + line * 2) * 4];
        assert_eq!(vertical(0)[0][0], -37.5);
        assert_eq!(vertical(2)[2][0], 37.5);
        assert_eq!(vertical(1)[0][0], 12.5 - LINE_WIDTH * 50.0 * 0.5);
        assert_eq!(vertical(1)[2][0], 12.5 + LINE_WIDTH * 50.0 * 0.5);
        let inner = (vertical(1)[0][0] + vertical(1)[2][0]) * 0.5;
        assert_eq!(vertical(2)[2][0] - inner, 25.0);
    }

    #[test]
    fn asymmetric_rectangle_keeps_spacing_and_clips_each_last_edge() {
        let half = Vec2::new(62.5, 37.5);
        let positions = positions(&sized_mesh(half, 50.0)).to_vec();
        assert_eq!(positions.len(), (1 + 4 + 3) * 4);
        assert!(
            positions
                .iter()
                .all(|p| p[0] >= -half.x && p[0] <= half.x && p[2] >= -half.y && p[2] <= half.y)
        );
        assert_eq!((positions[0][0], positions[2][0]), (-62.5, 62.5));
        assert_eq!((positions[0][2], positions[2][2]), (-37.5, 37.5));
        let x_strip = |line: usize| &positions[(1 + line * 2) * 4..(2 + line * 2) * 4];
        let z_strip = |line: usize| &positions[(2 + line * 2) * 4..(3 + line * 2) * 4];
        let x1 = (x_strip(1)[0][0] + x_strip(1)[2][0]) * 0.5;
        let x2 = (x_strip(2)[0][0] + x_strip(2)[2][0]) * 0.5;
        assert_eq!((x1, x2, x2 - x1), (-12.5, 37.5, 50.0));
        assert_eq!((z_strip(1)[0][2] + z_strip(1)[2][2]) * 0.5, 12.5);
        assert_eq!(x_strip(3)[2][0], half.x);
        assert_eq!(z_strip(2)[2][2], half.y);
    }

    #[test]
    fn fill_is_faint_and_lines_are_white_with_upward_normals() {
        let grid = mesh(2);
        let Some(VertexAttributeValues::Float32x4(colors)) = grid.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("grid colors");
        };
        assert!(
            colors[..4]
                .iter()
                .all(|c| *c == [1.0, 1.0, 1.0, FILL_ALPHA])
        );
        assert!(colors[4..].iter().all(|c| *c == [1.0, 1.0, 1.0, 1.0]));
        let Some(VertexAttributeValues::Float32x3(normals)) =
            grid.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("grid normals");
        };
        assert!(normals.iter().all(|n| *n == [0.0, 1.0, 0.0]));
    }
}
