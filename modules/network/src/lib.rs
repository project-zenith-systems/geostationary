use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::{log, prelude::*};
use tokio::sync::mpsc;

mod client;
mod config;
mod protocol;
mod runtime;
mod server;

pub use protocol::{ClientId, ClientMessage, EntityState, NetId, ServerMessage};
use runtime::{
    ClientEventReceiver, ClientEventSender, NetworkRuntime, NetworkTasks, ServerCommand,
    ServerEventReceiver, ServerEventSender,
};

/// Bounded channel buffer size for client outbound messages.
/// Prevents memory exhaustion if game code produces messages faster than network can send.
const CLIENT_BUFFER_SIZE: usize = 100;

/// Network update rate in Hz (updates per second).
const NETWORK_UPDATE_RATE: f32 = 30.0;
pub const NETWORK_UPDATE_INTERVAL: f32 = 1.0 / NETWORK_UPDATE_RATE;

/// System set for network systems. Game code should read network events
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

/// Resource to track the state of the server.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Server {
    next_net_id: u64,
}

impl Default for Server {
    fn default() -> Self {
        Self { next_net_id: 1 }
    }
}

impl Server {
    pub fn next_net_id(&mut self) -> NetId {
        let id = self.next_net_id;
        self.next_net_id += 1;
        NetId(id)
    }
}

/// Resource to track the state of the client.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client {
    pub local_id: Option<ClientId>,
}

impl Default for Client {
    fn default() -> Self {
        Self { local_id: None }
    }
}

/// Component: which client's input controls this entity (server-side only).
#[derive(Component, Debug, Clone, Copy)]
pub struct ControlledByClient(pub ClientId);

/// Events emitted by the server side of the network layer.
#[derive(Message, Clone, Debug)]
pub enum ServerEvent {
    HostingStarted {
        port: u16,
    },
    HostingStopped,
    ClientConnected {
        id: ClientId,
        addr: SocketAddr,
    },
    ClientMessageReceived {
        from: ClientId,
        message: ClientMessage,
    },
    ClientDisconnected {
        id: ClientId,
    },
    Error(String),
}

/// Events emitted by the client side of the network layer.
#[derive(Message, Clone, Debug)]
pub enum ClientEvent {
    Connected,
    Disconnected { reason: String },
    ServerMessageReceived(ServerMessage),
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

    /// Send a message to a specific client.
    pub fn send_to(&self, client: ClientId, message: &ServerMessage) {
        if let Err(e) = self.tx.send(ServerCommand::SendTo {
            client,
            message: message.clone(),
        }) {
            log::warn!("Failed to send message to client {}: {}", client.0, e);
        }
    }

    /// Broadcast a message to all connected clients.
    pub fn broadcast(&self, message: &ServerMessage) {
        if let Err(e) = self.tx.send(ServerCommand::Broadcast {
            message: message.clone(),
        }) {
            log::warn!("Failed to broadcast message: {}", e);
        }
    }
}

/// Resource for sending messages from the client to the server.
/// Inserted when the client task starts, removed on disconnect.
#[derive(Resource, Clone)]
pub struct NetClientSender {
    tx: mpsc::Sender<ClientMessage>,
}

impl NetClientSender {
    /// Create a new client sender from a channel.
    pub(crate) fn new(tx: mpsc::Sender<ClientMessage>) -> Self {
        Self { tx }
    }

    /// Send a message to the server.
    /// Returns true if the message was sent, false if the channel is full or closed.
    pub fn send(&self, message: &ClientMessage) -> bool {
        // TODO return an error
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
        let (server_event_tx, server_event_rx) = mpsc::unbounded_channel();
        let (client_event_tx, client_event_rx) = mpsc::unbounded_channel();

        app.insert_resource(NetworkRuntime::new());
        app.insert_resource(NetworkTasks::default());
        app.insert_resource(ServerEventSender(server_event_tx));
        app.insert_resource(ServerEventReceiver(server_event_rx));
        app.insert_resource(ClientEventSender(client_event_tx));
        app.insert_resource(ClientEventReceiver(client_event_rx));
        app.add_message::<NetCommand>();
        app.add_message::<ServerEvent>();
        app.add_message::<ClientEvent>();
        app.configure_sets(PreUpdate, NetworkSet::Receive.before(NetworkSet::Send));
        app.add_systems(
            PreUpdate,
            (drain_server_events, drain_client_events).in_set(NetworkSet::Receive),
        );
        app.add_systems(PreUpdate, process_net_commands.in_set(NetworkSet::Send));
    }
}

/// Drains server events from the async mpsc channel and writes them as Bevy messages.
fn drain_server_events(
    mut commands: Commands,
    mut receiver: ResMut<ServerEventReceiver>,
    mut writer: MessageWriter<ServerEvent>,
) {
    let mut count = 0;
    while count < MAX_NET_EVENTS_PER_FRAME {
        match receiver.0.try_recv() {
            Ok(event) => {
                // Remove NetServerSender when hosting stops
                if matches!(&event, ServerEvent::HostingStopped) {
                    commands.remove_resource::<NetServerSender>();
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

/// Drains client events from the async mpsc channel and writes them as Bevy messages.
fn drain_client_events(
    mut commands: Commands,
    mut receiver: ResMut<ClientEventReceiver>,
    mut writer: MessageWriter<ClientEvent>,
) {
    let mut count = 0;
    while count < MAX_NET_EVENTS_PER_FRAME {
        match receiver.0.try_recv() {
            Ok(event) => {
                // Remove NetClientSender when disconnected
                if matches!(&event, ClientEvent::Disconnected { .. }) {
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
    server_event_tx: Res<ServerEventSender>,
    client_event_tx: Res<ClientEventSender>,
    mut tasks: ResMut<NetworkTasks>,
) {
    // Clean up any finished tasks before processing new commands
    tasks.cleanup_finished();

    for command in commands_reader.read() {
        match command {
            NetCommand::Host { port } => {
                // Prevent duplicate hosting
                if tasks.is_hosting() {
                    let _ = server_event_tx
                        .0
                        .send(ServerEvent::Error("Already hosting a server".into()));
                    continue;
                }

                // Create server command channel and insert NetServerSender resource
                let (server_cmd_tx, server_cmd_rx) = mpsc::unbounded_channel();
                commands.insert_resource(NetServerSender::new(server_cmd_tx));

                let tx = server_event_tx.0.clone();
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle =
                    runtime.spawn(server::run_server(*port, tx, server_cmd_rx, token_clone));
                tasks.server_task = Some((handle, cancel_token));
            }
            NetCommand::Connect { addr } => {
                // Prevent duplicate connections
                if tasks.is_connected() {
                    let _ = client_event_tx
                        .0
                        .send(ClientEvent::Error("Already connected to a server".into()));
                    continue;
                }

                // Create bounded client message channel and insert NetClientSender resource.
                // Bounded channel provides backpressure if game code sends faster than network can handle.
                let (client_msg_tx, client_msg_rx) = mpsc::channel(CLIENT_BUFFER_SIZE);
                commands.insert_resource(NetClientSender::new(client_msg_tx));

                let tx = client_event_tx.0.clone();
                let addr = *addr;
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle =
                    runtime.spawn(client::run_client(addr, tx, client_msg_rx, token_clone));
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
    struct ClientEventCount {
        current: usize,
    }

    fn reset_count(mut count: ResMut<ClientEventCount>) {
        count.current = 0;
    }

    fn count_net_events(
        mut reader: MessageReader<ClientEvent>,
        mut count: ResMut<ClientEventCount>,
    ) {
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

        app.insert_resource(ClientEventReceiver(event_rx));
        app.init_resource::<ClientEventCount>();
        app.add_message::<ClientEvent>();
        app.add_systems(
            Update,
            (reset_count, drain_client_events, count_net_events).chain(),
        );

        // Send more events than the cap
        for i in 0..(MAX_NET_EVENTS_PER_FRAME + 50) {
            event_tx
                .send(ClientEvent::Error(format!("Test event {}", i)))
                .expect("Failed to send event");
        }

        // Run one frame
        app.update();

        // Read all the events that were processed
        let count = app.world().resource::<ClientEventCount>().current;

        // Should have processed exactly MAX_NET_EVENTS_PER_FRAME events
        assert_eq!(
            count, MAX_NET_EVENTS_PER_FRAME,
            "Should process exactly MAX_NET_EVENTS_PER_FRAME events per frame"
        );

        // Run another frame to process remaining events
        app.update();

        let count2 = app.world().resource::<ClientEventCount>().current;

        // Should have processed the remaining 50 events
        assert_eq!(count2, 50, "Should process remaining events in next frame");
    }

    #[test]
    fn test_net_server_sender_send_to() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = NetServerSender::new(tx);

        let client = ClientId(42);
        let message = ServerMessage::Welcome { client_id: client };

        sender.send_to(client, &message);

        let received = rx.try_recv().expect("Should receive command");
        match received {
            ServerCommand::SendTo {
                client: recv_client,
                message: recv_msg,
            } => {
                assert_eq!(recv_client, client);
                match recv_msg {
                    ServerMessage::Welcome { client_id } => {
                        assert_eq!(client_id, client);
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

        let message = ServerMessage::EntityDespawned { net_id: NetId(7) };

        sender.broadcast(&message);

        let received = rx.try_recv().expect("Should receive command");
        match received {
            ServerCommand::Broadcast { message: recv_msg } => match recv_msg {
                ServerMessage::EntityDespawned { net_id } => {
                    assert_eq!(net_id, NetId(7));
                }
                _ => panic!("Expected EntityDespawned message"),
            },
            _ => panic!("Expected Broadcast command"),
        }
    }

    #[test]
    fn test_net_client_sender_send() {
        let (tx, mut rx) = mpsc::channel(10);
        let sender = NetClientSender::new(tx);

        let message = ClientMessage::Input {
            direction: [1.0, 0.0, -1.0],
        };

        let result = sender.send(&message);
        assert!(result, "Message should be sent successfully");

        let received = rx.try_recv().expect("Should receive message");
        match received {
            ClientMessage::Input { direction } => {
                assert_eq!(direction, [1.0, 0.0, -1.0]);
            }
            ClientMessage::Hello => panic!("Expected Input message"),
        }
    }

    #[test]
    fn test_net_client_sender_buffer_full() {
        let (tx, _rx) = mpsc::channel(1);
        let sender = NetClientSender::new(tx);

        let message = ClientMessage::Input {
            direction: [1.0, 0.0, -1.0],
        };

        // First send should succeed
        assert!(sender.send(&message), "First send should succeed");
        // Second send should fail because buffer is full
        assert!(
            !sender.send(&message),
            "Second send should fail when buffer full"
        );
    }
}
