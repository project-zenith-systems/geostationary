use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::log;
use bytes::Bytes;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

use crate::config;
use crate::protocol::{ClientMessage, decode, encode};
use crate::runtime::ServerCommand;
use crate::{ClientId, NetEvent};

/// Bounded channel buffer size per client to prevent memory exhaustion from slow clients.
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

    // Shared state for client ID assignment and per-client write channels
    let next_client_id = Arc::new(AtomicU64::new(1));
    let client_senders: Arc<tokio::sync::Mutex<HashMap<ClientId, mpsc::Sender<Bytes>>>> =
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
                let next_client_id = next_client_id.clone();
                let client_senders = client_senders.clone();

                tokio::spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            let addr = connection.remote_address();

                            // Assign incrementing ClientId
                            let client_id = ClientId(next_client_id.fetch_add(1, Ordering::SeqCst));
                            log::info!("Client connected from {} with ClientId {}", addr, client_id.0);

                            // Open bi-directional stream (with cancellation support).
                            // ClientConnected is deferred until the write channel is
                            // registered so that game code can send messages (e.g.
                            // Welcome) immediately upon seeing the event.
                            let accept_result = tokio::select! {
                                result = connection.accept_bi() => result,
                                _ = cancel_token_clone.cancelled() => {
                                    log::info!("Server shutdown while waiting for bi-directional stream from client {}", client_id.0);
                                    connection.close(0u32.into(), b"server shutdown");
                                    return;
                                }
                                _ = connection.closed() => {
                                    log::info!("Connection closed before bi-directional stream opened for client {}", client_id.0);
                                    return;
                                }
                            };

                            match accept_result {
                                Ok((send_stream, recv_stream)) => {
                                    // Wrap streams with LengthDelimitedCodec
                                    let mut framed_read = FramedRead::new(recv_stream, LengthDelimitedCodec::new());
                                    let mut framed_write = FramedWrite::new(send_stream, LengthDelimitedCodec::new());

                                    // Create bounded channel for this client's write loop
                                    let (write_tx, mut write_rx) = mpsc::channel::<Bytes>(PER_PEER_BUFFER_SIZE);

                                    // Register client sender, then announce the
                                    // connection so game code can send messages
                                    // (e.g. Welcome) that will be deliverable
                                    // immediately.
                                    {
                                        let mut senders = client_senders.lock().await;
                                        senders.insert(client_id, write_tx);
                                    }

                                    let _ = event_tx.send(NetEvent::ClientConnected {
                                        id: client_id,
                                        addr,
                                    });

                                    let event_tx_read = event_tx.clone();
                                    let cancel_token_read = cancel_token_clone.clone();
                                    let cancel_token_write = cancel_token_clone.clone();
                                    let client_senders_cleanup = client_senders.clone();

                                    // Create per-client cancellation token to coordinate shutdown
                                    let client_cancel = CancellationToken::new();

                                    // Spawn per-client read loop
                                    let client_cancel_read = client_cancel.clone();
                                    let mut read_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::select! {
                                                _ = cancel_token_read.cancelled() => {
                                                    log::debug!("Read loop cancelled for client {} (server shutdown)", client_id.0);
                                                    break;
                                                }
                                                _ = client_cancel_read.cancelled() => {
                                                    log::debug!("Read loop cancelled for client {} (client shutdown)", client_id.0);
                                                    break;
                                                }
                                                frame = framed_read.next() => {
                                                    match frame {
                                                        Some(Ok(bytes)) => {
                                                            // Decode ClientMessage
                                                            match decode::<ClientMessage>(&bytes) {
                                                                Ok(message) => {
                                                                    let _ = event_tx_read.send(NetEvent::ClientMessageReceived {
                                                                        from: client_id,
                                                                        message,
                                                                    });
                                                                }
                                                                Err(e) => {
                                                                    log::warn!("Failed to decode message from client {}: {}", client_id.0, e);
                                                                }
                                                            }
                                                        }
                                                        Some(Err(e)) => {
                                                            log::warn!("Stream error from client {}: {}", client_id.0, e);
                                                            break;
                                                        }
                                                        None => {
                                                            log::info!("Client {} disconnected (read stream closed)", client_id.0);
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });

                                    // Spawn per-client write loop
                                    let client_cancel_write = client_cancel.clone();
                                    let mut write_handle = tokio::spawn(async move {
                                        loop {
                                            tokio::select! {
                                                _ = cancel_token_write.cancelled() => {
                                                    log::debug!("Write loop cancelled for client {} (server shutdown)", client_id.0);
                                                    break;
                                                }
                                                _ = client_cancel_write.cancelled() => {
                                                    log::debug!("Write loop cancelled for client {} (client shutdown)", client_id.0);
                                                    break;
                                                }
                                                bytes = write_rx.recv() => {
                                                    match bytes {
                                                        Some(bytes) => {
                                                            if let Err(e) = framed_write.send(bytes).await {
                                                                log::warn!("Failed to send to client {} (stream error): {}", client_id.0, e);
                                                                break;
                                                            }
                                                        }
                                                        None => {
                                                            log::debug!("Write channel closed for client {}", client_id.0);
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });

                                    // Wait for any task to complete or connection to close, then cancel client tasks
                                    tokio::select! {
                                        _ = &mut read_handle => {
                                            log::debug!("Read task completed for client {}", client_id.0);
                                        }
                                        _ = &mut write_handle => {
                                            log::debug!("Write task completed for client {}", client_id.0);
                                        }
                                        _ = connection.closed() => {
                                            log::info!("Connection closed for client {}", client_id.0);
                                        }
                                    }

                                    // Cancel client tasks to ensure both stop cleanly
                                    client_cancel.cancel();

                                    // Wait for both tasks to complete (prevents events after disconnect)
                                    let (read_result, write_result) = tokio::join!(read_handle, write_handle);
                                    if let Err(e) = read_result {
                                        log::warn!("Read task for client {} panicked: {}", client_id.0, e);
                                    }
                                    if let Err(e) = write_result {
                                        log::warn!("Write task for client {} panicked: {}", client_id.0, e);
                                    }

                                    // Cleanup: remove client sender and emit disconnect event
                                    {
                                        let mut senders = client_senders_cleanup.lock().await;
                                        senders.remove(&client_id);
                                    }

                                    let _ = event_tx.send(NetEvent::ClientDisconnected { id: client_id });
                                }
                                Err(e) => {
                                    log::warn!("Failed to accept bi-directional stream from {}: {}", addr, e);
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
                    Some(ServerCommand::SendTo { client, message }) => {
                        // Encode message
                        match encode(&message) {
                            Ok(bytes) => {
                                let senders = client_senders.lock().await;
                                if let Some(sender) = senders.get(&client) {
                                    // Note: Send failures are logged but not surfaced as errors because:
                                    // 1. Sends are fire-and-forget from the caller's perspective
                                    // 2. Client disconnections are already reported via ClientDisconnected events
                                    // 3. With bounded channels, send failures can indicate backpressure or disconnection
                                    if let Err(e) = sender.try_send(Bytes::from(bytes)) {
                                        log::debug!("Failed to route message to client {} (buffer full or disconnecting): {}", client.0, e);
                                    }
                                } else {
                                    log::debug!("Client {} not found for send_to (already disconnected)", client.0);
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to encode message for client {}: {}", client.0, e);
                            }
                        }
                    }
                    Some(ServerCommand::Broadcast { message }) => {
                        // Encode message once
                        match encode(&message) {
                            Ok(bytes) => {
                                let bytes = Bytes::from(bytes);
                                let senders = client_senders.lock().await;
                                // Note: Individual broadcast failures are logged but don't fail the whole broadcast.
                                // This is intentional - some clients may be disconnecting while others are healthy.
                                // Disconnections are reported separately via ClientDisconnected events.
                                for (client_id, sender) in senders.iter() {
                                    if let Err(e) = sender.try_send(bytes.clone()) {
                                        log::debug!("Failed to broadcast to client {} (buffer full or disconnecting): {}", client_id.0, e);
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
