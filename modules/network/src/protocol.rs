use serde::{Deserialize, Serialize};

/// Unique identifier for a peer in the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub u64);

/// Spatial state of a peer, sent in authoritative updates from host to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerState {
    pub id: PeerId,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
}

/// Messages sent from host to peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostMessage {
    /// Assigns the connecting peer their identity.
    Welcome { peer_id: PeerId },
    /// Notifies that a new peer entered the game.
    PeerJoined { id: PeerId, position: [f32; 3] },
    /// Notifies that a peer disconnected.
    PeerLeft { id: PeerId },
    /// Authoritative spatial state update for all peers.
    StateUpdate { peers: Vec<PeerState> },
}

/// Messages sent from peers to host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMessage {
    /// Input vector from the peer.
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
    fn test_peer_id_roundtrip() {
        let peer_id = PeerId(42);
        let encoded = encode(&peer_id).unwrap();
        let decoded: PeerId = decode(&encoded).unwrap();
        assert_eq!(peer_id, decoded);
    }

    #[test]
    fn test_peer_state_roundtrip() {
        let state = PeerState {
            id: PeerId(123),
            position: [1.0, 2.0, 3.0],
            velocity: [0.5, 0.0, -0.5],
        };
        let encoded = encode(&state).unwrap();
        let decoded: PeerState = decode(&encoded).unwrap();
        assert_eq!(state.id, decoded.id);
        assert_eq!(state.position, decoded.position);
        assert_eq!(state.velocity, decoded.velocity);
    }

    #[test]
    fn test_host_message_welcome_roundtrip() {
        let msg = HostMessage::Welcome {
            peer_id: PeerId(99),
        };
        let encoded = encode(&msg).unwrap();
        let decoded: HostMessage = decode(&encoded).unwrap();
        match decoded {
            HostMessage::Welcome { peer_id } => {
                assert_eq!(peer_id, PeerId(99));
            }
            _ => panic!("Expected Welcome variant"),
        }
    }

    #[test]
    fn test_host_message_peer_joined_roundtrip() {
        let msg = HostMessage::PeerJoined {
            id: PeerId(5),
            position: [10.0, 20.0, 30.0],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: HostMessage = decode(&encoded).unwrap();
        match decoded {
            HostMessage::PeerJoined { id, position } => {
                assert_eq!(id, PeerId(5));
                assert_eq!(position, [10.0, 20.0, 30.0]);
            }
            _ => panic!("Expected PeerJoined variant"),
        }
    }

    #[test]
    fn test_host_message_peer_left_roundtrip() {
        let msg = HostMessage::PeerLeft { id: PeerId(7) };
        let encoded = encode(&msg).unwrap();
        let decoded: HostMessage = decode(&encoded).unwrap();
        match decoded {
            HostMessage::PeerLeft { id } => {
                assert_eq!(id, PeerId(7));
            }
            _ => panic!("Expected PeerLeft variant"),
        }
    }

    #[test]
    fn test_host_message_state_update_roundtrip() {
        let msg = HostMessage::StateUpdate {
            peers: vec![
                PeerState {
                    id: PeerId(1),
                    position: [1.0, 2.0, 3.0],
                    velocity: [0.1, 0.2, 0.3],
                },
                PeerState {
                    id: PeerId(2),
                    position: [4.0, 5.0, 6.0],
                    velocity: [0.4, 0.5, 0.6],
                },
            ],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: HostMessage = decode(&encoded).unwrap();
        match decoded {
            HostMessage::StateUpdate { peers } => {
                assert_eq!(peers.len(), 2);
                assert_eq!(peers[0].id, PeerId(1));
                assert_eq!(peers[0].position, [1.0, 2.0, 3.0]);
                assert_eq!(peers[0].velocity, [0.1, 0.2, 0.3]);
                assert_eq!(peers[1].id, PeerId(2));
                assert_eq!(peers[1].position, [4.0, 5.0, 6.0]);
                assert_eq!(peers[1].velocity, [0.4, 0.5, 0.6]);
            }
            _ => panic!("Expected StateUpdate variant"),
        }
    }

    #[test]
    fn test_peer_message_input_roundtrip() {
        let msg = PeerMessage::Input {
            direction: [1.0, 0.0, -1.0],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: PeerMessage = decode(&encoded).unwrap();
        match decoded {
            PeerMessage::Input { direction } => {
                assert_eq!(direction, [1.0, 0.0, -1.0]);
            }
        }
    }

    #[test]
    fn test_empty_state_update_roundtrip() {
        let msg = HostMessage::StateUpdate { peers: vec![] };
        let encoded = encode(&msg).unwrap();
        let decoded: HostMessage = decode(&encoded).unwrap();
        match decoded {
            HostMessage::StateUpdate { peers } => {
                assert_eq!(peers.len(), 0);
            }
            _ => panic!("Expected StateUpdate variant"),
        }
    }

    #[test]
    fn test_zero_direction_input() {
        let msg = PeerMessage::Input {
            direction: [0.0, 0.0, 0.0],
        };
        let encoded = encode(&msg).unwrap();
        let decoded: PeerMessage = decode(&encoded).unwrap();
        match decoded {
            PeerMessage::Input { direction } => {
                assert_eq!(direction, [0.0, 0.0, 0.0]);
            }
        }
    }
}
