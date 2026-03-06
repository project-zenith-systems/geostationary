use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
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
    // Centre on a 32×32 map.
    let center_x = 15.5_f32;
    let center_z = 15.5_f32;

    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            // scale controls world units per pixel; 0.03 keeps a 32×32 map
            // comfortably visible in a 1280×720 window.
            scale: 0.03,
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

/// Pan speed in world units per second when using WASD keys.
const PAN_SPEED: f32 = 15.0;

/// Zoom speed multiplier for scroll wheel input.
const ZOOM_SPEED: f32 = 0.003;

/// Minimum orthographic scale (max zoom in).
const MIN_SCALE: f32 = 0.005;

/// Maximum orthographic scale (max zoom out).
const MAX_SCALE: f32 = 0.15;

/// Pan sensitivity for middle-click drag (world units per pixel of mouse movement).
const DRAG_PAN_SENSITIVITY: f32 = 0.05;

/// System: WASD keyboard panning for the editor camera.
pub fn camera_pan_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut camera_query: Query<(&mut Transform, &Projection), With<EditorCamera>>,
) {
    let Ok((mut transform, projection)) = camera_query.single_mut() else {
        return;
    };

    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };

    let mut direction = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        direction.z -= 1.0; // Up on screen = -Z in world (camera up is NEG_Z)
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        direction.z += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        direction.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        direction.x += 1.0;
    }

    if direction != Vec3::ZERO {
        direction = direction.normalize();
        // Scale pan speed by the orthographic scale so panning feels consistent
        // at any zoom level.
        let speed = PAN_SPEED * (scale / 0.03);
        transform.translation += direction * speed * time.delta_secs();
    }
}

/// System: middle-click drag panning for the editor camera.
pub fn camera_pan_drag(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut camera_query: Query<(&mut Transform, &Projection), With<EditorCamera>>,
) {
    if !mouse_buttons.pressed(MouseButton::Middle) {
        // Drain events even when not panning so they don't accumulate.
        mouse_motion.read().count();
        return;
    }

    let Ok((mut transform, projection)) = camera_query.single_mut() else {
        mouse_motion.read().count();
        return;
    };

    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };

    let mut delta = Vec2::ZERO;
    for event in mouse_motion.read() {
        delta += event.delta;
    }

    if delta != Vec2::ZERO {
        // Drag panning: mouse moves right → camera moves left (screen follows cursor).
        // Scale sensitivity by orthographic scale so drag distance maps to
        // consistent world distance at any zoom level.
        let sensitivity = DRAG_PAN_SENSITIVITY * (scale / 0.03);
        transform.translation.x -= delta.x * sensitivity;
        transform.translation.z -= delta.y * sensitivity;
    }
}

/// System: scroll wheel zoom for the editor camera.
pub fn camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut camera_query: Query<&mut Projection, With<EditorCamera>>,
) {
    let Ok(mut projection) = camera_query.single_mut() else {
        scroll_events.read().count();
        return;
    };

    let mut scroll_delta = 0.0_f32;
    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * 3.0,
            MouseScrollUnit::Pixel => event.y,
        };
    }

    if scroll_delta != 0.0 {
        if let Projection::Orthographic(ref mut ortho) = *projection {
            // Scroll up → zoom in (decrease scale).
            ortho.scale = (ortho.scale - scroll_delta * ZOOM_SPEED).clamp(MIN_SCALE, MAX_SCALE);
        }
    }
}
