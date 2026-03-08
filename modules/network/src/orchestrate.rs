//! Bevy systems that consume [`ClientEvent`] / [`ServerEvent`] messages and
//! orchestrate state transitions and per-client sync barriers.
//!
//! These systems are registered by [`NetworkPlugin`] and are generic over the
//! application state type `S` so the network module remains game-agnostic.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::state::state::FreelyMutableState;

use crate::{
    Client, ClientEvent, ClientId, ClientInputReceived, ClientMessage, ModuleReadySent, NetCommand,
    NetServerSender, NetworkReceive, PlayerEvent, Server, ServerEvent, ServerMessage,
    StreamRegistry,
};

/// Tracks the initial-sync barrier for the client.
///
/// Initial sync is complete when:
/// - [`PendingSync::initial_state_done`] is `true` (server sent [`ServerMessage::InitialStateDone`])
/// - [`PendingSync::streams_ready`] equals [`PendingSync::expected_streams`]
///   (all module streams sent their [`StreamReady`] sentinel)
#[derive(Resource, Default)]
struct PendingSync {
    /// Number of server→client module streams to expect, set from [`ServerMessage::Welcome`].
    expected_streams: u8,
    /// Number of [`ClientEvent::StreamReady`] sentinels received so far.
    streams_ready: u8,
    /// Whether [`ServerMessage::InitialStateDone`] has been received.
    initial_state_done: bool,
}

impl PendingSync {
    /// Returns `true` when all stream sentinels and the `InitialStateDone` message have arrived.
    fn is_complete(&self) -> bool {
        self.initial_state_done && self.streams_ready >= self.expected_streams
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Tracks per-client initial-sync progress on the server.
///
/// Each module stream emits a [`ModuleReadySent`] event after writing its initial burst and
/// [`StreamReady`] sentinel for a connecting client.  When the count of ready modules for a
/// client reaches the total number of registered server→client streams, the server sends
/// [`ServerMessage::InitialStateDone`] on the control stream.
///
/// This is event-driven and correct across multi-frame initial sends — no scheduling order
/// is assumed.  Entries are cleaned up on [`PlayerEvent::Left`].
#[derive(Resource, Default)]
struct ClientInitSyncState {
    /// Maps ClientId → number of module streams that have reported ready for that client.
    ready_counts: HashMap<ClientId, u8>,
}

/// Registers the orchestration systems on `app` that are generic over the
/// application state `S`.
pub(crate) fn register_orchestrate_systems<S: FreelyMutableState + Copy>(
    app: &mut App,
    loading: S,
    in_game: S,
    disconnected: S,
) {
    app.init_resource::<PendingSync>();
    app.init_resource::<ClientInitSyncState>();
    app.add_systems(
        NetworkReceive,
        handle_client_events::<S>.run_if(resource_exists::<Client>),
    );
    app.add_systems(
        NetworkReceive,
        handle_server_events.run_if(resource_exists::<Server>),
    );
    app.add_systems(Update, track_module_ready.run_if(resource_exists::<Server>));

    // Store the state values so the generic system can read them.
    app.insert_resource(OrchestrationStates {
        loading,
        in_game,
        disconnected,
    });
}

/// Resource storing the state values used for transitions.
#[derive(Resource)]
pub(crate) struct OrchestrationStates<S: FreelyMutableState> {
    loading: S,
    in_game: S,
    disconnected: S,
}

impl<S: FreelyMutableState + Copy> OrchestrationStates<S> {
    /// Transition to the `loading` state unless we're already in `loading` or `in_game`.
    ///
    /// When `headless` is true (dedicated server), skips straight to `in_game` — there is
    /// no client-side sync barrier to wait for.
    pub(crate) fn transition_to_loading(
        &self,
        state: &State<S>,
        next_state: &mut NextState<S>,
        headless: bool,
    ) {
        if headless {
            // Dedicated server: skip the sync barrier entirely and go straight
            // to in_game.  Handles both the cold-start case (already in loading
            // after OnEnter ran load_map) and the from-menu case.
            if *state.get() != self.in_game {
                next_state.set(self.in_game);
            }
        } else if *state.get() != self.loading && *state.get() != self.in_game {
            next_state.set(self.loading);
        }
    }
}

fn handle_client_events<S: FreelyMutableState + Copy>(
    mut messages: MessageReader<ClientEvent>,
    mut next_state: ResMut<NextState<S>>,
    state: Res<State<S>>,
    mut client: ResMut<Client>,
    mut sync: ResMut<PendingSync>,
    mut net_commands: MessageWriter<NetCommand>,
    states: Res<OrchestrationStates<S>>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Connected => {
                info!("Connected to server");
                sync.reset();
            }
            ClientEvent::Disconnected { reason } => {
                info!("Disconnected: {reason}");
                next_state.set(states.disconnected);
            }
            ClientEvent::Error(msg) => {
                error!("Network error: {msg}");
                next_state.set(states.disconnected);
            }
            ClientEvent::StreamFrame { tag: _, data: _ } => {
                // Intentionally unhandled here; stream frames are processed in the respective module systems.
            }
            ClientEvent::StreamReady { tag } => {
                sync.streams_ready += 1;
                if sync.expected_streams > 0 && sync.streams_ready > sync.expected_streams {
                    warn!(
                        "Received more StreamReady sentinels than expected \
                         (tag={}, got {}, expected {}); server/client stream count mismatch",
                        tag, sync.streams_ready, sync.expected_streams
                    );
                }
                debug!(
                    "Stream {} ready ({}/{})",
                    tag, sync.streams_ready, sync.expected_streams
                );
                try_enter_in_game(&sync, &state, &mut next_state, &states);
            }
            ClientEvent::ServerMessageReceived(message) => match message {
                ServerMessage::Welcome {
                    client_id,
                    expected_streams,
                } => {
                    info!(
                        "Received Welcome, local ClientId assigned: {}, expecting {} module stream(s)",
                        client_id.0, expected_streams
                    );
                    client.local_id = Some(*client_id);
                    sync.expected_streams = *expected_streams;
                    try_enter_in_game(&sync, &state, &mut next_state, &states);
                }
                ServerMessage::InitialStateDone => {
                    info!("Server initial state done");
                    sync.initial_state_done = true;
                    try_enter_in_game(&sync, &state, &mut next_state, &states);
                }
                ServerMessage::Shutdown => {
                    info!("Server is shutting down");
                    net_commands.write(NetCommand::Disconnect);
                    next_state.set(states.disconnected);
                }
            },
        }
    }
}

/// Transitions from `Loading` to `InGame` once the initial-sync barrier is complete.
/// Only fires when the app is in the `loading` state — prevents spurious transitions
/// from other states (e.g. `Editor`).
fn try_enter_in_game<S: FreelyMutableState + Copy>(
    sync: &PendingSync,
    state: &State<S>,
    next_state: &mut NextState<S>,
    states: &OrchestrationStates<S>,
) {
    if sync.is_complete() && *state.get() == states.loading {
        info!("Initial sync complete, entering InGame");
        next_state.set(states.in_game);
    }
}

fn handle_server_events(
    mut messages: MessageReader<ServerEvent>,
    mut player: MessageWriter<PlayerEvent>,
    mut input: MessageWriter<ClientInputReceived>,
    mut sync_state: ResMut<ClientInitSyncState>,
) {
    for event in messages.read() {
        match event {
            ServerEvent::HostingStarted { port } => {
                info!("Hosting started on port {port}");
            }
            ServerEvent::HostingStopped => {
                // Server resource removal handled by drain_server_events in lib.rs
            }
            ServerEvent::Error(msg) => {
                error!("Network error: {msg}");
            }
            ServerEvent::ClientConnected { id, addr, name } => {
                info!("Client {} ('{}') connected from {addr}", id.0, name);
            }
            ServerEvent::ClientDisconnected { id } => {
                info!("Client {} disconnected", id.0);
                sync_state.ready_counts.remove(id);
                player.write(PlayerEvent::Left { id: *id });
            }
            ServerEvent::ClientMessageReceived { from, message } => {
                handle_client_message(from, message, &mut player, &mut input, &mut sync_state);
            }
            ServerEvent::ClientStreamFrame { .. } => {
                // Routed to per-tag StreamReader buffers by drain_server_events;
                // not processed here.
            }
        }
    }
}

fn handle_client_message(
    from: &ClientId,
    message: &ClientMessage,
    player: &mut MessageWriter<PlayerEvent>,
    input: &mut MessageWriter<ClientInputReceived>,
    sync_state: &mut ClientInitSyncState,
) {
    match message {
        ClientMessage::Hello { name } => {
            info!(
                "Received client hello from ClientId({}), name: {:?}",
                from.0, name
            );
            sync_state.ready_counts.insert(*from, 0);
            player.write(PlayerEvent::Joined {
                id: *from,
                name: name.clone(),
            });
        }
        ClientMessage::Input { direction } => {
            input.write(ClientInputReceived {
                from: *from,
                direction: *direction,
            });
        }
    }
}

/// Server-side system: collects [`ModuleReadySent`] events from module stream handlers and
/// sends [`ServerMessage::InitialStateDone`] to a client once every registered server→client
/// stream has reported its initial burst complete.
fn track_module_ready(
    mut ready_events: MessageReader<ModuleReadySent>,
    mut sync_state: ResMut<ClientInitSyncState>,
    registry: Res<StreamRegistry>,
    sender: Option<Res<NetServerSender>>,
) {
    let Some(sender) = sender else {
        return;
    };
    let expected = registry.server_to_client_count();

    for ModuleReadySent { client } in ready_events.read() {
        let count = sync_state.ready_counts.entry(*client).or_insert(0);
        *count += 1;
        if *count >= expected {
            sender.send_to(*client, &ServerMessage::InitialStateDone);
            info!(
                "All {} module stream(s) ready for ClientId({}); sent InitialStateDone",
                expected, client.0
            );
            sync_state.ready_counts.remove(client);
        }
    }
}
