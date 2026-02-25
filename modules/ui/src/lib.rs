use bevy::prelude::*;

pub mod button;
pub mod overlay;
pub mod theme;

pub use button::build_button;
pub use overlay::{OverlayTarget, WorldSpaceOverlay, update_world_space_overlays};
pub use theme::UiTheme;

#[derive(Default)]
pub struct UiPlugin {
    messages: Vec<fn(&mut App)>,
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<WorldSpaceOverlay>();
        app.register_type::<OverlayTarget>();
        app.init_resource::<UiTheme>();
        app.add_systems(Startup, spawn_ui_camera);
        app.add_systems(PreUpdate, button::change_button_colors);
        app.add_systems(Update, update_world_space_overlays);

        for register in &self.messages {
            register(app);
        }
    }
}

fn spawn_ui_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
    ));
}

impl UiPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers button press handling for message type `T`.
    /// The message type itself must be registered by the plugin that owns it
    /// (via `app.add_message::<T>()`).
    pub fn with_event<T: Message + Clone>(mut self) -> Self {
        self.messages.push(|app| {
            app.add_systems(PreUpdate, button::process_button_messages::<T>);
        });
        self
    }
}
