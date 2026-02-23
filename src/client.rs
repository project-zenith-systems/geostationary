use bevy::prelude::*;
use network::{
    Client, ClientEvent, ClientMessage, NETWORK_UPDATE_INTERVAL, NetClientSender, NetworkSet,
};
use things::{InputDirection, PlayerControlled};

use crate::app_state::AppState;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputSendTimer>();
        app.init_resource::<LastSentDirection>();
        app.add_systems(
            PreUpdate,
            handle_client_events
                .run_if(resource_exists::<Client>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(Update, send_client_input.run_if(resource_exists::<Client>));
    }
}

/// Timer for throttling client input sends.
#[derive(Resource)]
struct InputSendTimer(Timer);

impl Default for InputSendTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            NETWORK_UPDATE_INTERVAL,
            TimerMode::Repeating,
        ))
    }
}

/// Tracks the last input direction sent to avoid redundant messages.
#[derive(Resource, Default)]
struct LastSentDirection(Vec3);

fn handle_client_events(
    mut messages: MessageReader<ClientEvent>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Connected => {
                info!("Connected to server");
                next_state.set(AppState::InGame);
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
                // TODO: count toward the initial-sync barrier once expected_streams > 0.
                debug!("Stream {} ready", tag);
            }
            ClientEvent::ServerMessageReceived(_) => {}
        }
    }
}

/// System that sends client input direction to the server.
/// Reads InputDirection component (written by player module) and sends
/// via NetClientSender. Throttled to NETWORK_UPDATE_RATE to reduce traffic.
fn send_client_input(
    time: Res<Time>,
    mut timer: ResMut<InputSendTimer>,
    client_sender: Option<Res<NetClientSender>>,
    mut last_sent: ResMut<LastSentDirection>,
    query: Query<&InputDirection, With<PlayerControlled>>,
) {
    let Some(sender) = client_sender else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let Ok(input) = query.single() else {
        return;
    };

    let direction = input.0;
    if direction == last_sent.0 {
        return;
    }

    last_sent.0 = direction;
    if let Err(e) = sender.send(&ClientMessage::Input {
        direction: direction.into(),
    }) {
        error!("Failed to send client input: {e}");
    }
}
