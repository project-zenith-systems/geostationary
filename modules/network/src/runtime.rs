use std::sync::Arc;

use bevy::prelude::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{HostMessage, NetEvent};

/// Internal commands for server message sending.
/// Bevy → server task bridge: sent from game systems to server task.
#[derive(Clone, Debug)]
pub(crate) enum ServerCommand {
    SendTo { peer: crate::PeerId, message: HostMessage },
    Broadcast { message: HostMessage },
}

#[derive(Resource)]
pub(crate) struct NetworkRuntime {
    rt: Arc<tokio::runtime::Runtime>,
}

impl NetworkRuntime {
    pub(crate) fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime");
        Self { rt: Arc::new(rt) }
    }

    pub(crate) fn spawn(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> JoinHandle<()> {
        self.rt.spawn(future)
    }
}

/// async → Bevy bridge: async tasks clone this sender to emit events.
#[derive(Resource, Clone)]
pub(crate) struct NetEventSender(pub(crate) mpsc::UnboundedSender<NetEvent>);

/// async → Bevy bridge: drained each frame in PreUpdate.
#[derive(Resource)]
pub(crate) struct NetEventReceiver(pub(crate) mpsc::UnboundedReceiver<NetEvent>);

/// Tracks active network tasks and their cancellation tokens.
#[derive(Resource, Default)]
pub(crate) struct NetworkTasks {
    pub(crate) server_task: Option<(JoinHandle<()>, CancellationToken)>,
    pub(crate) client_task: Option<(JoinHandle<()>, CancellationToken)>,
}

impl NetworkTasks {
    /// Returns true if a server task is running or shutting down.
    /// Checks if the handle exists and is not finished.
    pub(crate) fn is_hosting(&self) -> bool {
        self.server_task
            .as_ref()
            .map(|(handle, _)| !handle.is_finished())
            .unwrap_or(false)
    }

    /// Returns true if a client task is running or disconnecting.
    /// Checks if the handle exists and is not finished.
    pub(crate) fn is_connected(&self) -> bool {
        self.client_task
            .as_ref()
            .map(|(handle, _)| !handle.is_finished())
            .unwrap_or(false)
    }

    /// Initiates graceful server shutdown via cancellation token.
    /// The task remains tracked until it finishes to prevent port conflicts.
    pub(crate) fn stop_hosting(&mut self) {
        if let Some((_handle, token)) = &self.server_task {
            token.cancel();
            // Keep the handle so is_hosting() remains true until task exits
        }
    }

    /// Initiates graceful client disconnect via cancellation token.
    /// The task remains tracked until it finishes to prevent overlapping connections.
    pub(crate) fn disconnect(&mut self) {
        if let Some((_handle, token)) = &self.client_task {
            token.cancel();
            // Keep the handle so is_connected() remains true until task exits
        }
    }

    /// Removes finished tasks to free up state for new connections.
    /// Should be called regularly to clean up completed tasks.
    pub(crate) fn cleanup_finished(&mut self) {
        if let Some((handle, _)) = &self.server_task {
            if handle.is_finished() {
                self.server_task = None;
            }
        }
        if let Some((handle, _)) = &self.client_task {
            if handle.is_finished() {
                self.client_task = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hosting() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        let handle = rt.spawn(async {});

        let mut tasks = NetworkTasks::default();
        assert!(!tasks.is_hosting());

        tasks.server_task = Some((handle, cancel_token));
        assert!(tasks.is_hosting());
    }

    #[test]
    fn test_is_connected() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        let handle = rt.spawn(async {});

        let mut tasks = NetworkTasks::default();
        assert!(!tasks.is_connected());

        tasks.client_task = Some((handle, cancel_token));
        assert!(tasks.is_connected());
    }

    #[test]
    fn test_stop_hosting() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        // Create a task that waits for cancellation
        let token_clone = cancel_token.clone();
        let handle = rt.spawn(async move {
            token_clone.cancelled().await;
        });

        let mut tasks = NetworkTasks::default();
        let handle_clone = tasks
            .server_task
            .insert((handle, cancel_token.clone()))
            .0
            .abort_handle();

        assert!(tasks.is_hosting());
        tasks.stop_hosting();
        // Task is still tracked (not finished yet) but cancellation is requested
        assert!(tasks.is_hosting());
        assert!(cancel_token.is_cancelled());

        // Wait for the task to finish using a blocking approach
        drop(handle_clone);
        rt.block_on(async {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        });
        tasks.cleanup_finished();
        assert!(!tasks.is_hosting());
    }

    #[test]
    fn test_disconnect() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        // Create a task that waits for cancellation
        let token_clone = cancel_token.clone();
        let handle = rt.spawn(async move {
            token_clone.cancelled().await;
        });

        let mut tasks = NetworkTasks::default();
        let handle_clone = tasks
            .client_task
            .insert((handle, cancel_token.clone()))
            .0
            .abort_handle();

        assert!(tasks.is_connected());
        tasks.disconnect();
        // Task is still tracked (not finished yet) but cancellation is requested
        assert!(tasks.is_connected());
        assert!(cancel_token.is_cancelled());

        // Wait for the task to finish using a blocking approach
        drop(handle_clone);
        rt.block_on(async {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        });
        tasks.cleanup_finished();
        assert!(!tasks.is_connected());
    }

    #[test]
    fn test_stop_hosting_when_not_hosting() {
        let mut tasks = NetworkTasks::default();
        // Should not panic
        tasks.stop_hosting();
        assert!(!tasks.is_hosting());
    }

    #[test]
    fn test_disconnect_when_not_connected() {
        let mut tasks = NetworkTasks::default();
        // Should not panic
        tasks.disconnect();
        assert!(!tasks.is_connected());
    }

    #[test]
    fn test_cleanup_finished() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        let handle = rt.spawn(async {});

        let mut tasks = NetworkTasks::default();
        tasks.server_task = Some((handle, cancel_token));

        // Wait for the task to finish deterministically
        rt.block_on(async {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        });

        // Cleanup should remove finished tasks
        tasks.cleanup_finished();
        assert!(!tasks.is_hosting());
    }

    #[test]
    fn test_is_hosting_detects_finished_task() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        let handle = rt.spawn(async {});

        let mut tasks = NetworkTasks::default();
        tasks.server_task = Some((handle, cancel_token));

        // Wait for the task to finish deterministically
        rt.block_on(async {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        });

        // is_hosting should return false for finished tasks
        assert!(!tasks.is_hosting());
    }

    #[test]
    fn test_is_connected_detects_finished_task() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        let handle = rt.spawn(async {});

        let mut tasks = NetworkTasks::default();
        tasks.client_task = Some((handle, cancel_token));

        // Wait for the task to finish deterministically
        rt.block_on(async {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        });

        // is_connected should return false for finished tasks
        assert!(!tasks.is_connected());
    }
}
