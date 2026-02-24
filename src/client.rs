use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{Client, ClientEvent, NetworkSet, NetId, ServerMessage};
use things::NetIdIndex;

use crate::app_state::AppState;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_net_id_added);
        app.init_resource::<PendingSync>();
        app.add_systems(OnExit(AppState::InGame), clear_net_id_index);
        app.add_systems(
            OnEnter(AppState::InGame),
            setup_client_scene.run_if(resource_exists::<Client>),
        );
        app.add_systems(
            PreUpdate,
            handle_client_events
                .run_if(resource_exists::<Client>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
    }
}

/// Tracks the initial-sync barrier for the client.
///
/// Initial sync is complete when:
/// - [`PendingSync::initial_state_done`] is `true` (server sent [`ServerMessage::InitialStateDone`])
/// - [`PendingSync::streams_ready`] equals [`PendingSync::expected_streams`]
///   (all module streams sent their [`network::StreamReady`] sentinel)
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

/// System that sets up client-only scene elements when entering [`AppState::InGame`].
///
/// Spawns directional lighting for clients (and listen-servers, which also hold a [`Client`]
/// resource).  Dedicated headless servers have no [`Client`] resource and skip this system.
fn setup_client_scene(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::new(4.0, 0.0, 4.0), Vec3::Y),
        DespawnOnExit(AppState::InGame),
    ));
}

/// Inserts [`DespawnOnExit`] on every replicated entity when it receives a [`NetId`].
///
/// The `things` module owns lifecycle management but cannot reference [`AppState`],
/// so state-scoped cleanup is wired here instead.
fn on_net_id_added(trigger: On<Add, NetId>, mut commands: Commands) {
    commands
        .entity(trigger.event_target())
        .insert(DespawnOnExit(AppState::InGame));
}

/// Clears the [`NetIdIndex`] when leaving [`AppState::InGame`].
///
/// Entities are already despawned via [`DespawnOnExit`]; this removes the now-stale
/// mappings so a subsequent connection starts with a clean index.
fn clear_net_id_index(mut net_id_index: ResMut<NetIdIndex>) {
    net_id_index.0.clear();
}

fn handle_client_events(
    mut messages: MessageReader<ClientEvent>,
    mut next_state: ResMut<NextState<AppState>>,
    state: Res<State<AppState>>,
    mut client: ResMut<Client>,
    mut sync: ResMut<PendingSync>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Connected => {
                info!("Connected to server");
                // Reset sync state and wait for Welcome + module StreamReady sentinels
                // + InitialStateDone before entering InGame.
                sync.reset();
            }
            ClientEvent::Disconnected { reason } => {
                info!("Disconnected: {reason}");
                next_state.set(AppState::MainMenu);
            }
            ClientEvent::Error(msg) => {
                error!("Network error: {msg}");
                next_state.set(AppState::MainMenu);
            }
            ClientEvent::StreamFrame { tag, data: _ } => {
                debug!("Stream frame received on tag={}", tag);
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
                try_enter_in_game(&sync, &state, &mut next_state);
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
                    try_enter_in_game(&sync, &state, &mut next_state);
                }
                ServerMessage::InitialStateDone => {
                    info!("Server initial state done");
                    sync.initial_state_done = true;
                    try_enter_in_game(&sync, &state, &mut next_state);
                }
            },
        }
    }
}

/// Transitions to [`AppState::InGame`] if the initial-sync barrier is complete and the app
/// is not already in that state.  Guards against redundant state changes that would trigger
/// a spurious [`OnExit`]/[`OnEnter`] cycle.
fn try_enter_in_game(
    sync: &PendingSync,
    state: &State<AppState>,
    next_state: &mut NextState<AppState>,
) {
    if sync.is_complete() && *state.get() != AppState::InGame {
        info!("Initial sync complete, entering InGame");
        next_state.set(AppState::InGame);
    }
}
