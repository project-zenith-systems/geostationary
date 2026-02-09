use std::net::SocketAddr;

use bevy::prelude::*;
use tokio::sync::mpsc;

mod client;
mod config;
mod runtime;
mod server;

use runtime::{NetEventReceiver, NetEventSender, NetworkRuntime};

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
}

/// Events emitted by the network layer back to game code.
#[derive(Message, Clone, Debug)]
pub enum NetEvent {
    HostingStarted { port: u16 },
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
) {
    for command in commands_reader.read() {
        match command {
            NetCommand::Host { port } => {
                let tx = event_tx.0.clone();
                runtime.spawn(server::run_server(*port, tx));
            }
            NetCommand::Connect { addr } => {
                let tx = event_tx.0.clone();
                let addr = *addr;
                runtime.spawn(client::run_client(addr, tx));
            }
        }
    }
}
