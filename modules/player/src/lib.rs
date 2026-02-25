use bevy::prelude::*;
use things::{DisplayName, InputDirection};

pub use things::PlayerControlled;

/// Marker component for nameplate UI overlay nodes.
///
/// Spawned automatically when a [`DisplayName`] component is added to an entity.
/// The [`update_nameplate_positions`] system projects the tracked entity's
/// world position to screen space each frame, positioning the UI node above it.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Nameplate;

/// Links a nameplate UI node back to the 3D entity it tracks.
#[derive(Component, Debug, Clone, Copy)]
pub struct NameplateTarget(pub Entity);

/// Vertical world-space offset above the tracked entity's origin.
const NAMEPLATE_WORLD_OFFSET: f32 = 2.0;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Nameplate>();
        app.add_observer(spawn_nameplate);
        app.add_observer(despawn_nameplate);
        app.add_systems(Update, (read_player_input, update_nameplate_positions));
    }
}

/// Reads keyboard input and writes InputDirection on PlayerControlled entities.
fn read_player_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut InputDirection, With<PlayerControlled>>,
) {
    for mut input in query.iter_mut() {
        let mut direction = Vec3::ZERO;
        if keyboard.pressed(KeyCode::KeyW) {
            direction.z -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyS) {
            direction.z += 1.0;
        }
        if keyboard.pressed(KeyCode::KeyA) {
            direction.x -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyD) {
            direction.x += 1.0;
        }
        input.0 = direction;
    }
}

/// Observer that runs when a [`DisplayName`] component is added to an entity.
///
/// Spawns an absolutely-positioned UI [`Text`] node with a [`Nameplate`] marker
/// and a [`NameplateTarget`] linking it back to the 3D entity.
/// [`update_nameplate_positions`] moves it to the correct screen position each frame.
fn spawn_nameplate(
    trigger: On<Add, DisplayName>,
    mut commands: Commands,
    names: Query<&DisplayName>,
) {
    let entity = trigger.event_target();
    let display_name = names
        .get(entity)
        .expect("DisplayName missing on trigger target");

    commands.spawn((
        Text::new(display_name.0.clone()),
        TextFont::from_font_size(20.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        Nameplate,
        NameplateTarget(entity),
    ));
}

/// Observer that runs when a [`DisplayName`] component is removed from an entity.
///
/// Despawns every [`Nameplate`] UI node whose [`NameplateTarget`] points at the entity
/// that lost its name, preventing stale overlay nodes from accumulating.
fn despawn_nameplate(
    trigger: On<Remove, DisplayName>,
    nameplates: Query<(Entity, &NameplateTarget), With<Nameplate>>,
    mut commands: Commands,
) {
    let removed = trigger.event_target();
    for (nameplate_entity, target) in nameplates.iter() {
        if target.0 == removed {
            commands.entity(nameplate_entity).despawn();
        }
    }
}

/// Projects each [`Nameplate`]'s tracked entity from world space to screen
/// space using the active camera, positioning the UI node above the entity.
///
/// Hides the nameplate when the entity is behind the camera.
fn update_nameplate_positions(
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    target_query: Query<&GlobalTransform>,
    mut nameplate_query: Query<
        (&mut Node, &mut Visibility, &ComputedNode, &NameplateTarget),
        With<Nameplate>,
    >,
) {
    let Ok((camera, camera_gt)) = camera_query.single() else {
        return;
    };
    for (mut node, mut visibility, computed, target) in nameplate_query.iter_mut() {
        let Ok(target_gt) = target_query.get(target.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let world_pos = target_gt.translation() + Vec3::Y * NAMEPLATE_WORLD_OFFSET;
        if let Ok(viewport_pos) = camera.world_to_viewport(camera_gt, world_pos) {
            let size = computed.size();
            node.left = Val::Px((viewport_pos.x - size.x * 0.5).round());
            node.top = Val::Px((viewport_pos.y - size.y * 0.5).round());
            *visibility = Visibility::Inherited;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that [`spawn_nameplate`] creates a [`Nameplate`] UI entity
    /// targeting the entity that received a [`DisplayName`].
    #[test]
    fn spawn_nameplate_creates_ui_entity() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_observer(spawn_nameplate);

        let target = app.world_mut().spawn(DisplayName("Hero".to_string())).id();

        app.update();

        // Find the nameplate entity.
        let mut nameplate_query = app
            .world_mut()
            .query_filtered::<(Entity, &NameplateTarget), With<Nameplate>>();
        let nameplates: Vec<_> = nameplate_query.iter(app.world()).collect();
        assert_eq!(nameplates.len(), 1, "Exactly one nameplate expected");
        let (_, nameplate_target) = nameplates[0];
        assert_eq!(
            nameplate_target.0, target,
            "Nameplate should target the entity that received DisplayName"
        );
    }

    /// Verifies that [`despawn_nameplate`] removes the [`Nameplate`] UI entity when
    /// [`DisplayName`] is removed from the target entity.
    #[test]
    fn despawn_nameplate_removes_ui_entity() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_observer(spawn_nameplate);
        app.add_observer(despawn_nameplate);

        let target = app.world_mut().spawn(DisplayName("Hero".to_string())).id();
        app.update();

        // One nameplate should exist after spawning.
        let count = app
            .world_mut()
            .query_filtered::<Entity, With<Nameplate>>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1, "Nameplate should exist before removal");

        // Remove DisplayName — observer should schedule despawn.
        app.world_mut().entity_mut(target).remove::<DisplayName>();
        app.update();

        let count = app
            .world_mut()
            .query_filtered::<Entity, With<Nameplate>>()
            .iter(app.world())
            .count();
        assert_eq!(count, 0, "Nameplate should be despawned after DisplayName removal");
    }
}
