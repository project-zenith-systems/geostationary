use bevy::prelude::*;
use input::{PointerAction, WorldHit};
use items::{ItemDropRequest, ItemPickupRequest, ItemStoreRequest, ItemTakeRequest};
use network::{
    ClientId, ControlledByClient, Headless, NetId, Server, StreamDef, StreamDirection,
    StreamReader, StreamRegistry, StreamSender,
};
use things::NetIdIndex;
use tiles::{Tile, TileKind, TileMutated, Tilemap, TilesStreamMessage};
use ui::{UiTheme, WorldSpaceOverlay, build_button};
use wincode::{SchemaRead, SchemaWrite};

/// Stream tag for the client→server interactions stream (stream 4).
pub const INTERACTIONS_STREAM_TAG: u8 = 4;

/// Wire enum sent from client to server on stream 4.
///
/// Each variant corresponds to a player-initiated interaction request.
/// The server decodes this in [`dispatch_interaction`] and applies the
/// corresponding game logic.
#[derive(Message, Debug, Clone, SchemaRead, SchemaWrite)]
pub enum InteractionRequest {
    /// Request to change a tile at the given grid position to a new kind.
    TileToggle {
        position: [i32; 2],
        kind: TileKind,
    },
    /// Request to pick up an item from the world.
    ItemPickup { item: NetId },
    /// Request to drop a held item at the given world position.
    ItemDrop {
        item: NetId,
        drop_position: [f32; 3],
    },
    /// Request to store a held item into a container.
    StoreInContainer {
        item: NetId,
        container: NetId,
    },
    /// Request to take an item from a container into the player's hand.
    TakeFromContainer {
        item: NetId,
        container: NetId,
    },
}

/// Event fired when a context-menu action button is pressed.
///
/// Carries the target tile position and the [`TileKind`] to toggle the tile to.
/// Register this type with [`UiPlugin::with_event::<ContextMenuAction>()`] so
/// that the button press is forwarded as a Bevy event.
#[derive(Message, Clone, Copy, Debug)]
pub struct ContextMenuAction {
    /// Grid position of the tile to toggle.
    pub position: IVec2,
    /// The new tile kind to apply (e.g. `Floor` for "Remove Wall").
    pub kind: TileKind,
}

/// Resource that tracks the root entity of the currently-open context menu.
///
/// Present only while a menu is open.  Removed (and the entity despawned) when
/// the menu is dismissed by [`dismiss_context_menu`] or replaced by
/// [`build_context_menu`].
#[derive(Resource)]
struct ActiveMenu(Entity);

/// System that reads [`WorldHit`] events and spawns a right-click context menu.
///
/// - Dismisses any previously open menu before opening a new one (handles
///   the "right-click elsewhere" dismiss case).
/// - Looks up the hit entity's [`Tile`] position in the [`Tilemap`] to
///   determine available actions:
///   - `Wall`  → "Remove Wall" (toggles to `Floor`)
///   - `Floor` → "Build Wall"  (toggles to `Wall`)
///   - Non-tile entities produce no menu.
/// - Spawns a floating panel via [`WorldSpaceOverlay`] anchored to the hit
///   world position.
///
/// Gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn build_context_menu(
    mut commands: Commands,
    mut hit_events: MessageReader<WorldHit>,
    tile_query: Query<&Tile>,
    tilemap: Option<Res<Tilemap>>,
    active_menu: Option<Res<ActiveMenu>>,
    theme: Res<UiTheme>,
) {
    // Collect right-click hits to avoid borrowing issues inside the loop.
    let hits: Vec<WorldHit> = hit_events
        .read()
        .copied()
        .filter(|h| h.button == MouseButton::Right)
        .collect();
    if hits.is_empty() {
        return;
    }

    // Use the last hit if multiple arrive in the same frame (only one menu at a time).
    let hit = *hits.last().unwrap();

    // Dismiss any previously open menu.
    if let Some(menu) = active_menu.as_deref() {
        commands.entity(menu.0).despawn();
        commands.remove_resource::<ActiveMenu>();
    }

    let Some(ref tilemap) = tilemap else {
        return;
    };

    // Resolve the hit entity as a tile.
    let Ok(tile) = tile_query.get(hit.entity) else {
        return; // Thing or other entity → no menu.
    };

    let Some(kind) = tilemap.get(tile.position) else {
        return;
    };

    // Determine the label and target kind from the current tile state.
    let (label, target_kind) = match kind {
        TileKind::Wall => ("Remove Wall", TileKind::Floor),
        TileKind::Floor => ("Build Wall", TileKind::Wall),
    };

    let position = tile.position;

    // Build the action button.
    let button = build_button(&theme)
        .with_text(label)
        .with_event(ContextMenuAction {
            position,
            kind: target_kind,
        })
        .build(&mut commands);

    // Spawn the menu root: a floating panel positioned at the hit world location.
    let menu_root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(4.0)),
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(theme.background),
            WorldSpaceOverlay {
                world_pos: hit.world_pos,
            },
        ))
        .add_children(&[button])
        .id();

    commands.insert_resource(ActiveMenu(menu_root));
}

/// System that reads [`ContextMenuAction`] events and writes an [`InteractionRequest`]
/// message for the `send_interaction` system to send on stream 4.
///
/// The menu is dismissed by [`dismiss_context_menu`] on the same frame via the
/// left-click [`PointerAction`] that triggered the button press.
///
/// Gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn handle_menu_selection(
    mut actions: MessageReader<ContextMenuAction>,
    mut interaction_requests: MessageWriter<InteractionRequest>,
) {
    for action in actions.read() {
        interaction_requests.write(InteractionRequest::TileToggle {
            position: [action.position.x, action.position.y],
            kind: action.kind,
        });
    }
}

/// System that dismisses the active context menu.
///
/// Triggers on:
/// - `MouseButton::Left` click (including clicking an action button)
/// - [`KeyCode::Escape`]
///
/// Despawns the menu root entity (and all children) and removes the
/// [`ActiveMenu`] resource to prevent entity leaks.
///
/// Gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn dismiss_context_menu(
    mut commands: Commands,
    mut pointer_events: MessageReader<PointerAction>,
    keys: Res<ButtonInput<KeyCode>>,
    active_menu: Option<Res<ActiveMenu>>,
) {
    let dismiss = keys.just_pressed(KeyCode::Escape)
        || pointer_events
            .read()
            .any(|a| a.button == MouseButton::Left);

    if dismiss {
        if let Some(menu) = active_menu.as_deref() {
            commands.entity(menu.0).despawn();
            commands.remove_resource::<ActiveMenu>();
        }
    }
}

/// Client-side system that reads [`InteractionRequest`] messages and sends them to
/// the server on stream 4.
///
/// Runs in `Update`, gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn send_interaction(
    mut requests: MessageReader<InteractionRequest>,
    sender: Option<Res<StreamSender<InteractionRequest>>>,
) {
    let Some(ref s) = sender else {
        // Drain the queue even when disconnected so messages don't accumulate.
        for _ in requests.read() {}
        return;
    };
    for req in requests.read() {
        if let Err(e) = s.send(req) {
            error!("Failed to send InteractionRequest to server: {}", e);
        }
    }
}

/// Server-side system that drains [`InteractionRequest`] messages from stream 4.
///
/// - **`TileToggle`:** Validates the request (bounds check, no-op guard), applies
///   the mutation to [`Tilemap`], broadcasts [`TilesStreamMessage::TileMutated`] on
///   stream 1 to all clients, and fires a local [`TileMutated`] Bevy event so the
///   listen-server's [`apply_tile_mutation`] system can update visuals.
/// - **Item operations:** Resolves actor and item entities from [`NetIdIndex`] and
///   fires the corresponding server-side Bevy request events
///   ([`ItemPickupRequest`], [`ItemDropRequest`], [`ItemStoreRequest`],
///   [`ItemTakeRequest`]).
///
/// Runs in `Update`, gated on [`Server`] resource.
fn dispatch_interaction(
    mut reader: ResMut<StreamReader<InteractionRequest>>,
    mut tilemap: Option<ResMut<Tilemap>>,
    tiles_sender: Option<Res<StreamSender<TilesStreamMessage>>>,
    mut mutation_events: MessageWriter<TileMutated>,
    net_id_index: Option<Res<NetIdIndex>>,
    actor_query: Query<(Entity, &ControlledByClient)>,
    mut pickup_req: MessageWriter<ItemPickupRequest>,
    mut drop_req: MessageWriter<ItemDropRequest>,
    mut store_req: MessageWriter<ItemStoreRequest>,
    mut take_req: MessageWriter<ItemTakeRequest>,
) {
    for (from, request) in reader.drain_from_client() {
        match request {
            InteractionRequest::TileToggle { position, kind } => {
                let pos = IVec2::new(position[0], position[1]);

                let Some(ref mut tm) = tilemap else {
                    warn!("dispatch_interaction TileToggle: Tilemap resource not available");
                    continue;
                };

                // Validate: position must be within the tilemap bounds.
                let Some(current) = tm.get(pos) else {
                    warn!(
                        "TileToggle from {:?}: position {:?} is out of bounds",
                        from, pos
                    );
                    continue;
                };

                // Validate: requested kind must differ from the current tile.
                if current == kind {
                    debug!(
                        "TileToggle from {:?}: tile at {:?} is already {:?}, ignoring",
                        from, pos, kind
                    );
                    continue;
                }

                tm.set(pos, kind);

                // Fire local Bevy event so the listen-server updates its own visuals.
                mutation_events.write(TileMutated { position: pos, kind });

                // Broadcast the mutation to all connected clients on stream 1.
                let Some(ref ts) = tiles_sender else {
                    error!("dispatch_interaction: tiles stream sender not available");
                    continue;
                };
                if let Err(e) = ts.broadcast(&TilesStreamMessage::TileMutated {
                    position,
                    kind,
                }) {
                    error!("Failed to broadcast TileMutated: {}", e);
                }
            }

            InteractionRequest::ItemPickup { item: item_id } => {
                let Some(ref idx) = net_id_index else {
                    warn!("dispatch_interaction ItemPickup: NetIdIndex not available");
                    continue;
                };
                let Some(&item) = idx.0.get(&item_id) else {
                    warn!("dispatch_interaction ItemPickup: unknown item NetId {:?}", item_id);
                    continue;
                };
                let Some(actor) = resolve_actor(&actor_query, from) else {
                    warn!("dispatch_interaction ItemPickup: no actor for client {:?}", from);
                    continue;
                };
                pickup_req.write(ItemPickupRequest { actor, item });
            }

            InteractionRequest::ItemDrop { item: item_id, drop_position } => {
                let Some(ref idx) = net_id_index else {
                    warn!("dispatch_interaction ItemDrop: NetIdIndex not available");
                    continue;
                };
                let Some(&item) = idx.0.get(&item_id) else {
                    warn!("dispatch_interaction ItemDrop: unknown item NetId {:?}", item_id);
                    continue;
                };
                let Some(actor) = resolve_actor(&actor_query, from) else {
                    warn!("dispatch_interaction ItemDrop: no actor for client {:?}", from);
                    continue;
                };
                drop_req.write(ItemDropRequest {
                    actor,
                    item,
                    drop_position: Vec3::from(drop_position),
                });
            }

            InteractionRequest::StoreInContainer { item: item_id, container: container_id } => {
                let Some(ref idx) = net_id_index else {
                    warn!("dispatch_interaction StoreInContainer: NetIdIndex not available");
                    continue;
                };
                let Some(&item) = idx.0.get(&item_id) else {
                    warn!("dispatch_interaction StoreInContainer: unknown item NetId {:?}", item_id);
                    continue;
                };
                let Some(&container) = idx.0.get(&container_id) else {
                    warn!("dispatch_interaction StoreInContainer: unknown container NetId {:?}", container_id);
                    continue;
                };
                let Some(actor) = resolve_actor(&actor_query, from) else {
                    warn!("dispatch_interaction StoreInContainer: no actor for client {:?}", from);
                    continue;
                };
                store_req.write(ItemStoreRequest { actor, item, container });
            }

            InteractionRequest::TakeFromContainer { item: item_id, container: container_id } => {
                let Some(ref idx) = net_id_index else {
                    warn!("dispatch_interaction TakeFromContainer: NetIdIndex not available");
                    continue;
                };
                let Some(&item) = idx.0.get(&item_id) else {
                    warn!("dispatch_interaction TakeFromContainer: unknown item NetId {:?}", item_id);
                    continue;
                };
                let Some(&container) = idx.0.get(&container_id) else {
                    warn!("dispatch_interaction TakeFromContainer: unknown container NetId {:?}", container_id);
                    continue;
                };
                let Some(actor) = resolve_actor(&actor_query, from) else {
                    warn!("dispatch_interaction TakeFromContainer: no actor for client {:?}", from);
                    continue;
                };
                take_req.write(ItemTakeRequest { actor, item, container });
            }
        }
    }
}

/// Resolves the entity controlled by `client` from the actor query.
fn resolve_actor(
    actor_query: &Query<(Entity, &ControlledByClient)>,
    client: ClientId,
) -> Option<Entity> {
    actor_query
        .iter()
        .find(|(_, ctrl)| ctrl.0 == client)
        .map(|(e, _)| e)
}

/// Plugin that wires up the right-click context-menu system and interaction stream.
///
/// All UI systems are gated on the provided game state and on the absence of the
/// [`Headless`] resource (context menus are client-only).
///
/// The [`dispatch_interaction`] system is gated on [`Server`] resource and runs
/// on both dedicated servers and listen-servers.
///
/// Register the context-menu button event type in `main.rs`:
/// ```ignore
/// UiPlugin::new()
///     .with_event::<MenuEvent>()
///     .with_event::<ContextMenuAction>()
/// ```
pub struct InteractionsPlugin<S: States + Copy> {
    state: S,
}

impl<S: States + Copy> InteractionsPlugin<S> {
    /// Creates the plugin gated on `state`.
    pub fn in_state(state: S) -> Self {
        Self { state }
    }
}

impl<S: States + Copy> Plugin for InteractionsPlugin<S> {
    fn build(&self, app: &mut App) {
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();

        let state = self.state;
        app.add_systems(
            Update,
            (
                dismiss_context_menu,
                build_context_menu.after(dismiss_context_menu),
                handle_menu_selection,
                send_interaction.after(handle_menu_selection),
            )
                .run_if(in_state(state))
                .run_if(not(resource_exists::<Headless>)),
        );

        app.add_systems(
            Update,
            dispatch_interaction
                .run_if(in_state(state))
                .run_if(resource_exists::<Server>),
        );

        // Register stream 4 (client→server interactions stream).
        // Requires NetworkPlugin to be added first.
        let mut registry = app.world_mut().get_resource_mut::<StreamRegistry>().expect(
            "InteractionsPlugin requires NetworkPlugin to be added before it (StreamRegistry not found)",
        );
        let (sender, reader): (
            StreamSender<InteractionRequest>,
            StreamReader<InteractionRequest>,
        ) = registry.register(StreamDef {
            tag: INTERACTIONS_STREAM_TAG,
            name: "interactions",
            direction: StreamDirection::ClientToServer,
        });
        app.insert_resource(sender);
        app.insert_resource(reader);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that [`build_context_menu`] opens a menu when a [`WorldHit`]
    /// targeting a wall tile is received, and that the [`ActiveMenu`] resource
    /// is inserted.
    #[test]
    fn build_context_menu_inserts_active_menu_for_wall() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorldHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();

        // Spawn a wall tile entity.
        let tile_entity = app
            .world_mut()
            .spawn(Tile {
                position: IVec2::new(1, 1),
            })
            .id();

        // Insert a tilemap with that tile as a wall.
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        app.insert_resource(tilemap);

        // Emit a WorldHit targeting the tile entity.
        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(WorldHit {
                button: MouseButton::Right,
                entity: tile_entity,
                world_pos: Vec3::new(1.0, 0.0, 1.0),
            });

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu resource should be present after a WorldHit on a wall"
        );
    }

    /// Verifies that [`build_context_menu`] does NOT open a menu when the hit
    /// entity is not a tile (e.g. a `Thing`).
    #[test]
    fn build_context_menu_ignores_non_tile_entity() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorldHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();

        // Spawn an entity WITHOUT a Tile component.
        let non_tile = app.world_mut().spawn_empty().id();

        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        app.insert_resource(tilemap);

        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(WorldHit {
                button: MouseButton::Right,
                entity: non_tile,
                world_pos: Vec3::ZERO,
            });

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            !app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu resource should NOT be present for a non-tile hit"
        );
    }

    /// Verifies that [`build_context_menu`] ignores left-click [`WorldHit`] events.
    #[test]
    fn build_context_menu_ignores_left_click_hit() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorldHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();

        // Spawn a wall tile entity.
        let tile_entity = app
            .world_mut()
            .spawn(Tile {
                position: IVec2::new(1, 1),
            })
            .id();

        // Insert a tilemap with that tile as a wall.
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        app.insert_resource(tilemap);

        // Emit a left-click WorldHit targeting the tile entity.
        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(WorldHit {
                button: MouseButton::Left,
                entity: tile_entity,
                world_pos: Vec3::new(1.0, 0.0, 1.0),
            });

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            !app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu resource should NOT be present for a left-click WorldHit"
        );
    }

    /// Verifies that [`dismiss_context_menu`] removes the [`ActiveMenu`]
    /// resource when Escape is pressed.
    #[test]
    fn dismiss_context_menu_on_escape() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<PointerAction>();
        app.init_resource::<ButtonInput<KeyCode>>();

        // Spawn a dummy menu root entity.
        let menu_entity = app.world_mut().spawn_empty().id();
        app.insert_resource(ActiveMenu(menu_entity));

        // Press Escape.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);

        app.add_systems(Update, dismiss_context_menu);
        app.update();

        assert!(
            !app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu should be removed after Escape is pressed"
        );
    }

    /// Verifies that [`handle_menu_selection`] fires an [`InteractionRequest::TileToggle`]
    /// when a [`ContextMenuAction`] event is received.
    #[test]
    fn handle_menu_selection_fires_interaction_request() {
        #[derive(Resource, Default)]
        struct Captured(Vec<InteractionRequest>);

        fn capture(
            mut reader: MessageReader<InteractionRequest>,
            mut captured: ResMut<Captured>,
        ) {
            for req in reader.read() {
                captured.0.push(req.clone());
            }
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<Captured>();

        app.world_mut()
            .resource_mut::<Messages<ContextMenuAction>>()
            .write(ContextMenuAction {
                position: IVec2::new(2, 3),
                kind: TileKind::Floor,
            });

        app.add_systems(Update, (handle_menu_selection, capture.after(handle_menu_selection)));
        app.update();

        let captured = app.world().resource::<Captured>();
        assert_eq!(captured.0.len(), 1);
        match captured.0[0] {
            InteractionRequest::TileToggle { position, kind } => {
                assert_eq!(position, [2, 3]);
                assert_eq!(kind, TileKind::Floor);
            }
            ref other => panic!("expected TileToggle, got {:?}", other),
        }
    }

    /// Verifies that [`dispatch_interaction`] handles a `TileToggle` correctly:
    /// mutates the tilemap, fires a [`TileMutated`] event, and marks the
    /// stream reader as drained.
    #[test]
    fn dispatch_interaction_handles_tile_toggle() {
        use network::{StreamRegistry, StreamDef, StreamDirection};

        #[derive(Resource, Default)]
        struct CapturedMutations(Vec<TileMutated>);

        fn capture_mutations(
            mut reader: MessageReader<TileMutated>,
            mut captured: ResMut<CapturedMutations>,
        ) {
            captured.0.extend(reader.read().copied());
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<StreamRegistry>();
        app.add_message::<InteractionRequest>();
        app.add_message::<TileMutated>();
        app.add_message::<ItemPickupRequest>();
        app.add_message::<ItemDropRequest>();
        app.add_message::<ItemStoreRequest>();
        app.add_message::<ItemTakeRequest>();
        app.init_resource::<CapturedMutations>();

        // Register stream 4 so StreamReader<InteractionRequest> exists.
        let (sender, reader): (
            StreamSender<InteractionRequest>,
            StreamReader<InteractionRequest>,
        ) = app
            .world_mut()
            .resource_mut::<StreamRegistry>()
            .register(StreamDef {
                tag: INTERACTIONS_STREAM_TAG,
                name: "interactions",
                direction: StreamDirection::ClientToServer,
            });
        app.insert_resource(sender);
        app.insert_resource(reader);

        // Register stream 1 so StreamSender<TilesStreamMessage> exists.
        let (tiles_sender, tiles_reader): (
            StreamSender<TilesStreamMessage>,
            StreamReader<TilesStreamMessage>,
        ) = app
            .world_mut()
            .resource_mut::<StreamRegistry>()
            .register(StreamDef {
                tag: tiles::TILES_STREAM_TAG,
                name: "tiles",
                direction: StreamDirection::ServerToClient,
            });
        app.insert_resource(tiles_sender);
        app.insert_resource(tiles_reader);

        // Insert a tilemap with a wall at (1, 1).
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        app.insert_resource(tilemap);

        // Inject a TileToggle InteractionRequest directly into the reader buffer.
        let from = ClientId(42);
        let request = InteractionRequest::TileToggle {
            position: [1, 1],
            kind: TileKind::Floor,
        };
        let bytes = wincode::serialize(&request).expect("serialize");
        {
            use bytes::Bytes;
            app.world_mut()
                .resource_mut::<StreamRegistry>()
                .route_client_stream_frame(from, INTERACTIONS_STREAM_TAG, Bytes::from(bytes));
        }

        app.add_systems(Update, (dispatch_interaction, capture_mutations.after(dispatch_interaction)));
        app.update();

        // Tilemap should now have Floor at (1, 1).
        let tilemap = app.world().resource::<Tilemap>();
        assert_eq!(
            tilemap.get(IVec2::new(1, 1)),
            Some(TileKind::Floor),
            "Tilemap should be mutated to Floor after TileToggle"
        );

        // TileMutated event should have been fired.
        let captured = app.world().resource::<CapturedMutations>();
        assert_eq!(captured.0.len(), 1, "Expected one TileMutated event");
        assert_eq!(captured.0[0].position, IVec2::new(1, 1));
        assert_eq!(captured.0[0].kind, TileKind::Floor);
    }

    /// Verifies that [`dispatch_interaction`] rejects a `TileToggle` when the
    /// tile is already the requested kind (no-op guard).
    #[test]
    fn dispatch_interaction_rejects_no_op_tile_toggle() {
        use network::{StreamRegistry, StreamDef, StreamDirection};

        #[derive(Resource, Default)]
        struct CapturedMutations(Vec<TileMutated>);

        fn capture_mutations(
            mut reader: MessageReader<TileMutated>,
            mut captured: ResMut<CapturedMutations>,
        ) {
            captured.0.extend(reader.read().copied());
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<StreamRegistry>();
        app.add_message::<InteractionRequest>();
        app.add_message::<TileMutated>();
        app.add_message::<ItemPickupRequest>();
        app.add_message::<ItemDropRequest>();
        app.add_message::<ItemStoreRequest>();
        app.add_message::<ItemTakeRequest>();
        app.init_resource::<CapturedMutations>();

        let (sender, reader): (
            StreamSender<InteractionRequest>,
            StreamReader<InteractionRequest>,
        ) = app
            .world_mut()
            .resource_mut::<StreamRegistry>()
            .register(StreamDef {
                tag: INTERACTIONS_STREAM_TAG,
                name: "interactions",
                direction: StreamDirection::ClientToServer,
            });
        app.insert_resource(sender);
        app.insert_resource(reader);

        let (tiles_sender, tiles_reader): (
            StreamSender<TilesStreamMessage>,
            StreamReader<TilesStreamMessage>,
        ) = app
            .world_mut()
            .resource_mut::<StreamRegistry>()
            .register(StreamDef {
                tag: tiles::TILES_STREAM_TAG,
                name: "tiles",
                direction: StreamDirection::ServerToClient,
            });
        app.insert_resource(tiles_sender);
        app.insert_resource(tiles_reader);

        // Tilemap with Floor at (1, 1) — request also asks for Floor → no-op.
        let tilemap = Tilemap::new(3, 3, TileKind::Floor);
        app.insert_resource(tilemap);

        let from = ClientId(1);
        let request = InteractionRequest::TileToggle {
            position: [1, 1],
            kind: TileKind::Floor,
        };
        let bytes = wincode::serialize(&request).expect("serialize");
        {
            use bytes::Bytes;
            app.world_mut()
                .resource_mut::<StreamRegistry>()
                .route_client_stream_frame(from, INTERACTIONS_STREAM_TAG, Bytes::from(bytes));
        }

        app.add_systems(Update, (dispatch_interaction, capture_mutations.after(dispatch_interaction)));
        app.update();

        // Tilemap unchanged.
        let tilemap = app.world().resource::<Tilemap>();
        assert_eq!(tilemap.get(IVec2::new(1, 1)), Some(TileKind::Floor));

        // No TileMutated event.
        let captured = app.world().resource::<CapturedMutations>();
        assert_eq!(captured.0.len(), 0, "No mutation event expected for no-op toggle");
    }

    /// Verifies that [`resolve_actor`] returns the correct entity for a matching
    /// client and `None` when no entity is controlled by the given client.
    #[test]
    fn resolve_actor_finds_matching_entity_and_returns_none_for_unknown() {
        #[derive(Resource, Default)]
        struct Result {
            found: Option<Entity>,
            not_found: bool,
        }

        let client_a = ClientId(10);
        let unknown = ClientId(99);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Result>();

        let entity_a = app.world_mut().spawn(ControlledByClient(client_a)).id();
        app.world_mut().spawn(ControlledByClient(ClientId(20)));

        app.add_systems(Update, move |q: Query<(Entity, &ControlledByClient)>, mut res: ResMut<Result>| {
            res.found = resolve_actor(&q, client_a);
            res.not_found = resolve_actor(&q, unknown).is_none();
        });
        app.update();

        let result = app.world().resource::<Result>();
        assert_eq!(result.found, Some(entity_a), "Should find entity for client_a");
        assert!(result.not_found, "Should return None for unknown client");
    }
}

