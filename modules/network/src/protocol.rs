use bevy::ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Unique identifier for a client in the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientId(pub u64);

/// Unique identifier for a replicated entity, server-assigned.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetId(pub u64);

/// Spatial state of a replicated entity, sent in authoritative updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityState {
    pub net_id: NetId,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
}

/// Messages sent from Server to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Assigns the connecting client their identity.
    Welcome { client_id: ClientId },
    /// A replicated entity was spawned. `kind` is an opaque tag for future use.
    EntitySpawned {
        net_id: NetId,
        kind: u16,
        position: [f32; 3],
        velocity: [f32; 3],
    },
    /// A replicated entity was despawned.
    EntityDespawned { net_id: NetId },
    /// Authoritative spatial state update for all replicated entities.
    StateUpdate { entities: Vec<EntityState> },
    /// Tells a client which entity they control (camera, input).
    AssignControl { net_id: NetId },
}

/// Messages sent from clients to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Input vector from the client.
    Input { direction: [f32; 3] },
}

/// Encodes a message using bincode.
pub(crate) fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, bincode::Error> {
    bincode::serialize(msg)
}

/// Decodes a message using bincode.
pub(crate) fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, bincode::Error> {
    bincode::deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_id_roundtrip() {
        let client_id = ClientId(42);
        let encoded = encode(&client_id).unwrap();
        let decoded: ClientId = decode(&encoded).unwrap();
        assert_eq!(client_id, decoded);
    }

    #[test]
    fn test_net_id_roundtrip() {
        let net_id = NetId(99);
        let encoded = encode(&net_id).unwrap();
        let decoded: NetId = decode(&encoded).unwrap();
        assert_eq!(net_id, decoded);
    }

    #[test]
    fn test_entity_state_roundtrip() {
        let state = EntityState {
            net_id: NetId(123),
            position: [1.0, 2.0, 3.0],
            velocity: [0.5, 0.0, -0.5],
        };
        let encoded = encode(&state).unwrap();
        let decoded: EntityState = decode(&encoded).unwrap();
        assert_eq!(state.net_id, decoded.net_id);
        assert_eq!(state.position, decoded.position);
        assert_eq!(state.velocity, decoded.velocity);
    }

    #[test]
    fn test_server_message_welcome_roundtrip() {
        let msg = ServerMessage::Welcome {
            client_id: ClientId(99),
        };
        let encoded = encode(&msg).unwrap();
        let decoded: ServerMessage = decode(&encoded).unwrap();
        match decoded {
            ServerMessage::Welcome { client_id } => {
                assert_eq!(client_id, ClientId(99));
            }
            _ => panic!("Expected Welcome variant"),
        }
    }

    #[test]
    fn test_server_message_entity_spawned_roundtrip() {
        let msg = ServerMessage::EntitySpawned {
            net_id: NetId(5),
            kind: 1,
            position: [10.0, 20.0, 30.0],
            velocity: [0.1, 0.2, 0.3],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: ServerMessage = decode(&encoded).unwrap();
        match decoded {
            ServerMessage::EntitySpawned {
                net_id,
                kind,
                position,
                velocity,
            } => {
                assert_eq!(net_id, NetId(5));
                assert_eq!(kind, 1);
                assert_eq!(position, [10.0, 20.0, 30.0]);
                assert_eq!(velocity, [0.1, 0.2, 0.3]);
            }
            _ => panic!("Expected EntitySpawned variant"),
        }
    }

    #[test]
    fn test_server_message_entity_despawned_roundtrip() {
        let msg = ServerMessage::EntityDespawned { net_id: NetId(7) };
        let encoded = encode(&msg).unwrap();
        let decoded: ServerMessage = decode(&encoded).unwrap();
        match decoded {
            ServerMessage::EntityDespawned { net_id } => {
                assert_eq!(net_id, NetId(7));
            }
            _ => panic!("Expected EntityDespawned variant"),
        }
    }

    #[test]
    fn test_server_message_state_update_roundtrip() {
        let msg = ServerMessage::StateUpdate {
            entities: vec![
                EntityState {
                    net_id: NetId(1),
                    position: [1.0, 2.0, 3.0],
                    velocity: [0.1, 0.2, 0.3],
                },
                EntityState {
                    net_id: NetId(2),
                    position: [4.0, 5.0, 6.0],
                    velocity: [0.4, 0.5, 0.6],
                },
            ],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: ServerMessage = decode(&encoded).unwrap();
        match decoded {
            ServerMessage::StateUpdate { entities } => {
                assert_eq!(entities.len(), 2);
                assert_eq!(entities[0].net_id, NetId(1));
                assert_eq!(entities[0].position, [1.0, 2.0, 3.0]);
                assert_eq!(entities[0].velocity, [0.1, 0.2, 0.3]);
                assert_eq!(entities[1].net_id, NetId(2));
                assert_eq!(entities[1].position, [4.0, 5.0, 6.0]);
                assert_eq!(entities[1].velocity, [0.4, 0.5, 0.6]);
            }
            _ => panic!("Expected StateUpdate variant"),
        }
    }

    #[test]
    fn test_server_message_assign_control_roundtrip() {
        let msg = ServerMessage::AssignControl { net_id: NetId(42) };
        let encoded = encode(&msg).unwrap();
        let decoded: ServerMessage = decode(&encoded).unwrap();
        match decoded {
            ServerMessage::AssignControl { net_id } => {
                assert_eq!(net_id, NetId(42));
            }
            _ => panic!("Expected AssignControl variant"),
        }
    }

    #[test]
    fn test_client_message_input_roundtrip() {
        let msg = ClientMessage::Input {
            direction: [1.0, 0.0, -1.0],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: ClientMessage = decode(&encoded).unwrap();
        match decoded {
            ClientMessage::Input { direction } => {
                assert_eq!(direction, [1.0, 0.0, -1.0]);
            }
        }
    }

    #[test]
    fn test_empty_state_update_roundtrip() {
        let msg = ServerMessage::StateUpdate { entities: vec![] };
        let encoded = encode(&msg).unwrap();
        let decoded: ServerMessage = decode(&encoded).unwrap();
        match decoded {
            ServerMessage::StateUpdate { entities } => {
                assert_eq!(entities.len(), 0);
            }
            _ => panic!("Expected StateUpdate variant"),
        }
    }

    #[test]
    fn test_zero_direction_input() {
        let msg = ClientMessage::Input {
            direction: [0.0, 0.0, 0.0],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: ClientMessage = decode(&encoded).unwrap();
        match decoded {
            ClientMessage::Input { direction } => {
                assert_eq!(direction, [0.0, 0.0, 0.0]);
            }
        }
    }
}
