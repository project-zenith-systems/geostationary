use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::{log, prelude::*};
use tokio::sync::mpsc;

mod client;
mod config;
mod protocol;
mod runtime;
mod server;

pub use protocol::{HostMessage, PeerId, PeerMessage, PeerState};
use runtime::{NetEventReceiver, NetEventSender, NetworkRuntime, NetworkTasks, ServerCommand};

/// Bounded channel buffer size for client outbound messages.
/// Prevents memory exhaustion if game code produces messages faster than network can send.
const CLIENT_BUFFER_SIZE: usize = 100;

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
    PeerConnected { id: PeerId, addr: SocketAddr },
    Connected,
    Disconnected { reason: String },
    HostMessageReceived(HostMessage),
    PeerMessageReceived { from: PeerId, message: PeerMessage },
    PeerDisconnected { id: PeerId },
    Error(String),
}

/// Resource for sending messages from the server to clients.
/// Inserted when the server task starts, removed on stop.
#[derive(Resource, Clone)]
pub struct NetServerSender {
    tx: mpsc::UnboundedSender<ServerCommand>,
}

impl NetServerSender {
    /// Create a new server sender from a channel.
    pub(crate) fn new(tx: mpsc::UnboundedSender<ServerCommand>) -> Self {
        Self { tx }
    }

    /// Send a message to a specific peer.
    pub fn send_to(&self, peer: PeerId, message: &HostMessage) {
        if let Err(e) = self.tx.send(ServerCommand::SendTo { peer, message: message.clone() }) {
            log::warn!("Failed to send message to peer {}: {}", peer.0, e);
        }
    }

    /// Broadcast a message to all connected peers.
    pub fn broadcast(&self, message: &HostMessage) {
        if let Err(e) = self.tx.send(ServerCommand::Broadcast { message: message.clone() }) {
            log::warn!("Failed to broadcast message: {}", e);
        }
    }
}

/// Resource for sending messages from the client to the server.
/// Inserted when the client task starts, removed on disconnect.
#[derive(Resource, Clone)]
pub struct NetClientSender {
    tx: mpsc::Sender<PeerMessage>,
}

impl NetClientSender {
    /// Create a new client sender from a channel.
    pub(crate) fn new(tx: mpsc::Sender<PeerMessage>) -> Self {
        Self { tx }
    }

    /// Send a message to the server.
    /// Returns true if the message was sent, false if the channel is full or closed.
    pub fn send(&self, message: &PeerMessage) -> bool {
        match self.tx.try_send(message.clone()) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::debug!("Client send buffer full, message dropped");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::debug!("Client sender channel closed, message dropped");
                false
            }
        }
    }
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
fn drain_net_events(
    mut commands: Commands,
    mut receiver: ResMut<NetEventReceiver>,
    mut writer: MessageWriter<NetEvent>,
) {
    let mut count = 0;
    while count < MAX_NET_EVENTS_PER_FRAME {
        match receiver.0.try_recv() {
            Ok(event) => {
                // Remove NetServerSender when hosting stops
                if matches!(&event, NetEvent::HostingStopped) {
                    commands.remove_resource::<NetServerSender>();
                }
                // Remove NetClientSender when disconnected
                if matches!(&event, NetEvent::Disconnected { .. }) {
                    commands.remove_resource::<NetClientSender>();
                }
                
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
    mut commands: Commands,
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

                // Create server command channel and insert NetServerSender resource
                let (server_cmd_tx, server_cmd_rx) = mpsc::unbounded_channel();
                commands.insert_resource(NetServerSender::new(server_cmd_tx));

                let tx = event_tx.0.clone();
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle = runtime.spawn(server::run_server(*port, tx, server_cmd_rx, token_clone));
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

                // Create bounded client message channel and insert NetClientSender resource.
                // Bounded channel provides backpressure if game code sends faster than network can handle.
                let (client_msg_tx, client_msg_rx) = mpsc::channel(CLIENT_BUFFER_SIZE);
                commands.insert_resource(NetClientSender::new(client_msg_tx));

                let tx = event_tx.0.clone();
                let addr = *addr;
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle = runtime.spawn(client::run_client(addr, tx, client_msg_rx, token_clone));
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

    #[test]
    fn test_net_server_sender_send_to() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = NetServerSender::new(tx);

        let peer = PeerId(42);
        let message = HostMessage::Welcome { peer_id: peer };

        sender.send_to(peer, &message);

        let received = rx.try_recv().expect("Should receive command");
        match received {
            ServerCommand::SendTo { peer: recv_peer, message: recv_msg } => {
                assert_eq!(recv_peer, peer);
                match recv_msg {
                    HostMessage::Welcome { peer_id } => {
                        assert_eq!(peer_id, peer);
                    }
                    _ => panic!("Expected Welcome message"),
                }
            }
            _ => panic!("Expected SendTo command"),
        }
    }

    #[test]
    fn test_net_server_sender_broadcast() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = NetServerSender::new(tx);

        let message = HostMessage::PeerLeft { id: PeerId(7) };

        sender.broadcast(&message);

        let received = rx.try_recv().expect("Should receive command");
        match received {
            ServerCommand::Broadcast { message: recv_msg } => {
                match recv_msg {
                    HostMessage::PeerLeft { id } => {
                        assert_eq!(id, PeerId(7));
                    }
                    _ => panic!("Expected PeerLeft message"),
                }
            }
            _ => panic!("Expected Broadcast command"),
        }
    }

    #[test]
    fn test_net_client_sender_send() {
        let (tx, mut rx) = mpsc::channel(10);
        let sender = NetClientSender::new(tx);

        let message = PeerMessage::Input {
            direction: [1.0, 0.0, -1.0],
        };

        let result = sender.send(&message);
        assert!(result, "Message should be sent successfully");

        let received = rx.try_recv().expect("Should receive message");
        match received {
            PeerMessage::Input { direction } => {
                assert_eq!(direction, [1.0, 0.0, -1.0]);
            }
        }
    }

    #[test]
    fn test_net_client_sender_buffer_full() {
        let (tx, _rx) = mpsc::channel(1);
        let sender = NetClientSender::new(tx);

        let message = PeerMessage::Input {
            direction: [1.0, 0.0, -1.0],
        };

        // First send should succeed
        assert!(sender.send(&message), "First send should succeed");
        // Second send should fail because buffer is full
        assert!(!sender.send(&message), "Second send should fail when buffer full");
    }

}
