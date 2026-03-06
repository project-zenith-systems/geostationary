//! Editor mode (`AppState::Editor`).
//!
//! A live Bevy world with simulation disabled, providing:
//! - Orthographic top-down camera with pan (WASD / middle-click drag) and zoom (scroll)
//! - Tile palette: select Floor or Wall, left-click/drag to paint
//! - Entity palette: select a template (ball, can, toolbox), click a floor tile to place a spawn marker
//! - Spawn markers are visible overlays, not simulated entities
//! - Right-click on a spawn marker to delete it
//! - Save/Load `.station.ron` files via `MapLayer` trait

use bevy::prelude::*;
use shared::app_state::AppState;
use tiles::{Tile, TileKind, Tilemap};

pub mod camera;
pub mod grid;
pub mod io;
pub mod painting;
pub mod palette;
pub mod spawns;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // Resources.
        app.init_resource::<palette::EditorSelectedTile>();
        app.init_resource::<palette::EditorSelectedEntity>();
        app.init_resource::<palette::EditorTool>();
        app.init_resource::<io::EditorMapFile>();

        // Messages for palette UI events and save/load.
        app.add_message::<palette::EditorUiEvent>();
        app.add_message::<io::EditorSaveEvent>();
        app.add_message::<io::EditorLoadEvent>();

        // OnEnter: spawn camera, tilemap, palette UI, spawn marker assets.
        app.add_systems(
            OnEnter(AppState::Editor),
            (
                camera::spawn_editor_camera,
                setup_editor_tilemap,
                palette::spawn_palette_ui,
                spawns::init_spawn_marker_assets,
            ),
        );

        // OnExit: clean up editor world.
        app.add_systems(OnExit(AppState::Editor), teardown_editor_world);

        // Update systems gated on Editor state — split into groups to stay
        // within Bevy's tuple size limits for `.run_if()`.
        let in_editor = in_state(AppState::Editor);

        // Camera controls.
        app.add_systems(
            Update,
            (
                camera::camera_pan_keyboard,
                camera::camera_pan_drag,
                camera::camera_zoom,
            )
                .run_if(in_editor.clone()),
        );

        // Painting, spawns, and palette UI.
        // UI systems run first so that tool selection is processed before
        // world-editing systems, preventing a click from both selecting a
        // tool and painting/placing in the same frame.
        app.add_systems(
            Update,
            (
                palette::handle_palette_buttons,
                palette::process_palette_events,
                painting::paint_tiles.run_if(is_tile_tool),
                spawns::place_spawn_marker,
                spawns::delete_spawn_marker,
            )
                .chain()
                .run_if(in_editor.clone()),
        );

        // Save/load and exit handler.
        app.add_systems(
            Update,
            (io::handle_save, io::handle_load, handle_editor_exit).run_if(in_editor),
        );
    }
}

/// Run condition: returns true when the editor tool is set to [`palette::EditorTool::Tile`].
fn is_tile_tool(tool: Option<Res<palette::EditorTool>>) -> bool {
    tool.is_some_and(|t| *t == palette::EditorTool::Tile)
}

/// Inserts a default 32×32 `Tilemap` resource for the editor.
///
/// Creates a room with perimeter walls and interior floor tiles.
fn setup_editor_tilemap(mut commands: Commands) {
    commands.insert_resource(default_editor_tilemap());
}

/// Creates a default 32×32 tilemap with perimeter walls and floor interior.
pub fn default_editor_tilemap() -> Tilemap {
    let size = 32_u32;
    let mut tilemap = Tilemap::new(size, size, TileKind::Floor);

    // Build perimeter walls.
    for x in 0..size as i32 {
        tilemap.set(IVec2::new(x, 0), TileKind::Wall);
        tilemap.set(IVec2::new(x, size as i32 - 1), TileKind::Wall);
    }
    for y in 0..size as i32 {
        tilemap.set(IVec2::new(0, y), TileKind::Wall);
        tilemap.set(IVec2::new(size as i32 - 1, y), TileKind::Wall);
    }

    tilemap
}

/// Removes editor resources and despawns all tile and spawn marker entities
/// when leaving the editor.
fn teardown_editor_world(
    mut commands: Commands,
    tile_entities: Query<Entity, With<Tile>>,
    spawn_marker_entities: Query<Entity, With<spawns::EditorSpawnMarker>>,
) {
    commands.remove_resource::<Tilemap>();
    commands.remove_resource::<spawns::SpawnMarkerAssets>();

    for entity in &tile_entities {
        commands.entity(entity).despawn();
    }
    for entity in &spawn_marker_entities {
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
