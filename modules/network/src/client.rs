use std::net::SocketAddr;

use bevy::log;
use tokio::sync::mpsc;

use crate::NetEvent;
use crate::config;

pub(crate) async fn run_client(addr: SocketAddr, event_tx: mpsc::UnboundedSender<NetEvent>) {
    if let Err(e) = run_client_inner(addr, &event_tx).await {
        let _ = event_tx.send(NetEvent::Error(format!("Client error: {e}")));
    }
}

async fn run_client_inner(
    addr: SocketAddr,
    event_tx: &mpsc::UnboundedSender<NetEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client_config = config::build_client_config()?;

    let mut endpoint = quinn::Endpoint::client(([0, 0, 0, 0], 0u16).into())?;
    endpoint.set_default_client_config(client_config);

    log::info!("Connecting to {addr}...");
    let connection = endpoint.connect(addr, "localhost")?.await?;
    log::info!("Connected to {addr}");

    let _ = event_tx.send(NetEvent::Connected);

    // Hold the connection open until it closes
    connection.closed().await;
    log::info!("Disconnected from {addr}");
    let _ = event_tx.send(NetEvent::Disconnected {
        reason: "Connection closed".into(),
    });

    Ok(())
}
