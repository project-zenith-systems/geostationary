use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use interactions::InteractionsPlugin;
use network::{Headless, NetCommand, NetworkPlugin};
use physics::PhysicsPlugin;
use shared::config::AppConfig;
use items::{InteractionRange, ItemsPlugin};
use things::ThingsPlugin;
use tiles::TilesPlugin;

fn parse_log_level(s: &str) -> Level {
    match s.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" | "warning" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

fn main() {
    let app_config = shared::config::load_config();
    let log_level = parse_log_level(&app_config.debug.log_level);

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    // Dedicated headless server: minimal plugin set for physics + networking.
    // No window or rendering. Mesh/scene asset support is retained for physics.
    app.add_plugins(MinimalPlugins)
        .add_plugins(LogPlugin {
            level: log_level,
            ..default()
        })
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .add_plugins(bevy::mesh::MeshPlugin)
        .add_plugins(bevy::scene::ScenePlugin)
        .add_plugins(bevy::state::app::StatesPlugin)
        .insert_resource(Headless)
        .add_plugins(NetworkPlugin)
        .add_plugins(PhysicsPlugin)
        .add_plugins(TilesPlugin)
        .add_plugins(ThingsPlugin::<shared::app_state::AppState>::in_state(
            shared::app_state::AppState::InGame,
        ))
        .add_plugins(atmospherics::AtmosphericsPlugin)
        .add_plugins(creatures::CreaturesPlugin)
        .add_plugins(souls::SoulsPlugin)
        .add_plugins(shared::world_setup::WorldSetupPlugin)
        .add_plugins(shared::server::ServerPlugin)
        .add_plugins(ItemsPlugin)
        .add_plugins(InteractionsPlugin::<shared::app_state::AppState>::in_state(
            shared::app_state::AppState::InGame,
        ))
        .insert_resource(InteractionRange(app_config.items.interaction_range))
        .insert_state(shared::app_state::AppState::InGame)
        .add_systems(Startup, host_on_startup);

    app.run();
}

/// Startup system: auto-sends `NetCommand::Host` so the server begins listening immediately.
fn host_on_startup(mut net_commands: MessageWriter<NetCommand>, config: Res<AppConfig>) {
    net_commands.write(NetCommand::Host {
        port: config.network.port,
    });
    info!(
        "Headless server mode: auto-hosting on port {}",
        config.network.port
    );
}
