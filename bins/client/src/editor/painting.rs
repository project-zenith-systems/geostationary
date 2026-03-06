//! Tile painting: left-click or left-drag to paint the selected tile kind.
//!
//! Reads the cursor position, raycasts to the grid, and updates both the
//! [`Tilemap`] resource and the visual tile entities via [`TileMutated`].

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use tiles::{TileKind, TileMutated, Tilemap};

use super::camera::EditorCamera;
use super::grid;
use super::palette::EditorSelectedTile;

/// System: paint tiles on left-click or left-drag.
///
/// When the left mouse button is pressed (or held), traces the cursor ray to
/// the grid and sets the tile at that position to the currently selected
/// [`TileKind`] from [`EditorSelectedTile`].  Emits a [`TileMutated`] event
/// so the existing `apply_tile_mutation` system in `TilesPlugin` updates the
/// visual representation.
pub fn paint_tiles(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    mut tilemap: Option<ResMut<Tilemap>>,
    selected_tile: Option<Res<EditorSelectedTile>>,
    mut mutation_events: MessageWriter<TileMutated>,
) {
    if !mouse_buttons.pressed(MouseButton::Left) {
        return;
    }

    let Some(ref mut tilemap) = tilemap else {
        return;
    };
    let Some(selected) = selected_tile else {
        return;
    };

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

    // Only paint if the cell is within the tilemap bounds and the kind differs.
    if let Some(current_kind) = tilemap.get(grid_cell) {
        if current_kind != selected.0 {
            tilemap.set(grid_cell, selected.0);
            mutation_events.write(TileMutated {
                position: grid_cell,
                kind: selected.0,
            });
        }
    }
}
