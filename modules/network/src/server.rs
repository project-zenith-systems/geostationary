use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::log;
use bytes::Bytes;
use futures_util::stream::StreamExt;
use futures_util::sink::SinkExt;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

use crate::{NetEvent, PeerId};
use crate::config;
use crate::protocol::{decode, encode, PeerMessage};
use crate::runtime::ServerCommand;

/// Bounded channel buffer size per peer to prevent memory exhaustion from slow peers.
/// Allows brief bursts while providing backpressure.
const PER_PEER_BUFFER_SIZE: usize = 100;

pub(crate) async fn run_server(
    port: u16,
    event_tx: mpsc::UnboundedSender<NetEvent>,
    server_cmd_rx: mpsc::UnboundedReceiver<ServerCommand>,
    cancel_token: CancellationToken,
) {
    if let Err(e) = run_server_inner(port, &event_tx, server_cmd_rx, cancel_token).await {
        let _ = event_tx.send(NetEvent::Error(format!("Server error: {e}")));
        let _ = event_tx.send(NetEvent::HostingStopped);
    }
}

async fn run_server_inner(
    port: u16,
    event_tx: &mpsc::UnboundedSender<NetEvent>,
    mut server_cmd_rx: mpsc::UnboundedReceiver<ServerCommand>,
    cancel_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    let server_config = config::build_server_config()?;
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    log::info!("Server listening on {addr}");
    let _ = event_tx.send(NetEvent::HostingStarted { port });

    // Shared state for peer ID assignment and per-peer write channels
    let next_peer_id = Arc::new(AtomicU64::new(1));
    let peer_senders: Arc<tokio::sync::Mutex<HashMap<PeerId, mpsc::Sender<Bytes>>>> = 
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                log::info!("Server cancellation requested");
                break;
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    log::info!("Server endpoint closed");
                    break;
                };

                let event_tx = event_tx.clone();
                let cancel_token_clone = cancel_token.clone();
                let next_peer_id = next_peer_id.clone();
                let peer_senders = peer_senders.clone();
                
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            let addr = connection.remote_address();
                            
                            // Assign incrementing PeerId
                            let peer_id = PeerId(next_peer_id.fetch_add(1, Ordering::SeqCst));
                            log::info!("Client connected from {} with PeerId {}", addr, peer_id.0);
                            
                            let _ = event_tx.send(NetEvent::PeerConnected {
                                id: peer_id,
                                addr,
                            });

                            // Open bi-directional stream (with cancellation support)
                            let accept_result = tokio::select! {
                                result = connection.accept_bi() => result,
                                _ = cancel_token_clone.cancelled() => {
                                    log::info!("Server shutdown while waiting for bi-directional stream from peer {}", peer_id.0);
                                    connection.close(0u32.into(), b"server shutdown");
                                    let _ = event_tx.send(NetEvent::PeerDisconnected { id: peer_id });
                                    return;
                                }
                                _ = connection.closed() => {
                                    log::info!("Connection closed before bi-directional stream opened for peer {}", peer_id.0);
                                    let _ = event_tx.send(NetEvent::PeerDisconnected { id: peer_id });
                                    return;
                                }
                            };

                            match accept_result {
                                Ok((send_stream, recv_stream)) => {
                                    // Wrap streams with LengthDelimitedCodec
                                    let mut framed_read = FramedRead::new(recv_stream, LengthDelimitedCodec::new());
                                    let mut framed_write = FramedWrite::new(send_stream, LengthDelimitedCodec::new());

                                    // Create bounded channel for this peer's write loop
                                    let (write_tx, mut write_rx) = mpsc::channel::<Bytes>(PER_PEER_BUFFER_SIZE);
                                    
                                    // Register peer sender
                                    {
                                        let mut senders = peer_senders.lock().await;
                                        senders.insert(peer_id, write_tx);
                                    }

                                    let event_tx_read = event_tx.clone();
                                    let cancel_token_read = cancel_token_clone.clone();
                                    let cancel_token_write = cancel_token_clone.clone();
                                    let peer_senders_cleanup = peer_senders.clone();

                                    // Create per-peer cancellation token to coordinate shutdown
                                    let peer_cancel = CancellationToken::new();

                                    // Spawn per-peer read loop
                                    let peer_cancel_read = peer_cancel.clone();
                                    let mut read_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::select! {
                                                _ = cancel_token_read.cancelled() => {
                                                    log::debug!("Read loop cancelled for peer {} (server shutdown)", peer_id.0);
                                                    break;
                                                }
                                                _ = peer_cancel_read.cancelled() => {
                                                    log::debug!("Read loop cancelled for peer {} (peer shutdown)", peer_id.0);
                                                    break;
                                                }
                                                frame = framed_read.next() => {
                                                    match frame {
                                                        Some(Ok(bytes)) => {
                                                            // Decode PeerMessage
                                                            match decode::<PeerMessage>(&bytes) {
                                                                Ok(message) => {
                                                                    let _ = event_tx_read.send(NetEvent::PeerMessageReceived {
                                                                        from: peer_id,
                                                                        message,
                                                                    });
                                                                }
                                                                Err(e) => {
                                                                    log::warn!("Failed to decode message from peer {}: {}", peer_id.0, e);
                                                                }
                                                            }
                                                        }
                                                        Some(Err(e)) => {
                                                            log::warn!("Stream error from peer {}: {}", peer_id.0, e);
                                                            break;
                                                        }
                                                        None => {
                                                            log::info!("Peer {} disconnected (read stream closed)", peer_id.0);
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });

                                    // Spawn per-peer write loop
                                    let peer_cancel_write = peer_cancel.clone();
                                    let mut write_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::select! {
                                                _ = cancel_token_write.cancelled() => {
                                                    log::debug!("Write loop cancelled for peer {} (server shutdown)", peer_id.0);
                                                    break;
                                                }
                                                _ = peer_cancel_write.cancelled() => {
                                                    log::debug!("Write loop cancelled for peer {} (peer shutdown)", peer_id.0);
                                                    break;
                                                }
                                                bytes = write_rx.recv() => {
                                                    match bytes {
                                                        Some(bytes) => {
                                                            if let Err(e) = framed_write.send(bytes).await {
                                                                log::warn!("Failed to send to peer {} (stream error): {}", peer_id.0, e);
                                                                break;
                                                            }
                                                        }
                                                        None => {
                                                            log::debug!("Write channel closed for peer {}", peer_id.0);
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });

                                    // Wait for any task to complete or connection to close, then cancel peer tasks
                                    tokio::select! {
                                        _ = &mut read_handle => {
                                            log::debug!("Read task completed for peer {}", peer_id.0);
                                        }
                                        _ = &mut write_handle => {
                                            log::debug!("Write task completed for peer {}", peer_id.0);
                                        }
                                        _ = connection.closed() => {
                                            log::info!("Connection closed for peer {}", peer_id.0);
                                        }
                                    }

                                    // Cancel peer tasks to ensure both stop cleanly
                                    peer_cancel.cancel();

                                    // Wait for both tasks to complete (prevents events after disconnect)
                                    let (read_result, write_result) = tokio::join!(read_handle, write_handle);
                                    if let Err(e) = read_result {
                                        log::warn!("Read task for peer {} panicked: {}", peer_id.0, e);
                                    }
                                    if let Err(e) = write_result {
                                        log::warn!("Write task for peer {} panicked: {}", peer_id.0, e);
                                    }

                                    // Cleanup: remove peer sender and emit disconnect event
                                    {
                                        let mut senders = peer_senders_cleanup.lock().await;
                                        senders.remove(&peer_id);
                                    }
                                    
                                    let _ = event_tx.send(NetEvent::PeerDisconnected { id: peer_id });
                                }
                                Err(e) => {
                                    log::warn!("Failed to accept bi-directional stream from {}: {}", addr, e);
                                    let _ = event_tx.send(NetEvent::PeerDisconnected { id: peer_id });
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to accept connection: {e}");
                        }
                    }
                });
            }
            cmd = server_cmd_rx.recv() => {
                match cmd {
                    Some(ServerCommand::SendTo { peer, message }) => {
                        // Encode message
                        match encode(&message) {
                            Ok(bytes) => {
                                let senders = peer_senders.lock().await;
                                if let Some(sender) = senders.get(&peer) {
                                    // Note: Send failures are logged but not surfaced as errors because:
                                    // 1. Sends are fire-and-forget from the caller's perspective
                                    // 2. Peer disconnections are already reported via PeerDisconnected events
                                    // 3. With bounded channels, send failures can indicate backpressure or disconnection
                                    if let Err(e) = sender.try_send(Bytes::from(bytes)) {
                                        log::debug!("Failed to route message to peer {} (buffer full or disconnecting): {}", peer.0, e);
                                    }
                                } else {
                                    log::debug!("Peer {} not found for send_to (already disconnected)", peer.0);
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to encode message for peer {}: {}", peer.0, e);
                            }
                        }
                    }
                    Some(ServerCommand::Broadcast { message }) => {
                        // Encode message once
                        match encode(&message) {
                            Ok(bytes) => {
                                let bytes = Bytes::from(bytes);
                                let senders = peer_senders.lock().await;
                                // Note: Individual broadcast failures are logged but don't fail the whole broadcast.
                                // This is intentional - some peers may be disconnecting while others are healthy.
                                // Disconnections are reported separately via PeerDisconnected events.
                                for (peer_id, sender) in senders.iter() {
                                    if let Err(e) = sender.try_send(bytes.clone()) {
                                        log::debug!("Failed to broadcast to peer {} (buffer full or disconnecting): {}", peer_id.0, e);
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to encode broadcast message: {}", e);
                            }
                        }
                    }
                    None => {
                        log::info!("Server command channel closed");
                        break;
                    }
                }
            }
        }
    }

    let _ = event_tx.send(NetEvent::HostingStopped);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_id_incrementing() {
        let next_peer_id = Arc::new(AtomicU64::new(1));
        
        let id1 = PeerId(next_peer_id.fetch_add(1, Ordering::SeqCst));
        let id2 = PeerId(next_peer_id.fetch_add(1, Ordering::SeqCst));
        let id3 = PeerId(next_peer_id.fetch_add(1, Ordering::SeqCst));
        
        assert_eq!(id1, PeerId(1));
        assert_eq!(id2, PeerId(2));
        assert_eq!(id3, PeerId(3));
    }
}
