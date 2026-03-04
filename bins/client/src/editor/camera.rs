use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use shared::app_state::AppState;

/// Marker component for the editor's orthographic top-down camera.
#[derive(Component)]
pub struct EditorCamera;

/// Spawns an orthographic top-down camera when entering [`AppState::Editor`].
///
/// The camera looks straight down the Y axis so the XZ plane (tile grid) fills
/// the viewport.  `Vec3::NEG_Z` is used as the `up` direction so that the
/// positive-Z (row) axis points toward the top of the screen — consistent with
/// a top-down 2D-style view of the XZ grid.
///
/// `DespawnOnExit(AppState::Editor)` ensures the camera is cleaned up
/// automatically when leaving the editor, regardless of which state is entered
/// next.
pub fn spawn_editor_camera(mut commands: Commands) {
    // Centre on the test tilemap (16×10), keeping a margin above the origin.
    let center_x = 7.5_f32;
    let center_z = 4.5_f32;

    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            // scale controls world units per pixel; 0.02 keeps a 16×10 map
            // comfortably visible in a 1280×720 window.
            scale: 0.02,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(center_x, 20.0, center_z)
            .looking_at(Vec3::new(center_x, 0.0, center_z), Vec3::NEG_Z),
        AmbientLight {
            color: Color::WHITE,
            brightness: 500.0,
            affects_lightmapped_meshes: true,
        },
        EditorCamera,
        DespawnOnExit(AppState::Editor),
    ));
}
