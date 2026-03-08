use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use shared::app_state::AppState;

/// Marker component for the editor's perspective camera.
#[derive(Component)]
pub struct EditorCamera;

/// Orbit state for the editor camera.
///
/// The camera orbits around [`focus`] at [`distance`] away, looking down at
/// an angle defined by [`pitch`] with a horizontal angle defined by [`yaw`].
#[derive(Resource)]
pub struct EditorOrbit {
    /// World-space point the camera orbits around.
    pub focus: Vec3,
    /// Distance from the focus point.
    pub distance: f32,
    /// Vertical angle in radians (0 = horizontal, π/2 = straight down).
    pub pitch: f32,
    /// Horizontal angle in radians.
    pub yaw: f32,
}

impl Default for EditorOrbit {
    fn default() -> Self {
        Self {
            focus: Vec3::new(15.5, 0.0, 15.5),
            distance: 30.0,
            pitch: std::f32::consts::FRAC_PI_4, // 45 degrees
            yaw: 0.0,
        }
    }
}

impl EditorOrbit {
    /// Compute the camera [`Transform`] from the current orbit state.
    pub fn camera_transform(&self) -> Transform {
        let x = self.focus.x + self.distance * self.pitch.cos() * self.yaw.sin();
        let y = self.focus.y + self.distance * self.pitch.sin();
        let z = self.focus.z + self.distance * self.pitch.cos() * self.yaw.cos();
        Transform::from_xyz(x, y, z).looking_at(self.focus, Vec3::Y)
    }
}

/// Spawns a perspective camera angled down at the tile grid when entering
/// [`AppState::Editor`].
///
/// `DespawnOnExit(AppState::Editor)` ensures the camera is cleaned up
/// automatically when leaving the editor.
pub fn spawn_editor_camera(mut commands: Commands) {
    let orbit = EditorOrbit::default();
    let transform = orbit.camera_transform();

    commands.spawn((
        Camera3d::default(),
        transform,
        AmbientLight {
            color: Color::WHITE,
            brightness: 500.0,
            affects_lightmapped_meshes: true,
        },
        EditorCamera,
        DespawnOnExit(AppState::Editor),
    ));

    commands.insert_resource(orbit);
}

/// Pan speed in world units per second when using WASD / arrow keys.
const PAN_SPEED: f32 = 20.0;

/// Zoom speed multiplier for scroll wheel input.
const ZOOM_SPEED: f32 = 1.5;

/// Minimum orbit distance (max zoom in).
const MIN_DISTANCE: f32 = 5.0;

/// Maximum orbit distance (max zoom out).
const MAX_DISTANCE: f32 = 80.0;

/// Rotation speed in radians per second for Q/E keys.
const ROTATE_SPEED: f32 = 2.0;

/// Drag sensitivity for middle-click drag (world units per pixel of mouse movement).
const DRAG_PAN_SENSITIVITY: f32 = 0.05;

/// System: WASD keyboard panning for the editor camera (moves the orbit focus).
pub fn camera_pan_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut orbit: ResMut<EditorOrbit>,
    mut camera_query: Query<&mut Transform, With<EditorCamera>>,
) {
    let Ok(mut transform) = camera_query.single_mut() else {
        return;
    };

    let mut direction = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        direction.z -= 1.0;
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
        // Pan in the horizontal plane aligned with the camera's yaw.
        let rotated = Quat::from_rotation_y(orbit.yaw) * direction;
        let speed = PAN_SPEED * (orbit.distance / 30.0);
        orbit.focus += rotated * speed * time.delta_secs();
        *transform = orbit.camera_transform();
    }
}

/// System: Q/E keyboard rotation for the editor camera.
pub fn camera_rotate(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut orbit: ResMut<EditorOrbit>,
    mut camera_query: Query<&mut Transform, With<EditorCamera>>,
) {
    let Ok(mut transform) = camera_query.single_mut() else {
        return;
    };

    let mut rotation = 0.0_f32;
    if keys.pressed(KeyCode::KeyQ) {
        rotation -= 1.0;
    }
    if keys.pressed(KeyCode::KeyE) {
        rotation += 1.0;
    }

    if rotation != 0.0 {
        orbit.yaw += rotation * ROTATE_SPEED * time.delta_secs();
        *transform = orbit.camera_transform();
    }
}

/// System: middle-click drag panning for the editor camera.
pub fn camera_pan_drag(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut orbit: ResMut<EditorOrbit>,
    mut camera_query: Query<&mut Transform, With<EditorCamera>>,
) {
    if !mouse_buttons.pressed(MouseButton::Middle) {
        mouse_motion.read().count();
        return;
    }

    let Ok(mut transform) = camera_query.single_mut() else {
        mouse_motion.read().count();
        return;
    };

    let mut delta = Vec2::ZERO;
    for event in mouse_motion.read() {
        delta += event.delta;
    }

    if delta != Vec2::ZERO {
        let sensitivity = DRAG_PAN_SENSITIVITY * (orbit.distance / 30.0);
        // Pan in the horizontal plane aligned with the camera's yaw.
        let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
        let forward = Vec3::new(orbit.yaw.sin(), 0.0, orbit.yaw.cos());
        orbit.focus -= right * delta.x * sensitivity;
        orbit.focus -= forward * delta.y * sensitivity;
        *transform = orbit.camera_transform();
    }
}

/// System: scroll wheel zoom for the editor camera (changes orbit distance).
pub fn camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut orbit: ResMut<EditorOrbit>,
    mut camera_query: Query<&mut Transform, With<EditorCamera>>,
) {
    let Ok(mut transform) = camera_query.single_mut() else {
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
        orbit.distance =
            (orbit.distance - scroll_delta * ZOOM_SPEED).clamp(MIN_DISTANCE, MAX_DISTANCE);
        *transform = orbit.camera_transform();
    }
}
