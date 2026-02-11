use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::log;
use bytes::Bytes;
use futures::stream::StreamExt;
use futures::sink::SinkExt;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

use crate::{NetEvent, PeerId};
use crate::config;
use crate::protocol::{decode, encode, HostMessage, PeerMessage};
use crate::runtime::ServerCommand;

pub(crate) async fn run_server(
    port: u16,
    event_tx: mpsc::UnboundedSender<NetEvent>,
    server_cmd_rx: mpsc::UnboundedReceiver<ServerCommand>,
    cancel_token: CancellationToken,
) {
    if let Err(e) = run_server_inner(port, &event_tx, server_cmd_rx, cancel_token).await {
        let _ = event_tx.send(NetEvent::Error(format!("Server error: {e}")));
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
    let peer_senders: Arc<tokio::sync::Mutex<HashMap<PeerId, mpsc::UnboundedSender<Bytes>>>> = 
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

                            // Open bi-directional stream
                            match connection.accept_bi().await {
                                Ok((recv_stream, send_stream)) => {
                                    // Wrap streams with LengthDelimitedCodec
                                    let mut framed_read = FramedRead::new(recv_stream, LengthDelimitedCodec::new());
                                    let mut framed_write = FramedWrite::new(send_stream, LengthDelimitedCodec::new());

                                    // Create channel for this peer's write loop
                                    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Bytes>();
                                    
                                    // Register peer sender
                                    {
                                        let mut senders = peer_senders.lock().await;
                                        senders.insert(peer_id, write_tx);
                                    }

                                    let event_tx_read = event_tx.clone();
                                    let event_tx_write = event_tx.clone();
                                    let cancel_token_read = cancel_token_clone.clone();
                                    let cancel_token_write = cancel_token_clone.clone();
                                    let peer_senders_cleanup = peer_senders.clone();

                                    // Spawn per-peer read loop
                                    let read_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::select! {
                                                _ = cancel_token_read.cancelled() => {
                                                    log::debug!("Read loop cancelled for peer {}", peer_id.0);
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
                                    let write_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::select! {
                                                _ = cancel_token_write.cancelled() => {
                                                    log::debug!("Write loop cancelled for peer {}", peer_id.0);
                                                    break;
                                                }
                                                bytes = write_rx.recv() => {
                                                    match bytes {
                                                        Some(bytes) => {
                                                            if let Err(e) = framed_write.send(bytes).await {
                                                                log::warn!("Failed to send to peer {}: {}", peer_id.0, e);
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

                                    // Wait for either task to complete or connection close
                                    tokio::select! {
                                        _ = read_handle => {
                                            log::info!("Read task completed for peer {}", peer_id.0);
                                        }
                                        _ = write_handle => {
                                            log::info!("Write task completed for peer {}", peer_id.0);
                                        }
                                        _ = connection.closed() => {
                                            log::info!("Connection closed for peer {}", peer_id.0);
                                        }
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
                                    if let Err(e) = sender.send(Bytes::from(bytes)) {
                                        log::warn!("Failed to route message to peer {}: {}", peer.0, e);
                                    }
                                } else {
                                    log::warn!("Peer {} not found for send_to", peer.0);
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
                                for (peer_id, sender) in senders.iter() {
                                    if let Err(e) = sender.send(bytes.clone()) {
                                        log::warn!("Failed to broadcast to peer {}: {}", peer_id.0, e);
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
