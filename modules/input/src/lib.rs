use bevy::prelude::*;
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
    window: Query<&Window>,
    mut writer: EventWriter<PointerAction>,
) {
    let Ok(window) = window.single() else {
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
