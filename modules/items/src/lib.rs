use bevy::prelude::*;
use network::{NetId, PlayerEvent, Server, StreamSender};
use physics::{Collider, GravityScale, LinearVelocity, RigidBody};
use things::{
    HandSlot, ItemActionEvent, ItemEvent, NetIdIndex, PendingItemEvents, ThingsStreamMessage,
};

// ── Components ────────────────────────────────────────────────────────────────

/// Marker component for item entities — world objects that can be picked up,
/// dropped, stored in containers, and taken from containers.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Item;

/// Inventory container component.  Holds up to `capacity()` item entities in its
/// slot list.  Added automatically to every [`HandSlot`] entity by
/// [`init_hand_containers`].
///
/// The number of slots is the authoritative capacity; `capacity()` derives from
/// `slots.len()` to avoid any possibility of the two values diverging.
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct Container {
    pub slots: Vec<Option<Entity>>,
}

impl Container {
    /// Create an empty container with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: vec![None; capacity],
        }
    }

    /// The maximum number of items this container can hold (`slots.len()`).
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if at least one slot is empty.
    pub fn has_space(&self) -> bool {
        self.slots.iter().any(|s| s.is_none())
    }

    /// Insert `entity` into the first free slot.  Returns the slot index on
    /// success, or `None` if the container is full.
    pub fn insert(&mut self, entity: Entity) -> Option<usize> {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(entity);
                return Some(i);
            }
        }
        None
    }

    /// Remove `entity` from whichever slot holds it.  Returns `true` on success.
    pub fn remove(&mut self, entity: Entity) -> bool {
        for slot in self.slots.iter_mut() {
            if *slot == Some(entity) {
                *slot = None;
                return true;
            }
        }
        false
    }

    /// Returns `true` if the container holds `entity`.
    pub fn contains(&self, entity: Entity) -> bool {
        self.slots.iter().any(|s| *s == Some(entity))
    }
}

/// Physics snapshot stored on an item while it is held or stashed inside a
/// container.  Restored when the item is dropped back into the world.
///
/// `ConstantForce` is deliberately excluded — it is a per-frame force that
/// game systems must set each tick and does not need to be preserved.
#[derive(Component, Debug, Clone)]
pub struct StashedPhysics {
    pub collider: Collider,
    pub gravity: GravityScale,
}

// ── Resources ─────────────────────────────────────────────────────────────────

/// Maximum distance (in world units) within which an actor can interact with an
/// item or container.  Inserted into the app by `src/main.rs` from `AppConfig`
/// at startup.
#[derive(Resource, Debug, Clone, Copy)]
pub struct InteractionRange(pub f32);

impl Default for InteractionRange {
    fn default() -> Self {
        Self(2.0)
    }
}

// ── Request events ────────────────────────────────────────────────────────────

/// Server-side request: actor picks up an item from the world.
#[derive(Message, Clone, Debug)]
pub struct ItemPickupRequest {
    /// The creature (actor) performing the action.
    pub actor: Entity,
    /// The item entity to pick up.
    pub item: Entity,
}

/// Server-side request: actor drops a held item at a world position.
#[derive(Message, Clone, Debug)]
pub struct ItemDropRequest {
    /// The creature (actor) performing the action.
    pub actor: Entity,
    /// The item entity to drop (must currently be in the actor's hand).
    pub item: Entity,
    /// World position where the item should land.
    pub drop_position: Vec3,
}

/// Server-side request: actor stores a held item into a container.
#[derive(Message, Clone, Debug)]
pub struct ItemStoreRequest {
    /// The creature (actor) performing the action.
    pub actor: Entity,
    /// The item entity to store (must currently be in the actor's hand).
    pub item: Entity,
    /// The target container entity.
    pub container: Entity,
}

/// Server-side request: actor takes an item from a container into their hand.
#[derive(Message, Clone, Debug)]
pub struct ItemTakeRequest {
    /// The creature (actor) performing the action.
    pub actor: Entity,
    /// The item entity to take.
    pub item: Entity,
    /// The container that currently holds the item.
    pub container: Entity,
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Reactive system: adds `Container { capacity: 1 }` to every newly-added
/// [`HandSlot`] entity.  Hand entities are spawned at runtime (when players
/// connect), so this runs in `Update` watching `Added<HandSlot>`.
fn init_hand_containers(mut commands: Commands, query: Query<Entity, Added<HandSlot>>) {
    for entity in query.iter() {
        commands.entity(entity).insert(Container::with_capacity(1));
    }
}

/// Server system that processes all four item interaction request events.
///
/// For each request it:
/// 1. Validates that all referenced entities exist and constraints are met
///    (range, space, item marker, physics presence, stashed physics for drop).
/// 2. Executes the operation via `Commands`.
/// 3. Fires an [`ItemActionEvent`] so other systems (e.g. replication) can react.
///
/// Validation failures are logged as warnings and the request is silently
/// dropped — no error is sent back to the client in this iteration.
///
/// The system is gated on [`Server`] so it only runs in server builds; on
/// clients no request messages will be written and the resource is absent.
#[allow(clippy::too_many_arguments)]
fn handle_item_interaction(
    mut commands: Commands,
    interaction_range: Res<InteractionRange>,
    mut pickup_req: MessageReader<ItemPickupRequest>,
    mut drop_req: MessageReader<ItemDropRequest>,
    mut store_req: MessageReader<ItemStoreRequest>,
    mut take_req: MessageReader<ItemTakeRequest>,
    transforms: Query<&GlobalTransform>,
    children: Query<&Children>,
    hand_slot_q: Query<Entity, With<HandSlot>>,
    mut containers: Query<&mut Container>,
    items_q: Query<
        (
            Option<&Collider>,
            Option<&GravityScale>,
            Option<&StashedPhysics>,
            Option<&ChildOf>,
        ),
        With<Item>,
    >,
    mut action_events: MessageWriter<ItemActionEvent>,
) {
    let range = interaction_range.0;

    // Collect events first to avoid simultaneous mutable borrows on readers.
    let pickups: Vec<_> = pickup_req.read().cloned().collect();
    let drops: Vec<_> = drop_req.read().cloned().collect();
    let stores: Vec<_> = store_req.read().cloned().collect();
    let takes: Vec<_> = take_req.read().cloned().collect();

    // ── Pickup ────────────────────────────────────────────────────────────────
    for req in pickups {
        // Validate: item must have Item component.
        let Ok((maybe_collider, maybe_gravity, maybe_stash, maybe_parent)) =
            items_q.get(req.item)
        else {
            warn!("ItemPickupRequest: entity {:?} is not an Item", req.item);
            continue;
        };

        // Validate: item must not already be held / stashed.
        if maybe_stash.is_some() || maybe_parent.is_some() {
            warn!(
                "ItemPickupRequest: item {:?} is already held or parented — ignoring",
                req.item
            );
            continue;
        }

        // Validate: item must have its own Collider and GravityScale so that
        // physics can be faithfully stashed and restored.  Fabricating defaults
        // here would make an originally non-physical item become a dynamic
        // rigid body after a pickup/drop cycle.
        let (Some(collider), Some(gravity)) = (maybe_collider, maybe_gravity) else {
            warn!(
                "ItemPickupRequest: item {:?} is missing Collider or GravityScale — cannot stash physics",
                req.item
            );
            continue;
        };

        // Validate: actor and item must have transforms for range check.
        let (Ok(actor_gt), Ok(item_gt)) =
            (transforms.get(req.actor), transforms.get(req.item))
        else {
            warn!("ItemPickupRequest: actor or item has no GlobalTransform");
            continue;
        };
        let distance = actor_gt.translation().distance(item_gt.translation());
        if distance > range {
            warn!(
                "ItemPickupRequest: item {:?} is out of range ({:.2} > {:.2})",
                req.item, distance, range
            );
            continue;
        }

        // Find an actor hand slot that has a Container with free space.
        let Some(hand_entity) =
            find_hand_slot_with_space(req.actor, &children, &hand_slot_q, &containers)
        else {
            warn!(
                "ItemPickupRequest: actor {:?} has no hand with free space",
                req.actor
            );
            continue;
        };

        // Stash physics and reparent.
        commands
            .entity(req.item)
            .insert(StashedPhysics {
                collider: collider.clone(),
                gravity: *gravity,
            })
            .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>()
            // Reset local transform so the item aligns with the hand anchor.
            .insert((Transform::IDENTITY, ChildOf(hand_entity)));

        // Update hand container immediately (before commands are applied).
        if let Ok(mut container) = containers.get_mut(hand_entity) {
            container.insert(req.item);
        }

        action_events.write(ItemActionEvent::PickedUp {
            item: req.item,
            hand: hand_entity,
        });
    }

    // ── Drop ──────────────────────────────────────────────────────────────────
    for req in drops {
        // Validate: item must have Item component with StashedPhysics.
        let Ok((_, _, maybe_stash, _)) = items_q.get(req.item) else {
            warn!("ItemDropRequest: entity {:?} is not an Item", req.item);
            continue;
        };
        let Some(stash) = maybe_stash else {
            warn!(
                "ItemDropRequest: item {:?} has no StashedPhysics (not held)",
                req.item
            );
            continue;
        };

        // Validate: drop_position must be within interaction range of the actor.
        let Ok(actor_gt) = transforms.get(req.actor) else {
            warn!("ItemDropRequest: actor {:?} has no GlobalTransform", req.actor);
            continue;
        };
        let drop_distance = actor_gt.translation().distance(req.drop_position);
        if drop_distance > range {
            warn!(
                "ItemDropRequest: drop_position {:?} is out of range ({:.2} > {:.2})",
                req.drop_position, drop_distance, range
            );
            continue;
        }

        // Find the hand slot container that holds this item.
        let Some(hand_entity) =
            find_hand_slot_containing(req.actor, req.item, &children, &hand_slot_q, &containers)
        else {
            warn!(
                "ItemDropRequest: item {:?} is not in actor {:?}'s hand container",
                req.item, req.actor
            );
            continue;
        };

        let stash = stash.clone();

        // Restore physics, deparent, place at drop position.
        commands
            .entity(req.item)
            .remove::<ChildOf>()
            .remove::<StashedPhysics>()
            .insert(Transform::from_translation(req.drop_position))
            .insert((
                RigidBody::Dynamic,
                stash.collider,
                stash.gravity,
                LinearVelocity::default(),
            ));

        // Update hand container immediately.
        if let Ok(mut container) = containers.get_mut(hand_entity) {
            container.remove(req.item);
        }

        action_events.write(ItemActionEvent::Dropped {
            item: req.item,
            position: req.drop_position,
        });
    }

    // ── Store ─────────────────────────────────────────────────────────────────
    for req in stores {
        // Validate: item must be an Item.
        if items_q.get(req.item).is_err() {
            warn!("ItemStoreRequest: entity {:?} is not an Item", req.item);
            continue;
        }

        // Validate: item must be in actor's hand container.
        let Some(hand_entity) =
            find_hand_slot_containing(req.actor, req.item, &children, &hand_slot_q, &containers)
        else {
            warn!(
                "ItemStoreRequest: item {:?} is not in actor {:?}'s hand container",
                req.item, req.actor
            );
            continue;
        };

        // Validate: distance to target container — both transforms are required.
        match (
            transforms.get(req.actor),
            transforms.get(req.container),
        ) {
            (Ok(actor_gt), Ok(container_gt)) => {
                let distance = actor_gt.translation().distance(container_gt.translation());
                if distance > range {
                    warn!(
                        "ItemStoreRequest: container {:?} is out of range ({:.2} > {:.2})",
                        req.container, distance, range
                    );
                    continue;
                }
            }
            (actor_res, container_res) => {
                warn!(
                    "ItemStoreRequest: missing GlobalTransform (actor missing: {}, container missing: {}) — rejecting",
                    actor_res.is_err(),
                    container_res.is_err()
                );
                continue;
            }
        }

        // Validate: target container has space.
        let has_space = containers
            .get(req.container)
            .map(|c| c.has_space())
            .unwrap_or(false);
        if !has_space {
            warn!("ItemStoreRequest: container {:?} is full", req.container);
            continue;
        }

        // Deparent, hide, update containers.
        commands
            .entity(req.item)
            .remove::<ChildOf>()
            .insert(Visibility::Hidden);

        if let Ok(mut hand_container) = containers.get_mut(hand_entity) {
            hand_container.remove(req.item);
        }
        if let Ok(mut target_container) = containers.get_mut(req.container) {
            target_container.insert(req.item);
        }

        action_events.write(ItemActionEvent::Stored {
            item: req.item,
            container: req.container,
        });
    }

    // ── Take ──────────────────────────────────────────────────────────────────
    for req in takes {
        // Validate: item must be an Item.
        let Ok((maybe_collider, maybe_gravity, _, _)) = items_q.get(req.item) else {
            warn!("ItemTakeRequest: entity {:?} is not an Item", req.item);
            continue;
        };

        // Validate: item must be in the specified container.
        match containers.get(req.container) {
            Ok(container) => {
                if !container.contains(req.item) {
                    warn!(
                        "ItemTakeRequest: item {:?} is not in container {:?}",
                        req.item, req.container
                    );
                    continue;
                }
            }
            Err(_) => {
                warn!(
                    "ItemTakeRequest: entity {:?} has no Container component (requested as container for item {:?})",
                    req.container, req.item
                );
                continue;
            }
        }

        // Validate: actor must have a hand with space.
        let Some(hand_entity) =
            find_hand_slot_with_space(req.actor, &children, &hand_slot_q, &containers)
        else {
            warn!(
                "ItemTakeRequest: actor {:?} has no hand with free space",
                req.actor
            );
            continue;
        };

        // Validate: distance to container — both transforms are required.
        match (
            transforms.get(req.actor),
            transforms.get(req.container),
        ) {
            (Ok(actor_gt), Ok(container_gt)) => {
                let distance = actor_gt.translation().distance(container_gt.translation());
                if distance > range {
                    warn!(
                        "ItemTakeRequest: container {:?} is out of range ({:.2} > {:.2})",
                        req.container, distance, range
                    );
                    continue;
                }
            }
            (actor_res, container_res) => {
                warn!(
                    "ItemTakeRequest: missing GlobalTransform (actor missing: {}, container missing: {}) — rejecting",
                    actor_res.is_err(),
                    container_res.is_err()
                );
                continue;
            }
        }

        // Validate: item must have the required physics components to be taken.
        let (Some(collider), Some(gravity)) = (maybe_collider, maybe_gravity) else {
            warn!(
                "ItemTakeRequest: item {:?} is missing Collider and/or GravityScale — rejecting",
                req.item
            );
            continue;
        };

        // Remove from source container now that we know the item can be held.
        if let Ok(mut src_container) = containers.get_mut(req.container) {
            src_container.remove(req.item);
        }

        // Ensure the item is in a non-physical "held" state: remove any
        // physics components so a dynamic rigid body is never parented under a
        // hand slot (which would cause jitter/collisions).  Stash the physics
        // components so they can be restored on drop (same rule as pickup).
        commands
            .entity(req.item)
            .insert(StashedPhysics {
                collider: collider.clone(),
                gravity: *gravity,
            })
            .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>();

        // Show and reparent to hand, resetting local transform to the hand anchor.
        commands
            .entity(req.item)
            .insert((Visibility::Inherited, Transform::IDENTITY, ChildOf(hand_entity)));
        if let Ok(mut hand_container) = containers.get_mut(hand_entity) {
            hand_container.insert(req.item);
        }

        action_events.write(ItemActionEvent::Taken {
            item: req.item,
            hand: hand_entity,
        });
    }
}

/// Find the first hand-slot entity that is a child of `actor`, has a
/// [`Container`], and has at least one free slot.
fn find_hand_slot_with_space(
    actor: Entity,
    children: &Query<&Children>,
    hand_slot_q: &Query<Entity, With<HandSlot>>,
    containers: &Query<&mut Container>,
) -> Option<Entity> {
    let Ok(actor_children) = children.get(actor) else {
        return None;
    };
    for &child in actor_children {
        if hand_slot_q.get(child).is_ok() {
            if let Ok(container) = containers.get(child) {
                if container.has_space() {
                    return Some(child);
                }
            }
        }
    }
    None
}

/// Find the hand-slot entity that is a child of `actor` and whose [`Container`]
/// holds `item`.
fn find_hand_slot_containing(
    actor: Entity,
    item: Entity,
    children: &Query<&Children>,
    hand_slot_q: &Query<Entity, With<HandSlot>>,
    containers: &Query<&mut Container>,
) -> Option<Entity> {
    let Ok(actor_children) = children.get(actor) else {
        return None;
    };
    for &child in actor_children {
        if hand_slot_q.get(child).is_ok() {
            if let Ok(container) = containers.get(child) {
                if container.contains(item) {
                    return Some(child);
                }
            }
        }
    }
    None
}

// ── Client-side item event handler ───────────────────────────────────────────

/// Applies [`ItemEvent`] messages that arrived on stream 3 to the local ECS state.
///
/// Drains [`PendingItemEvents`] each `Update` tick (populated by
/// `handle_entity_lifecycle` in `PreUpdate`).  Runs on clients only.
///
/// - **PickedUp**: strip physics, insert [`StashedPhysics`], reparent item to
///   the holder creature's [`HandSlot`], update the hand's [`Container`] slots.
/// - **Dropped**: restore physics from [`StashedPhysics`], deparent, set world
///   position, clear the former hand's [`Container`] slot.
/// - **Stored**: strip physics if present, insert [`StashedPhysics`], deparent,
///   set [`Visibility::Hidden`], insert item into the target container's slots.
/// - **Taken**: show item, reparent to creature's hand, remove from all
///   containers that hold it, update the hand's [`Container`] slot.
fn handle_item_event(
    mut commands: Commands,
    mut pending: ResMut<PendingItemEvents>,
    net_id_index: Res<NetIdIndex>,
    mut containers: Query<&mut Container>,
    items_q: Query<
        (
            Option<&Collider>,
            Option<&GravityScale>,
            Option<&StashedPhysics>,
            Option<&ChildOf>,
        ),
        With<Item>,
    >,
    children: Query<&Children>,
    hand_slot_q: Query<Entity, With<HandSlot>>,
) {
    let events: Vec<ItemEvent> = pending.0.drain(..).collect();
    for event in events {
        match event {
            ItemEvent::PickedUp { item, holder } => {
                let Some(&item_entity) = net_id_index.0.get(&item) else {
                    warn!("handle_item_event: PickedUp item NetId({}) not found", item.0);
                    continue;
                };
                let Some(&creature_entity) = net_id_index.0.get(&holder) else {
                    warn!(
                        "handle_item_event: PickedUp holder NetId({}) not found",
                        holder.0
                    );
                    continue;
                };
                let Some(hand_entity) = find_hand_slot_with_space(
                    creature_entity,
                    &children,
                    &hand_slot_q,
                    &containers,
                ) else {
                    warn!(
                        "handle_item_event: PickedUp holder has no hand with free space"
                    );
                    continue;
                };
                let Ok((maybe_collider, maybe_gravity, _, _)) = items_q.get(item_entity) else {
                    warn!("handle_item_event: PickedUp item entity has no Item component");
                    continue;
                };
                if let (Some(col), Some(grav)) = (maybe_collider, maybe_gravity) {
                    commands
                        .entity(item_entity)
                        .insert(StashedPhysics {
                            collider: col.clone(),
                            gravity: *grav,
                        })
                        .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>();
                }
                commands
                    .entity(item_entity)
                    .insert((Transform::IDENTITY, ChildOf(hand_entity)));
                if let Ok(mut container) = containers.get_mut(hand_entity) {
                    container.insert(item_entity);
                }
            }

            ItemEvent::Dropped { item, position } => {
                let Some(&item_entity) = net_id_index.0.get(&item) else {
                    warn!("handle_item_event: Dropped item NetId({}) not found", item.0);
                    continue;
                };
                let Ok((_, _, maybe_stash, maybe_child_of)) = items_q.get(item_entity) else {
                    warn!("handle_item_event: Dropped item entity has no Item component");
                    continue;
                };
                // Remove item from its hand container slot.
                if let Some(child_of) = maybe_child_of {
                    if let Ok(mut container) = containers.get_mut(child_of.parent()) {
                        container.remove(item_entity);
                    }
                }
                let drop_pos = Vec3::from_array(position);
                if let Some(stash) = maybe_stash.cloned() {
                    commands
                        .entity(item_entity)
                        .remove::<ChildOf>()
                        .remove::<StashedPhysics>()
                        .insert(Transform::from_translation(drop_pos))
                        .insert((
                            RigidBody::Dynamic,
                            stash.collider,
                            stash.gravity,
                            LinearVelocity::default(),
                        ));
                } else {
                    commands
                        .entity(item_entity)
                        .remove::<ChildOf>()
                        .insert(Transform::from_translation(drop_pos));
                }
            }

            ItemEvent::Stored { item, container } => {
                let Some(&item_entity) = net_id_index.0.get(&item) else {
                    warn!("handle_item_event: Stored item NetId({}) not found", item.0);
                    continue;
                };
                let Some(&container_entity) = net_id_index.0.get(&container) else {
                    warn!(
                        "handle_item_event: Stored container NetId({}) not found",
                        container.0
                    );
                    continue;
                };
                let Ok((maybe_collider, maybe_gravity, _, maybe_child_of)) =
                    items_q.get(item_entity)
                else {
                    warn!("handle_item_event: Stored item entity has no Item component");
                    continue;
                };
                // Remove item from its current hand container if held.
                if let Some(child_of) = maybe_child_of {
                    if let Ok(mut hand_container) = containers.get_mut(child_of.parent()) {
                        hand_container.remove(item_entity);
                    }
                }
                // Strip physics if still present (e.g. for items received during initial sync).
                if let (Some(col), Some(grav)) = (maybe_collider, maybe_gravity) {
                    commands
                        .entity(item_entity)
                        .insert(StashedPhysics {
                            collider: col.clone(),
                            gravity: *grav,
                        })
                        .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>();
                }
                commands
                    .entity(item_entity)
                    .remove::<ChildOf>()
                    .insert(Visibility::Hidden);
                if let Ok(mut target_container) = containers.get_mut(container_entity) {
                    target_container.insert(item_entity);
                }
            }

            ItemEvent::Taken { item, holder } => {
                let Some(&item_entity) = net_id_index.0.get(&item) else {
                    warn!("handle_item_event: Taken item NetId({}) not found", item.0);
                    continue;
                };
                let Some(&creature_entity) = net_id_index.0.get(&holder) else {
                    warn!(
                        "handle_item_event: Taken holder NetId({}) not found",
                        holder.0
                    );
                    continue;
                };
                let Some(hand_entity) = find_hand_slot_with_space(
                    creature_entity,
                    &children,
                    &hand_slot_q,
                    &containers,
                ) else {
                    warn!("handle_item_event: Taken holder has no hand with free space");
                    continue;
                };
                let Ok((maybe_collider, maybe_gravity, _, _)) = items_q.get(item_entity) else {
                    warn!("handle_item_event: Taken item entity has no Item component");
                    continue;
                };
                // Strip physics if somehow still present (shouldn't be after Stored,
                // but guard against unexpected states).
                if let (Some(col), Some(grav)) = (maybe_collider, maybe_gravity) {
                    commands
                        .entity(item_entity)
                        .insert(StashedPhysics {
                            collider: col.clone(),
                            gravity: *grav,
                        })
                        .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>();
                }
                // Remove item from all containers that hold it (source container).
                for mut container in containers.iter_mut() {
                    container.remove(item_entity);
                }
                commands.entity(item_entity).insert((
                    Visibility::Inherited,
                    Transform::IDENTITY,
                    ChildOf(hand_entity),
                ));
                if let Ok(mut hand_container) = containers.get_mut(hand_entity) {
                    hand_container.insert(item_entity);
                }
            }
        }
    }
}

// ── Server-side initial-sync for stored items ─────────────────────────────────

/// Sends [`ItemEvent::Stored`] for every item currently held in a non-hand
/// container to a newly joined client.
///
/// Runs in `PreUpdate` after [`things::ThingsSet::HandleClientJoined`] so that
/// the client has already received [`ThingsStreamMessage::EntitySpawned`] for
/// every entity before these events arrive.
///
/// Hand-held items are covered by the [`ThingsStreamMessage::ItemEvent(PickedUp)`]
/// sent in `handle_client_joined` (the things module); this system covers items
/// that are inside world containers (i.e., entities with [`Container`] that are
/// NOT [`HandSlot`] entities, and therefore have a [`NetId`] of their own).
fn broadcast_stored_on_join(
    mut player_events: MessageReader<PlayerEvent>,
    containers: Query<(Entity, &Container, &NetId), Without<HandSlot>>,
    net_ids: Query<&NetId>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
) {
    for event in player_events.read() {
        let PlayerEvent::Joined { id: from, .. } = event else {
            continue;
        };
        for (container_entity, container, &container_net_id) in containers.iter() {
            for item_entity in container.slots.iter().filter_map(|s| *s) {
                let Ok(&item_net_id) = net_ids.get(item_entity) else {
                    warn!(
                        "broadcast_stored_on_join: item in container {:?} has no NetId",
                        container_entity
                    );
                    continue;
                };
                if let Err(e) = stream_sender.send_to(
                    *from,
                    &ThingsStreamMessage::ItemEvent(ItemEvent::Stored {
                        item: item_net_id,
                        container: container_net_id,
                    }),
                ) {
                    error!(
                        "broadcast_stored_on_join: failed to send to ClientId({}): {e}",
                        from.0
                    );
                }
            }
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Plugin that registers all item components, resources, events, and systems.
///
/// Must be added after [`things::ThingsPlugin`] (which registers [`HandSlot`])
/// and [`physics::PhysicsPlugin`] (which registers physics components).
///
/// `src/main.rs` is responsible for inserting the [`InteractionRange`] resource
/// from `AppConfig` at startup (to avoid a circular dependency between the
/// workspace crate and this module).
pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Item>();
        app.register_type::<Container>();

        app.add_message::<ItemPickupRequest>();
        app.add_message::<ItemDropRequest>();
        app.add_message::<ItemStoreRequest>();
        app.add_message::<ItemTakeRequest>();

        app.init_resource::<InteractionRange>();

        app.add_systems(
            Update,
            (
                init_hand_containers,
                handle_item_interaction.run_if(resource_exists::<Server>),
                handle_item_event.run_if(resource_exists::<network::Client>),
            ),
        );
        app.add_systems(
            PreUpdate,
            broadcast_stored_on_join
                .run_if(resource_exists::<Server>)
                .after(things::ThingsSet::HandleClientJoined),
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Build a minimal headless `App` suitable for items unit tests.
    /// Includes physics so we can test component removal/restoration.
    fn test_app() -> App {
        use physics::PhysicsPlugin;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugin,
            bevy::camera::visibility::VisibilityPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )));
        // Register component types required by the items systems.
        app.register_type::<Item>();
        app.register_type::<Container>();
        app.register_type::<HandSlot>();
        // Add all item messages.
        app.add_message::<ItemPickupRequest>();
        app.add_message::<ItemDropRequest>();
        app.add_message::<ItemStoreRequest>();
        app.add_message::<ItemTakeRequest>();
        app.add_message::<ItemActionEvent>();
        // Add the interaction systems.
        app.add_systems(Update, (init_hand_containers, handle_item_interaction));
        app.insert_resource(InteractionRange(2.0));
        app.finish();
        app
    }

    /// Spawn a creature-like actor with a HandSlot child.
    /// Inserting `ChildOf(actor)` on the hand causes Bevy's relationship
    /// infrastructure to update `Children` on the actor automatically.
    /// Returns (actor_entity, hand_slot_entity).
    fn spawn_actor(app: &mut App, pos: Vec3) -> (Entity, Entity) {
        let actor = app
            .world_mut()
            .spawn(Transform::from_translation(pos))
            .id();
        let hand = app
            .world_mut()
            .spawn((
                HandSlot {
                    side: things::HandSide::Right,
                },
                Transform::default(),
                ChildOf(actor),
            ))
            .id();
        (actor, hand)
    }

    /// Spawn an item entity with physics components at the given position.
    fn spawn_item(app: &mut App, pos: Vec3) -> Entity {
        app.world_mut()
            .spawn((
                Item,
                Transform::from_translation(pos),
                RigidBody::Dynamic,
                Collider::sphere(0.3),
                GravityScale(1.0),
                LinearVelocity::default(),
            ))
            .id()
    }

    // ── init_hand_containers ──────────────────────────────────────────────────

    #[test]
    fn hand_slot_gets_container_on_next_update() {
        let mut app = test_app();
        let (_, hand) = spawn_actor(&mut app, Vec3::ZERO);

        // Before update: no Container yet.
        assert!(app.world().get::<Container>(hand).is_none());

        app.update();

        // After update: Container with 1 slot.
        let container = app
            .world()
            .get::<Container>(hand)
            .expect("HandSlot should have a Container after update");
        assert_eq!(container.capacity(), 1);
        assert_eq!(container.slots.len(), 1);
        assert!(container.slots[0].is_none());
    }

    // ── Pickup ────────────────────────────────────────────────────────────────

    #[test]
    fn pickup_in_range_succeeds() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        app.update(); // init_hand_containers gives the hand a Container

        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();

        // Physics components must be removed.
        assert!(
            app.world().get::<RigidBody>(item).is_none(),
            "RigidBody should be removed after pickup"
        );
        assert!(
            app.world().get::<Collider>(item).is_none(),
            "Collider should be removed after pickup"
        );
        assert!(
            app.world().get::<LinearVelocity>(item).is_none(),
            "LinearVelocity should be removed after pickup"
        );

        // StashedPhysics must be present.
        assert!(
            app.world().get::<StashedPhysics>(item).is_some(),
            "StashedPhysics should be present after pickup"
        );

        // Item must be parented to the hand slot.
        assert!(
            app.world().get::<ChildOf>(item).is_some(),
            "item should be parented after pickup"
        );

        // Hand container must record the item.
        let container = app.world().get::<Container>(hand).unwrap();
        assert!(container.contains(item), "hand container should contain item");
    }

    #[test]
    fn pickup_out_of_range_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(10.0, 0.0, 0.0)); // outside range 2.0
        app.update();

        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();

        // Item should still have physics (not picked up).
        assert!(
            app.world().get::<RigidBody>(item).is_some(),
            "RigidBody should remain when out of range"
        );
        let container = app.world().get::<Container>(hand).unwrap();
        assert!(
            !container.contains(item),
            "hand container should not contain out-of-range item"
        );
    }

    #[test]
    fn pickup_non_item_entity_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        // Spawn entity WITHOUT Item component.
        let non_item = app
            .world_mut()
            .spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))
            .id();
        app.update();

        app.world_mut()
            .write_message(ItemPickupRequest { actor, item: non_item });
        app.update();

        let container = app.world().get::<Container>(hand).unwrap();
        assert!(
            !container.contains(non_item),
            "non-item entity should not be picked up"
        );
    }

    #[test]
    fn pickup_hand_full_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item1 = spawn_item(&mut app, Vec3::new(0.5, 0.0, 0.0));
        let item2 = spawn_item(&mut app, Vec3::new(0.8, 0.0, 0.0));
        app.update(); // init_hand_containers

        // Pick up first item.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item: item1 });
        app.update();
        assert!(app.world().get::<Container>(hand).unwrap().contains(item1));

        // Attempt to pick up second item with full hand.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item: item2 });
        app.update();

        // Second item should still be in the world (physics intact).
        assert!(
            app.world().get::<RigidBody>(item2).is_some(),
            "item2 RigidBody should remain when hand is full"
        );
        let container = app.world().get::<Container>(hand).unwrap();
        assert!(
            !container.contains(item2),
            "full hand should not accept item2"
        );
    }

    #[test]
    fn pickup_already_held_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        // A second actor nearby, also in range.
        let (actor2, _hand2) = spawn_actor(&mut app, Vec3::new(0.5, 0.0, 0.0));
        let item = spawn_item(&mut app, Vec3::new(0.3, 0.0, 0.0));
        app.update();

        // actor2 picks up the item first.
        app.world_mut()
            .write_message(ItemPickupRequest { actor: actor2, item });
        app.update();
        assert!(app.world().get::<StashedPhysics>(item).is_some());

        // actor1 tries to pick up the same already-held item — should fail.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();

        // item should not be in actor1's hand.
        assert!(
            !app.world().get::<Container>(hand).unwrap().contains(item),
            "already-held item should not be pickable by a second actor"
        );
    }

    #[test]
    fn pickup_item_missing_physics_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        // Spawn an item WITHOUT physics components.
        let item = app
            .world_mut()
            .spawn((Item, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0))))
            .id();
        app.update();

        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();

        // Non-physical item should be rejected — no StashedPhysics fabricated.
        assert!(
            app.world().get::<StashedPhysics>(item).is_none(),
            "non-physical item should not get StashedPhysics"
        );
        let container = app.world().get::<Container>(hand).unwrap();
        assert!(
            !container.contains(item),
            "non-physical item should not be picked up"
        );
    }

    // ── Drop ──────────────────────────────────────────────────────────────────

    #[test]
    fn drop_restores_physics_and_position() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        app.update();

        // Pick up.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();
        assert!(app.world().get::<StashedPhysics>(item).is_some());

        // Drop within interaction range (distance 1.5 < 2.0).
        let drop_pos = Vec3::new(1.5, 0.0, 0.0);
        app.world_mut().write_message(ItemDropRequest {
            actor,
            item,
            drop_position: drop_pos,
        });
        app.update();

        // Physics components restored.
        assert!(
            app.world().get::<RigidBody>(item).is_some(),
            "RigidBody should be restored after drop"
        );
        assert!(
            app.world().get::<Collider>(item).is_some(),
            "Collider should be restored after drop"
        );
        assert!(
            app.world().get::<GravityScale>(item).is_some(),
            "GravityScale should be restored after drop"
        );

        // StashedPhysics removed.
        assert!(
            app.world().get::<StashedPhysics>(item).is_none(),
            "StashedPhysics should be removed after drop"
        );

        // No longer parented.
        assert!(
            app.world().get::<ChildOf>(item).is_none(),
            "item should have no parent after drop"
        );

        // Hand container emptied.
        let container = app.world().get::<Container>(hand).unwrap();
        assert!(
            !container.contains(item),
            "hand container should not contain item after drop"
        );

        // After one more update the drop position has been applied and gravity
        // starts acting — verify the entity has a downward velocity showing
        // that physics is fully active (consistent with the avian3d spike in
        // physics/src/lib.rs::deparented_entity_with_restored_physics_falls).
        app.update();
        app.update();
        let vel = app.world().get::<LinearVelocity>(item).unwrap();
        assert!(
            vel.y < 0.0,
            "item should be falling after drop (gravity active), got y vel = {}",
            vel.y
        );
    }

    #[test]
    fn drop_not_held_item_fails() {
        let mut app = test_app();
        let (actor, _hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        app.update();

        // Attempt to drop without picking up first (no StashedPhysics).
        app.world_mut().write_message(ItemDropRequest {
            actor,
            item,
            drop_position: Vec3::ZERO,
        });
        app.update();

        // Item should still have physics and not have been moved.
        assert!(
            app.world().get::<RigidBody>(item).is_some(),
            "RigidBody should remain when drop fails"
        );
    }

    #[test]
    fn drop_out_of_range_fails() {
        let mut app = test_app();
        let (actor, _hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        app.update();

        // Pick up the item first.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();
        assert!(app.world().get::<StashedPhysics>(item).is_some());

        // Attempt to drop at a position far outside interaction range.
        app.world_mut().write_message(ItemDropRequest {
            actor,
            item,
            drop_position: Vec3::new(50.0, 0.0, 0.0),
        });
        app.update();

        // StashedPhysics should still be present — drop was rejected.
        assert!(
            app.world().get::<StashedPhysics>(item).is_some(),
            "item should still be held when drop position is out of range"
        );
    }

    // ── Store ─────────────────────────────────────────────────────────────────

    #[test]
    fn store_moves_item_to_container_and_hides_it() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        // A nearby external container.
        let ext_container = app
            .world_mut()
            .spawn((
                Container::with_capacity(4),
                Transform::from_translation(Vec3::new(1.5, 0.0, 0.0)),
            ))
            .id();
        app.update();

        // Pick up item.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();
        assert!(app.world().get::<Container>(hand).unwrap().contains(item));

        // Store item.
        app.world_mut().write_message(ItemStoreRequest {
            actor,
            item,
            container: ext_container,
        });
        app.update();

        // Item should be hidden.
        assert_eq!(
            *app.world().get::<Visibility>(item).unwrap(),
            Visibility::Hidden,
            "stored item should be hidden"
        );

        // Item should be in ext_container, not in hand.
        assert!(
            app.world()
                .get::<Container>(ext_container)
                .unwrap()
                .contains(item),
            "item should be in external container after store"
        );
        assert!(
            !app.world().get::<Container>(hand).unwrap().contains(item),
            "item should not be in hand after store"
        );
        // Item should be deparented.
        assert!(
            app.world().get::<ChildOf>(item).is_none(),
            "stored item should have no parent"
        );
    }

    #[test]
    fn store_container_full_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item1 = spawn_item(&mut app, Vec3::new(0.5, 0.0, 0.0));
        let item2 = spawn_item(&mut app, Vec3::new(0.8, 0.0, 0.0));
        // A full container (pre-filled with item2, capacity derived from slot count).
        let full_container = app
            .world_mut()
            .spawn((
                Container {
                    slots: vec![Some(item2)],
                },
                Transform::from_translation(Vec3::new(1.5, 0.0, 0.0)),
            ))
            .id();
        app.update();

        // Pick up item1.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item: item1 });
        app.update();

        // Attempt to store into the full container.
        app.world_mut().write_message(ItemStoreRequest {
            actor,
            item: item1,
            container: full_container,
        });
        app.update();

        // item1 should still be in hand (store failed).
        assert!(
            app.world().get::<Container>(hand).unwrap().contains(item1),
            "item1 should remain in hand when container is full"
        );
    }

    // ── Take ──────────────────────────────────────────────────────────────────

    #[test]
    fn take_moves_item_from_container_to_hand() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.5, 0.0, 0.0));
        // Container pre-holding the item (as if it was stored).
        let src_container = app
            .world_mut()
            .spawn((
                Container {
                    slots: vec![Some(item), None],
                },
                Transform::from_translation(Vec3::new(1.5, 0.0, 0.0)),
            ))
            .id();
        // Mark item as hidden (as it would be when stored).
        app.world_mut()
            .entity_mut(item)
            .insert(Visibility::Hidden);
        app.update();

        app.world_mut().write_message(ItemTakeRequest {
            actor,
            item,
            container: src_container,
        });
        app.update();

        // Item should be in hand and not in source container.
        assert!(
            app.world().get::<Container>(hand).unwrap().contains(item),
            "item should be in hand after take"
        );
        assert!(
            !app.world()
                .get::<Container>(src_container)
                .unwrap()
                .contains(item),
            "item should not be in source container after take"
        );

        // Visibility should be restored to Inherited.
        let vis = app.world().get::<Visibility>(item).unwrap();
        assert_ne!(
            *vis,
            Visibility::Hidden,
            "taken item should not be hidden"
        );

        // Item reparented to hand.
        let parent = app
            .world()
            .get::<ChildOf>(item)
            .expect("taken item should have a parent");
        assert_eq!(parent.parent(), hand, "item parent should be hand slot");

        // Physics removed and StashedPhysics inserted so the item is in held state.
        assert!(
            app.world().get::<RigidBody>(item).is_none(),
            "RigidBody should be removed when taken into hand"
        );
        assert!(
            app.world().get::<StashedPhysics>(item).is_some(),
            "StashedPhysics should be present after take"
        );
    }

    #[test]
    fn take_item_not_in_container_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        let empty_container = app
            .world_mut()
            .spawn((
                Container::with_capacity(2),
                Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            ))
            .id();
        app.update();

        app.world_mut().write_message(ItemTakeRequest {
            actor,
            item,
            container: empty_container,
        });
        app.update();

        // Hand should remain empty.
        assert!(
            !app.world().get::<Container>(hand).unwrap().contains(item),
            "take from wrong container should fail"
        );
    }

    #[test]
    fn take_out_of_range_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::ZERO);
        // Container at far distance.
        let far_container = app
            .world_mut()
            .spawn((
                Container {
                    slots: vec![Some(item)],
                },
                Transform::from_translation(Vec3::new(100.0, 0.0, 0.0)),
            ))
            .id();
        app.update();

        app.world_mut().write_message(ItemTakeRequest {
            actor,
            item,
            container: far_container,
        });
        app.update();

        assert!(
            !app.world().get::<Container>(hand).unwrap().contains(item),
            "take from out-of-range container should fail"
        );
    }

    #[test]
    fn take_hand_full_fails() {
        let mut app = test_app();
        let (actor, hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item1 = spawn_item(&mut app, Vec3::new(0.5, 0.0, 0.0));
        let item2 = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        let src_container = app
            .world_mut()
            .spawn((
                Container {
                    slots: vec![Some(item2)],
                },
                Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            ))
            .id();
        app.update();

        // Fill hand with item1.
        app.world_mut()
            .write_message(ItemPickupRequest { actor, item: item1 });
        app.update();
        assert!(app.world().get::<Container>(hand).unwrap().contains(item1));

        // Attempt take while hand is full.
        app.world_mut().write_message(ItemTakeRequest {
            actor,
            item: item2,
            container: src_container,
        });
        app.update();

        assert!(
            !app.world().get::<Container>(hand).unwrap().contains(item2),
            "take into full hand should fail"
        );
        assert!(
            app.world()
                .get::<Container>(src_container)
                .unwrap()
                .contains(item2),
            "item2 should remain in source container when hand is full"
        );
    }

    // ── StashedPhysics lifecycle ──────────────────────────────────────────────

    #[test]
    fn stashed_physics_added_on_pickup_removed_on_drop() {
        let mut app = test_app();
        let (actor, _hand) = spawn_actor(&mut app, Vec3::ZERO);
        let item = spawn_item(&mut app, Vec3::new(1.0, 0.0, 0.0));
        app.update();

        assert!(
            app.world().get::<StashedPhysics>(item).is_none(),
            "no StashedPhysics before pickup"
        );

        app.world_mut()
            .write_message(ItemPickupRequest { actor, item });
        app.update();
        assert!(
            app.world().get::<StashedPhysics>(item).is_some(),
            "StashedPhysics present after pickup"
        );

        app.world_mut().write_message(ItemDropRequest {
            actor,
            item,
            drop_position: Vec3::ZERO,
        });
        app.update();
        assert!(
            app.world().get::<StashedPhysics>(item).is_none(),
            "StashedPhysics removed after drop"
        );
    }
}
