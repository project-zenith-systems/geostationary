use bevy::prelude::*;
use network::{Client, ClientMessage, NETWORK_UPDATE_INTERVAL, NetClientSender};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputSendTimer>();
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

/// System that sends client input to the server.
/// Reads keyboard and sends ClientMessage::Input via NetClientSender.
/// Throttled to NETWORK_UPDATE_RATE to reduce network traffic.
fn send_client_input(
    time: Res<Time>,
    mut timer: ResMut<InputSendTimer>,
    keyboard: Res<ButtonInput<KeyCode>>,
    client_sender: Option<Res<NetClientSender>>,
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

    if direction != Vec3::ZERO {
        if !sender.send(&ClientMessage::Input {
            direction: direction.into(),
        }) {
            error!("Failed to send client input: send buffer full or closed");
        }
    }
}
