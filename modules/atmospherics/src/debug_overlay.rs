use bevy::prelude::*;
use tiles::Tilemap;

use crate::GasGrid;

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
pub fn toggle_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<AtmosDebugOverlay>,
) {
    if keyboard.just_pressed(KeyCode::F3) {
        overlay.0 = !overlay.0;
        info!("Atmospheric debug overlay: {}", if overlay.0 { "ON" } else { "OFF" });
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
    let has_quads = !existing_quads.is_empty();

    if !overlay.0 || has_quads {
        return;
    }

    // If the Tilemap resource is missing, cannot spawn overlay quads
    let Some(tilemap) = tilemap else {
        warn!("Cannot spawn overlay quads: Tilemap resource missing");
        return;
    };

    // Create a single shared mesh for all quads (1x1 plane)
    let quad_mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)));

    // Spawn one quad per tile at y=0.01 (just above floor at y=0.0)
    for (pos, kind) in tilemap.iter() {
        // Only spawn quads on floor tiles
        if !kind.is_walkable() {
            continue;
        }

        let world_x = pos.x as f32;
        let world_z = pos.y as f32;

        // Start with green (normal pressure) - will be updated by color update system
        let material = materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 1.0, 0.0, 0.5), // Semi-transparent green
            alpha_mode: AlphaMode::Blend,
            unlit: true, // Unlit so it's always visible
            ..default()
        });

        commands.spawn((
            Mesh3d(quad_mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(world_x, 0.01, world_z),
            OverlayQuad {
                position: pos,
                mesh: quad_mesh.clone(),
                material: material.clone(),
            },
        ));
    }

    info!(
        "Spawned {} overlay quads",
        tilemap.iter().filter(|(_, k)| k.is_walkable()).count()
    );
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
            // Clean up shared mesh and materials once
            let mut shared_mesh: Option<Handle<Mesh>> = None;
            let mut cleaned_materials = std::collections::HashSet::new();
            
            for (entity, quad) in existing_quads.iter() {
                // Capture the shared mesh handle from the first quad
                if shared_mesh.is_none() {
                    shared_mesh = Some(quad.mesh.clone());
                }
                // Clean up material if not already cleaned
                if cleaned_materials.insert(quad.material.id()) {
                    materials.remove(&quad.material);
                }
                commands.entity(entity).despawn();
            }
            
            // Remove the shared mesh from assets once
            if let Some(mesh) = shared_mesh {
                meshes.remove(&mesh);
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

    // Despawn overlay quads and clean up their meshes and materials
    let mut shared_mesh: Option<Handle<Mesh>> = None;
    let mut cleaned_materials = std::collections::HashSet::new();
    
    for (entity, quad) in existing_quads.iter() {
        // Capture the shared mesh handle from the first quad
        if shared_mesh.is_none() {
            shared_mesh = Some(quad.mesh.clone());
        }
        // Clean up material if not already cleaned (each quad has its own material)
        if cleaned_materials.insert(quad.material.id()) {
            materials.remove(&quad.material);
        }
        commands.entity(entity).despawn();
    }
    
    // Remove the shared mesh from assets once, after all entities are despawned
    if let Some(mesh) = shared_mesh {
        meshes.remove(&mesh);
    }

    info!("Despawned {} overlay quads", quad_count);
}

/// System that updates the color of overlay quads based on the current pressure.
/// Only runs when the overlay is active.
/// Color mapping: blue (vacuum, p < 0.5) -> green (normal, p ≈ 1.0) -> red (high, p > 1.5)
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
        // p < 0.5: blue (vacuum)
        // p = 1.0: green (normal)
        // p > 1.5: red (high pressure)
        let color = if pressure < 1.0 {
            // Vacuum to normal: blue to green
            let t = pressure.clamp(0.0, 1.0);
            // t=0: blue (0, 0, 1), t=1: green (0, 1, 0)
            Color::srgba(0.0, t, 1.0 - t, 0.5)
        } else if pressure < 1.5 {
            // Normal to high: green to red
            let t = ((pressure - 1.0) / 0.5).clamp(0.0, 1.0);
            // t=0: green (0, 1, 0), t=1: red (1, 0, 0)
            Color::srgba(t, 1.0 - t, 0.0, 0.5)
        } else {
            // High pressure: red, getting darker as pressure increases
            let intensity = (1.0 - (pressure - 1.5) * 0.2).clamp(0.5, 1.0);
            Color::srgba(intensity, 0.0, 0.0, 0.5)
        };

        material.base_color = color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
