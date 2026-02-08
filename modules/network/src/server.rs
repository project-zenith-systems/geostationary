use std::net::SocketAddr;

use bevy::log;
use tokio::sync::mpsc;

use crate::NetEvent;
use crate::config;

pub(crate) async fn run_server(port: u16, event_tx: mpsc::UnboundedSender<NetEvent>) {
    if let Err(e) = run_server_inner(port, &event_tx).await {
        let _ = event_tx.send(NetEvent::Error(format!("Server error: {e}")));
    }
}

async fn run_server_inner(
    port: u16,
    event_tx: &mpsc::UnboundedSender<NetEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let server_config = config::build_server_config()?;
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    log::info!("Server listening on {addr}");
    let _ = event_tx.send(NetEvent::HostingStarted { port });

    loop {
        let Some(incoming) = endpoint.accept().await else {
            log::info!("Server endpoint closed");
            break;
        };

        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    log::info!("Client connected from {}", connection.remote_address());
                    let _ = event_tx.send(NetEvent::ClientConnected {
                        addr: connection.remote_address(),
                    });

                    // Hold the connection open until it closes
                    connection.closed().await;
                    log::info!("Client disconnected: {}", connection.remote_address());
                }
                Err(e) => {
                    log::warn!("Failed to accept connection: {e}");
                }
            }
        });
    }

    Ok(())
}
