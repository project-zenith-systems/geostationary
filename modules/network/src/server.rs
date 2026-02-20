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
use crate::{ClientId, ServerEvent};

/// Bounded channel buffer size per client to prevent memory exhaustion from slow clients.
/// Allows brief bursts while providing backpressure.
const PER_PEER_BUFFER_SIZE: usize = 100;

pub(crate) async fn run_server(
    port: u16,
    event_tx: mpsc::UnboundedSender<ServerEvent>,
    server_cmd_rx: mpsc::UnboundedReceiver<ServerCommand>,
    cancel_token: CancellationToken,
) {
    if let Err(e) = run_server_inner(port, &event_tx, server_cmd_rx, cancel_token).await {
        if let Err(err) = event_tx.send(ServerEvent::Error(format!("Server error: {e}"))) {
            log::error!("Failed to send ServerEvent::Error: {}", err);
        }
        if let Err(err) = event_tx.send(ServerEvent::HostingStopped) {
            log::error!("Failed to send ServerEvent::HostingStopped: {}", err);
        }
    }
}

async fn run_server_inner(
    port: u16,
    event_tx: &mpsc::UnboundedSender<ServerEvent>,
    mut server_cmd_rx: mpsc::UnboundedReceiver<ServerCommand>,
    cancel_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    let server_config = config::build_server_config()?;
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    log::info!("Server listening on {addr}");
    if let Err(e) = event_tx.send(ServerEvent::HostingStarted { port }) {
        log::error!("Failed to send HostingStarted event: {}", e);
    }

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

                                    if let Err(err) = event_tx.send(ServerEvent::ClientConnected {
                                        id: client_id,
                                        addr,
                                    }) {
                                        log::error!("Failed to send ClientConnected event: {}", err);
                                    }

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
                                                                    if let Err(err) = event_tx_read.send(ServerEvent::ClientMessageReceived {
                                                                        from: client_id,
                                                                        message,
                                                                    }) {
                                                                        log::error!("Failed to send ClientMessageReceived event: {}", err);
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    log::error!("Failed to decode message from client {}: {}", client_id.0, e);
                                                                }
                                                            }
                                                        }
                                                        Some(Err(e)) => {
                                                            log::error!("Stream error from client {}: {}", client_id.0, e);
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
                                                                log::error!("Failed to send to client {} (stream error): {}", client_id.0, e);
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
                                        log::error!("Read task for client {} panicked: {}", client_id.0, e);
                                    }
                                    if let Err(e) = write_result {
                                        log::error!("Write task for client {} panicked: {}", client_id.0, e);
                                    }

                                    // Cleanup: remove client sender and emit disconnect event
                                    {
                                        let mut senders = client_senders_cleanup.lock().await;
                                        senders.remove(&client_id);
                                    }

                                    if let Err(err) = event_tx.send(ServerEvent::ClientDisconnected { id: client_id }) {
                                        log::error!("Failed to send ClientDisconnected event: {}", err);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to accept bi-directional stream from {}: {}", addr, e);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to accept connection: {}", e);
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
                                    if let Err(e) = sender.try_send(Bytes::from(bytes)) {
                                        log::error!("Failed to route message to client {} (buffer full or disconnecting): {}", client.0, e);
                                    }
                                } else {
                                    log::error!("Client {} not found for send_to (already disconnected)", client.0);
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to encode message for client {}: {}", client.0, e);
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
                                        log::error!("Failed to broadcast to client {} (buffer full or disconnecting): {}", client_id.0, e);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to encode broadcast message: {}", e);
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

    if let Err(err) = event_tx.send(ServerEvent::HostingStopped) {
        log::error!("Failed to send ServerEvent::HostingStopped: {}", err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::sink::SinkExt;
    use futures_util::stream::StreamExt;
    use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

    use crate::config;

    /// Stream tag bytes matching the plan's server→client stream table.
    const TAG_TILES: u8 = 1;
    const TAG_ATMOS: u8 = 2;
    const TAG_THINGS: u8 = 3;

    /// Byte sequence used as a `StreamReady` sentinel in this spike.
    const STREAM_READY: &[u8] = b"READY";

    /// Spike: Quinn multi-stream
    ///
    /// Proves that a single `quinn::Connection` can carry three independent
    /// server→client unidirectional streams, each prefixed with a routing tag
    /// byte and independently framed with `LengthDelimitedCodec`, and that
    /// `StreamReady` sentinels arrive correctly on all streams regardless of
    /// the order in which the client calls `accept_uni()`.
    #[tokio::test]
    async fn test_quinn_multi_stream_spike() {
        let server_config = config::build_server_config().expect("server config");
        let client_config = config::build_client_config().expect("client config");

        let server_addr: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
        let server_endpoint =
            quinn::Endpoint::server(server_config, server_addr).expect("server endpoint");
        let bound_addr = server_endpoint.local_addr().expect("bound addr");

        // Server: accept one connection then open 3 unidirectional streams.
        let server_task = tokio::spawn(async move {
            let incoming = server_endpoint
                .accept()
                .await
                .expect("server: incoming connection");
            let connection = incoming.await.expect("server: connection established");

            for tag in [TAG_TILES, TAG_ATMOS, TAG_THINGS] {
                let mut send = connection.open_uni().await.expect("server: open_uni");

                // The first byte on the stream is the routing tag byte.
                // Writing it also makes the stream visible to accept_uni() on
                // the remote side (Quinn only delivers a stream once it has data).
                send.write_all(&[tag]).await.expect("server: write tag byte");

                // Remaining bytes use independent LengthDelimitedCodec framing.
                let mut framed = FramedWrite::new(send, LengthDelimitedCodec::new());

                let payload = Bytes::from(format!("data-{tag}").into_bytes());
                framed.send(payload).await.expect("server: send data frame");

                // StreamReady sentinel — marks the end of the initial burst on
                // this stream, mirroring the plan's InitialStateDone handshake.
                framed
                    .send(Bytes::from_static(STREAM_READY))
                    .await
                    .expect("server: send StreamReady");
            }

            // Keep the connection open until the client closes it, so that
            // in-flight stream data is not cut short by a connection reset.
            connection.closed().await;
        });

        // Client: connect and accept all 3 unidirectional streams.
        let mut client_endpoint =
            quinn::Endpoint::client(([0, 0, 0, 0], 0u16).into()).expect("client endpoint");
        client_endpoint.set_default_client_config(client_config);
        let connection = client_endpoint
            .connect(bound_addr, "localhost")
            .expect("client: connect call")
            .await
            .expect("client: connection established");

        let mut received_tags: Vec<u8> = Vec::new();

        // Streams may arrive in any order; accept all 3 and verify each one
        // independently using its own codec instance.
        for _ in 0..3 {
            let mut recv = connection.accept_uni().await.expect("client: accept_uni");

            // Read the single routing tag byte that precedes framed data.
            let mut tag_buf = [0u8; 1];
            recv.read_exact(&mut tag_buf)
                .await
                .expect("client: read tag byte");
            let tag = tag_buf[0];

            // Each stream has its own LengthDelimitedCodec; framing is
            // independent across streams.
            let mut framed = FramedRead::new(recv, LengthDelimitedCodec::new());

            let data_frame = framed
                .next()
                .await
                .expect("client: data frame present")
                .expect("client: data frame ok");
            assert_eq!(
                std::str::from_utf8(&data_frame).expect("utf8"),
                format!("data-{tag}"),
                "stream tag={tag}: payload mismatch"
            );

            let ready_frame = framed
                .next()
                .await
                .expect("client: StreamReady present")
                .expect("client: StreamReady ok");
            assert_eq!(
                &ready_frame[..],
                STREAM_READY,
                "stream tag={tag}: StreamReady sentinel mismatch"
            );

            received_tags.push(tag);
        }

        // Verify all 3 tagged streams must have been received; order may differ from
        // the order the server opened them.
        received_tags.sort_unstable();
        assert_eq!(
            received_tags,
            [TAG_TILES, TAG_ATMOS, TAG_THINGS],
            "all 3 tagged streams received"
        );

        // Explicitly close the connection so the server's connection.closed()
        // future resolves, unblocking server_task.
        connection.close(0u32.into(), b"done");

        server_task.await.expect("server task completed");
    }
}
