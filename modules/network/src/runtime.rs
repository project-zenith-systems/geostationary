use std::sync::Arc;

use bevy::prelude::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::NetEvent;

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

    pub(crate) fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) -> JoinHandle<()> {
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
    pub(crate) fn is_hosting(&self) -> bool {
        self.server_task.is_some()
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.client_task.is_some()
    }

    pub(crate) fn stop_hosting(&mut self) {
        if let Some((handle, token)) = self.server_task.take() {
            token.cancel();
            handle.abort();
        }
    }

    pub(crate) fn disconnect(&mut self) {
        if let Some((handle, token)) = self.client_task.take() {
            token.cancel();
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_tasks_default() {
        let tasks = NetworkTasks::default();
        assert!(!tasks.is_hosting());
        assert!(!tasks.is_connected());
    }

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
        let handle = rt.spawn(async {});
        
        let mut tasks = NetworkTasks::default();
        tasks.server_task = Some((handle, cancel_token.clone()));
        
        assert!(tasks.is_hosting());
        tasks.stop_hosting();
        assert!(!tasks.is_hosting());
        assert!(cancel_token.is_cancelled());
    }

    #[test]
    fn test_disconnect() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cancel_token = CancellationToken::new();
        let handle = rt.spawn(async {});
        
        let mut tasks = NetworkTasks::default();
        tasks.client_task = Some((handle, cancel_token.clone()));
        
        assert!(tasks.is_connected());
        tasks.disconnect();
        assert!(!tasks.is_connected());
        assert!(cancel_token.is_cancelled());
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
}
