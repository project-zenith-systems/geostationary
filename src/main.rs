use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use ui::UiPlugin;

mod app_state;
mod main_menu;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Geostationary".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(
            UiPlugin::new()
                .with_event::<MenuEvent>(),
        )
        .add_plugins(MainMenuPlugin)
        .init_state::<app_state::AppState>()
        .add_systems(Startup, spawn_camera)
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}
