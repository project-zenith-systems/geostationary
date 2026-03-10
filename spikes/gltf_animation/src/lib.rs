//! # Spike 1 — GLTF scene hierarchy and AnimationPlayer access
//!
//! Time-box: 30 minutes.
//!
//! ## Findings
//!
//! ### Q1. Entity hierarchy after spawning a `SceneRoot` from `.glb`
//!
//! Spawning `SceneRoot(asset_server.load("model.glb#Scene0"))` produces a
//! hierarchy that mirrors the GLTF node tree. The entity that owns the
//! `SceneRoot` component sits at the top. Beneath it, Bevy creates one
//! entity per GLTF node, each with a `Name` component matching the GLTF
//! node name, plus `Transform` / `GlobalTransform`.
//!
//! `AnimationPlayer` is **not** on the `SceneRoot` entity itself — it is
//! placed on a **descendant** entity (typically the skeleton root node).
//! To find it, walk `Children::iter_descendants()` and query for
//! `AnimationPlayer`. The official Bevy 0.18 `animated_mesh.rs` example
//! demonstrates this pattern inside a `SceneInstanceReady` observer.
//!
//! ### Q2. Finding a named bone entity
//!
//! Yes. Every GLTF node becomes an entity with a `Name` component. Bone
//! nodes are no exception. Walking `Children::iter_descendants(scene_root)`
//! and matching on `Name` finds bones like `"hand.R"`, `"upper_arm.R"`,
//! etc. This is confirmed by the Bevy skinned-mesh examples that reference
//! bone entities by name after scene load.
//!
//! ### Q3. `AnimationGraph` in Bevy 0.18
//!
//! `AnimationGraph` is a Bevy `Asset`. Each node in the graph is either:
//! - a **clip node** (references an `AnimationClip` asset handle), or
//! - a **blend node** (blends child outputs by weight).
//!
//! Construction helpers:
//! - `AnimationGraph::from_clip(handle)` → graph with one clip node
//! - `AnimationGraph::from_clips([h1, h2, …])` → graph with N clip nodes
//!   under the root blend node
//!
//! Returns `(AnimationGraph, Vec<AnimationNodeIndex>)` — the indices are
//! used to address clips when telling `AnimationPlayer` what to play.
//!
//! Playback flow:
//! 1. Add the graph as an asset: `let handle = graphs.add(graph);`
//! 2. Insert `AnimationGraphHandle(handle)` on the entity with `AnimationPlayer`
//! 3. Use `AnimationTransitions` component for crossfade:
//!    ```ignore
//!    transitions.play(&mut player, idle_index, Duration::ZERO).repeat();
//!    // later…
//!    transitions.play(&mut player, walk_index, Duration::from_millis(250)).repeat();
//!    ```
//!
//! `AnimationTransitions` manages blending weights internally and prevents
//! conflicts that arise from calling `player.play()` directly when
//! switching clips.
//!
//! ### Q4. Scene readiness detection
//!
//! Bevy 0.18 fires `SceneInstanceReady` as a **trigger** on the entity
//! that owns the `SceneRoot` component, once all scene children are
//! spawned. The recommended pattern is an **observer**:
//!
//! ```ignore
//! commands
//!     .spawn((MyMarker, SceneRoot(handle)))
//!     .observe(on_scene_ready);
//!
//! fn on_scene_ready(
//!     trigger: On<SceneInstanceReady>,
//!     children: Query<&Children>,
//!     players: Query<&mut AnimationPlayer>,
//! ) {
//!     for child in children.iter_descendants(trigger.entity) { … }
//! }
//! ```
//!
//! An alternative is polling for `Added<AnimationPlayer>` in an `Update`
//! system (used in the `animated_mesh_control.rs` example). Both work;
//! the observer is more targeted and avoids per-frame query overhead.
//!
//! **Recommendation for this project:** Use the `SceneInstanceReady`
//! observer to initialise `AnimationController` and `IkChain`, since both
//! require the full GLTF hierarchy to exist.
//!
//! ## Plan impact
//!
//! No findings invalidate the plan. Confirmed:
//! - `AnimationPlayer` is on a descendant — plan's descendant-walk is correct
//! - Named bone lookup works — plan's `Name("hand.R")` approach is valid
//! - `AnimationGraph::from_clips` + `AnimationTransitions` supports the
//!   idle/walk crossfade design
//! - `SceneInstanceReady` observer is the preferred readiness mechanism
//! - `bevy_animation` **and** `bevy_gltf` features are both required
//!
//! One refinement: the plan's `AnimationController` should store
//! `AnimationNodeIndex` values (not clip handles) and the entity that holds
//! `AnimationPlayer` (which differs from the `SceneRoot` entity). This is
//! consistent with the plan's existing design but worth making explicit.

#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use bevy::asset::uuid::Uuid;
    use std::time::Duration;

    /// AnimationGraph can be built from multiple clip handles and returns
    /// one `AnimationNodeIndex` per clip plus the graph itself.
    #[test]
    fn animation_graph_from_clips_returns_correct_node_count() {
        // Create dummy handles — the graph only stores handles, it does not
        // load the assets, so no asset server is required.
        let clip_idle: Handle<AnimationClip> = Handle::default();
        let clip_walk: Handle<AnimationClip> = Handle::from(Uuid::from_u128(1));

        let (graph, indices) = AnimationGraph::from_clips([
            clip_idle.clone(),
            clip_walk.clone(),
        ]);

        // Two clips → two node indices.
        assert_eq!(indices.len(), 2, "from_clips should return one index per clip");

        // Indices should be distinct.
        assert_ne!(indices[0], indices[1], "clip node indices must differ");

        // Root node exists.
        let root = graph.root;
        assert!(
            graph.get(root).is_some(),
            "root node must exist in the graph"
        );
    }

    /// AnimationGraph::from_clip creates a single-clip graph.
    #[test]
    fn animation_graph_from_single_clip() {
        let clip: Handle<AnimationClip> = Handle::default();

        let (graph, index) = AnimationGraph::from_clip(clip);

        // Single clip → single AnimationNodeIndex (not a Vec).
        let _ = index;

        // Root node exists.
        let root = graph.root;
        assert!(graph.get(root).is_some(), "root node must exist");
    }

    /// AnimationTransitions can be default-constructed and used to drive
    /// playback on an AnimationPlayer.
    #[test]
    fn animation_transitions_play_sets_active_animation() {
        let clip_idle: Handle<AnimationClip> = Handle::default();
        let clip_walk: Handle<AnimationClip> = Handle::from(Uuid::from_u128(1));

        let (_graph, indices) = AnimationGraph::from_clips([
            clip_idle.clone(),
            clip_walk.clone(),
        ]);

        // Create an AnimationPlayer and AnimationTransitions.
        let mut player = AnimationPlayer::default();
        let mut transitions = AnimationTransitions::new();

        // Play idle with zero-duration transition (instant switch).
        transitions
            .play(&mut player, indices[0], Duration::ZERO)
            .repeat();

        // The player should now have at least one playing animation.
        assert!(
            player.playing_animations().count() > 0,
            "player should have an active animation after transitions.play()"
        );

        // Switch to walk with a 250ms crossfade.
        transitions
            .play(&mut player, indices[1], Duration::from_millis(250))
            .repeat();

        // After requesting a transition, the player may have two active
        // animations (old fading out, new fading in) until the transition
        // system ticks. At minimum, the new animation is queued.
        assert!(
            player.playing_animations().count() > 0,
            "player should have animations after crossfade request"
        );
    }

    /// GltfAssetLabel can construct asset paths for scenes and animations.
    #[test]
    fn gltf_asset_label_path_construction() {
        use bevy::gltf::GltfAssetLabel;

        let scene_path = GltfAssetLabel::Scene(0).from_asset("models/creature.glb");
        assert!(
            scene_path.to_string().contains("Scene0"),
            "scene label should include Scene0: {scene_path}"
        );

        let anim0 = GltfAssetLabel::Animation(0).from_asset("models/creature.glb");
        assert!(
            anim0.to_string().contains("Animation0"),
            "animation label should include Animation0: {anim0}"
        );

        let anim1 = GltfAssetLabel::Animation(1).from_asset("models/creature.glb");
        assert!(
            anim1.to_string().contains("Animation1"),
            "animation label should include Animation1: {anim1}"
        );
    }
}
