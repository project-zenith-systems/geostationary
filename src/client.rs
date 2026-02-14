use bevy::prelude::*;
use network::{Client, ClientMessage, NETWORK_UPDATE_INTERVAL, NetClientSender};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputSendTimer>();
        app.init_resource::<LastSentDirection>();
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

/// System that sends client input to the server.
/// Reads keyboard and sends ClientMessage::Input via NetClientSender.
/// Throttled to NETWORK_UPDATE_RATE to reduce network traffic.
fn send_client_input(
    time: Res<Time>,
    mut timer: ResMut<InputSendTimer>,
    keyboard: Res<ButtonInput<KeyCode>>,
    client_sender: Option<Res<NetClientSender>>,
    mut last_sent: ResMut<LastSentDirection>,
) {
    let Some(sender) = client_sender else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let mut direction = Vec3::ZERO;

    if keyboard.pressed(KeyCode::KeyW) {
        direction.z -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction.z += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }

    if direction == last_sent.0 {
        return;
    }

    last_sent.0 = direction;
    if !sender.send(&ClientMessage::Input {
        direction: direction.into(),
    }) {
        error!("Failed to send client input: send buffer full or closed");
    }
}
