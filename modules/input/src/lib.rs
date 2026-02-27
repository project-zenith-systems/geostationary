use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use network::Headless;

/// Normalised pointer event fired on mouse button press.
#[derive(Message, Debug, Clone, Copy)]
pub struct PointerAction {
    pub button: MouseButton,
    pub screen_pos: Vec2,
}

/// Generic hit-test result emitted by raycasting systems in domain modules
/// (`raycast_tiles`, `raycast_things`, …).
///
/// Carries the hit entity and its 3D world position so downstream systems
/// (e.g. the `interactions` module) can inspect the entity's components to
/// determine what kind of thing was hit without any tile- or thing-specific
/// types at this layer.
#[derive(Message, Debug, Clone, Copy)]
pub struct WorldHit {
    /// The entity that was hit (a tile entity, a thing entity, etc.).
    pub entity: Entity,
    /// The 3D world-space position of the hit point.
    pub world_pos: Vec3,
}

/// System that emits [`PointerAction`] events for each mouse button just pressed.
///
/// Runs in `PreUpdate`, gated on the provided game state and absence of [`Headless`].
fn emit_pointer_actions(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut writer: MessageWriter<PointerAction>,
) {
    let Ok(window) = window.single() else {
        warn!("emit_pointer_actions: no primary window found");
        return;
    };
    let Some(screen_pos) = window.cursor_position() else {
        return;
    };
    for button in mouse.get_just_pressed() {
        writer.write(PointerAction {
            button: *button,
            screen_pos,
        });
    }
}

pub struct InputPlugin<S: States + Copy> {
    state: S,
}

impl<S: States + Copy> InputPlugin<S> {
    pub fn in_state(state: S) -> Self {
        Self { state }
    }
}

impl<S: States + Copy> Plugin for InputPlugin<S> {
    fn build(&self, app: &mut App) {
        app.add_message::<PointerAction>();
        app.add_message::<WorldHit>();
        let state = self.state;
        app.add_systems(
            PreUpdate,
            emit_pointer_actions
                .run_if(in_state(state))
                .run_if(not(resource_exists::<Headless>)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::window::WindowResolution;

    #[derive(Resource, Default)]
    struct CapturedActions(Vec<PointerAction>);

    fn capture(mut reader: MessageReader<PointerAction>, mut captured: ResMut<CapturedActions>) {
        captured.0.extend(reader.read().copied());
    }

    fn make_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<PointerAction>();
        app.insert_resource(ButtonInput::<MouseButton>::default());
        app.init_resource::<CapturedActions>();
        app.add_systems(Update, emit_pointer_actions);
        app.add_systems(Update, capture.after(emit_pointer_actions));
        app
    }

    fn spawn_primary_window(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                Window {
                    resolution: WindowResolution::new(800, 600),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id()
    }

    #[test]
    fn test_emits_pointer_action_on_mouse_press() {
        let mut app = make_test_app();

        let window_entity = spawn_primary_window(&mut app);
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(200.0, 150.0)));

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);

        app.update();

        let actions = app.world().resource::<CapturedActions>();
        assert_eq!(actions.0.len(), 1);
        assert_eq!(actions.0[0].button, MouseButton::Left);
        assert_eq!(actions.0[0].screen_pos, Vec2::new(200.0, 150.0));
    }

    #[test]
    fn test_no_event_when_cursor_outside_window() {
        let mut app = make_test_app();
        spawn_primary_window(&mut app);

        // cursor position defaults to None (cursor is outside window)
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);

        app.update();

        let actions = app.world().resource::<CapturedActions>();
        assert!(
            actions.0.is_empty(),
            "no events expected when cursor is outside window"
        );
    }

    #[test]
    fn test_no_event_when_no_primary_window() {
        let mut app = make_test_app();

        // no primary window spawned
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);

        app.update();

        let actions = app.world().resource::<CapturedActions>();
        assert!(
            actions.0.is_empty(),
            "no events expected when no primary window exists"
        );
    }
}
