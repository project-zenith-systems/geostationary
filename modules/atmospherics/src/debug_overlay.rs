use bevy::prelude::*;
use tiles::{TileKind, TileMutated, Tilemap};

use crate::GasGrid;

/// Normal pressure threshold used by the overlay color scale.
///
/// Values are in the same pressure units as `GasGrid::pressure_at`, where
/// station baseline pressure is configured around `101.325`.
const OVERLAY_NORMAL_PRESSURE: f32 = 101.325;
/// High-pressure threshold for the overlay color scale (`1.5x` normal).
const OVERLAY_HIGH_PRESSURE: f32 = OVERLAY_NORMAL_PRESSURE * 1.5;

/// Resource that controls the atmospheric pressure debug overlay.
/// When true, the overlay is visible. When false, it is hidden.
#[derive(Resource, Default)]
pub struct AtmosDebugOverlay(pub bool);

/// Marker component for overlay quad entities.
/// Each entity represents one cell in the gas grid, positioned at the corresponding tile location.
#[derive(Component)]
pub struct OverlayQuad {
    pub position: IVec2,
    /// Mesh handle stored for proper cleanup when the quad is despawned.
    /// This prevents memory leaks by allowing the mesh to be removed from the asset store.
    mesh: Handle<Mesh>,
    /// Material handle stored for proper cleanup when the quad is despawned.
    /// This prevents material asset leaks during overlay toggle cycles.
    material: Handle<StandardMaterial>,
}

/// System that toggles the debug overlay on F3 keypress.
pub fn toggle_overlay(keyboard: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<AtmosDebugOverlay>) {
    if keyboard.just_pressed(KeyCode::F3) {
        overlay.0 = !overlay.0;
        info!(
            "Atmospheric debug overlay: {}",
            if overlay.0 { "ON" } else { "OFF" }
        );
    }
}

/// System that spawns overlay quads when the overlay is enabled and none exist.
/// Spawns one quad per walkable floor tile and creates the required mesh and materials.
pub fn spawn_overlay_quads(
    mut commands: Commands,
    overlay: Res<AtmosDebugOverlay>,
    tilemap: Option<Res<Tilemap>>,
    existing_quads: Query<&OverlayQuad>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !overlay.0 {
        return;
    }

    // If the Tilemap resource is missing, cannot spawn overlay quads
    let Some(tilemap) = tilemap else {
        warn!("Cannot spawn overlay quads: Tilemap resource missing");
        return;
    };

    let has_quads = !existing_quads.is_empty();
    if has_quads && !tilemap.is_changed() {
        return;
    }

    let mut existing_positions = std::collections::HashSet::new();
    for quad in existing_quads.iter() {
        existing_positions.insert(quad.position);
    }

    let mut quad_mesh: Option<Handle<Mesh>> = None;
    let mut spawned_count = 0;

    for (pos, kind) in tilemap.iter() {
        // Only spawn quads on floor tiles
        if !kind.is_walkable() {
            continue;
        }

        if existing_positions.contains(&pos) {
            continue;
        }

        let world_x = pos.x as f32;
        let world_z = pos.y as f32;

        let mesh = quad_mesh
            .get_or_insert_with(|| meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(0.5))))
            .clone();

        // Start with green (normal pressure) - will be updated by color update system
        let material = materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 1.0, 0.0, 0.5), // Semi-transparent green
            alpha_mode: AlphaMode::Blend,
            unlit: true, // Unlit so it's always visible
            ..default()
        });

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(world_x, 0.01, world_z),
            OverlayQuad {
                position: pos,
                mesh,
                material,
            },
        ));
        spawned_count += 1;
    }

    if spawned_count > 0 {
        info!("Spawned {} overlay quads", spawned_count);
    }
}

/// System that despawns overlay quads when the overlay is disabled or the Tilemap is removed.
/// Despawns all quads and cleans up their meshes and materials to prevent leaks.
pub fn despawn_overlay_quads(
    mut commands: Commands,
    overlay: Res<AtmosDebugOverlay>,
    tilemap: Option<Res<Tilemap>>,
    existing_quads: Query<(Entity, &OverlayQuad)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let quad_count = existing_quads.iter().count();

    // If the Tilemap resource is missing, ensure any existing overlay quads are cleaned up
    if tilemap.is_none() {
        if quad_count > 0 {
            // Clean up all unique meshes and materials once
            let mut cleaned_meshes = std::collections::HashSet::new();
            let mut cleaned_materials = std::collections::HashSet::new();

            for (entity, quad) in existing_quads.iter() {
                // Clean up mesh if not already cleaned
                if cleaned_meshes.insert(quad.mesh.id()) {
                    meshes.remove(&quad.mesh);
                }
                // Clean up material if not already cleaned
                if cleaned_materials.insert(quad.material.id()) {
                    materials.remove(&quad.material);
                }
                commands.entity(entity).despawn();
            }

            info!(
                "Despawned {} overlay quads because Tilemap resource is missing",
                quad_count
            );
        }
        return;
    }

    if overlay.0 || quad_count == 0 {
        return;
    }

    // Despawn overlay quads and clean up all unique meshes and materials
    let mut cleaned_meshes = std::collections::HashSet::new();
    let mut cleaned_materials = std::collections::HashSet::new();

    for (entity, quad) in existing_quads.iter() {
        // Clean up mesh if not already cleaned
        if cleaned_meshes.insert(quad.mesh.id()) {
            meshes.remove(&quad.mesh);
        }
        // Clean up material if not already cleaned (each quad has its own material)
        if cleaned_materials.insert(quad.material.id()) {
            materials.remove(&quad.material);
        }
        commands.entity(entity).despawn();
    }

    info!("Despawned {} overlay quads", quad_count);
}

/// System that updates the color of overlay quads based on the current pressure.
/// Only runs when the overlay is active.
/// Color mapping: blue (vacuum, p = 0) -> green (normal, p ≈ 101.325) -> red (high, p > 151.9875)
pub fn update_overlay_colors(
    overlay: Res<AtmosDebugOverlay>,
    gas_grid: Option<Res<GasGrid>>,
    quads: Query<(&OverlayQuad, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !overlay.0 {
        return;
    }

    let Some(gas_grid) = gas_grid else {
        return;
    };

    for (quad, material_handle) in quads.iter() {
        let Some(material) = materials.get_mut(&material_handle.0) else {
            continue;
        };

        let pressure = gas_grid.pressure_at(quad.position).unwrap_or(0.0);

        // Color mapping based on pressure:
        // p = 0.0: blue (vacuum)
        // p = OVERLAY_NORMAL_PRESSURE: green (normal)
        // p >= OVERLAY_HIGH_PRESSURE: red (high pressure)
        let color = if pressure < OVERLAY_NORMAL_PRESSURE {
            // Vacuum to normal: blue to green
            let t = (pressure / OVERLAY_NORMAL_PRESSURE).clamp(0.0, 1.0);
            // t=0: blue (0, 0, 1), t=1: green (0, 1, 0)
            Color::srgba(0.0, t, 1.0 - t, 0.5)
        } else if pressure < OVERLAY_HIGH_PRESSURE {
            // Normal to high: green to red
            let t = ((pressure - OVERLAY_NORMAL_PRESSURE)
                / (OVERLAY_HIGH_PRESSURE - OVERLAY_NORMAL_PRESSURE))
                .clamp(0.0, 1.0);
            // t=0: green (0, 1, 0), t=1: red (1, 0, 0)
            Color::srgba(t, 1.0 - t, 0.0, 0.5)
        } else {
            // High pressure: red, getting darker as pressure increases
            let intensity = (1.0
                - ((pressure - OVERLAY_HIGH_PRESSURE) / OVERLAY_NORMAL_PRESSURE) * 0.2)
                .clamp(0.5, 1.0);
            Color::srgba(intensity, 0.0, 0.0, 0.5)
        };

        material.base_color = color;
    }
}

/// System that reacts to [`TileMutated`] events while the overlay is active.
///
/// - If a tile becomes a wall (not walkable), the corresponding overlay quad is despawned
///   and its per-quad material asset is freed; the shared mesh is also removed if this
///   was the last quad referencing it (otherwise the mesh is kept for remaining quads).
/// - If a tile becomes walkable (floor), a new overlay quad is spawned for that position
///   if one does not already exist.
///
/// This ensures the overlay stays in sync with the tilemap when tiles are built or
/// demolished while the overlay is visible.
pub fn update_overlay_on_tile_mutation(
    mut commands: Commands,
    overlay: Res<AtmosDebugOverlay>,
    mut tile_events: MessageReader<TileMutated>,
    existing_quads: Query<(Entity, &OverlayQuad)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !overlay.0 {
        return;
    }

    // Collapse events: if the same position receives multiple mutations in the same
    // frame, only the last one matters (e.g. Floor then Wall → treat as Wall).
    // This also prevents duplicate quads or missed despawns for same-position events
    // whose deferred spawns/despawns are not yet visible to the `existing_quads` query.
    let mut final_states: std::collections::HashMap<IVec2, TileKind> =
        std::collections::HashMap::new();
    for event in tile_events.read() {
        final_states.insert(event.position, event.kind);
    }

    // Seed the shared mesh handle from any pre-existing quad.  We cache it in a
    // local variable so that multiple walkable events in the same frame all reuse
    // the same mesh handle even though Commands-deferred spawns are not yet visible
    // to the `existing_quads` query.
    let mut shared_mesh: Option<Handle<Mesh>> = existing_quads
        .iter()
        .next()
        .map(|(_, q)| q.mesh.clone());

    for (position, kind) in final_states {
        if kind.is_walkable() {
            // Tile became walkable — spawn an overlay quad if one doesn't already exist.
            let already_exists = existing_quads.iter().any(|(_, q)| q.position == position);
            if !already_exists {
                let world_x = position.x as f32;
                let world_z = position.y as f32;
                // Reuse the cached shared mesh handle, or create one (and cache it for
                // all subsequent spawns in this invocation).
                let mesh = shared_mesh
                    .get_or_insert_with(|| meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(0.5))))
                    .clone();
                let material = materials.add(StandardMaterial {
                    base_color: Color::srgba(0.0, 1.0, 0.0, 0.5),
                    alpha_mode: AlphaMode::Blend,
                    unlit: true,
                    ..default()
                });
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(world_x, 0.01, world_z),
                    OverlayQuad {
                        position,
                        mesh,
                        material,
                    },
                ));
                debug!("Spawned overlay quad at {:?} (tile became walkable)", position);
            }
        } else {
            // Tile became a wall — despawn the overlay quad at this position (if any).
            for (entity, quad) in existing_quads.iter() {
                if quad.position == position {
                    // If this is the last overlay quad using this mesh, remove the shared
                    // mesh asset as well and clear the cached handle so subsequent events
                    // in this batch will create a fresh mesh.
                    let is_last_for_mesh = existing_quads
                        .iter()
                        .filter(|(_, q)| q.mesh.id() == quad.mesh.id())
                        .count()
                        == 1;
                    if is_last_for_mesh {
                        meshes.remove(&quad.mesh);
                        shared_mesh = None;
                    }
                    // Remove the per-quad material (each quad owns a unique material handle).
                    materials.remove(&quad.material);
                    commands.entity(entity).despawn();
                    debug!("Despawned overlay quad at {:?} (tile became wall)", position);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiles::TileKind;

    // ── helpers ──────────────────────────────────────────────────────────────

    /// A resource used to inject a [`TileMutated`] message into the
    /// test app's message queue from outside a system. Can be reused
    /// across multiple frames by setting a new value before each `update()`.
    #[derive(Resource, Default)]
    struct PendingMutation(Vec<TileMutated>);

    /// System that drains [`PendingMutation`] into the message stream so that
    /// `update_overlay_on_tile_mutation` can read it in the same frame.
    fn inject_pending_mutation(
        mut writer: MessageWriter<TileMutated>,
        mut pending: ResMut<PendingMutation>,
    ) {
        for msg in pending.0.drain(..) {
            writer.write(msg);
        }
    }

    /// Build a minimal [`App`] wired up for overlay mutation tests.
    fn make_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.insert_resource(AtmosDebugOverlay(true));
        app.add_message::<TileMutated>();
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_resource::<PendingMutation>();
        app.add_systems(
            Update,
            (inject_pending_mutation, update_overlay_on_tile_mutation).chain(),
        );
        app
    }

    /// Count the number of [`OverlayQuad`] entities currently in the world.
    fn quad_count(app: &mut App) -> usize {
        let mut q = app.world_mut().query::<&OverlayQuad>();
        q.iter(app.world()).count()
    }

    /// Spawn a bare `OverlayQuad` entity with real mesh/material assets but
    /// without rendering components (`Mesh3d`, `MeshMaterial3d`, `Transform`),
    /// which are not needed for the mutation system tests.
    fn spawn_test_quad(app: &mut App, position: IVec2) -> Entity {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..default()
            });
        app.world_mut()
            .spawn(OverlayQuad {
                position,
                mesh,
                material,
            })
            .id()
    }

    /// Helper: write a single mutation event and advance by one frame.
    fn fire_mutation(app: &mut App, position: IVec2, kind: TileKind) {
        app.world_mut()
            .resource_mut::<PendingMutation>()
            .0
            .push(TileMutated { position, kind });
        app.update();
    }

    /// Helper: write multiple mutation events in the same frame and advance.
    fn fire_mutations(app: &mut App, events: impl IntoIterator<Item = (IVec2, TileKind)>) {
        let mut pending = app.world_mut().resource_mut::<PendingMutation>();
        for (position, kind) in events {
            pending.0.push(TileMutated { position, kind });
        }
        app.update();
    }

    // ── pre-existing unit tests ───────────────────────────────────────────

    #[test]
    fn test_overlay_default_off() {
        let overlay = AtmosDebugOverlay::default();
        assert!(!overlay.0, "Overlay should be off by default");
    }

    #[test]
    fn test_toggle_overlay_manual() {
        // Test manual toggle without using ButtonInput simulation
        let mut overlay = AtmosDebugOverlay::default();
        assert!(!overlay.0, "Should start off");

        overlay.0 = !overlay.0;
        assert!(overlay.0, "Should be on after first toggle");

        overlay.0 = !overlay.0;
        assert!(!overlay.0, "Should be off after second toggle");
    }

    // ── ECS-style mutation system tests ──────────────────────────────────

    #[test]
    fn test_mutation_skips_when_overlay_off() {
        let mut app = make_test_app();
        app.insert_resource(AtmosDebugOverlay(false));
        spawn_test_quad(&mut app, IVec2::new(1, 1));

        fire_mutation(&mut app, IVec2::new(1, 1), TileKind::Wall);

        assert_eq!(
            quad_count(&mut app),
            1,
            "quad should not be despawned when overlay is off"
        );
    }

    #[test]
    fn test_mutation_despawns_quad_when_tile_becomes_wall() {
        let mut app = make_test_app();
        spawn_test_quad(&mut app, IVec2::new(2, 3));

        fire_mutation(&mut app, IVec2::new(2, 3), TileKind::Wall);

        assert_eq!(
            quad_count(&mut app),
            0,
            "quad should be despawned when tile becomes a wall"
        );
    }

    #[test]
    fn test_mutation_does_not_despawn_other_quads() {
        let mut app = make_test_app();
        spawn_test_quad(&mut app, IVec2::new(1, 1));
        spawn_test_quad(&mut app, IVec2::new(2, 2));

        fire_mutation(&mut app, IVec2::new(1, 1), TileKind::Wall);

        assert_eq!(
            quad_count(&mut app),
            1,
            "only the targeted position's quad should be despawned"
        );
    }

    #[test]
    fn test_mutation_spawns_quad_when_tile_becomes_walkable() {
        let mut app = make_test_app();

        fire_mutation(&mut app, IVec2::new(3, 4), TileKind::Floor);

        assert_eq!(
            quad_count(&mut app),
            1,
            "a new quad should be spawned when a tile becomes walkable"
        );
    }

    #[test]
    fn test_mutation_no_duplicate_quad_for_existing_walkable_tile() {
        let mut app = make_test_app();
        spawn_test_quad(&mut app, IVec2::new(5, 6));

        fire_mutation(&mut app, IVec2::new(5, 6), TileKind::Floor);

        assert_eq!(
            quad_count(&mut app),
            1,
            "no duplicate quad should be spawned for a position that already has one"
        );
    }

    #[test]
    fn test_new_quad_reuses_existing_shared_mesh() {
        let mut app = make_test_app();

        // Spawn an initial quad with a known mesh handle.
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..default()
            });
        let initial_mesh_id = mesh.id();
        app.world_mut().spawn(OverlayQuad {
            position: IVec2::new(0, 0),
            mesh,
            material,
        });

        // A floor mutation at a different position should reuse the existing mesh.
        fire_mutation(&mut app, IVec2::new(1, 0), TileKind::Floor);

        let mut q = app.world_mut().query::<&OverlayQuad>();
        let new_mesh_id = q
            .iter(app.world())
            .find(|quad| quad.position == IVec2::new(1, 0))
            .map(|quad| quad.mesh.id())
            .expect("a new quad should exist at (1, 0)");

        assert_eq!(
            new_mesh_id, initial_mesh_id,
            "mutation-spawned quad should reuse the shared mesh from existing quads"
        );
    }

    #[test]
    fn test_shared_mesh_not_removed_when_other_quads_still_use_it() {
        let mut app = make_test_app();

        // Spawn two quads that share the same mesh handle.
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)));
        let mat1 = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..default()
            });
        let mat2 = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..default()
            });
        let mesh_id = mesh.id();
        app.world_mut().spawn(OverlayQuad {
            position: IVec2::new(0, 0),
            mesh: mesh.clone(),
            material: mat1,
        });
        app.world_mut().spawn(OverlayQuad {
            position: IVec2::new(1, 0),
            mesh,
            material: mat2,
        });

        // Despawning one quad via a wall mutation should NOT remove the shared mesh.
        fire_mutation(&mut app, IVec2::new(0, 0), TileKind::Wall);

        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(mesh_id)
                .is_some(),
            "shared mesh should not be removed while other quads still reference it"
        );
    }

    #[test]
    fn test_multiple_walkable_events_same_frame_share_one_mesh() {
        let mut app = make_test_app();

        // Fire two walkable mutations in the same frame with no prior quads.
        // Both new quads must share the same mesh handle (not create separate meshes).
        fire_mutations(
            &mut app,
            [
                (IVec2::new(0, 0), TileKind::Floor),
                (IVec2::new(1, 0), TileKind::Floor),
            ],
        );

        assert_eq!(
            quad_count(&mut app),
            2,
            "both walkable mutations should produce quads"
        );

        let mut q = app.world_mut().query::<&OverlayQuad>();
        let mesh_ids: std::collections::HashSet<_> =
            q.iter(app.world()).map(|quad| quad.mesh.id()).collect();

        assert_eq!(
            mesh_ids.len(),
            1,
            "both quads spawned in the same frame should share one mesh handle"
        );
    }

    #[test]
    fn test_same_position_floor_then_wall_no_quad_spawned() {
        let mut app = make_test_app();

        // Floor then Wall for the same position in the same frame: the Wall event
        // wins (events collapsed by position) and no quad should be spawned.
        fire_mutations(
            &mut app,
            [
                (IVec2::new(5, 5), TileKind::Floor),
                (IVec2::new(5, 5), TileKind::Wall),
            ],
        );

        assert_eq!(
            quad_count(&mut app),
            0,
            "Wall event should win when Floor+Wall arrive for the same position"
        );
    }

    #[test]
    fn test_last_quad_despawned_removes_mesh() {
        let mut app = make_test_app();

        // Spawn a single quad with a known mesh handle.
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..default()
            });
        let mesh_id = mesh.id();
        app.world_mut().spawn(OverlayQuad {
            position: IVec2::new(0, 0),
            mesh,
            material,
        });

        // Despawning the only quad via a wall mutation should remove the shared mesh.
        fire_mutation(&mut app, IVec2::new(0, 0), TileKind::Wall);

        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(mesh_id)
                .is_none(),
            "shared mesh should be removed when the last quad referencing it is despawned"
        );
    }
}
