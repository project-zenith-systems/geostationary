use std::net::SocketAddr;

use bevy::log::LogPlugin;
use bevy::prelude::*;
use editor::EditorPlugin;
use input::InputPlugin;
use interactions::{ContextMenuAction, InteractionsPlugin};
use items::{InteractionRange, ItemsPlugin};
use main_menu::{MainMenuConfig, MainMenuPlugin, MenuEvent};
use network::{Headless, NetCommand, NetworkPlugin, NetworkReceive, Server, ServerEvent};
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use shared::app_state::AppState;
use shared::config::AppConfig;
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;
use world::{MapPath, WorldPlugin};

mod editor;

fn main() {
    let start_in_editor = std::env::args().any(|a| a == "--editor");

    let app_config = shared::config::load_config();

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin::from(&app_config))
            .set(LogPlugin::from(&app_config)),
    )
    .add_plugins(
        UiPlugin::new()
            .with_event::<MenuEvent>()
            .with_event::<ContextMenuAction>(),
    )
    .insert_resource(MainMenuConfig {
        port: app_config.network.port,
        player_name: app_config.souls.player_name.clone(),
    })
    .add_plugins(MainMenuPlugin {
        state: AppState::MainMenu,
        editor_state: AppState::Editor,
    })
    .add_plugins(NetworkPlugin {
        loading: AppState::Loading,
        in_game: AppState::InGame,
        disconnected: AppState::MainMenu,
    })
    .add_plugins(PhysicsPlugin);

    if app_config.debug.physics_debug {
        app.add_plugins(PhysicsDebugPlugin);
    }

    app.add_plugins(WorldPlugin {
        loading: AppState::Loading,
        in_game: AppState::InGame,
    })
    .add_plugins(TilesPlugin::in_state(AppState::InGame))
    .add_plugins(ThingsPlugin::<AppState>::in_state(AppState::InGame))
    .add_plugins(atmospherics::AtmosphericsPlugin::new(
        AppState::Loading,
        AppState::InGame,
        app_config.atmospherics.standard_pressure,
        app_config.atmospherics.pressure_force_scale,
        app_config.atmospherics.diffusion_rate,
    ))
    .add_plugins(creatures::CreaturesPlugin)
    .add_plugins(souls::SoulsPlugin)
    .add_plugins(player::PlayerPlugin)
    .add_plugins(camera::CameraPlugin::<AppState>::in_state(AppState::InGame))
    .add_plugins(EditorPlugin)
    .add_plugins(shared::templates::TemplatesPlugin)
    .add_plugins(InputPlugin::<AppState>::in_state(AppState::InGame))
    .add_plugins(InteractionsPlugin::<AppState>::in_state(AppState::InGame))
    .add_plugins(ItemsPlugin)
    .insert_resource(InteractionRange(app_config.items.interaction_range))
    .add_systems(NetworkReceive, listen_server_self_connect)
    .add_systems(
        OnEnter(AppState::Loading),
        ensure_map_path_on_host.before(world::loader::load_map),
    );

    if start_in_editor {
        app.insert_state(AppState::Editor);
    } else {
        app.init_state::<AppState>();
    }

    app.run();
}

/// On a listen-server, inserts [`MapPath`] from config so [`world::loader::load_map`]
/// can find the map file.  Runs before `load_map` on `OnEnter(Loading)`.
///
/// Pure clients (no [`Server`] resource) skip this — they receive world state
/// over the network rather than loading from disk.
fn ensure_map_path_on_host(world: &mut World) {
    if world.contains_resource::<Server>() && !world.contains_resource::<MapPath>() {
        let config = world.resource::<AppConfig>();
        let map_path = config.world.map_path.clone();
        world.insert_resource(MapPath::new(map_path));
    }
}

/// On a listen-server (non-headless), automatically connects to self when hosting starts.
fn listen_server_self_connect(
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    config: Res<AppConfig>,
    headless: Option<Res<Headless>>,
) {
    if headless.is_some() {
        return;
    }
    for event in messages.read() {
        if let ServerEvent::HostingStarted { port } = event {
            let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
            info!("Listen-server: connecting to self at {addr}");
            net_commands.write(NetCommand::Connect {
                addr,
                name: config.souls.player_name.clone(),
            });
        }
    }
}
