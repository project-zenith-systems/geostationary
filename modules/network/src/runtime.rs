use std::sync::Arc;

use bevy::prelude::*;
use tokio::sync::mpsc;

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

    pub(crate) fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        self.rt.spawn(future);
    }
}

/// async → Bevy bridge: async tasks clone this sender to emit events.
#[derive(Resource, Clone)]
pub(crate) struct NetEventSender(pub(crate) mpsc::UnboundedSender<NetEvent>);

/// async → Bevy bridge: drained each frame in PreUpdate.
#[derive(Resource)]
pub(crate) struct NetEventReceiver(pub(crate) mpsc::UnboundedReceiver<NetEvent>);
