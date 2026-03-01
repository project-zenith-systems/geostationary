use std::sync::atomic::{AtomicBool, Ordering};

use bevy::app::AppExit;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use network::{Headless, NetCommand, NetServerSender, NetworkPlugin, ServerMessage};
use physics::PhysicsPlugin;
use shared::config::AppConfig;
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

/// Set to `true` by the CTRL-C / SIGINT handler.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Marker resource inserted once graceful shutdown has been initiated.
/// Holds a frame counter used to give the async network layer time to
/// flush the [`ServerMessage::Shutdown`] broadcast before connections
/// are cancelled.
#[derive(Resource)]
struct ShuttingDown {
    /// Number of frames elapsed since the shutdown broadcast was sent.
    frames_elapsed: u32,
}

fn main() {
    ctrlc::set_handler(|| {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    })
    .expect(
        "Failed to set CTRL-C handler: another signal handler may already be registered",
    );

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
        .insert_state(shared::app_state::AppState::InGame)
        .add_systems(Startup, host_on_startup)
        .add_systems(Update, check_shutdown_signal);

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

/// Checks for a pending CTRL-C / SIGINT signal and performs a graceful shutdown:
///
/// * **Phase 1** (first frame the signal is detected): broadcast [`ServerMessage::Shutdown`]
///   to all connected clients so they can disconnect cleanly, then insert the
///   [`ShuttingDown`] marker resource.
/// * **Phase 2a** (after [`SHUTDOWN_FLUSH_FRAMES`] more frames): send
///   [`NetCommand::StopHosting`] so the network layer begins tearing down.
/// * **Phase 2b** (once [`NetServerSender`] is removed): request [`AppExit`].
///   [`NetServerSender`] is removed by `drain_server_events` when it receives
///   `ServerEvent::HostingStopped`, which is emitted by the server task once
///   hosting has stopped (it no longer accepts new connections), but not
///   necessarily after all existing connections have fully closed.  Waiting for
///   this guarantees that `process_net_commands` (which runs in `PreUpdate`) has
///   had a full frame to process the [`NetCommand::StopHosting`] written in the
///   previous `Update` before the process exits.
///
/// The flush delay gives the async network layer several frames (~100 ms at 30 Hz)
/// to deliver the [`ServerMessage::Shutdown`] broadcast through both the server-command
/// channel and the per-client write channels before the cancellation token fires.
const SHUTDOWN_FLUSH_FRAMES: u32 = 3;

fn check_shutdown_signal(
    mut commands: Commands,
    sender: Option<Res<NetServerSender>>,
    shutting_down: Option<ResMut<ShuttingDown>>,
    mut net_commands: MessageWriter<NetCommand>,
    mut app_exit: MessageWriter<AppExit>,
) {
    if !SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
        return;
    }

    match shutting_down {
        None => {
            // Phase 1: notify clients and mark shutdown in progress.
            if let Some(sender_ref) = sender.as_ref() {
                sender_ref.broadcast(&ServerMessage::Shutdown);
            }
            commands.insert_resource(ShuttingDown { frames_elapsed: 0 });
            info!("Shutdown signal received: notifying clients and stopping server...");
        }
        Some(mut state) => {
            state.frames_elapsed += 1;
            if state.frames_elapsed == SHUTDOWN_FLUSH_FRAMES {
                // Phase 2a: after the flush window, request the network to stop hosting.
                // StopHosting will be processed by process_net_commands in the next PreUpdate.
                net_commands.write(NetCommand::StopHosting);
            }

            // Phase 2b: once hosting has actually stopped (NetServerSender removed by
            // drain_server_events on HostingStopped), it is safe to exit.
            if state.frames_elapsed >= SHUTDOWN_FLUSH_FRAMES && sender.is_none() {
                app_exit.write(AppExit::Success);
            }
        }
    }
}
