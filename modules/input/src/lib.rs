use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use network::Headless;
use tiles::TileKind;

/// Normalised pointer event fired on mouse button press.
#[derive(Event, Debug, Clone, Copy)]
pub struct PointerAction {
    pub button: MouseButton,
    pub screen_pos: Vec2,
}

/// Shared hit-test result type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldHit {
    Tile { position: IVec2, kind: TileKind },
    Thing { entity: Entity, kind: u16 },
}

/// System that emits [`PointerAction`] events for each mouse button just pressed.
///
/// Runs in `PreUpdate`, gated on the provided game state and absence of [`Headless`].
fn emit_pointer_actions(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut writer: EventWriter<PointerAction>,
) {
    let Ok(window) = window.single() else {
        warn!("emit_pointer_actions: no primary window found");
        return;
    };
    let Some(screen_pos) = window.cursor_position() else {
        return;
    };
    for button in mouse.get_just_pressed() {
        writer.send(PointerAction {
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
        app.add_event::<PointerAction>();
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

    fn capture(mut reader: EventReader<PointerAction>, mut captured: ResMut<CapturedActions>) {
        captured.0.extend(reader.read().copied());
    }

    fn make_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<PointerAction>();
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
                    resolution: WindowResolution::new(800.0, 600.0),
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
