//! Editor mode (`AppState::Editor`).
//!
//! # Spike findings
//!
//! This module is the outcome of *Spike 2: Editor app state and camera*
//! (see `docs/plans/map-authoring.md`).  It provides a minimal but runnable
//! editor to confirm the three spike questions:
//!
//! ## Q1 – `AppState::Editor` coexists with `MainMenu` / `InGame`
//!
//! Each state uses `DespawnOnExit` independently.  Main-menu entities carry
//! `DespawnOnExit(AppState::MainMenu)` and are cleaned up as soon as
//! `AppState::MainMenu` is exited — whether the next state is `InGame` *or*
//! `Editor`.  Editor entities carry `DespawnOnExit(AppState::Editor)` and are
//! cleaned up on editor exit.  The `on_net_id_added` observer in `client.rs`
//! hardcodes `DespawnOnExit(AppState::InGame)` for replicated entities; since
//! the editor never has a live network connection those observers never fire,
//! so there is no interference.  The `clear_net_id_index` system also only
//! runs on `OnExit(AppState::InGame)` and is unaffected.
//!
//! ## Q2 – Orthographic camera + XZ-plane raycasting
//!
//! An orthographic top-down camera produces rays with direction ≈ `(0, −1, 0)`.
//! Intersecting with the y = 0 plane always produces a valid XZ world position
//! (see [`grid::ray_to_grid_cell`]).  The `round()` convention used in
//! `grid.rs` matches the existing `raycast_tiles` system in the `tiles` module,
//! so both paths identify the same tile entity by grid position.
//!
//! ## Q3 – Tile entity reuse
//!
//! The editor inserts a `Tilemap` resource on `OnEnter(AppState::Editor)`.
//! The existing `spawn_tile_meshes` system in `TilesPlugin` picks it up
//! automatically and spawns full mesh + collider entities — no editor-specific
//! spawn logic needed.  On exit the `Tilemap` resource and all `Tile` entities
//! are removed by [`teardown_editor_world`], leaving the ECS clean for the
//! next state.

use bevy::prelude::*;
use shared::app_state::AppState;
use tiles::{Tile, Tilemap};

pub mod camera;
pub mod grid;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Editor), (camera::spawn_editor_camera, setup_editor_tilemap));
        app.add_systems(OnExit(AppState::Editor), teardown_editor_world);
        app.add_systems(
            Update,
            (grid::log_hovered_cell, handle_editor_exit).run_if(in_state(AppState::Editor)),
        );
    }
}

/// Inserts the test-room `Tilemap` resource so the `TilesPlugin`'s
/// `spawn_tile_meshes` system materialises tile entities automatically.
///
/// In the full editor this would load (or create) a map file; for the spike it
/// reuses `Tilemap::test_room()` to confirm that the game's existing rendering
/// path works unmodified inside `AppState::Editor`.
fn setup_editor_tilemap(mut commands: Commands) {
    commands.insert_resource(Tilemap::test_room());
}

/// Removes the `Tilemap` resource and despawns all `Tile` entities when leaving
/// the editor.
///
/// Tile entities spawned by `TilesPlugin::spawn_tile_meshes` do not carry a
/// `DespawnOnExit` marker (that system is shared with `InGame` and is
/// state-agnostic), so explicit cleanup is needed here.
fn teardown_editor_world(
    mut commands: Commands,
    tile_entities: Query<Entity, With<Tile>>,
) {
    commands.remove_resource::<Tilemap>();
    for entity in &tile_entities {
        commands.entity(entity).despawn();
    }
}

/// Returns to `AppState::MainMenu` when Escape is pressed.
fn handle_editor_exit(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_state.set(AppState::MainMenu);
    }
}
