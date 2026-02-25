use bevy::prelude::*;
use things::{DisplayName, InputDirection};
use ui::{OverlayTarget, WorldSpaceOverlay};

pub use things::PlayerControlled;

/// Marker component for nameplate UI overlay nodes.
///
/// Spawned automatically when a [`DisplayName`] component is added to an entity.
/// The [`ui::update_world_space_overlays`] system (registered by [`ui::UiPlugin`])
/// projects the tracked entity's world position to screen space each frame.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Nameplate;

/// Vertical world-space offset above the tracked entity's origin.
const NAMEPLATE_WORLD_OFFSET: f32 = 2.0;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Nameplate>();
        app.add_observer(spawn_nameplate);
        app.add_systems(Update, read_player_input);
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
/// Spawns an absolutely-positioned UI [`Text`] node with a [`Nameplate`] marker,
/// a [`WorldSpaceOverlay`] for projection, and an [`OverlayTarget`] linking the
/// node back to the 3D entity.  The [`ui::update_world_space_overlays`] system
/// moves the node to the correct screen position each frame.
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
        WorldSpaceOverlay::default(),
        OverlayTarget {
            entity,
            offset: Vec3::Y * NAMEPLATE_WORLD_OFFSET,
        },
        Nameplate,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that [`spawn_nameplate`] creates a [`Nameplate`] UI entity with
    /// [`WorldSpaceOverlay`] and [`OverlayTarget`] targeting the entity that
    /// received a [`DisplayName`].
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
            .query_filtered::<(Entity, &OverlayTarget), With<Nameplate>>();
        let nameplates: Vec<_> = nameplate_query.iter(app.world()).collect();
        assert_eq!(nameplates.len(), 1, "Exactly one nameplate expected");
        let (_, overlay_target) = nameplates[0];
        assert_eq!(
            overlay_target.entity, target,
            "OverlayTarget should point to the entity that received DisplayName"
        );
    }

    /// Verifies that the nameplate's [`OverlayTarget`] uses the expected vertical offset.
    #[test]
    fn nameplate_overlay_target_has_correct_offset() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_observer(spawn_nameplate);

        app.world_mut().spawn(DisplayName("Bob".to_string()));
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&OverlayTarget, With<Nameplate>>();
        let target = q.single(app.world()).unwrap();
        assert!(
            (target.offset - Vec3::Y * NAMEPLATE_WORLD_OFFSET).length() < 0.001,
            "Offset should be Vec3::Y * NAMEPLATE_WORLD_OFFSET ({NAMEPLATE_WORLD_OFFSET})"
        );
    }
}

