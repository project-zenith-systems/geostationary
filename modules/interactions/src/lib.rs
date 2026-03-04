use bevy::prelude::*;
use input::{PointerAction, WorldHit};
use items::{Container, InteractionRange, Item, ItemDropRequest, ItemPickupRequest, ItemStoreRequest, ItemTakeRequest};
use network::{
    ClientId, ControlledByClient, Headless, NetId, Server, StreamDef, StreamDirection,
    StreamReader, StreamRegistry, StreamSender,
};
use things::{DisplayName, HandSlot, NetIdIndex, PlayerControlled};
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
/// Each variant encodes enough information for [`handle_menu_selection`] to build
/// the corresponding [`InteractionRequest`] without further world queries.
/// Register this type with [`UiPlugin::with_event::<ContextMenuAction>()`] so
/// that the button press is forwarded as a Bevy event.
#[derive(Message, Clone, Copy, Debug)]
pub enum ContextMenuAction {
    /// Toggle a tile to a new kind (e.g. "Build Wall" / "Remove Wall").
    TileToggle {
        position: IVec2,
        kind: TileKind,
    },
    /// Pick up the identified item from the world.
    ItemPickup { item: NetId },
    /// Drop the held item at the given world position.
    ItemDrop { item: NetId, drop_position: Vec3 },
    /// Store the held item into a container.
    StoreInContainer { item: NetId, container: NetId },
    /// Take an item from a container into the player's hand.
    TakeFromContainer { item: NetId, container: NetId },
}

/// The single best [`WorldHit`] for a frame, resolved by [`resolve_world_hits`].
///
/// Downstream systems (`default_interaction`, `build_context_menu`) read this
/// instead of raw [`WorldHit`] events so that only the closest-to-camera hit is
/// acted on when multiple raycasters fire in the same frame.
#[derive(Message, Clone, Copy, Debug)]
pub struct ResolvedHit {
    pub hit: WorldHit,
}

/// Resource that tracks the root entity of the currently-open context menu.
///
/// Present only while a menu is open.  Removed (and the entity despawned) when
/// the menu is dismissed by [`dismiss_context_menu`] or replaced by
/// [`build_context_menu`].
#[derive(Resource)]
struct ActiveMenu(Entity);

/// System that collects all [`WorldHit`] messages in a frame and emits a single
/// [`ResolvedHit`] per mouse button containing the hit closest to the camera.
///
/// Both `raycast_tiles` and `raycast_things` emit [`WorldHit`] independently; this
/// system acts as the tie-breaker so downstream logic only ever sees one winner per
/// click.
///
/// This system should be scheduled to run after the raycast systems and is typically
/// gated on `not(resource_exists::<Headless>)` by the plugin that registers it.
fn resolve_world_hits(
    mut hit_events: MessageReader<WorldHit>,
    mut resolved: MessageWriter<ResolvedHit>,
    camera_q: Query<&GlobalTransform, With<Camera3d>>,
) {
    let hits: Vec<WorldHit> = hit_events.read().copied().collect();
    if hits.is_empty() {
        return;
    }

    let Ok(cam_tf) = camera_q.single() else {
        warn!("resolve_world_hits: expected exactly one Camera3d, found {}", camera_q.iter().count());
        return;
    };
    let cam_pos = cam_tf.translation();

    // For each button that produced hits, pick the closest-to-camera hit.
    for button in [MouseButton::Left, MouseButton::Right] {
        let closest = hits
            .iter()
            .filter(|h| h.button == button)
            .min_by(|a, b| {
                let da = cam_pos.distance_squared(a.world_pos);
                let db = cam_pos.distance_squared(b.world_pos);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(&hit) = closest {
            resolved.write(ResolvedHit { hit });
        }
    }
}

/// System that handles the left-click default action on a [`ResolvedHit`].
///
/// If the hit entity has an [`Item`] component, sends
/// `InteractionRequest::ItemPickup { item }` on stream 4.  All other entity types
/// are silently ignored.
///
/// Gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn default_interaction(
    mut resolved: MessageReader<ResolvedHit>,
    item_q: Query<&NetId, With<Item>>,
    mut requests: MessageWriter<InteractionRequest>,
) {
    for r in resolved.read() {
        if r.hit.button != MouseButton::Left {
            continue;
        }
        if let Ok(&net_id) = item_q.get(r.hit.entity) {
            requests.write(InteractionRequest::ItemPickup { item: net_id });
        }
    }
}

/// Returns the [`NetId`] of the first item held in any [`HandSlot`] owned by
/// `player`, or `None` when the hands are empty.
fn held_item(
    player: Entity,
    children_q: &Query<&Children>,
    hand_container_q: &Query<&Container, With<HandSlot>>,
    net_id_q: &Query<&NetId>,
) -> Option<NetId> {
    let children = children_q.get(player).ok()?;
    for child in children.iter() {
        if let Ok(container) = hand_container_q.get(child) {
            for slot in &container.slots {
                if let Some(item_entity) = slot {
                    if let Ok(&net_id) = net_id_q.get(*item_entity) {
                        return Some(net_id);
                    }
                }
            }
        }
    }
    None
}

/// System that reads right-click [`ResolvedHit`] events and spawns a context menu.
///
/// Actions depend on what was hit and whether the local player is holding an item:
/// - `Item` entity → "Pick up"
/// - `Container` entity, hand empty, container has items → "Take from {name}"
/// - `Container` entity, hand holding item → "Store in {name}"
/// - `Tile(Wall)` → "Remove Wall"
/// - `Tile(Floor)`, hand empty → "Build Wall"
/// - `Tile(Floor)`, hand holding item → "Drop", "Build Wall"
///
/// Gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn build_context_menu(
    mut commands: Commands,
    mut resolved_hits: MessageReader<ResolvedHit>,
    tile_query: Query<&Tile>,
    item_q: Query<(), With<Item>>,
    world_container_q: Query<(&NetId, &Container, Option<&DisplayName>), Without<HandSlot>>,
    tilemap: Option<Res<Tilemap>>,
    active_menu: Option<Res<ActiveMenu>>,
    theme: Res<UiTheme>,
    player_q: Query<(Entity, &GlobalTransform), With<PlayerControlled>>,
    children_q: Query<&Children>,
    hand_container_q: Query<&Container, With<HandSlot>>,
    net_id_q: Query<&NetId>,
    interaction_range: Res<InteractionRange>,
) {
    // Collect right-click resolved hits.
    let hits: Vec<ResolvedHit> = resolved_hits
        .read()
        .copied()
        .filter(|r| r.hit.button == MouseButton::Right)
        .collect();
    if hits.is_empty() {
        return;
    }

    // Only one menu at a time; use the last hit if multiple arrive.
    let resolved = *hits.last().unwrap();
    let hit = resolved.hit;

    // Dismiss any previously open menu.
    if let Some(menu) = active_menu.as_deref() {
        commands.entity(menu.0).despawn();
        commands.remove_resource::<ActiveMenu>();
    }

    // Determine whether the local player is holding an item and their position.
    let player = player_q.single().ok();
    let (player_entity, player_pos) = match player {
        Some((e, gt)) => (Some(e), Some(gt.translation())),
        None => (None, None),
    };
    let holding: Option<NetId> = player_entity.and_then(|p| {
        held_item(p, &children_q, &hand_container_q, &net_id_q)
    });
    let in_range = player_pos
        .map(|pos| pos.distance(hit.world_pos) <= interaction_range.0)
        .unwrap_or(false);

    // Collect action buttons for this hit.
    let mut buttons: Vec<Entity> = Vec::new();

    if item_q.get(hit.entity).is_ok() {
        // Hit an Item on the floor → "Pick up" (if in range).
        if in_range {
            if let Ok(&item_net_id) = net_id_q.get(hit.entity) {
                let btn = build_button(&theme)
                    .with_text("Pick up")
                    .with_event(ContextMenuAction::ItemPickup { item: item_net_id })
                    .build(&mut commands);
                buttons.push(btn);
            } else {
                warn!("build_context_menu: Item entity {:?} has no NetId", hit.entity);
            }
        }
    } else if let Ok((&container_net_id, container, display_name)) =
        world_container_q.get(hit.entity)
    {
        if in_range {
            let name = display_name.map(|d| d.0.as_str()).unwrap_or("container");
            if let Some(held_net_id) = holding {
                // Holding item + container → "Store in {name}".
                let label = format!("Store in {name}");
                let btn = build_button(&theme)
                    .with_text(&label)
                    .with_event(ContextMenuAction::StoreInContainer {
                        item: held_net_id,
                        container: container_net_id,
                    })
                    .build(&mut commands);
                buttons.push(btn);
            } else {
                // Hand empty + container has items → "Take from {name}".
                let first_item = container.slots.iter().find_map(|s| *s);
                if let Some(item_entity) = first_item {
                    if let Ok(&item_net_id) = net_id_q.get(item_entity) {
                        let label = format!("Take from {name}");
                        let btn = build_button(&theme)
                            .with_text(&label)
                            .with_event(ContextMenuAction::TakeFromContainer {
                                item: item_net_id,
                                container: container_net_id,
                            })
                            .build(&mut commands);
                        buttons.push(btn);
                    }
                }
            }
        }
    } else if let Ok(tile) = tile_query.get(hit.entity) {
        let Some(ref tilemap) = tilemap else { return };
        let Some(kind) = tilemap.get(tile.position) else { return };
        let position = tile.position;
        match kind {
            TileKind::Wall => {
                let btn = build_button(&theme)
                    .with_text("Remove Wall")
                    .with_event(ContextMenuAction::TileToggle {
                        position,
                        kind: TileKind::Floor,
                    })
                    .build(&mut commands);
                buttons.push(btn);
            }
            TileKind::Floor => {
                if let Some(held_net_id) = holding {
                    if in_range {
                        let drop_btn = build_button(&theme)
                            .with_text("Drop")
                            .with_event(ContextMenuAction::ItemDrop {
                                item: held_net_id,
                                drop_position: hit.world_pos,
                            })
                            .build(&mut commands);
                        buttons.push(drop_btn);
                    }
                }
                let wall_btn = build_button(&theme)
                    .with_text("Build Wall")
                    .with_event(ContextMenuAction::TileToggle {
                        position,
                        kind: TileKind::Wall,
                    })
                    .build(&mut commands);
                buttons.push(wall_btn);
            }
        }
    }

    if buttons.is_empty() {
        return;
    }

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
        .add_children(&buttons)
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
        let req = match *action {
            ContextMenuAction::TileToggle { position, kind } => {
                InteractionRequest::TileToggle {
                    position: [position.x, position.y],
                    kind,
                }
            }
            ContextMenuAction::ItemPickup { item } => InteractionRequest::ItemPickup { item },
            ContextMenuAction::ItemDrop { item, drop_position } => InteractionRequest::ItemDrop {
                item,
                drop_position: drop_position.to_array(),
            },
            ContextMenuAction::StoreInContainer { item, container } => {
                InteractionRequest::StoreInContainer { item, container }
            }
            ContextMenuAction::TakeFromContainer { item, container } => {
                InteractionRequest::TakeFromContainer { item, container }
            }
        };
        interaction_requests.write(req);
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

            // All item variants require NetIdIndex and an actor.
            InteractionRequest::ItemPickup { item: item_id } => {
                let Some(ref idx) = net_id_index else {
                    warn!("dispatch_interaction: NetIdIndex not available for item request");
                    continue;
                };
                let Some(&item) = idx.0.get(&item_id) else {
                    warn!("dispatch_interaction ItemPickup: unknown NetId {:?}", item_id);
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
                    warn!("dispatch_interaction: NetIdIndex not available for item request");
                    continue;
                };
                let Some(&item) = idx.0.get(&item_id) else {
                    warn!("dispatch_interaction ItemDrop: unknown NetId {:?}", item_id);
                    continue;
                };
                let Some(actor) = resolve_actor(&actor_query, from) else {
                    warn!("dispatch_interaction ItemDrop: no actor for client {:?}", from);
                    continue;
                };
                let pos = Vec3::from_array(drop_position);
                drop_req.write(ItemDropRequest {
                    actor,
                    item,
                    drop_position: pos,
                });
            }

            InteractionRequest::StoreInContainer { item: item_id, container: container_id } => {
                let Some(ref idx) = net_id_index else {
                    warn!("dispatch_interaction: NetIdIndex not available for item request");
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
                    warn!("dispatch_interaction: NetIdIndex not available for item request");
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
        app.add_message::<PointerAction>();
        app.add_message::<WorldHit>();
        app.add_message::<ResolvedHit>();

        let state = self.state;
        app.add_systems(
            Update,
            (
                resolve_world_hits,
                default_interaction.after(resolve_world_hits),
                dismiss_context_menu,
                build_context_menu
                    .after(dismiss_context_menu)
                    .after(resolve_world_hits),
                handle_menu_selection.after(build_context_menu),
                send_interaction
                    .after(default_interaction)
                    .after(handle_menu_selection),
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

    /// Verifies that [`build_context_menu`] opens a menu when a [`ResolvedHit`]
    /// targeting a wall tile is received, and that the [`ActiveMenu`] resource
    /// is inserted.
    #[test]
    fn build_context_menu_inserts_active_menu_for_wall() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ResolvedHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();
        app.init_resource::<InteractionRange>();

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

        // Emit a ResolvedHit targeting the tile entity.
        app.world_mut()
            .resource_mut::<Messages<ResolvedHit>>()
            .write(ResolvedHit {
                hit: WorldHit {
                    button: MouseButton::Right,
                    entity: tile_entity,
                    world_pos: Vec3::new(1.0, 0.0, 1.0),
                },
            });

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu resource should be present after a ResolvedHit on a wall"
        );
    }

    /// Verifies that [`build_context_menu`] does NOT open a menu when the hit
    /// entity is not a tile, item, or container.
    #[test]
    fn build_context_menu_ignores_non_tile_entity() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ResolvedHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();
        app.init_resource::<InteractionRange>();

        // Spawn an entity WITHOUT a Tile, Item, or Container component.
        let non_tile = app.world_mut().spawn_empty().id();

        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        app.insert_resource(tilemap);

        app.world_mut()
            .resource_mut::<Messages<ResolvedHit>>()
            .write(ResolvedHit {
                hit: WorldHit {
                    button: MouseButton::Right,
                    entity: non_tile,
                    world_pos: Vec3::ZERO,
                },
            });

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            !app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu resource should NOT be present for a non-tile, non-item, non-container hit"
        );
    }

    /// Verifies that [`build_context_menu`] ignores left-click [`ResolvedHit`] events.
    #[test]
    fn build_context_menu_ignores_left_click_hit() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ResolvedHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();
        app.init_resource::<InteractionRange>();

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

        // Emit a left-click ResolvedHit targeting the tile entity.
        app.world_mut()
            .resource_mut::<Messages<ResolvedHit>>()
            .write(ResolvedHit {
                hit: WorldHit {
                    button: MouseButton::Left,
                    entity: tile_entity,
                    world_pos: Vec3::new(1.0, 0.0, 1.0),
                },
            });

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            !app.world().contains_resource::<ActiveMenu>(),
            "ActiveMenu resource should NOT be present for a left-click ResolvedHit"
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
            .write(ContextMenuAction::TileToggle {
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

    // ── resolve_world_hits ──────────────────────────────────────────────────

    fn make_world_hit(button: MouseButton, entity: Entity, world_pos: Vec3) -> WorldHit {
        WorldHit { button, entity, world_pos }
    }

    fn make_resolve_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorldHit>();
        app.add_message::<ResolvedHit>();
        // Spawn a Camera3d at the origin so resolve_world_hits can query it.
        app.world_mut().spawn((Camera3d::default(), GlobalTransform::IDENTITY));
        app
    }

    /// A single [`WorldHit`] is forwarded unchanged as a [`ResolvedHit`].
    #[test]
    fn resolve_world_hits_single_hit_is_resolved() {
        #[derive(Resource, Default)]
        struct Captured(Vec<ResolvedHit>);

        let mut app = make_resolve_app();
        app.init_resource::<Captured>();

        let entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(make_world_hit(MouseButton::Left, entity, Vec3::new(0.0, 0.0, 3.0)));

        app.add_systems(Update, (
            resolve_world_hits,
            (|mut r: MessageReader<ResolvedHit>, mut c: ResMut<Captured>| {
                c.0.extend(r.read().copied());
            })
            .after(resolve_world_hits),
        ));
        app.update();

        let captured = app.world().resource::<Captured>();
        assert_eq!(captured.0.len(), 1, "Expected one ResolvedHit");
        assert_eq!(captured.0[0].hit.entity, entity);
        assert_eq!(captured.0[0].hit.button, MouseButton::Left);
    }

    /// When multiple [`WorldHit`]s arrive for the same button, the one closest
    /// to the camera is chosen.
    #[test]
    fn resolve_world_hits_picks_closest_when_multiple() {
        #[derive(Resource, Default)]
        struct Captured(Vec<ResolvedHit>);

        let mut app = make_resolve_app();
        app.init_resource::<Captured>();

        let near_entity = app.world_mut().spawn_empty().id();
        let far_entity = app.world_mut().spawn_empty().id();

        // Camera is at the origin; near_entity is closer.
        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(make_world_hit(MouseButton::Left, near_entity, Vec3::new(0.0, 0.0, 2.0)));
        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(make_world_hit(MouseButton::Left, far_entity, Vec3::new(0.0, 0.0, 10.0)));

        app.add_systems(Update, (
            resolve_world_hits,
            (|mut r: MessageReader<ResolvedHit>, mut c: ResMut<Captured>| {
                c.0.extend(r.read().copied());
            })
            .after(resolve_world_hits),
        ));
        app.update();

        let captured = app.world().resource::<Captured>();
        assert_eq!(captured.0.len(), 1, "Expected exactly one ResolvedHit");
        assert_eq!(
            captured.0[0].hit.entity, near_entity,
            "Should resolve to the closest hit"
        );
    }

    /// No [`WorldHit`]s → no [`ResolvedHit`] emitted.
    #[test]
    fn resolve_world_hits_no_input_no_output() {
        #[derive(Resource, Default)]
        struct Captured(Vec<ResolvedHit>);

        let mut app = make_resolve_app();
        app.init_resource::<Captured>();

        app.add_systems(Update, (
            resolve_world_hits,
            (|mut r: MessageReader<ResolvedHit>, mut c: ResMut<Captured>| {
                c.0.extend(r.read().copied());
            })
            .after(resolve_world_hits),
        ));
        app.update();

        let captured = app.world().resource::<Captured>();
        assert!(captured.0.is_empty(), "No ResolvedHit expected when there are no WorldHits");
    }

    // ── default_interaction ─────────────────────────────────────────────────

    /// Left-clicking an entity with an [`Item`] component sends
    /// `InteractionRequest::ItemPickup`.
    #[test]
    fn default_interaction_left_click_item_sends_pickup() {
        #[derive(Resource, Default)]
        struct Captured(Vec<InteractionRequest>);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ResolvedHit>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<Captured>();

        let net_id = NetId(42);
        let item_entity = app.world_mut().spawn((Item, net_id)).id();

        app.world_mut()
            .resource_mut::<Messages<ResolvedHit>>()
            .write(ResolvedHit {
                hit: WorldHit {
                    button: MouseButton::Left,
                    entity: item_entity,
                    world_pos: Vec3::ZERO,
                },
            });

        app.add_systems(Update, (
            default_interaction,
            (|mut r: MessageReader<InteractionRequest>, mut c: ResMut<Captured>| {
                c.0.extend(r.read().cloned());
            })
            .after(default_interaction),
        ));
        app.update();

        let captured = app.world().resource::<Captured>();
        assert_eq!(captured.0.len(), 1, "Expected one InteractionRequest");
        match captured.0[0] {
            InteractionRequest::ItemPickup { item } => assert_eq!(item, net_id),
            ref other => panic!("Expected ItemPickup, got {:?}", other),
        }
    }

    /// Left-clicking a non-Item entity does not send any [`InteractionRequest`].
    #[test]
    fn default_interaction_left_click_non_item_does_nothing() {
        #[derive(Resource, Default)]
        struct Captured(Vec<InteractionRequest>);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ResolvedHit>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<Captured>();

        let non_item = app.world_mut().spawn_empty().id();

        app.world_mut()
            .resource_mut::<Messages<ResolvedHit>>()
            .write(ResolvedHit {
                hit: WorldHit {
                    button: MouseButton::Left,
                    entity: non_item,
                    world_pos: Vec3::ZERO,
                },
            });

        app.add_systems(Update, (
            default_interaction,
            (|mut r: MessageReader<InteractionRequest>, mut c: ResMut<Captured>| {
                c.0.extend(r.read().cloned());
            })
            .after(default_interaction),
        ));
        app.update();

        let captured = app.world().resource::<Captured>();
        assert!(captured.0.is_empty(), "No InteractionRequest expected for non-Item hit");
    }

    // ── context menu entries ─────────────────────────────────────────────────

    fn make_context_menu_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ResolvedHit>();
        app.add_message::<ContextMenuAction>();
        app.add_message::<InteractionRequest>();
        app.init_resource::<UiTheme>();
        app.init_resource::<InteractionRange>();
        app
    }

    fn emit_right_click(app: &mut App, entity: Entity, world_pos: Vec3) {
        app.world_mut()
            .resource_mut::<Messages<ResolvedHit>>()
            .write(ResolvedHit {
                hit: WorldHit {
                    button: MouseButton::Right,
                    entity,
                    world_pos,
                },
            });
    }

    /// Right-clicking an [`Item`] on the floor opens a menu with "Pick up".
    #[test]
    fn context_menu_item_on_floor_shows_pick_up() {
        let mut app = make_context_menu_app();

        // Player at the origin — within range of the item.
        app.world_mut().spawn((PlayerControlled, GlobalTransform::IDENTITY));

        let net_id = NetId(7);
        let item_entity = app.world_mut().spawn((Item, net_id)).id();
        emit_right_click(&mut app, item_entity, Vec3::ZERO);

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            app.world().contains_resource::<ActiveMenu>(),
            "Menu should open for an Item entity"
        );
    }

    /// Right-clicking a floor [`Tile`] while holding an item shows "Drop" and "Build Wall".
    #[test]
    fn context_menu_floor_while_holding_item_shows_drop_and_build_wall() {
        let mut app = make_context_menu_app();

        // Spawn player with a hand slot holding an item.
        let item_net_id = NetId(99);
        let item_entity = app.world_mut().spawn((Item, item_net_id)).id();

        let mut hand_container = Container::with_capacity(1);
        hand_container.insert(item_entity);
        let hand = app
            .world_mut()
            .spawn((HandSlot { side: things::HandSide::Right }, hand_container))
            .id();

        let player = app
            .world_mut()
            .spawn((PlayerControlled, GlobalTransform::IDENTITY))
            .add_children(&[hand])
            .id();
        let _ = player;

        // Spawn a floor tile within interaction range of the player at origin.
        let tile_entity = app
            .world_mut()
            .spawn(Tile { position: IVec2::new(1, 0) })
            .id();
        let tilemap = Tilemap::new(5, 5, TileKind::Floor);
        app.insert_resource(tilemap);

        emit_right_click(&mut app, tile_entity, Vec3::new(1.0, 0.0, 0.0));

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            app.world().contains_resource::<ActiveMenu>(),
            "Menu should open for a floor tile while holding item"
        );

        // The menu root should have exactly 2 button children: "Drop" and "Build Wall".
        let menu_entity = app.world().resource::<ActiveMenu>().0;
        let child_count = app
            .world()
            .get::<Children>(menu_entity)
            .map(|c| c.len())
            .unwrap_or(0);
        assert_eq!(child_count, 2, "Expected two buttons (Drop + Build Wall) in the context menu");
    }

    /// Right-clicking a [`Container`] while holding an item shows "Store in {name}".
    #[test]
    fn context_menu_container_while_holding_item_shows_store() {
        let mut app = make_context_menu_app();

        // Spawn player with a hand slot holding an item.
        let item_net_id = NetId(55);
        let item_entity = app.world_mut().spawn((Item, item_net_id)).id();

        let mut hand_container = Container::with_capacity(1);
        hand_container.insert(item_entity);
        let hand = app
            .world_mut()
            .spawn((HandSlot { side: things::HandSide::Right }, hand_container))
            .id();
        app.world_mut()
            .spawn((PlayerControlled, GlobalTransform::IDENTITY))
            .add_children(&[hand]);

        // Spawn a world container (not a HandSlot).
        let container_net_id = NetId(10);
        let container_entity = app
            .world_mut()
            .spawn((
                container_net_id,
                Container::with_capacity(4),
                DisplayName("Crate".to_string()),
            ))
            .id();

        emit_right_click(&mut app, container_entity, Vec3::ZERO);

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            app.world().contains_resource::<ActiveMenu>(),
            "Menu should open when right-clicking a container while holding item"
        );
    }

    /// Right-clicking a [`Container`] with items while the hand is empty shows
    /// "Take from {name}".
    #[test]
    fn context_menu_container_hand_empty_shows_take() {
        let mut app = make_context_menu_app();

        // Spawn player with empty hand slot.
        let hand = app
            .world_mut()
            .spawn((HandSlot { side: things::HandSide::Right }, Container::with_capacity(1)))
            .id();
        app.world_mut()
            .spawn((PlayerControlled, GlobalTransform::IDENTITY))
            .add_children(&[hand]);

        // Spawn a world container with one item.
        let item_net_id = NetId(33);
        let item_entity = app.world_mut().spawn((Item, item_net_id)).id();
        let container_net_id = NetId(20);
        let mut world_container = Container::with_capacity(4);
        world_container.insert(item_entity);
        let container_entity = app
            .world_mut()
            .spawn((
                container_net_id,
                world_container,
                DisplayName("Locker".to_string()),
            ))
            .id();

        emit_right_click(&mut app, container_entity, Vec3::ZERO);

        app.add_systems(Update, build_context_menu);
        app.update();

        assert!(
            app.world().contains_resource::<ActiveMenu>(),
            "Menu should open when right-clicking a non-empty container with empty hands"
        );
    }
}

