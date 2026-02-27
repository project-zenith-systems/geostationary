use std::collections::{HashMap, VecDeque};
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

use protocol::encode as proto_encode;
pub use protocol::{ClientId, ClientMessage, EntityState, NetId, ServerMessage, StreamReady};
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
    Connect { addr: SocketAddr, name: String },
    StopHosting,
    Disconnect,
}

/// Marker resource inserted when the application starts in dedicated-server (headless) mode.
///
/// Plugins can check for this resource (e.g. `resource_exists::<Headless>`) to skip
/// visual-only systems such as mesh spawning, debug overlays, and input handling.
/// Headless servers use `MinimalPlugins` and omit all rendering and windowing plugins.
#[derive(Resource, Default)]
pub struct Headless;

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

/// Server-side lifecycle events for player (client) connections.
///
/// Domain modules (e.g. `tiles`, `things`, `souls`) should listen to this instead of
/// raw [`ServerEvent`] variants so they are decoupled from the network layer.
#[derive(Message, Clone, Debug)]
pub enum PlayerEvent {
    /// Emitted when a client completes the handshake.
    Joined { id: ClientId, name: String },
    /// Emitted when a client disconnects.
    Left { id: ClientId },
}

/// Server-side event emitted by a module stream handler after it has sent its initial data
/// burst and the [`StreamReady`] sentinel to a specific client.
///
/// The orchestration system in `src/server.rs` collects these, and once all registered
/// server→client module streams have reported ready for a given client it sends
/// [`ServerMessage::InitialStateDone`] on the control stream.  This is event-driven and
/// works correctly even when initial data spans multiple frames.
#[derive(Message, Clone, Debug)]
pub struct ModuleReadySent {
    pub client: ClientId,
}

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
    /// Raw framed data received from a client on a registered client→server module stream.
    /// Routed internally to per-tag [`StreamReader`] buffers; not emitted as a Bevy message.
    ClientStreamFrame {
        from: ClientId,
        tag: u8,
        data: Bytes,
    },
    Error(String),
}

/// Events emitted by the client side of the network layer.
#[derive(Message, Clone, Debug)]
pub enum ClientEvent {
    Connected,
    Disconnected {
        reason: String,
    },
    ServerMessageReceived(ServerMessage),
    /// Raw framed data received on a module stream (non-control, tag > 0).
    /// Modules subscribe to this event and filter by `tag` to decode their own messages.
    StreamFrame {
        tag: u8,
        data: Bytes,
    },
    /// Emitted when a module stream sends the [`StreamReady`] sentinel.
    /// The client tracks these to determine when initial sync is complete.
    StreamReady {
        tag: u8,
    },
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
            log::error!("Failed to send message to client {}: {}", client.0, e);
        }
    }

    /// Broadcast a message to all connected clients.
    pub fn broadcast(&self, message: &ServerMessage) {
        if let Err(e) = self.tx.send(ServerCommand::Broadcast {
            message: message.clone(),
        }) {
            log::error!("Failed to broadcast message: {}", e);
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

/// Shared sender end for a client→server stream's write channel.
/// `None` when no client is connected; replaced each time the client connects.
type SharedClientStreamTx = Arc<Mutex<Option<mpsc::Sender<Bytes>>>>;

/// Typed resource that modules use to write messages to their registered stream.
///
/// Obtain one by calling [`StreamRegistry::register`] and inserting the
/// returned value as a Bevy resource.  The sender is live only while a server
/// is running (for [`StreamDirection::ServerToClient`]) or while a client is
/// connected (for [`StreamDirection::ClientToServer`]); if the relevant session
/// is inactive, send attempts log a warning and return
/// [`Err(StreamSendError::Closed)`].
pub struct StreamSender<T: Send + Sync + 'static> {
    tag: u8,
    direction: StreamDirection,
    shared_tx: SharedStreamTx,
    client_tx: SharedClientStreamTx,
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
                    log::error!(
                        "StreamSender (tag {}): command buffer full, message dropped",
                        self.tag
                    );
                    Err(StreamSendError::BufferFull)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::Closed),
            },
            None => {
                log::error!(
                    "StreamSender (tag {}): send called but no server is running",
                    self.tag
                );
                Err(StreamSendError::Closed)
            }
        }
    }

    fn send_raw_client(&self, data: Bytes) -> Result<(), StreamSendError> {
        let guard = self.client_tx.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(tx) => match tx.try_send(data) {
                Ok(_) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    log::error!(
                        "StreamSender (tag {}): client stream buffer full, message dropped",
                        self.tag
                    );
                    Err(StreamSendError::BufferFull)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::Closed),
            },
            None => {
                log::error!(
                    "StreamSender (tag {}): send called but no client is connected",
                    self.tag
                );
                Err(StreamSendError::Closed)
            }
        }
    }

    /// Send the [`StreamReady`] sentinel to a specific client.
    /// Call this after all initial-burst data has been sent on this stream.
    ///
    /// Only valid for streams registered with [`StreamDirection::ServerToClient`].
    pub fn send_stream_ready_to(&self, client: ClientId) -> Result<(), StreamSendError> {
        if self.direction != StreamDirection::ServerToClient {
            log::error!(
                "StreamSender (tag {}): send_stream_ready_to called on a ClientToServer stream",
                self.tag
            );
            return Err(StreamSendError::Closed);
        }
        let bytes = proto_encode(&StreamReady).map_err(|e| {
            log::error!(
                "StreamSender (tag {}): failed to encode StreamReady: {}",
                self.tag,
                e
            );
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
    ///
    /// Only valid for streams registered with [`StreamDirection::ServerToClient`].
    pub fn send_to(&self, client: ClientId, msg: &T) -> Result<(), StreamSendError> {
        if self.direction != StreamDirection::ServerToClient {
            log::error!(
                "StreamSender (tag {}): send_to called on a ClientToServer stream",
                self.tag
            );
            return Err(StreamSendError::Closed);
        }
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
    ///
    /// Only valid for streams registered with [`StreamDirection::ServerToClient`].
    pub fn broadcast(&self, msg: &T) -> Result<(), StreamSendError> {
        if self.direction != StreamDirection::ServerToClient {
            log::error!(
                "StreamSender (tag {}): broadcast called on a ClientToServer stream",
                self.tag
            );
            return Err(StreamSendError::Closed);
        }
        let bytes = protocol::encode(msg).map_err(|e| {
            log::error!("StreamSender (tag {}): encode failed: {}", self.tag, e);
            StreamSendError::Encode
        })?;
        self.send_raw(StreamWriteCmd::Broadcast {
            data: Bytes::from(bytes),
        })
    }

    /// Encode `msg` and send it to the server on this client→server stream.
    ///
    /// Only valid for streams registered with [`StreamDirection::ClientToServer`].
    /// Returns [`Err(StreamSendError::Closed)`] if no client is currently connected.
    pub fn send(&self, msg: &T) -> Result<(), StreamSendError> {
        if self.direction != StreamDirection::ClientToServer {
            log::error!(
                "StreamSender (tag {}): send called on a ServerToClient stream",
                self.tag
            );
            return Err(StreamSendError::Closed);
        }
        let bytes = protocol::encode(msg).map_err(|e| {
            log::error!("StreamSender (tag {}): encode failed: {}", self.tag, e);
            StreamSendError::Encode
        })?;
        self.send_raw_client(Bytes::from(bytes))
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
    /// Per-tag receive buffers for server→client frames, shared with [`StreamReader`] instances.
    per_stream_bufs: HashMap<u8, Arc<Mutex<VecDeque<Bytes>>>>,
    /// Per-tag receive buffers for client→server frames, shared with [`StreamReader`] instances.
    per_client_stream_bufs: HashMap<u8, Arc<Mutex<VecDeque<(ClientId, Bytes)>>>>,
    /// Per-tag shared sender channels for client→server streams.
    /// Each `SharedClientStreamTx` is wired to the live client stream when a client connects
    /// and set to `None` on disconnect.
    shared_client_txs: HashMap<u8, SharedClientStreamTx>,
    /// StreamReady events deferred to the next frame.  When a StreamReady
    /// arrives in the same drain batch as the data frames, game systems have
    /// not yet had a chance to process that data.  By buffering the ready
    /// sentinel here and emitting it on the *next* `drain_client_events` call,
    /// we guarantee at least one full frame of processing before downstream
    /// systems see the ready signal.
    deferred_ready: Vec<u8>,
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            shared_tx: Arc::new(Mutex::new(None)),
            per_stream_bufs: HashMap::new(),
            per_client_stream_bufs: HashMap::new(),
            shared_client_txs: HashMap::new(),
            deferred_ready: Vec::new(),
        }
    }
}

impl StreamRegistry {
    /// Register a stream and return a [`StreamSender<T>`] for writing to it and
    /// a [`StreamReader<T>`] for reading decoded frames on the client side.
    ///
    /// Insert the sender as a Bevy resource so server-side systems can write to the
    /// stream.  Insert the reader as a Bevy resource so client-side systems can
    /// receive decoded messages without polling [`ClientEvent`].
    ///
    /// # Panics
    /// Panics if `def.tag == 0` (reserved for the control stream) or if the
    /// tag has already been registered.
    pub fn register<T: Send + Sync + 'static>(
        &mut self,
        def: StreamDef,
    ) -> (StreamSender<T>, StreamReader<T>) {
        assert_ne!(
            def.tag, 0,
            "stream tag 0 is reserved for the control stream"
        );
        assert!(
            !self.entries.iter().any(|e| e.tag == def.tag),
            "stream tag {} is already registered",
            def.tag
        );
        let tag = def.tag;
        let def_direction = def.direction;
        log::info!(
            "StreamRegistry: registered stream '{}' (tag={}, direction={:?})",
            def.name,
            tag,
            def.direction
        );

        let (server_to_client_buf, client_to_server_buf, client_tx) = match def.direction {
            StreamDirection::ServerToClient => {
                let buf = Arc::new(Mutex::new(VecDeque::<Bytes>::new()));
                self.per_stream_bufs.insert(tag, buf.clone());
                // ClientToServer fields are unused for ServerToClient streams.
                let unused_client_to_server_buf = Arc::new(Mutex::new(VecDeque::new()));
                let unused_client_tx: SharedClientStreamTx = Arc::new(Mutex::new(None));
                (buf, unused_client_to_server_buf, unused_client_tx)
            }
            StreamDirection::ClientToServer => {
                let buf = Arc::new(Mutex::new(VecDeque::<(ClientId, Bytes)>::new()));
                self.per_client_stream_bufs.insert(tag, buf.clone());
                let shared_client_tx: SharedClientStreamTx = Arc::new(Mutex::new(None));
                self.shared_client_txs.insert(tag, shared_client_tx.clone());
                // ServerToClient buf is unused for ClientToServer streams.
                let unused_server_to_client_buf = Arc::new(Mutex::new(VecDeque::new()));
                (unused_server_to_client_buf, buf, shared_client_tx)
            }
        };

        self.entries.push(def);
        let sender = StreamSender {
            tag,
            direction: def_direction,
            shared_tx: self.shared_tx.clone(),
            client_tx,
            _phantom: std::marker::PhantomData,
        };
        let reader = StreamReader {
            buf: server_to_client_buf,
            client_buf: client_to_server_buf,
            _phantom: std::marker::PhantomData,
        };
        (sender, reader)
    }

    /// Route a raw stream frame to the per-tag receive buffer so that the
    /// corresponding [`StreamReader`] can decode it.
    pub(crate) fn route_stream_frame(&self, tag: u8, data: Bytes) {
        if let Some(buf) = self.per_stream_bufs.get(&tag) {
            buf.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(data);
        } else {
            log::error!("route_stream_frame: received frame for unregistered stream tag {tag}");
        }
    }

    /// Route a raw client→server frame to the per-tag server-side receive buffer so that
    /// the corresponding [`StreamReader`] can decode it via [`StreamReader::drain_from_client`].
    pub(crate) fn route_client_stream_frame(&self, from: ClientId, tag: u8, data: Bytes) {
        if let Some(buf) = self.per_client_stream_bufs.get(&tag) {
            buf.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back((from, data));
        } else {
            log::error!(
                "route_client_stream_frame: received frame for unregistered stream tag {tag}"
            );
        }
    }

    /// Number of registered server→client streams.
    /// Sent as `expected_streams` in the `Welcome` message.
    pub fn server_to_client_count(&self) -> u8 {
        let count = self
            .entries
            .iter()
            .filter(|d| d.direction == StreamDirection::ServerToClient)
            .count();
        u8::try_from(count).expect("too many server→client streams registered; maximum is 255")
    }

    /// Called by the server startup path.  Creates a fresh command channel,
    /// wires `shared_tx` to the new sender, and returns the stream definitions
    /// and command receiver to pass to the server task.
    pub(crate) fn prepare_server_start(
        &mut self,
    ) -> (Vec<StreamDef>, mpsc::Receiver<(u8, StreamWriteCmd)>) {
        let (tx, rx) = mpsc::channel(STREAM_CMD_BUFFER_SIZE);
        *self.shared_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let defs = self.entries.clone();
        log::info!(
            "StreamRegistry: server started with {} stream(s)",
            defs.len()
        );
        (defs, rx)
    }

    /// Called when the server stops.  Disconnects stream senders so that
    /// [`StreamSender`] calls made while no server is running are rejected.
    pub(crate) fn on_server_stop(&self) {
        *self.shared_tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
        log::info!("StreamRegistry: server stopped, stream senders disconnected");
    }

    /// Called by the client connect path.  For each registered client→server stream,
    /// creates a fresh byte channel, wires the sender to the `SharedClientStreamTx`,
    /// and returns `(tag, receiver)` pairs for the client task to open QUIC streams.
    pub(crate) fn prepare_client_connect(&mut self) -> Vec<(u8, mpsc::Receiver<Bytes>)> {
        let mut receivers = Vec::new();
        for def in &self.entries {
            if def.direction == StreamDirection::ClientToServer {
                let (tx, rx) = mpsc::channel(STREAM_CMD_BUFFER_SIZE);
                if let Some(shared) = self.shared_client_txs.get(&def.tag) {
                    *shared.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
                }
                receivers.push((def.tag, rx));
            }
        }
        log::info!(
            "StreamRegistry: client connecting with {} client→server stream(s)",
            receivers.len()
        );
        receivers
    }

    /// Called when the client disconnects.  Disconnects client stream senders so that
    /// [`StreamSender::send`] calls made while no client is connected are rejected.
    pub(crate) fn on_client_disconnect(&self) {
        for shared in self.shared_client_txs.values() {
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        if !self.shared_client_txs.is_empty() {
            log::info!("StreamRegistry: client disconnected, client stream senders disconnected");
        }
    }

    /// Buffer a `StreamReady` tag to be emitted next frame instead of
    /// immediately, giving game systems a full frame to process the data.
    pub(crate) fn defer_stream_ready(&mut self, tag: u8) {
        log::debug!(
            "StreamRegistry: deferring StreamReady for tag={} to next frame",
            tag
        );
        self.deferred_ready.push(tag);
    }

    /// Drain all deferred `StreamReady` tags, returning them for emission.
    pub(crate) fn take_deferred_ready(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.deferred_ready)
    }
}

/// Typed resource for receiving decoded stream frames from the server on a specific stream tag.
///
/// Obtain one by calling [`StreamRegistry::register`] and inserting the returned value as a
/// Bevy resource.  Each call to [`StreamReader::drain`] returns an iterator over all messages
/// that arrived since the last drain.
///
/// For streams registered with [`StreamDirection::ClientToServer`], use
/// [`StreamReader::drain_from_client`] on the server side to receive frames alongside the
/// sending client's [`ClientId`].
pub struct StreamReader<T: Send + Sync + 'static> {
    /// Receive buffer for server→client frames (used on client side).
    buf: Arc<Mutex<VecDeque<Bytes>>>,
    /// Receive buffer for client→server frames (used on server side).
    client_buf: Arc<Mutex<VecDeque<(ClientId, Bytes)>>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> bevy::ecs::resource::Resource for StreamReader<T> {}

impl<T: Send + Sync + 'static> StreamReader<T>
where
    for<'de> T: wincode::SchemaRead<'de, wincode::config::DefaultConfig, Dst = T>,
{
    /// Drain all buffered frames, decoding each to `T`.
    /// Frames that fail to decode are logged as errors and skipped.
    pub fn drain(&mut self) -> impl Iterator<Item = T> {
        let frames: Vec<Bytes> = {
            let mut guard = self.buf.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain(..).collect()
        };
        frames.into_iter().filter_map(|b| {
            protocol::decode::<T>(&b)
                .map_err(|e| log::error!("StreamReader: decode error: {e}"))
                .ok()
        })
    }

    /// Drain all buffered client→server frames, decoding each to `(ClientId, T)`.
    ///
    /// Only meaningful for streams registered with [`StreamDirection::ClientToServer`].
    /// Frames that fail to decode are logged as errors and skipped.
    pub fn drain_from_client(&mut self) -> impl Iterator<Item = (ClientId, T)> {
        let frames: Vec<(ClientId, Bytes)> = {
            let mut guard = self.client_buf.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain(..).collect()
        };
        frames.into_iter().filter_map(|(from, b)| {
            protocol::decode::<T>(&b)
                .map_err(|e| log::error!("StreamReader: decode error: {e}"))
                .ok()
                .map(|msg| (from, msg))
        })
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
        app.add_message::<PlayerEvent>();
        app.add_message::<ModuleReadySent>();
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

                // Route client→server stream frames to per-tag StreamReader buffers.
                // These are not emitted as Bevy messages; modules read them via StreamReader.
                if let ServerEvent::ClientStreamFrame { from, tag, data } = &event {
                    registry.route_client_stream_frame(*from, *tag, data.clone());
                    count += 1;
                    continue;
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
    mut registry: ResMut<StreamRegistry>,
) {
    // Emit StreamReady events that were deferred from the previous frame.
    // This guarantees game systems had a full frame to process the stream data
    // before any downstream system sees the ready signal.
    for tag in registry.take_deferred_ready() {
        log::debug!("Emitting deferred StreamReady for tag={}", tag);
        writer.write(ClientEvent::StreamReady { tag });
    }

    let mut count = 0;
    while count < MAX_NET_EVENTS_PER_FRAME {
        match receiver.0.try_recv() {
            Ok(event) => {
                // Remove session and sender resources when disconnected
                if matches!(&event, ClientEvent::Disconnected { .. }) {
                    commands.remove_resource::<Client>();
                    commands.remove_resource::<NetClientSender>();
                    registry.on_client_disconnect();
                }

                // Route stream frames to per-tag StreamReader buffers.
                if let ClientEvent::StreamFrame { tag, data } = &event {
                    registry.route_stream_frame(*tag, data.clone());
                }

                // Defer StreamReady to next frame so game systems can process
                // the stream data before the ready signal is visible.
                if let ClientEvent::StreamReady { tag } = &event {
                    registry.defer_stream_ready(*tag);
                    count += 1;
                    continue;
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
            NetCommand::Connect { addr, name } => {
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

                // Prepare client→server stream channels from the registry.
                let client_stream_rxs = registry.prepare_client_connect();

                let tx = client_event_tx.0.clone();
                let addr = *addr;
                let name = name.clone();
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let token_clone = cancel_token.clone();
                let handle = runtime.spawn(client::run_client(
                    addr,
                    tx,
                    client_msg_rx,
                    token_clone,
                    name,
                    client_stream_rxs,
                ));
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
        app.init_resource::<StreamRegistry>();
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

        let message = ServerMessage::InitialStateDone;

        sender.broadcast(&message);

        let received = rx.try_recv().expect("Should receive command");
        match received {
            ServerCommand::Broadcast { message: recv_msg } => match recv_msg {
                ServerMessage::InitialStateDone => {}
                _ => panic!("Expected InitialStateDone message"),
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
        let (sender, _reader): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
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
        let (sender, _reader): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
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

    #[test]
    fn test_stream_registry_client_to_server_register_and_count() {
        let mut registry = StreamRegistry::default();
        assert_eq!(registry.server_to_client_count(), 0);

        registry.register::<ServerMessage>(StreamDef {
            tag: 4,
            name: "tile-input",
            direction: StreamDirection::ClientToServer,
        });
        // ClientToServer streams do not count toward server_to_client_count
        assert_eq!(registry.server_to_client_count(), 0);

        // But we can also have a ServerToClient stream alongside it
        registry.register::<ServerMessage>(StreamDef {
            tag: 1,
            name: "tiles",
            direction: StreamDirection::ServerToClient,
        });
        assert_eq!(registry.server_to_client_count(), 1);
    }

    #[test]
    fn test_stream_sender_client_closed_when_not_connected() {
        let mut registry = StreamRegistry::default();
        let (sender, _reader): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
            tag: 4,
            name: "tile-input",
            direction: StreamDirection::ClientToServer,
        });
        // No client connected → client_tx is None → send returns Closed
        let result = sender.send(&ServerMessage::InitialStateDone);
        assert_eq!(result, Err(StreamSendError::Closed));
    }

    #[test]
    fn test_stream_sender_client_send_and_disconnect() {
        let mut registry = StreamRegistry::default();
        let (sender, _reader): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
            tag: 4,
            name: "tile-input",
            direction: StreamDirection::ClientToServer,
        });

        // Connect: prepare client channels
        let rxs = registry.prepare_client_connect();
        assert_eq!(rxs.len(), 1);
        assert_eq!(rxs[0].0, 4);

        // After prepare_client_connect the channel is live
        let result = sender.send(&ServerMessage::InitialStateDone);
        assert!(result.is_ok(), "send should succeed while client is connected");

        // After on_client_disconnect the channel closes
        registry.on_client_disconnect();
        let result = sender.send(&ServerMessage::InitialStateDone);
        assert_eq!(result, Err(StreamSendError::Closed));
    }

    #[test]
    fn test_route_client_stream_frame_and_drain() {
        let mut registry = StreamRegistry::default();
        let (_sender, mut reader): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
            tag: 4,
            name: "tile-input",
            direction: StreamDirection::ClientToServer,
        });

        // Simulate receiving a frame from a client
        let client = ClientId(7);
        let msg = ServerMessage::InitialStateDone;
        let encoded = Bytes::from(protocol::encode(&msg).expect("encode"));
        registry.route_client_stream_frame(client, 4, encoded);

        // Drain should yield the decoded message with the sender's ClientId
        let frames: Vec<(ClientId, ServerMessage)> = reader.drain_from_client().collect();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, client);
        assert!(matches!(frames[0].1, ServerMessage::InitialStateDone));
    }

    #[test]
    fn test_route_client_stream_frame_unknown_tag_logs_error() {
        let registry = StreamRegistry::default();
        // Routing to an unregistered tag should not panic (it just logs an error)
        let client = ClientId(1);
        registry.route_client_stream_frame(client, 99, Bytes::from_static(b"data"));
        // No panic = test passes
    }

    #[test]
    fn test_prepare_client_connect_only_client_to_server() {
        let mut registry = StreamRegistry::default();
        registry.register::<ServerMessage>(StreamDef {
            tag: 1,
            name: "tiles",
            direction: StreamDirection::ServerToClient,
        });
        registry.register::<ServerMessage>(StreamDef {
            tag: 4,
            name: "tile-input",
            direction: StreamDirection::ClientToServer,
        });

        // prepare_client_connect should only return receivers for ClientToServer streams
        let rxs = registry.prepare_client_connect();
        assert_eq!(rxs.len(), 1);
        assert_eq!(rxs[0].0, 4);
    }

    #[test]
    fn test_stream_sender_direction_guard_server_methods_on_client_to_server() {
        let mut registry = StreamRegistry::default();
        let (sender, _): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
            tag: 4,
            name: "tile-input",
            direction: StreamDirection::ClientToServer,
        });

        // Server-only methods should return Closed on a ClientToServer stream.
        assert_eq!(
            sender.send_stream_ready_to(ClientId(1)),
            Err(StreamSendError::Closed),
            "send_stream_ready_to on ClientToServer should return Closed"
        );
        assert_eq!(
            sender.send_to(ClientId(1), &ServerMessage::InitialStateDone),
            Err(StreamSendError::Closed),
            "send_to on ClientToServer should return Closed"
        );
        assert_eq!(
            sender.broadcast(&ServerMessage::InitialStateDone),
            Err(StreamSendError::Closed),
            "broadcast on ClientToServer should return Closed"
        );
    }

    #[test]
    fn test_stream_sender_direction_guard_send_on_server_to_client() {
        let mut registry = StreamRegistry::default();
        let (sender, _): (StreamSender<ServerMessage>, _) = registry.register(StreamDef {
            tag: 1,
            name: "tiles",
            direction: StreamDirection::ServerToClient,
        });

        // Client-only method should return Closed on a ServerToClient stream.
        assert_eq!(
            sender.send(&ServerMessage::InitialStateDone),
            Err(StreamSendError::Closed),
            "send on ServerToClient should return Closed"
        );
    }
}
