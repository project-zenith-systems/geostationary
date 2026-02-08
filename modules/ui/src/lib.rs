use bevy::prelude::*;

pub mod button;
pub mod theme;

pub use button::build_button;
pub use theme::UiTheme;

#[derive(Default)]
pub struct UiPlugin {
    messages: Vec<fn(&mut App)>,
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiTheme>();
        app.add_systems(PreUpdate, button::change_button_colors);

        for register in &self.messages {
            register(app);
        }
    }
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
