use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::{log, prelude::*};
use tokio::sync::mpsc;

mod client;
mod config;
mod protocol;
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

/// Maximum number of network events to process per frame to prevent stalling.
const MAX_NET_EVENTS_PER_FRAME: usize = 100;

/// Flag to track if we've already warned about hitting the event cap.
static CAP_WARNING_LOGGED: AtomicBool = AtomicBool::new(false);

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
    let mut count = 0;
    while count < MAX_NET_EVENTS_PER_FRAME {
        match receiver.0.try_recv() {
            Ok(event) => {
                writer.write(event);
                count += 1;
            }
            Err(_) => return, // Channel empty, no more events to process
        }
    }

    // If we processed MAX_NET_EVENTS_PER_FRAME events, warn if there are more waiting
    if receiver.0.is_empty() {
        return;
    }

    if !CAP_WARNING_LOGGED.swap(true, Ordering::SeqCst) {
        log::warn!(
            "Hit MAX_NET_EVENTS_PER_FRAME limit of {MAX_NET_EVENTS_PER_FRAME}. \
            Additional events will be processed next frame. \
            This warning will only be shown once."
        );
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
                    let _ = event_tx
                        .0
                        .send(NetEvent::Error("Already hosting a server".into()));
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
                    let _ = event_tx
                        .0
                        .send(NetEvent::Error("Already connected to a server".into()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct NetEventCount {
        current: usize,
    }

    fn reset_count(mut count: ResMut<NetEventCount>) {
        count.current = 0;
    }

    fn count_net_events(mut reader: MessageReader<NetEvent>, mut count: ResMut<NetEventCount>) {
        for _event in reader.read() {
            count.current += 1;
        }
    }

    #[test]
    fn test_max_events_per_frame_cap() {
        // Create a test app
        let mut app = App::new();

        // Set up the channel manually
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        app.insert_resource(NetEventReceiver(event_rx));
        app.init_resource::<NetEventCount>();
        app.add_message::<NetEvent>();
        app.add_systems(
            Update,
            (reset_count, drain_net_events, count_net_events).chain(),
        );

        // Send more events than the cap
        for i in 0..(MAX_NET_EVENTS_PER_FRAME + 50) {
            event_tx
                .send(NetEvent::Error(format!("Test event {}", i)))
                .expect("Failed to send event");
        }

        // Run one frame
        app.update();

        // Read all the events that were processed
        let count = app.world().resource::<NetEventCount>().current;

        // Should have processed exactly MAX_NET_EVENTS_PER_FRAME events
        assert_eq!(
            count, MAX_NET_EVENTS_PER_FRAME,
            "Should process exactly MAX_NET_EVENTS_PER_FRAME events per frame"
        );

        // Run another frame to process remaining events
        app.update();

        let count2 = app.world().resource::<NetEventCount>().current;

        // Should have processed the remaining 50 events
        assert_eq!(count2, 50, "Should process remaining events in next frame");
    }
}
