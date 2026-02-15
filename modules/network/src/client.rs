use std::net::SocketAddr;

use bevy::log;
use bytes::Bytes;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

use crate::ClientEvent;
use crate::config;
use crate::protocol::{ClientMessage, ServerMessage, decode, encode};

pub(crate) async fn run_client(
    addr: SocketAddr,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    client_msg_rx: mpsc::Receiver<ClientMessage>,
    cancel_token: CancellationToken,
) {
    if let Err(e) = run_client_inner(addr, &event_tx, client_msg_rx, cancel_token).await {
        let reason = format!("Client error: {e}");
        if let Err(err) = event_tx.send(ClientEvent::Error(reason.clone())) {
            log::error!("Failed to send ClientEvent::Error: {}", err);
        }
        if let Err(err) = event_tx.send(ClientEvent::Disconnected { reason }) {
            log::error!("Failed to send ClientEvent::Disconnected: {}", err);
        }
    }
}

async fn run_client_inner(
    addr: SocketAddr,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
    mut client_msg_rx: mpsc::Receiver<ClientMessage>,
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

    if let Err(err) = event_tx.send(ClientEvent::Connected) {
        log::error!("Failed to send Connected event: {}", err);
    }

    // Open bi-directional stream
    let open_result = tokio::select! {
        result = connection.open_bi() => result,
        _ = cancel_token.cancelled() => {
            log::info!("Client disconnect requested before opening stream");
            connection.close(0u32.into(), b"disconnect requested");
            if let Err(err) = event_tx.send(ClientEvent::Disconnected {
                reason: "Disconnect requested".into(),
            }) {
                log::error!("Failed to send Disconnected event: {}", err);
            }
            return Ok(());
        }
        _ = connection.closed() => {
            log::info!("Connection closed before opening bi-directional stream");
            if let Err(err) = event_tx.send(ClientEvent::Disconnected {
                reason: "Connection closed by remote".into(),
            }) {
                log::error!("Failed to send Disconnected event: {}", err);
            }
            return Ok(());
        }
    };

    let (send_stream, recv_stream) = match open_result {
        Ok(streams) => streams,
        Err(e) => {
            log::warn!("Failed to open bi-directional stream: {}", e);
            if let Err(err) = event_tx.send(ClientEvent::Disconnected {
                reason: format!("Failed to open stream: {}", e),
            }) {
                log::error!("Failed to send Disconnected event: {}", err);
            }
            return Ok(());
        }
    };

    // Wrap streams with LengthDelimitedCodec
    let mut framed_read = FramedRead::new(recv_stream, LengthDelimitedCodec::new());
    let mut framed_write = FramedWrite::new(send_stream, LengthDelimitedCodec::new());

    // Important: notify peer about the opened stream immediately.
    // Quinn only surfaces open_bi to accept_bi after data is written.
    if let Ok(bytes) = encode(&ClientMessage::Hello) {
        if let Err(e) = framed_write.send(Bytes::from(bytes)).await {
            log::error!("Failed to send client hello: {}", e);
            if let Err(err) = event_tx.send(ClientEvent::Disconnected {
                reason: format!("Failed to send client hello: {e}"),
            }) {
                log::error!("Failed to send Disconnected event: {}", err);
            }
            return Ok(());
        }
    } else {
        if let Err(err) = event_tx.send(ClientEvent::Disconnected {
            reason: "Failed to encode client hello".into(),
        }) {
            log::error!("Failed to send Disconnected event: {}", err);
        }
        return Ok(());
    }

    // Create per-client cancellation token to coordinate shutdown
    let client_cancel = CancellationToken::new();

    // Spawn read loop
    let client_cancel_read = client_cancel.clone();
    let event_tx_read = event_tx.clone();
    let cancel_token_read = cancel_token.clone();
    let mut read_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_token_read.cancelled() => {
                    log::debug!("Read loop cancelled (disconnect requested)");
                    break;
                }
                _ = client_cancel_read.cancelled() => {
                    log::debug!("Read loop cancelled (client shutdown)");
                    break;
                }
                frame = framed_read.next() => {
                    match frame {
                        Some(Ok(bytes)) => {
                            // Decode ServerMessage
                            match decode::<ServerMessage>(&bytes) {
                                Ok(message) => {
                                    if let Err(err) = event_tx_read.send(ClientEvent::ServerMessageReceived(message)) {
                                        log::error!("Failed to send ServerMessageReceived event: {}", err);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to decode message from host: {}", e);
                                }
                            }
                        }
                        Some(Err(e)) => {
                            log::error!("Stream error from host: {}", e);
                            break;
                        }
                        None => {
                            log::info!("Disconnected (read stream closed)");
                            break;
                        }
                    }
                }
            }
        }
    });

    // Spawn write loop
    let client_cancel_write = client_cancel.clone();
    let cancel_token_write = cancel_token.clone();
    let mut write_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_token_write.cancelled() => {
                    log::debug!("Write loop cancelled (disconnect requested)");
                    break;
                }
                _ = client_cancel_write.cancelled() => {
                    log::debug!("Write loop cancelled (client shutdown)");
                    break;
                }
                message = client_msg_rx.recv() => {
                    match message {
                        Some(message) => {
                            // Encode ClientMessage
                            match encode(&message) {
                                Ok(bytes) => {
                                    // Wrap send in select to observe cancellation during send
                                    tokio::select! {
                                        result = framed_write.send(Bytes::from(bytes)) => {
                                            if let Err(e) = result {
                                                log::error!("Failed to send message to host (stream error): {}", e);
                                                break;
                                            }
                                        }
                                        _ = cancel_token_write.cancelled() => {
                                            log::debug!("Write loop cancelled during send (disconnect requested)");
                                            break;
                                        }
                                        _ = client_cancel_write.cancelled() => {
                                            log::debug!("Write loop cancelled during send (client shutdown)");
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to encode message: {}", e);
                                }
                            }
                        }
                        None => {
                            log::debug!("Write channel closed");
                            break;
                        }
                    }
                }
            }
        }
    });

    // Wait for any task to complete, connection to close, or cancellation
    let reason = tokio::select! {
        _ = cancel_token.cancelled() => {
            log::info!("Client disconnect requested");
            connection.close(0u32.into(), b"disconnect requested");
            "Disconnect requested"
        }
        _ = &mut read_handle => {
            log::debug!("Read task completed");
            "Read stream closed"
        }
        _ = &mut write_handle => {
            log::debug!("Write task completed");
            "Write stream closed"
        }
        _ = connection.closed() => {
            log::info!("Connection closed");
            "Connection closed by remote"
        }
    };

    // Cancel client tasks to ensure both stop cleanly
    client_cancel.cancel();

    // Wait for both tasks to complete (prevents events after disconnect)
    let (read_result, write_result) = tokio::join!(read_handle, write_handle);
    if let Err(e) = read_result {
        log::warn!("Read task panicked: {}", e);
    }
    if let Err(e) = write_result {
        log::warn!("Write task panicked: {}", e);
    }

    log::info!("Disconnected from {addr}");
    if let Err(err) = event_tx.send(ClientEvent::Disconnected {
        reason: reason.into(),
    }) {
        log::error!("Failed to send ClientEvent::Disconnected: {}", err);
    }

    Ok(())
}
