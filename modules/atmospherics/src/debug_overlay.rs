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

/// System that spawns or despawns overlay quads based on the overlay toggle state.
/// When the overlay is enabled and no quads exist, spawns one quad per floor tile.
/// When the overlay is disabled, despawns all quads.
pub fn manage_overlay_quads(
    mut commands: Commands,
    overlay: Res<AtmosDebugOverlay>,
    tilemap: Option<Res<Tilemap>>,
    existing_quads: Query<Entity, With<OverlayQuad>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let quad_count = existing_quads.iter().count();

    if overlay.0 && quad_count == 0 {
        // Spawn overlay quads
        let Some(tilemap) = tilemap else {
            warn!("Cannot spawn overlay quads: Tilemap resource missing");
            return;
        };

        // Create a shared quad mesh (1x1 plane)
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
                MeshMaterial3d(material),
                Transform::from_xyz(world_x, 0.01, world_z),
                OverlayQuad { position: pos },
            ));
        }

        info!("Spawned {} overlay quads", tilemap.iter().filter(|(_, k)| k.is_walkable()).count());
    } else if !overlay.0 && quad_count > 0 {
        // Despawn overlay quads
        for entity in existing_quads.iter() {
            commands.entity(entity).despawn();
        }
        info!("Despawned {} overlay quads", quad_count);
    }
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
        // p ≈ 1.0: green (normal)
        // p > 1.5: red (high pressure)
        let color = if pressure < 0.5 {
            // Vacuum: blue
            let intensity = (pressure / 0.5).clamp(0.0, 1.0);
            Color::srgba(0.0, 0.0, 0.5 + intensity * 0.5, 0.5)
        } else if pressure < 1.5 {
            // Normal to high: green to red
            let t = ((pressure - 0.5) / 1.0).clamp(0.0, 1.0);
            Color::srgba(t, 1.0 - t, 0.0, 0.5)
        } else {
            // High pressure: red
            let intensity = (1.0 - (pressure - 1.5) * 0.2).clamp(0.5, 1.0);
            Color::srgba(intensity, 0.0, 0.0, 0.5)
        };

        material.base_color = color;
    }
}
