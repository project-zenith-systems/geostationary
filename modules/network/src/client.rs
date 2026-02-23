use std::net::SocketAddr;

use bevy::log;
use bytes::Bytes;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

use crate::ClientEvent;
use crate::config;
use crate::protocol::{ClientMessage, ServerMessage, StreamReady, decode, encode};

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

    // Open bi-directional control stream (tag 0).
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
            log::error!("Failed to open bi-directional stream: {}", e);
            if let Err(err) = event_tx.send(ClientEvent::Disconnected {
                reason: format!("Failed to open stream: {}", e),
            }) {
                log::error!("Failed to send Disconnected event: {}", err);
            }
            return Ok(());
        }
    };

    // Wrap control stream with LengthDelimitedCodec.
    let mut framed_read = FramedRead::new(recv_stream, LengthDelimitedCodec::new());
    let mut framed_write = FramedWrite::new(send_stream, LengthDelimitedCodec::new());

    // Send Hello immediately — this makes the bi stream visible to accept_bi()
    // on the server and delivers the client's display name.
    // TODO(souls): read name from config instead of hardcoded empty string.
    let hello = ClientMessage::Hello {
        name: String::new(),
    };
    if let Ok(bytes) = encode(&hello) {
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

    // Create per-client cancellation token to coordinate shutdown.
    let client_cancel = CancellationToken::new();

    // Uni-stream accept loop: runs concurrently with the control-stream loops.
    // For each server→client unidirectional stream: read the tag byte, then
    // route framed messages to ClientEvent::StreamFrame / StreamReady events.
    // Per-stream tasks are tracked in a JoinSet and awaited before the handle
    // returns, preventing post-disconnect events from leaking out.
    // Pre-compute the canonical StreamReady encoding for exact byte comparison.
    // Using decode::<StreamReady> is unreliable because wincode may not check
    // for trailing bytes on unit structs, causing false positives.
    let stream_ready_bytes: Bytes =
        Bytes::from(encode(&StreamReady).expect("StreamReady must encode"));

    let event_tx_uni = event_tx.clone();
    let cancel_token_uni = cancel_token.clone();
    let client_cancel_uni = client_cancel.clone();
    let connection_uni = connection.clone();
    let uni_accept_handle = tokio::spawn(async move {
        let mut stream_tasks: JoinSet<()> = JoinSet::new();
        loop {
            tokio::select! {
                _ = cancel_token_uni.cancelled() => {
                    log::debug!("Uni-stream accept loop cancelled (disconnect requested)");
                    break;
                }
                _ = client_cancel_uni.cancelled() => {
                    log::debug!("Uni-stream accept loop cancelled (client shutdown)");
                    break;
                }
                result = connection_uni.accept_uni() => {
                    match result {
                        Ok(mut recv) => {
                            // Read 1-byte routing tag.
                            let mut tag_buf = [0u8; 1];
                            if let Err(e) = recv.read_exact(&mut tag_buf).await {
                                log::error!("Failed to read stream tag byte: {}", e);
                                // Tag read failure is stream-local; continue accepting other streams.
                                continue;
                            }
                            let tag = tag_buf[0];
                            if tag == 0 {
                                log::warn!("Ignoring uni stream with reserved tag=0");
                                continue;
                            }
                            log::info!("Accepted uni stream tag={}", tag);

                            // Spawn an independent read loop for this stream with cancellation support.
                            let frame_tx = event_tx_uni.clone();
                            let cancel_token_stream = cancel_token_uni.clone();
                            let client_cancel_stream = client_cancel_uni.clone();
                            let ready_bytes = stream_ready_bytes.clone();
                            stream_tasks.spawn(async move {
                                let mut framed =
                                    FramedRead::new(recv, LengthDelimitedCodec::new());
                                loop {
                                    tokio::select! {
                                        _ = cancel_token_stream.cancelled() => {
                                            log::debug!("Uni-stream reader cancelled (disconnect requested), tag={}", tag);
                                            break;
                                        }
                                        _ = client_cancel_stream.cancelled() => {
                                            log::debug!("Uni-stream reader cancelled (client shutdown), tag={}", tag);
                                            break;
                                        }
                                        frame = framed.next() => {
                                            match frame {
                                                Some(Ok(bytes)) => {
                                                    // Detect the StreamReady sentinel via exact
                                                    // byte comparison against the pre-computed
                                                    // canonical encoding.
                                                    if bytes[..] == ready_bytes[..] {
                                                        log::info!("StreamReady received on tag={}", tag);
                                                        let _ = frame_tx
                                                            .send(ClientEvent::StreamReady { tag });
                                                    } else {
                                                        log::info!("StreamFrame received on tag={} ({} bytes)", tag, bytes.len());
                                                        let _ = frame_tx.send(
                                                            ClientEvent::StreamFrame {
                                                                tag,
                                                                data: bytes.freeze(),
                                                            },
                                                        );
                                                    }
                                                }
                                                Some(Err(e)) => {
                                                    log::error!(
                                                        "Stream tag={} read error: {}",
                                                        tag, e
                                                    );
                                                    break;
                                                }
                                                None => {
                                                    // Stream closed naturally.
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            log::debug!("accept_uni ended: {}", e);
                            break;
                        }
                    }
                }
            }
        }
        // Await all per-stream reader tasks before returning so that no
        // StreamFrame/StreamReady events can be emitted after Disconnected.
        stream_tasks.shutdown().await;
    });

    // Control-stream read loop
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

    // Control-stream write loop
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
                            match encode(&message) {
                                Ok(bytes) => {
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

    // Wait for any task to complete, connection to close, or cancellation.
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

    // Cancel all client tasks to ensure they stop cleanly.
    client_cancel.cancel();

    let (read_result, write_result, _) =
        tokio::join!(read_handle, write_handle, uni_accept_handle);
    if let Err(e) = read_result {
        log::error!("Read task panicked: {}", e);
    }
    if let Err(e) = write_result {
        log::error!("Write task panicked: {}", e);
    }

    log::info!("Disconnected from {addr}");
    if let Err(err) = event_tx.send(ClientEvent::Disconnected {
        reason: reason.into(),
    }) {
        log::error!("Failed to send ClientEvent::Disconnected: {}", err);
    }

    Ok(())
}
