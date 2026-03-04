use bevy::prelude::*;

/// Fired by [`WorldPlugin`] immediately before layer loading begins.
/// Systems that need to prepare resources before any [`MapLayer::load()`]
/// runs can observe this event.
#[derive(Message, Debug, Clone)]
pub struct WorldLoading;

/// Fired by [`WorldPlugin`] once every registered layer has been loaded
/// successfully. Systems that need the fully-loaded world state (physics
/// setup, atmos initialisation, network readiness checks) should run after
/// observing this event.
#[derive(Message, Debug, Clone)]
pub struct WorldReady;

/// Fired by [`WorldPlugin`] when the world is being torn down (e.g. on
/// `OnExit(AppState::InGame)`). Systems that need to clean up their own
/// resources before entities and layers are destroyed should respond here.
#[derive(Message, Debug, Clone)]
pub struct WorldTeardown;
