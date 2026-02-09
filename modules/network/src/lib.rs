use std::net::SocketAddr;

use bevy::prelude::*;
use tokio::sync::mpsc;

mod client;
mod config;
mod runtime;
mod server;

use runtime::{NetEventReceiver, NetEventSender, NetworkRuntime, NetworkTasks};

/// System set for network systems. Game code should read `NetEvent` messages
/// after `NetworkSet::Receive` and write `NetCommand` messages before
/// `NetworkSet::Send`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkSet {
    /// Drains async events into Bevy messages.
    Receive,
    /// Processes commands and dispatches them to async tasks.
    Send,
}

/// Commands sent by game code to control the network layer.
#[derive(Message, Clone, Debug)]
pub enum NetCommand {
    Host { port: u16 },
    Connect { addr: SocketAddr },
    StopHosting,
    Disconnect,
}

/// Events emitted by the network layer back to game code.
#[derive(Message, Clone, Debug)]
pub enum NetEvent {
    HostingStarted { port: u16 },
    HostingStopped,
    ClientConnected { addr: SocketAddr },
    Connected,
    Disconnected { reason: String },
    Error(String),
}

pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        app.insert_resource(NetworkRuntime::new());
        app.insert_resource(NetworkTasks::default());
        app.insert_resource(NetEventSender(event_tx));
        app.insert_resource(NetEventReceiver(event_rx));
        app.add_message::<NetCommand>();
        app.add_message::<NetEvent>();
        app.configure_sets(PreUpdate, NetworkSet::Receive.before(NetworkSet::Send));
        app.add_systems(PreUpdate, drain_net_events.in_set(NetworkSet::Receive));
        app.add_systems(PreUpdate, process_net_commands.in_set(NetworkSet::Send));
    }
}

/// Drains events from the async mpsc channel and writes them as Bevy messages.
fn drain_net_events(mut receiver: ResMut<NetEventReceiver>, mut writer: MessageWriter<NetEvent>) {
    while let Ok(event) = receiver.0.try_recv() {
        writer.write(event);
    }
}

/// Reads NetCommand Bevy messages and spawns async tasks accordingly.
fn process_net_commands(
    mut commands_reader: MessageReader<NetCommand>,
    runtime: Res<NetworkRuntime>,
    event_tx: Res<NetEventSender>,
    mut tasks: ResMut<NetworkTasks>,
) {
    // Clean up any finished tasks before processing new commands
    tasks.cleanup_finished();

    for command in commands_reader.read() {
        match command {
            NetCommand::Host { port } => {
                // Prevent duplicate hosting
                if tasks.is_hosting() {
                    let _ = event_tx.0.send(NetEvent::Error(
                        "Already hosting a server".into()
                    ));
                    continue;
                }

                let tx = event_tx.0.clone();
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle = runtime.spawn(server::run_server(*port, tx, token_clone));
                tasks.server_task = Some((handle, cancel_token));
            }
            NetCommand::Connect { addr } => {
                // Prevent duplicate connections
                if tasks.is_connected() {
                    let _ = event_tx.0.send(NetEvent::Error(
                        "Already connected to a server".into()
                    ));
                    continue;
                }

                let tx = event_tx.0.clone();
                let addr = *addr;
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle = runtime.spawn(client::run_client(addr, tx, token_clone));
                tasks.client_task = Some((handle, cancel_token));
            }
            NetCommand::StopHosting => {
                tasks.stop_hosting();
            }
            NetCommand::Disconnect => {
                tasks.disconnect();
            }
        }
    }
}
