//! Spawn point placement and marker rendering in the editor.
//!
//! Spawn markers are lightweight entities with [`SpawnMarker`] + [`Thing`] +
//! [`Transform`] plus a visual mesh from the thing's template.  They are
//! **not** simulated (no physics components) and are only used to author map
//! spawn points.  The visual is created via [`SpawnThingVisual`] so entities
//! look the same as they do in-game.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use things::{SpawnMarker, SpawnThingVisual};
use tiles::{TileGrid, TileKind};

use super::camera::EditorCamera;
use super::grid;
use super::palette::{EditorSelectedEntity, EditorTool};

/// Marker component for editor-specific spawn point overlays.
/// Distinguished from gameplay spawn markers so the editor can manage them
/// independently.
#[derive(Component)]
pub struct EditorSpawnMarker;

/// System: left-click on a floor tile to place a spawn marker when in Entity tool mode.
#[allow(clippy::too_many_arguments)]
pub fn place_spawn_marker(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    ui_interactions: Query<&Interaction>,
    grid: Option<Res<TileGrid<TileKind>>>,
    selected_entity: Option<Res<EditorSelectedEntity>>,
    tool: Option<Res<EditorTool>>,
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
    let Some(grid) = grid else { return };

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
    if !grid.get_copy(grid_cell).is_some_and(|k| k.is_walkable()) {
        return;
    }

    let world_x = grid_cell.x as f32;
    let world_z = grid_cell.y as f32;
    let position = Vec3::new(world_x, 0.0, world_z);

    let entity = commands.spawn((SpawnMarker, EditorSpawnMarker)).id();
    commands.trigger(SpawnThingVisual {
        entity,
        kind: selected.kind,
        position,
    });

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
