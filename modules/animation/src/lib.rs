use bevy::prelude::*;
use bevy::transform::TransformSystems;
use core::time::Duration;
use std::collections::VecDeque;

use bevy::animation::graph::AnimationNodeIndex;

// ── Plugin ──────────────────────────────────────────────────────────────────

pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (
                drive_animation,
                solve_ik
                    .after(drive_animation)
                    .before(TransformSystems::Propagate),
            ),
        );
    }
}

// ── Animation state ─────────────────────────────────────────────────────────

/// High-level animation state driven by gameplay logic.
///
/// Wire-encodable via `From<u8>` / `Into<u8>` for network replication.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AnimState {
    #[default]
    Idle = 0,
    Walk = 1,
}

impl From<u8> for AnimState {
    fn from(value: u8) -> Self {
        match value {
            1 => AnimState::Walk,
            _ => AnimState::Idle,
        }
    }
}

impl From<AnimState> for u8 {
    fn from(state: AnimState) -> Self {
        state as u8
    }
}

// ── Animation controller ────────────────────────────────────────────────────

/// Maps [`AnimState`] variants to nodes in a Bevy [`AnimationGraph`].
///
/// Populated at scene-ready time (after the GLTF is fully loaded) — not at
/// spawn time, since clip handles are unavailable until then.
#[derive(Component)]
pub struct AnimationController {
    pub graph: Handle<AnimationGraph>,
    pub state_map: Vec<(AnimState, AnimationNodeIndex)>,
}

impl AnimationController {
    pub fn new(graph: Handle<AnimationGraph>, state_map: Vec<(AnimState, AnimationNodeIndex)>) -> Self {
        Self { graph, state_map }
    }

    /// Look up the graph node for a given animation state.
    pub fn node_for(&self, state: AnimState) -> Option<AnimationNodeIndex> {
        self.state_map
            .iter()
            .find(|(s, _)| *s == state)
            .map(|(_, idx)| *idx)
    }
}

// ── drive_animation system ──────────────────────────────────────────────────

/// Duration of the crossfade between animation clips.
const CROSSFADE_DURATION: Duration = Duration::from_millis(200);

/// Reacts to [`AnimState`] changes or freshly-added [`AnimationController`]
/// components and drives crossfade transitions on the entity's
/// [`AnimationPlayer`] (found among descendants of the root entity).
///
/// Ensures the player entity carries an [`AnimationGraphHandle`] referencing
/// the controller's graph before initiating any transition.
///
/// The `Added<AnimationController>` filter ensures the initial idle animation
/// starts as soon as the scene is ready, avoiding a T-pose on first load.
fn drive_animation(
    anim_q: Query<
        (Entity, &AnimState, &AnimationController),
        Or<(Changed<AnimState>, Added<AnimationController>, Added<AnimState>)>,
    >,
    children_q: Query<&Children>,
    mut player_q: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut commands: Commands,
) {
    for (entity, state, controller) in &anim_q {
        let Some(node) = controller.node_for(*state) else {
            continue;
        };

        // Walk descendants to find the AnimationPlayer (GLTF scenes place it
        // on a child entity, not the root).
        let Some(player_entity) = find_animation_player(entity, &children_q, &player_q) else {
            continue;
        };

        // Ensure the player entity references the correct animation graph so
        // that AnimationNodeIndex values resolve to the intended clips.
        // Only insert when missing to avoid spurious change-detection triggers.
        commands
            .entity(player_entity)
            .insert_if_new(AnimationGraphHandle(controller.graph.clone()));

        if let Ok((mut player, mut transitions)) = player_q.get_mut(player_entity) {
            transitions
                .play(&mut player, node, CROSSFADE_DURATION)
                .repeat();
        }
    }
}

/// Walk the entity hierarchy to find the first descendant with both an
/// [`AnimationPlayer`] and [`AnimationTransitions`] component.
fn find_animation_player(
    root: Entity,
    children_q: &Query<&Children>,
    player_q: &Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) -> Option<Entity> {
    // Check the root itself first.
    if player_q.contains(root) {
        return Some(root);
    }

    // BFS through descendants.
    let mut queue = VecDeque::new();
    if let Ok(children) = children_q.get(root) {
        for child in children.iter() {
            queue.push_back(child);
        }
    }

    while let Some(entity) = queue.pop_front() {
        if player_q.contains(entity) {
            return Some(entity);
        }
        if let Ok(children) = children_q.get(entity) {
            for child in children.iter() {
                queue.push_back(child);
            }
        }
    }

    None
}

// ── IK types ────────────────────────────────────────────────────────────────

/// A two-bone IK chain (e.g. upper_arm → forearm → hand).
///
/// Entity references and segment lengths are populated at scene-ready time
/// by name-matching against GLTF bone entities.
#[derive(Component, Clone)]
pub struct IkChain {
    /// Root of the chain (e.g. upper arm).
    pub root: Entity,
    /// Middle joint (e.g. forearm / elbow).
    pub mid: Entity,
    /// Tip / end-effector (e.g. hand).
    pub tip: Entity,
    /// Length of the upper segment (root → mid), set at construction time.
    pub upper_len: f32,
    /// Length of the lower segment (mid → tip), set at construction time.
    pub lower_len: f32,
}

/// Controls whether the IK chain should be actively solved.
#[derive(Component, Clone)]
pub struct HoldIk {
    /// When `true`, the [`solve_ik`] system will solve the chain.
    pub active: bool,
    /// Target position in the creature's local space.
    pub target: Vec3,
}

impl Default for HoldIk {
    fn default() -> Self {
        Self {
            active: false,
            target: Vec3::ZERO,
        }
    }
}

// ── solve_ik system ─────────────────────────────────────────────────────────

/// Two-bone IK solver. Runs in `PostUpdate` after `drive_animation` and
/// before `TransformSystems::Propagate`. When [`HoldIk::active`] is `true`,
/// solves the [`IkChain`] using the stored segment lengths and writes
/// local-space rotations to the root and mid bone entities.
///
/// Operates entirely in creature-local space — [`HoldIk::target`] is already
/// in local space, and bone positions are computed by walking the [`ChildOf`]
/// chain and composing local [`Transform`]s. No [`GlobalTransform`] is read,
/// so there is no one-frame lag from stale propagation data.
fn solve_ik(
    ik_q: Query<(Entity, &IkChain, &HoldIk)>,
    mut transform_q: Query<&mut Transform>,
    parent_q: Query<&ChildOf>,
) {
    for (creature_entity, chain, hold) in &ik_q {
        if !hold.active {
            continue;
        }

        // Target is already in creature-local space — no conversion needed.
        let target_local = hold.target;

        // Compute the root bone's position and its parent's rotation in the
        // creature's local coordinate frame by walking the ChildOf chain.
        let Some((root_pos, root_parent_rot)) = bone_local_data(
            chain.root,
            creature_entity,
            &transform_q,
            &parent_q,
        ) else {
            continue;
        };

        // Compute the mid bone's position in creature-local space as well, so
        // the IK solver can derive the correct bend plane from the actual
        // current joint layout instead of assuming a fixed forward axis.
        // NOTE: the parent rotation is intentionally discarded here; it is
        // re-queried after the root write-back below so that it reflects the
        // already-updated root transform.
        let Some((mid_pos, _)) = bone_local_data(
            chain.mid,
            creature_entity,
            &transform_q,
            &parent_q,
        ) else {
            continue;
        };

        // Compute the tip bone's position in creature-local space so it can
        // be passed to the solver (even though the current implementation
        // does not use it, this avoids a suspicious-looking Vec3::ZERO).
        let Some((tip_pos, _)) = bone_local_data(
            chain.tip,
            creature_entity,
            &transform_q,
            &parent_q,
        ) else {
            continue;
        };

        let upper_len = chain.upper_len;
        let lower_len = chain.lower_len;

        // Solve the two-bone IK in creature-local space.
        // The mid joint position (`mid_pos`) is used to establish the
        // pole-vector / bend plane; the tip joint position (`tip_pos`) is
        // likewise provided in creature-local space.
        let Some((root_rot, mid_rot)) =
            solve_two_bone(root_pos, mid_pos, tip_pos, target_local, upper_len, lower_len)
        else {
            continue;
        };

        // Write rotations back in bone-local space by removing the parent's
        // creature-local rotation contribution.
        if let Ok(mut root_tf) = transform_q.get_mut(chain.root) {
            root_tf.rotation = (root_parent_rot.inverse() * root_rot).normalize();
        }

        // Compute the mid bone's parent rotation in creature-local space by
        // walking the actual ChildOf chain, which may include intermediate
        // joints between the root and mid bones.
        let Some((_, mid_parent_rot)) =
            bone_local_data(chain.mid, creature_entity, &transform_q, &parent_q)
        else {
            continue;
        };

        if let Ok(mut mid_tf) = transform_q.get_mut(chain.mid) {
            mid_tf.rotation = (mid_parent_rot.inverse() * mid_rot).normalize();
        }
    }
}

/// Compute a bone's position and its parent's rotation in the `ancestor`
/// entity's local coordinate frame by walking the [`ChildOf`] chain from
/// `bone` up to `ancestor` and composing local [`Transform`]s.
///
/// Returns `None` if any entity in the chain is missing a [`Transform`] or
/// [`ChildOf`] component, or if `ancestor` is not an ancestor of `bone`.
///
/// The query type is `&Query<&mut Transform>` because the caller (`solve_ik`)
/// needs mutable access elsewhere; this function only reads via `get()`.
fn bone_local_data(
    bone: Entity,
    ancestor: Entity,
    transform_q: &Query<&mut Transform>,
    parent_q: &Query<&ChildOf>,
) -> Option<(Vec3, Quat)> {
    // Collect local transforms from bone up to (but not including) ancestor.
    let mut chain: Vec<Transform> = Vec::new();
    let mut current = bone;
    while current != ancestor {
        let tf = *transform_q.get(current).ok()?;
        chain.push(tf);
        current = parent_q.get(current).ok()?.0;
    }

    // Edge case: bone IS the ancestor (shouldn't occur in practice).
    if chain.is_empty() {
        return Some((Vec3::ZERO, Quat::IDENTITY));
    }

    // Compose from the ancestor side down to the bone.
    // chain = [bone_tf, bone_parent_tf, …, ancestor_child_tf]
    // Start from identity (ancestor's own local frame).
    let mut pos = Vec3::ZERO;
    let mut rot = Quat::IDENTITY;
    let mut scale = Vec3::ONE;

    // Apply all transforms EXCEPT the bone's own (chain[0]) to get the
    // bone-parent's transform in the ancestor's frame.
    for tf in chain[1..].iter().rev() {
        pos += rot * (scale * tf.translation);
        rot = (rot * tf.rotation).normalize();
        scale *= tf.scale;
    }

    let parent_rot = rot;

    // Apply the bone's own transform to obtain its position in the
    // ancestor's frame.
    let bone_tf = &chain[0];
    pos += rot * (scale * bone_tf.translation);

    Some((pos, parent_rot))
}

/// Analytical two-bone IK. Returns rotations (in the caller's coordinate
/// frame) for the root and mid joints. Targets beyond the chain length are
/// clamped to the maximum reachable distance and still produce a solution.
/// Returns `None` when the target coincides with the root position
/// (zero-length direction) or when either segment length is near-zero.
fn solve_two_bone(
    root_pos: Vec3,
    mid_pos: Vec3,
    _tip_pos: Vec3,
    target: Vec3,
    upper_len: f32,
    lower_len: f32,
) -> Option<(Quat, Quat)> {
    // Guard: degenerate segment lengths would cause NaN in law-of-cosines
    // denominators (division by 2 * upper_len * dist or 2 * upper_len *
    // lower_len).
    if upper_len <= f32::EPSILON || lower_len <= f32::EPSILON {
        return None;
    }

    let total_len = upper_len + lower_len;
    let to_target = target - root_pos;
    let dist = to_target.length();

    if dist < f32::EPSILON {
        return None;
    }

    // Clamp distance so the chain can always reach (fully extended or folded).
    // Ensure clamped value stays positive even for very short chains.
    let dist_clamped = dist.min(total_len - f32::EPSILON).max(f32::EPSILON);

    // Law of cosines: angle at the root joint.
    let cos_root = ((upper_len * upper_len + dist_clamped * dist_clamped
        - lower_len * lower_len)
        / (2.0 * upper_len * dist_clamped))
        .clamp(-1.0, 1.0);
    let root_angle = cos_root.acos();

    // Law of cosines: angle at the mid (elbow) joint.
    let cos_mid = ((upper_len * upper_len + lower_len * lower_len
        - dist_clamped * dist_clamped)
        / (2.0 * upper_len * lower_len))
        .clamp(-1.0, 1.0);
    let mid_angle = cos_mid.acos();

    // Build rotations. Use the current mid position to determine the bend
    // plane (pole vector) and the bone's rest direction.
    let dir_to_target = to_target / dist;

    // Derive the upper bone's rest direction from the actual current pose
    // instead of assuming a fixed +Y axis.
    let initial_dir = (mid_pos - root_pos).normalize_or_zero();
    let rest_dir = if initial_dir.length_squared() > f32::EPSILON {
        initial_dir
    } else {
        Vec3::Y
    };

    let pole_hint = if rest_dir.cross(dir_to_target).length_squared() > f32::EPSILON * f32::EPSILON {
        rest_dir.cross(dir_to_target).normalize()
    } else {
        // Fallback: use world up or forward as a pole hint.
        let alt = if dir_to_target.dot(Vec3::Y).abs() < 0.99 {
            Vec3::Y
        } else {
            Vec3::Z
        };
        dir_to_target.cross(alt).normalize()
    };

    // Root rotation: rotate the upper bone from its rest direction toward
    // the target, offset by the root angle in the bend plane.
    let root_rot = Quat::from_rotation_arc(rest_dir, dir_to_target)
        * Quat::from_axis_angle(pole_hint, -root_angle);

    // Mid rotation: bend the lower bone by the interior elbow angle.
    let mid_rot = root_rot * Quat::from_axis_angle(pole_hint, std::f32::consts::PI - mid_angle);

    Some((root_rot.normalize(), mid_rot.normalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anim_state_round_trip_u8() {
        assert_eq!(AnimState::from(0u8), AnimState::Idle);
        assert_eq!(AnimState::from(1u8), AnimState::Walk);
        // Unknown values fall back to Idle.
        assert_eq!(AnimState::from(255u8), AnimState::Idle);

        assert_eq!(u8::from(AnimState::Idle), 0);
        assert_eq!(u8::from(AnimState::Walk), 1);
    }

    #[test]
    fn controller_node_lookup() {
        // Use a default handle (not a real asset) for the test.
        let graph = Handle::<AnimationGraph>::default();
        let idle_idx = AnimationNodeIndex::new(0);
        let walk_idx = AnimationNodeIndex::new(1);

        let controller = AnimationController::new(
            graph,
            vec![
                (AnimState::Idle, idle_idx),
                (AnimState::Walk, walk_idx),
            ],
        );

        assert_eq!(controller.node_for(AnimState::Idle), Some(idle_idx));
        assert_eq!(controller.node_for(AnimState::Walk), Some(walk_idx));
    }

    #[test]
    fn hold_ik_default_inactive() {
        let hold = HoldIk::default();
        assert!(!hold.active);
        assert_eq!(hold.target, Vec3::ZERO);
    }

    #[test]
    fn two_bone_ik_basic_reach() {
        // Chain along +Y: root at origin, mid at (0,1,0), tip at (0,2,0).
        let root = Vec3::ZERO;
        let mid = Vec3::new(0.0, 1.0, 0.0);
        let tip = Vec3::new(0.0, 2.0, 0.0);

        // Target directly in front at +Z, reachable distance.
        let target = Vec3::new(0.0, 0.0, 1.5);

        let result = solve_two_bone(root, mid, tip, target, 1.0, 1.0);
        assert!(result.is_some(), "IK should find a solution for a reachable target");
    }

    #[test]
    fn two_bone_ik_unreachable_returns_clamped() {
        // Chain with total length 2.0; target at distance 5.0.
        let root = Vec3::ZERO;
        let mid = Vec3::new(0.0, 1.0, 0.0);
        let tip = Vec3::new(0.0, 2.0, 0.0);
        let target = Vec3::new(0.0, 5.0, 0.0);

        // Should still return a solution (clamped to max reach).
        let result = solve_two_bone(root, mid, tip, target, 1.0, 1.0);
        assert!(result.is_some(), "IK should clamp and return a solution");
    }

    #[test]
    fn two_bone_ik_zero_distance_returns_none() {
        let root = Vec3::ZERO;
        let mid = Vec3::new(0.0, 1.0, 0.0);
        let tip = Vec3::new(0.0, 2.0, 0.0);
        let target = Vec3::ZERO; // Same as root.

        let result = solve_two_bone(root, mid, tip, target, 1.0, 1.0);
        assert!(result.is_none(), "IK should return None when target is at root");
    }

    #[test]
    fn two_bone_ik_zero_segment_returns_none() {
        let root = Vec3::ZERO;
        let mid = Vec3::new(0.0, 1.0, 0.0);
        let tip = Vec3::new(0.0, 2.0, 0.0);
        let target = Vec3::new(0.0, 0.0, 1.5);

        // Zero upper_len should early-return None (avoids NaN).
        assert!(solve_two_bone(root, mid, tip, target, 0.0, 1.0).is_none());
        // Zero lower_len likewise.
        assert!(solve_two_bone(root, mid, tip, target, 1.0, 0.0).is_none());
    }
}
