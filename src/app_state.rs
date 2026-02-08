use bevy::prelude::*;

#[derive(States, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppState {
    #[default]
    MainMenu,
    InGame,
}
