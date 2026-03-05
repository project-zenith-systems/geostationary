use bevy::prelude::*;

/// Lifecycle message for the world loading sequence.
///
/// Intended to be sent immediately before layer loading begins. Systems that
/// need to prepare resources before any [`MapLayer::load()`] runs can observe
/// this message.
///
/// [`WorldPlugin`] registers this message type but does not send it — the
/// world loader system is responsible for firing it at the appropriate point
/// in the startup sequence.
#[derive(Message, Debug, Clone)]
pub struct WorldLoading;

/// Lifecycle message fired once every registered layer has loaded successfully.
///
/// Intended to be sent after [`MapLayerRegistry::load_all`] completes.
/// Systems that need the fully-loaded world state (physics setup, atmos
/// initialisation, network readiness checks) should run after observing
/// this message.
///
/// [`WorldPlugin`] registers this message type but does not send it.
#[derive(Message, Debug, Clone)]
pub struct WorldReady;

/// Lifecycle message for world teardown (e.g. on `OnExit(AppState::InGame)`).
///
/// Intended to be sent before entities and layer resources are destroyed.
/// Systems that need to clean up their own state before teardown should
/// observe this message.
///
/// [`WorldPlugin`] registers this message type but does not send it.
#[derive(Message, Debug, Clone)]
pub struct WorldTeardown;
