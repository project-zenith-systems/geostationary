use bevy::prelude::*;

/// Component that positions a UI [`Node`] at a projected world position.
///
/// Add to a UI node entity alongside `PositionType::Absolute`.
/// The [`update_world_space_overlays`] system projects [`WorldSpaceOverlay::world_pos`] through
/// the active 3D camera each frame and writes the result into `node.left` / `node.top`,
/// centering the node on the projected point.
///
/// The node is hidden when the target position is behind the camera.
///
/// ## Fixed position
/// Set `world_pos` directly. The position is used as-is each frame.
///
/// ## Entity tracking
/// Add an [`OverlayTarget`] alongside this component.  The system resolves the
/// entity's [`GlobalTransform`] (plus the optional offset) and stores the result
/// in `world_pos` before projecting, so the overlay always follows the entity.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct WorldSpaceOverlay {
    /// World-space position to project onto the viewport.
    ///
    /// Updated automatically each frame when an [`OverlayTarget`] is present.
    pub world_pos: Vec3,
}

/// Optional companion to [`WorldSpaceOverlay`] that tracks a 3D entity.
///
/// When present on the same entity as [`WorldSpaceOverlay`], the
/// [`update_world_space_overlays`] system reads the linked entity's
/// [`GlobalTransform`] and applies the additional [`Self::offset`] to produce
/// the world position that gets projected.
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct OverlayTarget {
    /// The 3D entity whose world position this overlay should track.
    pub entity: Entity,
    /// World-space offset added to the entity's translation each frame.
    pub offset: Vec3,
}

/// System that projects each [`WorldSpaceOverlay`] node to viewport space.
///
/// Registered in `PostUpdate`, after [`bevy::transform::TransformSystems::Propagate`],
/// so that entity [`GlobalTransform`]s are fully propagated before projection.
///
/// The system:
/// 1. Fetches the active [`Camera3d`] once. Hides all overlay nodes and returns
///    early if no camera is present (e.g. headless server or camera despawned).
/// 2. If the overlay has an [`OverlayTarget`], resolves the entity's
///    [`GlobalTransform`] (+ offset) and writes it into
///    [`WorldSpaceOverlay::world_pos`].
/// 3. Projects `world_pos` through the active [`Camera3d`].
/// 4. Centers the [`Node`] on the projected point via `left`/`top`.
/// 5. Hides the node when the target is behind the camera or the entity is missing.
pub fn update_world_space_overlays(
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    target_query: Query<&GlobalTransform>,
    mut overlay_query: Query<(
        &mut Node,
        &mut Visibility,
        &ComputedNode,
        &mut WorldSpaceOverlay,
        Option<&OverlayTarget>,
    )>,
) {
    // Fetch the camera once before iterating overlays.
    let camera_result = camera_query.single();

    for (mut node, mut visibility, computed, mut overlay, maybe_target) in
        overlay_query.iter_mut()
    {
        // Resolve world position from the linked entity when an OverlayTarget is present.
        if let Some(target) = maybe_target {
            let Ok(gt) = target_query.get(target.entity) else {
                *visibility = Visibility::Hidden;
                continue;
            };
            overlay.world_pos = gt.translation() + target.offset;
        }

        // Need an active 3D camera to project through.
        let Ok((camera, camera_gt)) = camera_result else {
            *visibility = Visibility::Hidden;
            continue;
        };

        // Project world position to viewport.
        if let Ok(viewport_pos) = camera.world_to_viewport(camera_gt, overlay.world_pos) {
            let size = computed.size();
            node.left = Val::Px((viewport_pos.x - size.x * 0.5).round());
            node.top = Val::Px((viewport_pos.y - size.y * 0.5).round());
            *visibility = Visibility::Inherited;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that [`WorldSpaceOverlay`] can be attached to a UI entity and that
    /// [`update_world_space_overlays`] hides the node when no camera exists.
    ///
    /// (Full projection testing requires a real camera viewport which is not
    /// available in headless tests; the hide-on-no-camera path is exercised here.)
    #[test]
    fn overlay_hidden_when_no_camera() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.register_type::<WorldSpaceOverlay>();
        app.add_systems(Update, update_world_space_overlays);

        let entity = app
            .world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                WorldSpaceOverlay {
                    world_pos: Vec3::new(1.0, 2.0, 3.0),
                },
                Visibility::Inherited,
                ComputedNode::default(),
            ))
            .id();

        app.update();

        // With no Camera3d present the system should hide the node.
        let vis = app.world().get::<Visibility>(entity).unwrap();
        assert_eq!(*vis, Visibility::Hidden);
    }

    /// Verifies that [`OverlayTarget`] resolves an entity's translation and stores
    /// it in [`WorldSpaceOverlay::world_pos`] each frame (without a camera the node
    /// is hidden, but the world_pos field is still written).
    #[test]
    fn overlay_target_writes_world_pos() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.register_type::<WorldSpaceOverlay>();
        app.add_systems(Update, update_world_space_overlays);

        let tracked_pos = Vec3::new(5.0, 0.0, 3.0);
        let offset = Vec3::Y * 2.0;

        let tracked_entity = app
            .world_mut()
            .spawn(GlobalTransform::from_translation(tracked_pos))
            .id();

        let overlay_entity = app
            .world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                WorldSpaceOverlay::default(),
                OverlayTarget {
                    entity: tracked_entity,
                    offset,
                },
                Visibility::Inherited,
                ComputedNode::default(),
            ))
            .id();

        app.update();

        let overlay = app
            .world()
            .get::<WorldSpaceOverlay>(overlay_entity)
            .unwrap();
        let expected = tracked_pos + offset;
        assert!(
            overlay.world_pos.distance(expected) < 0.001,
            "world_pos should equal tracked translation + offset. \
             Got {:?}, expected {:?}",
            overlay.world_pos,
            expected,
        );
    }

    /// Verifies that when the [`OverlayTarget`] entity is missing the overlay node
    /// is hidden (entity was despawned, e.g. after menu dismiss).
    #[test]
    fn overlay_hidden_when_target_entity_missing() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.register_type::<WorldSpaceOverlay>();
        app.add_systems(Update, update_world_space_overlays);

        // Spawn a real entity then immediately despawn it so the id is invalid.
        let dead_entity = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(dead_entity);

        let overlay_entity = app
            .world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                WorldSpaceOverlay::default(),
                OverlayTarget {
                    entity: dead_entity,
                    offset: Vec3::ZERO,
                },
                Visibility::Inherited,
                ComputedNode::default(),
            ))
            .id();

        app.update();

        let vis = app.world().get::<Visibility>(overlay_entity).unwrap();
        assert_eq!(*vis, Visibility::Hidden);
    }
}
