use bevy::prelude::*;

#[derive(States, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppState {
    #[default]
    MainMenu,
    /// Transitional state: map loading (server) and/or network sync (client)
    /// in progress. Entered when `NetCommand::Host` or `NetCommand::Connect`
    /// is processed; exits to `InGame` on sync completion or `MainMenu` on
    /// failure/disconnect.
    Loading,
    InGame,
    /// Map editor: orthographic top-down view, simulation disabled.
    /// Entered from the main menu; exits back to `MainMenu` via Escape.
    Editor,
}
