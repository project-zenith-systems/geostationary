use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::camera::EditorCamera;

/// Intersects a camera ray with the y = 0 plane (the tile grid surface) and
/// returns the world-space hit position together with the integer grid cell.
///
/// Grid cells are addressed the same way as [`tiles::Tile::position`]:
/// * column = `world_pos.x.round() as i32`
/// * row    = `world_pos.z.round() as i32`
///
/// Returns `None` when the ray is parallel to the XZ plane (camera looking
/// straight along the horizon) or when the intersection would be behind the
/// camera.
///
/// # Spike answers
///
/// * **Q2 – orthographic + XZ raycasting**: An orthographic top-down camera
///   produces rays with direction ≈ `(0, -1, 0)`.  Intersecting with y = 0
///   always succeeds (`dir.y ≠ 0`), giving exact sub-pixel XZ accuracy.
///   The `round()` convention matches the existing `raycast_tiles` system in
///   the `tiles` module, so the same tile entity can be identified by both
///   paths — confirming **Q3** (tile entities are reusable without changes).
pub fn ray_to_grid_cell(ray: Ray3d) -> Option<(Vec3, IVec2)> {
    let dir = Vec3::from(ray.direction);

    // Use the same practical threshold as `raycast_tiles` in the tiles module.
    if dir.y.abs() < 1e-4 {
        return None; // ray parallel to XZ plane
    }
    let t = -ray.origin.y / dir.y;
    if t < 0.0 {
        return None; // intersection is behind the camera
    }
    let world_pos = ray.origin + t * dir;
    // Match the `round()` convention used in the tiles module's raycast_tiles.
    let grid_cell = IVec2::new(world_pos.x.round() as i32, world_pos.z.round() as i32);
    Some((world_pos, grid_cell))
}

/// System: traces the cursor ray and logs the grid cell under the pointer.
///
/// In the full editor implementation this system drives tile painting; for the
/// spike it confirms at runtime that orthographic + XZ raycasting resolves
/// to the correct cell without requiring a running physics engine.
pub fn log_hovered_cell(
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
) {
    let Ok(window) = window_query.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
        return;
    };
    if let Some((_world_pos, grid_cell)) = ray_to_grid_cell(ray) {
        trace!("Editor cursor → grid cell {grid_cell}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Dir3;

    // Helper: construct a Ray3d pointing straight down from a given world position.
    fn ray_down(origin: Vec3) -> Ray3d {
        Ray3d::new(origin, Dir3::NEG_Y)
    }

    /// An orthographic top-down camera produces downward rays.
    /// Verifies that they intersect the y=0 plane at the expected XZ position
    /// and that `round()` maps the position to the correct grid cell.
    #[test]
    fn test_ray_to_grid_cell_direct_down() {
        // Camera 20 units above tile (3, 7): world x=3.5 z=7.5
        let ray = ray_down(Vec3::new(3.5, 20.0, 7.5));
        let (world_pos, grid_cell) = ray_to_grid_cell(ray).expect("downward ray must hit y=0");
        assert!(world_pos.y.abs() < 1e-4, "hit must be on y=0 plane, got y={}", world_pos.y);
        // round(3.5) == 4, round(7.5) == 8 (f32::round rounds half away from zero)
        assert_eq!(grid_cell, IVec2::new(4, 8));
    }

    /// Verifies the plain-center case: origin directly above a tile centre.
    #[test]
    fn test_ray_to_grid_cell_tile_centre() {
        let ray = ray_down(Vec3::new(5.0, 10.0, 3.0));
        let (_, grid_cell) = ray_to_grid_cell(ray).unwrap();
        assert_eq!(grid_cell, IVec2::new(5, 3));
    }

    /// A ray with a positive Y direction (pointing upward) should return None.
    #[test]
    fn test_ray_to_grid_cell_upward_ray_returns_none() {
        let ray = Ray3d::new(Vec3::new(0.0, 10.0, 0.0), Dir3::Y);
        assert!(
            ray_to_grid_cell(ray).is_none(),
            "upward ray must not intersect the y=0 plane from above"
        );
    }

    /// A ray parallel to the XZ plane (horizontal) should return None.
    #[test]
    fn test_ray_to_grid_cell_horizontal_ray_returns_none() {
        let ray = Ray3d::new(Vec3::new(0.0, 5.0, 0.0), Dir3::X);
        assert!(
            ray_to_grid_cell(ray).is_none(),
            "horizontal ray must not intersect the y=0 plane"
        );
    }

    /// Verifies that tile-boundary rounding works: world x just below 4.5 →
    /// round to 4; world x just above 4.5 → round to 5.
    #[test]
    fn test_ray_to_grid_cell_boundary_rounding() {
        let ray_below = ray_down(Vec3::new(4.49, 10.0, 0.0));
        let (_, cell_below) = ray_to_grid_cell(ray_below).unwrap();
        assert_eq!(cell_below.x, 4, "4.49 should round to column 4");

        let ray_above = ray_down(Vec3::new(4.51, 10.0, 0.0));
        let (_, cell_above) = ray_to_grid_cell(ray_above).unwrap();
        assert_eq!(cell_above.x, 5, "4.51 should round to column 5");
    }

    /// Verifies that negative grid coordinates are handled correctly.
    #[test]
    fn test_ray_to_grid_cell_negative_coords() {
        let ray = ray_down(Vec3::new(-2.0, 5.0, -3.0));
        let (_, grid_cell) = ray_to_grid_cell(ray).unwrap();
        assert_eq!(grid_cell, IVec2::new(-2, -3));
    }
}
