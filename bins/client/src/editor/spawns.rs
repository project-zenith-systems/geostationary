//! Spawn point placement and marker rendering in the editor.
//!
//! Spawn markers are lightweight entities with [`SpawnMarker`] + [`Thing`] +
//! [`Transform`] plus a visible mesh overlay.  They are **not** simulated
//! (no physics components) and are only used to author map spawn points.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use things::{SpawnMarker, Thing};
use tiles::Tilemap;

use super::camera::EditorCamera;
use super::grid;
use super::palette::{EditorSelectedEntity, EditorTool};

/// Marker component for editor-specific spawn point overlays.
/// Distinguished from gameplay spawn markers so the editor can manage them
/// independently.
#[derive(Component)]
pub struct EditorSpawnMarker;

/// Shared mesh and material handles for spawn marker overlays.
#[derive(Resource)]
pub struct SpawnMarkerAssets {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// Initialise the shared mesh and material used for spawn marker overlays.
pub fn init_spawn_marker_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Sphere::new(0.25));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.2, 0.8, 1.0, 0.8),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    commands.insert_resource(SpawnMarkerAssets { mesh, material });
}

/// System: left-click on a floor tile to place a spawn marker when in Entity tool mode.
#[allow(clippy::too_many_arguments)]
pub fn place_spawn_marker(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    ui_interactions: Query<&Interaction>,
    tilemap: Option<Res<Tilemap>>,
    selected_entity: Option<Res<EditorSelectedEntity>>,
    tool: Option<Res<EditorTool>>,
    assets: Option<Res<SpawnMarkerAssets>>,
    mut commands: Commands,
) {
    // Only activate in Entity tool mode.
    let Some(tool) = tool else { return };
    if *tool != EditorTool::Entity {
        return;
    }

    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    // Skip placement when the pointer is over a UI element.
    if ui_interactions.iter().any(|i| *i != Interaction::None) {
        return;
    }

    let Some(selected) = selected_entity.and_then(|s| s.0.as_ref().cloned()) else {
        return;
    };
    let Some(tilemap) = tilemap else { return };
    let Some(assets) = assets else { return };

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
    let Some((_world_pos, grid_cell)) = grid::ray_to_grid_cell(ray) else {
        return;
    };

    // Only place on floor tiles.
    if !tilemap.is_walkable(grid_cell) {
        return;
    }

    let world_x = grid_cell.x as f32;
    let world_z = grid_cell.y as f32;

    commands.spawn((
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.material.clone()),
        Transform::from_xyz(world_x, 0.0, world_z),
        SpawnMarker,
        Thing {
            kind: selected.kind,
        },
        EditorSpawnMarker,
    ));

    info!(
        "Editor: placed '{}' spawn marker at ({}, {})",
        selected.name, grid_cell.x, grid_cell.y
    );
}

/// System: right-click on a spawn marker to delete it.
///
/// Checks all editor spawn markers and deletes the one closest to the cursor
/// grid cell (within a 0.6 world-unit threshold).
pub fn delete_spawn_marker(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    ui_interactions: Query<&Interaction>,
    markers: Query<(Entity, &Transform), With<EditorSpawnMarker>>,
    mut commands: Commands,
) {
    if !mouse_buttons.just_pressed(MouseButton::Right) {
        return;
    }

    // Skip deletion when the pointer is over a UI element.
    if ui_interactions.iter().any(|i| *i != Interaction::None) {
        return;
    }

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
    let Some((world_pos, _grid_cell)) = grid::ray_to_grid_cell(ray) else {
        return;
    };

    // Find the closest marker within threshold.
    let threshold = 0.6;
    let mut closest: Option<(Entity, f32)> = None;
    for (entity, transform) in &markers {
        let dist = Vec2::new(
            transform.translation.x - world_pos.x,
            transform.translation.z - world_pos.z,
        )
        .length();
        if dist < threshold && (closest.is_none() || dist < closest.unwrap().1) {
            closest = Some((entity, dist));
        }
    }

    if let Some((entity, _)) = closest {
        commands.entity(entity).despawn();
        info!("Editor: deleted spawn marker");
    }
}
