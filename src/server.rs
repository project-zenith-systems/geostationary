use std::collections::HashMap;
use std::net::SocketAddr;

use bevy::prelude::*;
use network::{
    ClientId, ClientMessage, Headless, ModuleReadySent, NetCommand, NetServerSender, NetworkSet,
    PlayerEvent, Server, ServerEvent, ServerMessage, StreamRegistry,
};
use souls::ClientInputReceived;

use crate::config::AppConfig;

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientInitSyncState>();
        app.add_systems(
            PreUpdate,
            handle_server_events
                .run_if(resource_exists::<Server>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(
            Update,
            track_module_ready.run_if(resource_exists::<Server>),
        );
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

fn handle_server_events(
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut player: MessageWriter<PlayerEvent>,
    mut input: MessageWriter<ClientInputReceived>,
    mut sync_state: ResMut<ClientInitSyncState>,
    config: Res<AppConfig>,
    headless: Option<Res<Headless>>,
) {
    for event in messages.read() {
        match event {
            ServerEvent::HostingStarted { port } => {
                info!("Hosting started on port {port}");
                if headless.is_none() {
                    // Listen-server: connect to self so the local player joins its own game.
                    let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
                    info!("Connecting to self at {addr}");
                    net_commands.write(NetCommand::Connect {
                        addr,
                        name: config.souls.player_name.clone(),
                    });
                }
            }
            ServerEvent::HostingStopped => {
                // Server resource removal handled by NetworkPlugin
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
            // Register client in the sync tracker before emitting PlayerEvent::Joined so
            // that any ModuleReadySent events for this client are tracked from the start.
            sync_state.ready_counts.insert(*from, 0);
            // Entity catch-up and player spawning are handled by SoulsPlugin on PlayerEvent::Joined.
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
///
/// Because this is driven by explicit module events rather than frame scheduling, it works
/// correctly even when initial data spans multiple frames (e.g., large worlds streamed in
/// chunks).
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

