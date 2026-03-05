use bevy::prelude::*;

#[derive(States, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppState {
    #[default]
    MainMenu,
    InGame,
    /// Map editor: orthographic top-down view, simulation disabled.
    /// Entered from the main menu; exits back to `MainMenu` via Escape.
    Editor,
}
