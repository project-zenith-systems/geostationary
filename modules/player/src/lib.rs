use bevy::prelude::*;
use things::{DisplayName, InputDirection};

/// Marker component for player-controlled entities (camera target, input receiver).
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerControlled;

/// Marker component for billboard nameplate child entities.
///
/// Spawned automatically as a child of any entity that receives a [`DisplayName`]
/// component. The [`face_camera`] system rotates each nameplate every frame so
/// that it always faces the active camera (billboard behavior).
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Nameplate;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PlayerControlled>();
        app.register_type::<Nameplate>();
        app.add_observer(spawn_nameplate);
        app.add_systems(Update, (read_player_input, face_camera));
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
/// Spawns a [`Text2d`] child entity positioned above the parent with a
/// [`Nameplate`] marker.  The [`face_camera`] system keeps it facing the
/// camera each frame.
fn spawn_nameplate(trigger: On<Add, DisplayName>, mut commands: Commands, names: Query<&DisplayName>) {
    let entity = trigger.event_target();
    // DisplayName was just added so this query is guaranteed to succeed.
    let display_name = names.get(entity).expect("DisplayName missing on trigger target");

    commands.entity(entity).with_children(|parent| {
        parent.spawn((
            Text2d::new(display_name.0.clone()),
            TextFont {
                font_size: 20.0,
                ..default()
            },
            TextColor(Color::WHITE),
            Transform::from_xyz(0.0, 1.5, 0.0),
            Nameplate,
        ));
    });
}

/// Rotates every [`Nameplate`] entity to face the active camera (billboard).
///
/// Copies the camera's world-space rotation directly, so the text plane is
/// always perpendicular to the camera's forward vector.
fn face_camera(
    camera_query: Query<&GlobalTransform, With<Camera3d>>,
    mut nameplate_query: Query<&mut Transform, With<Nameplate>>,
) {
    let Ok(camera_gt) = camera_query.single() else {
        return;
    };
    let camera_rotation = camera_gt.rotation();
    for mut transform in nameplate_query.iter_mut() {
        transform.rotation = camera_rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that [`spawn_nameplate`] creates a [`Nameplate`] child entity
    /// when a [`DisplayName`] component is added to an entity.
    #[test]
    fn spawn_nameplate_creates_child() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_observer(spawn_nameplate);

        let parent = app
            .world_mut()
            .spawn(DisplayName("Hero".to_string()))
            .id();

        // Run one frame so the observer fires.
        app.update();

        let children = app.world().get::<Children>(parent);
        assert!(
            children.is_some(),
            "Nameplate child should have been spawned"
        );
        let children = children.unwrap();
        assert_eq!(children.len(), 1, "Exactly one nameplate child expected");
        let child = children[0];
        assert!(
            app.world().get::<Nameplate>(child).is_some(),
            "Child should have Nameplate marker"
        );
    }

    /// Verifies that [`face_camera`] copies the camera's rotation to a [`Nameplate`].
    #[test]
    fn face_camera_copies_camera_rotation() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, face_camera);

        // Spawn a camera with a known rotation.
        let angle = std::f32::consts::FRAC_PI_4;
        let camera_rotation = Quat::from_rotation_y(angle);
        app.world_mut().spawn((
            Camera3d::default(),
            GlobalTransform::from(Transform::from_rotation(camera_rotation)),
        ));

        // Spawn a nameplate with identity rotation.
        let nameplate = app
            .world_mut()
            .spawn((Transform::default(), Nameplate))
            .id();

        app.update();

        let transform = app.world().get::<Transform>(nameplate).unwrap();
        let dot = transform.rotation.dot(camera_rotation);
        assert!(
            dot.abs() > 0.999,
            "Nameplate rotation should match camera rotation. dot={dot}"
        );
    }
}
