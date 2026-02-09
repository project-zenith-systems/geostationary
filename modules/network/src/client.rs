use std::net::SocketAddr;

use bevy::log;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::NetEvent;
use crate::config;

pub(crate) async fn run_client(addr: SocketAddr, event_tx: mpsc::UnboundedSender<NetEvent>, cancel_token: CancellationToken) {
    if let Err(e) = run_client_inner(addr, &event_tx, cancel_token).await {
        let _ = event_tx.send(NetEvent::Error(format!("Client error: {e}")));
    }
}

async fn run_client_inner(
    addr: SocketAddr,
    event_tx: &mpsc::UnboundedSender<NetEvent>,
    cancel_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    let client_config = config::build_client_config()?;

    let bind_addr = match addr {
        SocketAddr::V4(_) => ([0, 0, 0, 0], 0u16).into(),
        SocketAddr::V6(_) => ([0u16; 8], 0u16).into(),
    };
    let mut endpoint = quinn::Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);

    log::info!("Connecting to {addr}...");
    let connection = endpoint.connect(addr, "localhost")?.await?;
    log::info!("Connected to {addr}");

    let _ = event_tx.send(NetEvent::Connected);

    // Wait for either the connection to close or cancellation
    tokio::select! {
        _ = cancel_token.cancelled() => {
            log::info!("Client disconnect requested");
            connection.close(0u32.into(), b"disconnect requested");
        }
        _ = connection.closed() => {
            log::info!("Connection closed by remote");
        }
    }

    log::info!("Disconnected from {addr}");
    let _ = event_tx.send(NetEvent::Disconnected {
        reason: "Connection closed".into(),
    });

    Ok(())
}
