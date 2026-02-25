use atmospherics::AtmosphericsPlugin;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use network::{Headless, NetCommand, NetworkPlugin};
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;

mod app_state;
mod client;
mod config;
mod main_menu;
mod server;
mod world_setup;

fn parse_log_level(s: &str) -> Level {
    match s.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" | "warning" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

fn log_plugin(level: Level) -> LogPlugin {
    LogPlugin {
        level,
        ..default()
    }
}

fn main() {
    let headless = std::env::args().any(|a| a == "--server");
    let app_config = config::load_config();
    let log_level = parse_log_level(&app_config.debug.log_level);

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    if headless {
        // Dedicated headless server: minimal plugin set for physics + networking, no window/rendering.
        // Plugin set derived from the headless Avian3D spike (see physics/src/lib.rs tests).
        app.add_plugins(MinimalPlugins)
            .add_plugins(log_plugin(log_level))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .add_plugins(bevy::mesh::MeshPlugin)
            .add_plugins(bevy::scene::ScenePlugin)
            .insert_resource(Headless)
            .add_plugins(NetworkPlugin)
            .add_plugins(PhysicsPlugin)
            .add_plugins(TilesPlugin)
            .add_plugins(ThingsPlugin)
            .add_plugins(AtmosphericsPlugin)
            .add_plugins(creatures::CreaturesPlugin)
            .add_plugins(souls::SoulsPlugin)
            .add_plugins(world_setup::WorldSetupPlugin)
            .add_plugins(server::ServerPlugin)
            .insert_state(app_state::AppState::InGame)
            .add_systems(Startup, host_on_startup);
    } else {
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: app_config.window.title.clone(),
                        ..default()
                    }),
                    ..default()
                })
                .set(log_plugin(log_level)),
        )
        .add_plugins(UiPlugin::new().with_event::<MenuEvent>())
        .add_plugins(MainMenuPlugin)
        .add_plugins(NetworkPlugin)
        .add_plugins(PhysicsPlugin);

        if app_config.debug.physics_debug {
            app.add_plugins(PhysicsDebugPlugin);
        }

        app.add_plugins(TilesPlugin)
            .add_plugins(ThingsPlugin)
            .add_plugins(AtmosphericsPlugin)
            .add_plugins(creatures::CreaturesPlugin)
            .add_plugins(souls::SoulsPlugin)
            .add_plugins(player::PlayerPlugin)
            .add_plugins(camera::CameraPlugin::<app_state::AppState>::in_state(
                app_state::AppState::InGame,
            ))
            .add_plugins(world_setup::WorldSetupPlugin)
            .add_plugins(client::ClientPlugin)
            .add_plugins(server::ServerPlugin)
            .init_state::<app_state::AppState>();
    }

    app.run();
}

/// Startup system for headless server mode: auto-sends `NetCommand::Host` so the server
/// begins listening immediately without requiring any user interaction.
fn host_on_startup(mut net_commands: MessageWriter<NetCommand>, config: Res<config::AppConfig>) {
    net_commands.write(NetCommand::Host {
        port: config.network.port,
    });
    info!("Headless server mode: auto-hosting on port {}", config.network.port);
}
