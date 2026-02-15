use bevy::ecs::component::Component;
use serde::{Deserialize, Serialize};
use wincode::config::DefaultConfig;
use wincode::{SchemaRead, SchemaWrite};

/// Unique identifier for a client in the network.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, SchemaRead, SchemaWrite,
)]
pub struct ClientId(pub u64);

/// Unique identifier for a replicated entity, server-assigned.
#[derive(
    Component,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    SchemaRead,
    SchemaWrite,
)]
pub struct NetId(pub u64);

/// Spatial state of a replicated entity, sent in authoritative updates.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaRead, SchemaWrite)]
pub struct EntityState {
    pub net_id: NetId,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
}

/// Messages sent from Server to clients.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaRead, SchemaWrite)]
pub enum ServerMessage {
    /// Assigns the connecting client their identity.
    Welcome { client_id: ClientId },
    /// A replicated entity was spawned. `kind` is an opaque tag for future use.
    EntitySpawned {
        net_id: NetId,
        kind: u16,
        position: [f32; 3],
        velocity: [f32; 3],
        /// If set, the receiving client with this ID should take control of this entity.
        owner: Option<ClientId>,
    },
    /// A replicated entity was despawned.
    EntityDespawned { net_id: NetId },
    /// Authoritative spatial state update for all replicated entities.
    StateUpdate { entities: Vec<EntityState> },
}

/// Messages sent from clients to server.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaRead, SchemaWrite)]
pub enum ClientMessage {
    /// Initial handshake sent immediately after stream open.
    Hello,
    /// Input vector from the client.
    Input { direction: [f32; 3] },
}

/// Encodes a message using wincode.
pub(crate) fn encode<T>(msg: &T) -> wincode::WriteResult<Vec<u8>>
where
    T: SchemaWrite<DefaultConfig, Src = T> + ?Sized,
{
    wincode::serialize(msg)
}

/// Decodes a message using wincode.
pub(crate) fn decode<T>(bytes: &[u8]) -> wincode::ReadResult<T>
where
    for<'de> T: SchemaRead<'de, DefaultConfig, Dst = T>,
{
    wincode::deserialize(bytes)
}
