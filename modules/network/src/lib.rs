use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::{log, prelude::*};
use tokio::sync::mpsc;

mod client;
mod config;
mod runtime;
mod server;

use runtime::{NetEventReceiver, NetEventSender, NetworkRuntime};

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
}

/// Events emitted by the network layer back to game code.
#[derive(Message, Clone, Debug)]
pub enum NetEvent {
    HostingStarted { port: u16 },
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

    if !CAP_WARNING_LOGGED.swap(true, Ordering::Relaxed) {
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
) {
    for command in commands_reader.read() {
        match command {
            NetCommand::Host { port } => {
                let tx = event_tx.0.clone();
                runtime.spawn(server::run_server(*port, tx));
            }
            NetCommand::Connect { addr } => {
                let tx = event_tx.0.clone();
                let addr = *addr;
                runtime.spawn(client::run_client(addr, tx));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_events_per_frame_cap() {
        // Create a test app
        let mut app = App::new();
        
        // Set up the channel manually
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        app.insert_resource(NetEventReceiver(event_rx));
        app.add_message::<NetEvent>();
        app.add_systems(Update, drain_net_events);
        
        // Send more events than the cap
        for i in 0..(MAX_NET_EVENTS_PER_FRAME + 50) {
            event_tx.send(NetEvent::Error(format!("Test event {}", i)))
                .expect("Failed to send event");
        }
        
        // Run one frame
        app.update();
        
        // Read all the events that were processed
        let mut count = 0;
        let mut reader = app.world_mut().get_resource_mut::<MessageReader<NetEvent>>()
            .expect("MessageReader should exist");
        for _event in reader.read() {
            count += 1;
        }
        
        // Should have processed exactly MAX_NET_EVENTS_PER_FRAME events
        assert_eq!(count, MAX_NET_EVENTS_PER_FRAME, 
            "Should process exactly MAX_NET_EVENTS_PER_FRAME events per frame");
        
        // Run another frame to process remaining events
        app.update();
        
        let mut count2 = 0;
        let mut reader = app.world_mut().get_resource_mut::<MessageReader<NetEvent>>()
            .expect("MessageReader should exist");
        for _event in reader.read() {
            count2 += 1;
        }
        
        // Should have processed the remaining 50 events
        assert_eq!(count2, 50, 
            "Should process remaining events in next frame");
    }
}
