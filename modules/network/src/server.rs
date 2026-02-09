use std::net::SocketAddr;

use bevy::log;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::NetEvent;
use crate::config;

pub(crate) async fn run_server(port: u16, event_tx: mpsc::UnboundedSender<NetEvent>, cancel_token: CancellationToken) {
    if let Err(e) = run_server_inner(port, &event_tx, cancel_token).await {
        let _ = event_tx.send(NetEvent::Error(format!("Server error: {e}")));
    }
}

async fn run_server_inner(
    port: u16,
    event_tx: &mpsc::UnboundedSender<NetEvent>,
    cancel_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    let server_config = config::build_server_config()?;
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    log::info!("Server listening on {addr}");
    let _ = event_tx.send(NetEvent::HostingStarted { port });

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
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            log::info!("Client connected from {}", connection.remote_address());
                            let _ = event_tx.send(NetEvent::ClientConnected {
                                addr: connection.remote_address(),
                            });

                            // Wait for either connection close or server shutdown
                            tokio::select! {
                                _ = connection.closed() => {
                                    log::info!("Client disconnected: {}", connection.remote_address());
                                }
                                _ = cancel_token_clone.cancelled() => {
                                    log::info!("Closing connection to {} due to server shutdown", connection.remote_address());
                                    connection.close(0u32.into(), b"server shutdown");
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to accept connection: {e}");
                        }
                    }
                });
            }
        }
    }

    let _ = event_tx.send(NetEvent::HostingStopped);
    Ok(())
}
