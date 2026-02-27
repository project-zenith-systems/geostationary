use bevy::prelude::*;
use input::{PointerAction, WorldHit};
use network::Headless;
use tiles::{Tile, TileKind, TileToggleRequest, Tilemap};
use ui::{UiTheme, WorldSpaceOverlay, build_button};

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
    // Collect hits to avoid borrowing issues inside the loop.
    let hits: Vec<WorldHit> = hit_events.read().copied().collect();
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

/// System that reads [`ContextMenuAction`] events and fires [`TileToggleRequest`].
///
/// The menu is dismissed by [`dismiss_context_menu`] on the same frame via the
/// left-click [`PointerAction`] that triggered the button press.
///
/// Gated on `in_state(S)` and `not(resource_exists::<Headless>)`.
fn handle_menu_selection(
    mut actions: MessageReader<ContextMenuAction>,
    mut toggle_requests: MessageWriter<TileToggleRequest>,
) {
    for action in actions.read() {
        toggle_requests.write(TileToggleRequest {
            position: action.position,
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

/// Plugin that wires up the right-click context-menu system.
///
/// All systems are gated on the provided game state and on the absence of the
/// [`Headless`] resource (context menus are client-only).
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
        let state = self.state;
        app.add_systems(
            Update,
            (
                dismiss_context_menu,
                build_context_menu.after(dismiss_context_menu),
                handle_menu_selection,
            )
                .run_if(in_state(state))
                .run_if(not(resource_exists::<Headless>)),
        );
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
        app.add_message::<TileToggleRequest>();
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
        app.add_message::<TileToggleRequest>();
        app.init_resource::<UiTheme>();

        // Spawn an entity WITHOUT a Tile component.
        let non_tile = app.world_mut().spawn_empty().id();

        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        app.insert_resource(tilemap);

        app.world_mut()
            .resource_mut::<Messages<WorldHit>>()
            .write(WorldHit {
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

    /// Verifies that [`handle_menu_selection`] fires a [`TileToggleRequest`]
    /// when a [`ContextMenuAction`] event is received.
    #[test]
    fn handle_menu_selection_fires_toggle_request() {
        #[derive(Resource, Default)]
        struct Captured(Vec<TileToggleRequest>);

        fn capture(
            mut reader: MessageReader<TileToggleRequest>,
            mut captured: ResMut<Captured>,
        ) {
            captured.0.extend(reader.read().copied());
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ContextMenuAction>();
        app.add_message::<TileToggleRequest>();
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
        assert_eq!(captured.0[0].position, IVec2::new(2, 3));
        assert_eq!(captured.0[0].kind, TileKind::Floor);
    }
}
