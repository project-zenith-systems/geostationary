use bevy::prelude::*;
#[cfg(feature = "client")]
use main_menu::{MainMenuPlugin, MenuEvent};
#[cfg(not(feature = "client"))]
use network::NetCommand;
use network::NetworkPlugin;
#[cfg(feature = "client")]
use physics::PhysicsDebugPlugin;
use physics::PhysicsPlugin;
use things::ThingsPlugin;
use tiles::TilesPlugin;
#[cfg(feature = "client")]
use ui::UiPlugin;

mod app_state;
#[cfg(feature = "client")]
mod client;
mod config;
#[cfg(feature = "client")]
mod main_menu;
mod server;
mod world_setup;

fn main() {
    let app_config = config::load_config();

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    #[cfg(feature = "client")]
    {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: app_config.window.title.clone(),
                ..default()
            }),
            ..default()
        }));
        app.add_plugins(UiPlugin::new().with_event::<MenuEvent>());
        app.add_plugins(MainMenuPlugin);
    }

    #[cfg(not(feature = "client"))]
    {
        app.add_plugins(headless_plugins());
    }

    app.add_plugins(NetworkPlugin);
    app.add_plugins(PhysicsPlugin);

    #[cfg(feature = "client")]
    if app_config.debug.physics_debug {
        app.add_plugins(PhysicsDebugPlugin);
    }

    app.add_plugins(TilesPlugin)
        .add_plugins(ThingsPlugin)
        .add_plugins(creatures::CreaturesPlugin);

    #[cfg(feature = "client")]
    {
        app.add_plugins(player::PlayerPlugin)
            .add_plugins(camera::CameraPlugin::<app_state::AppState>::in_state(
                app_state::AppState::InGame,
            ))
            .add_plugins(client::ClientPlugin);
    }

    app.add_plugins(world_setup::WorldSetupPlugin)
        .add_plugins(server::ServerPlugin)
        .init_state::<app_state::AppState>();

    #[cfg(not(feature = "client"))]
    {
        let port = app_config.network.port;
        app.add_systems(Startup, move |mut net_commands: MessageWriter<NetCommand>| {
            info!("Headless mode: hosting on port {port}");
            net_commands.write(NetCommand::Host { port });
        });
        app.add_systems(
            Startup,
            |mut next_state: ResMut<NextState<app_state::AppState>>| {
                next_state.set(app_state::AppState::InGame);
            },
        );
    }

    app.run();
}

/// Plugin set for headless/dedicated-server mode.
/// Provides the core engine (scheduling, time, transforms, state)
/// without windowing, rendering, or assets.
#[cfg(not(feature = "client"))]
fn headless_plugins() -> bevy::app::PluginGroupBuilder {
    MinimalPlugins
        .set(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .build()
        .add(bevy::log::LogPlugin::default())
        .add(bevy::transform::TransformPlugin)
        .add(bevy::state::app::StatesPlugin)
}
