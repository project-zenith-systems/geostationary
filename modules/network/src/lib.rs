use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::{log, prelude::*};
use bytes::Bytes;
use tokio::sync::mpsc;

mod client;
mod config;
mod protocol;
mod runtime;
mod server;

pub use protocol::{ClientId, ClientMessage, EntityState, NetId, ServerMessage, StreamReady};
use protocol::encode as proto_encode;
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
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Client {
    pub local_id: Option<ClientId>,
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
        /// Display name supplied by the client in its `Hello` message.
        name: String,
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
    /// Raw framed data received on a module stream (non-control, tag > 0).
    /// Modules subscribe to this event and filter by `tag` to decode their own messages.
    StreamFrame { tag: u8, data: Bytes },
    /// Emitted when a module stream sends the [`StreamReady`] sentinel.
    /// The client tracks these to determine when initial sync is complete.
    StreamReady { tag: u8 },
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

/// Error type for [`NetClientSender::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// The send buffer is full; the message was dropped.
    BufferFull,
    /// The channel is closed (client disconnected); the message was dropped.
    Closed,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::BufferFull => write!(f, "send buffer full"),
            SendError::Closed => write!(f, "channel closed"),
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
    pub fn send(&self, message: &ClientMessage) -> Result<(), SendError> {
        match self.tx.try_send(message.clone()) {
            Ok(_) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::debug!("Client send buffer full, message dropped");
                Err(SendError::BufferFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::debug!("Client sender channel closed, message dropped");
                Err(SendError::Closed)
            }
        }
    }
}

// Multi-stream infrastructure

/// Direction of a registered QUIC stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDirection {
    /// Server opens a unidirectional stream that the client accepts.
    ServerToClient,
    /// Client opens a stream that the server accepts.
    ClientToServer,
}

/// Declaration of a module stream registered with [`StreamRegistry`].
#[derive(Debug, Clone)]
pub struct StreamDef {
    /// Routing tag byte written as the first byte on every new stream.
    pub tag: u8,
    /// Human-readable name (used in log messages).
    pub name: &'static str,
    /// Which end initiates the stream.
    pub direction: StreamDirection,
}

/// Bounded buffer size for the StreamSender → server stream command channel.
const STREAM_CMD_BUFFER_SIZE: usize = 512;

/// Internal command for writing bytes to a specific module stream.
#[derive(Debug)]
pub(crate) enum StreamWriteCmd {
    /// Send `data` to one client.
    SendTo { client: ClientId, data: Bytes },
    /// Send `data` to all connected clients.
    Broadcast { data: Bytes },
}

/// Error type returned by [`StreamSender`] methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSendError {
    /// Serialisation of the message failed.
    Encode,
    /// The server is not running or the channel is closed.
    Closed,
    /// The stream command buffer is full; the message was dropped.
    BufferFull,
}

impl std::fmt::Display for StreamSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamSendError::Encode => write!(f, "encode error"),
            StreamSendError::Closed => write!(f, "channel closed / server not running"),
            StreamSendError::BufferFull => write!(f, "stream command buffer full"),
        }
    }
}

/// Shared sender end for a module stream's write-command channel.
/// `None` when no server is running; replaced each time the server starts.
type SharedStreamTx = Arc<Mutex<Option<mpsc::Sender<(u8, StreamWriteCmd)>>>>;

/// Typed resource that modules use to write messages to their registered stream.
///
/// Obtain one by calling [`StreamRegistry::register`] and inserting the
/// returned value as a Bevy resource.  The sender is live only while a server
/// is running; if no server is active, send attempts log a warning and return
/// [`Err(StreamSendError::Closed)`].
pub struct StreamSender<T: Send + Sync + 'static> {
    tag: u8,
    shared_tx: SharedStreamTx,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> bevy::ecs::resource::Resource for StreamSender<T> {}

impl<T: Send + Sync + 'static> StreamSender<T> {
    fn send_raw(&self, cmd: StreamWriteCmd) -> Result<(), StreamSendError> {
        let guard = self.shared_tx.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(tx) => match tx.try_send((self.tag, cmd)) {
                Ok(_) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    log::warn!(
                        "StreamSender (tag {}): command buffer full, message dropped",
                        self.tag
                    );
                    Err(StreamSendError::BufferFull)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::Closed),
            },
            None => {
                log::warn!(
                    "StreamSender (tag {}): send called but no server is running",
                    self.tag
                );
                Err(StreamSendError::Closed)
            }
        }
    }

    /// Send the [`StreamReady`] sentinel to a specific client.
    /// Call this after all initial-burst data has been sent on this stream.
    pub fn send_stream_ready_to(&self, client: ClientId) -> Result<(), StreamSendError> {
        let bytes = proto_encode(&StreamReady).map_err(|e| {
            log::error!("StreamSender (tag {}): failed to encode StreamReady: {}", self.tag, e);
            StreamSendError::Encode
        })?;
        self.send_raw(StreamWriteCmd::SendTo {
            client,
            data: Bytes::from(bytes),
        })
    }
}

impl<T> StreamSender<T>
where
    T: wincode::SchemaWrite<wincode::config::DefaultConfig, Src = T> + Send + Sync + 'static,
{
    /// Encode `msg` and send it to a specific client on this stream.
    pub fn send_to(&self, client: ClientId, msg: &T) -> Result<(), StreamSendError> {
        let bytes = protocol::encode(msg).map_err(|e| {
            log::error!("StreamSender (tag {}): encode failed: {}", self.tag, e);
            StreamSendError::Encode
        })?;
        self.send_raw(StreamWriteCmd::SendTo {
            client,
            data: Bytes::from(bytes),
        })
    }

    /// Encode `msg` and broadcast it to all connected clients on this stream.
    pub fn broadcast(&self, msg: &T) -> Result<(), StreamSendError> {
        let bytes = protocol::encode(msg).map_err(|e| {
            log::error!("StreamSender (tag {}): encode failed: {}", self.tag, e);
            StreamSendError::Encode
        })?;
        self.send_raw(StreamWriteCmd::Broadcast {
            data: Bytes::from(bytes),
        })
    }
}

/// Registry of module streams.  Modules call [`StreamRegistry::register`]
/// during their plugin's `build()` to declare the streams they own.
/// The network plugin reads the registry when hosting starts to know which
/// QUIC streams to open/accept on each new connection.
#[derive(Resource)]
pub struct StreamRegistry {
    entries: Vec<StreamDef>,
    /// Shared with every [`StreamSender`] created from this registry.
    /// Replaced with a live sender each time the server starts; set to `None`
    /// when the server stops.
    shared_tx: SharedStreamTx,
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            shared_tx: Arc::new(Mutex::new(None)),
        }
    }
}

impl StreamRegistry {
    /// Register a stream and return a [`StreamSender<T>`] for writing to it.
    ///
    /// Insert the returned sender as a Bevy resource so that your module's
    /// systems can write to the stream via [`StreamSender::send_to`] /
    /// [`StreamSender::broadcast`].
    ///
    /// # Panics
    /// Panics if `def.tag == 0` (reserved for the control stream) or if the
    /// tag has already been registered.
    pub fn register<T: Send + Sync + 'static>(&mut self, def: StreamDef) -> StreamSender<T> {
        assert_ne!(def.tag, 0, "stream tag 0 is reserved for the control stream");
        assert!(
            !self.entries.iter().any(|e| e.tag == def.tag),
            "stream tag {} is already registered",
            def.tag
        );
        let tag = def.tag;
        self.entries.push(def);
        StreamSender {
            tag,
            shared_tx: self.shared_tx.clone(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Number of registered server→client streams.
    /// Sent as `expected_streams` in the `Welcome` message.
    pub fn server_to_client_count(&self) -> u8 {
        self.entries
            .iter()
            .filter(|d| d.direction == StreamDirection::ServerToClient)
            .count() as u8
    }

    /// Called by the server startup path.  Creates a fresh command channel,
    /// wires `shared_tx` to the new sender, and returns the stream definitions
    /// and command receiver to pass to the server task.
    pub(crate) fn prepare_server_start(
        &mut self,
    ) -> (
        Vec<StreamDef>,
        mpsc::Receiver<(u8, StreamWriteCmd)>,
    ) {
        let (tx, rx) = mpsc::channel(STREAM_CMD_BUFFER_SIZE);
        *self.shared_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let defs = self.entries.clone();
        (defs, rx)
    }

    /// Called when the server stops.  Disconnects stream senders so that
    /// [`StreamSender`] calls made while no server is running are rejected.
    pub(crate) fn on_server_stop(&self) {
        *self.shared_tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        app.init_resource::<StreamRegistry>();
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
    registry: Res<StreamRegistry>,
) {
    let mut count = 0;
    while count < MAX_NET_EVENTS_PER_FRAME {
        match receiver.0.try_recv() {
            Ok(event) => {
                // Remove session and sender resources when hosting stops
                if matches!(&event, ServerEvent::HostingStopped) {
                    commands.remove_resource::<Server>();
                    commands.remove_resource::<NetServerSender>();
                    registry.on_server_stop();
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
                // Remove session and sender resources when disconnected
                if matches!(&event, ClientEvent::Disconnected { .. }) {
                    commands.remove_resource::<Client>();
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
    mut registry: ResMut<StreamRegistry>,
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

                // Insert session resources and create server command channel
                commands.insert_resource(Server::default());
                let (server_cmd_tx, server_cmd_rx) = mpsc::unbounded_channel();
                commands.insert_resource(NetServerSender::new(server_cmd_tx));

                // Prepare per-stream channels from the registry
                let (stream_defs, stream_cmd_rx) = registry.prepare_server_start();

                let tx = server_event_tx.0.clone();
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle = runtime.spawn(server::run_server(
                    *port,
                    tx,
                    server_cmd_rx,
                    token_clone,
                    stream_defs,
                    stream_cmd_rx,
                ));
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

                // Insert session resource and create bounded client message channel.
                // Bounded channel provides backpressure if game code sends faster than network can handle.
                commands.insert_resource(Client::default());
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
        let message = ServerMessage::Welcome {
            client_id: client,
            expected_streams: 0,
        };

        sender.send_to(client, &message);

        let received = rx.try_recv().expect("Should receive command");
        match received {
            ServerCommand::SendTo {
                client: recv_client,
                message: recv_msg,
            } => {
                assert_eq!(recv_client, client);
                match recv_msg {
                    ServerMessage::Welcome {
                        client_id,
                        expected_streams,
                    } => {
                        assert_eq!(client_id, client);
                        assert_eq!(expected_streams, 0);
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
        assert!(result.is_ok(), "Message should be sent successfully");

        let received = rx.try_recv().expect("Should receive message");
        match received {
            ClientMessage::Input { direction } => {
                assert_eq!(direction, [1.0, 0.0, -1.0]);
            }
            ClientMessage::Hello { .. } => panic!("Expected Input message"),
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
        assert!(sender.send(&message).is_ok(), "First send should succeed");
        // Second send should fail because buffer is full
        assert_eq!(
            sender.send(&message),
            Err(SendError::BufferFull),
            "Second send should fail when buffer full"
        );
    }

    #[test]
    fn test_stream_registry_register_and_count() {
        let mut registry = StreamRegistry::default();
        assert_eq!(registry.server_to_client_count(), 0);

        registry.register::<ServerMessage>(StreamDef {
            tag: 1,
            name: "tiles",
            direction: StreamDirection::ServerToClient,
        });
        assert_eq!(registry.server_to_client_count(), 1);

        registry.register::<ServerMessage>(StreamDef {
            tag: 2,
            name: "atmos",
            direction: StreamDirection::ServerToClient,
        });
        assert_eq!(registry.server_to_client_count(), 2);
    }

    #[test]
    fn test_stream_sender_closed_when_no_server() {
        let mut registry = StreamRegistry::default();
        let sender: StreamSender<ServerMessage> = registry.register(StreamDef {
            tag: 1,
            name: "test",
            direction: StreamDirection::ServerToClient,
        });
        // No server running → shared_tx is None → send_stream_ready_to returns Closed
        let result = sender.send_stream_ready_to(ClientId(1));
        assert_eq!(result, Err(StreamSendError::Closed));
    }

    #[test]
    fn test_stream_registry_prepare_and_stop() {
        let mut registry = StreamRegistry::default();
        let sender: StreamSender<ServerMessage> = registry.register(StreamDef {
            tag: 3,
            name: "things",
            direction: StreamDirection::ServerToClient,
        });

        // After prepare_server_start the shared_tx is live
        let (defs, _rx) = registry.prepare_server_start();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].tag, 3);

        // send_stream_ready_to should now succeed (channel is open)
        let result = sender.send_stream_ready_to(ClientId(99));
        assert!(result.is_ok(), "Should succeed while server is running");

        // After on_server_stop the channel closes
        registry.on_server_stop();
        let result = sender.send_stream_ready_to(ClientId(99));
        assert_eq!(result, Err(StreamSendError::Closed));
    }
}
