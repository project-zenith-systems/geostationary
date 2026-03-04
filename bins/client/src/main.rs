use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use input::InputPlugin;
use interactions::{ContextMenuAction, InteractionsPlugin};
use items::{InteractionRange, ItemsPlugin};
use main_menu::{MainMenuPlugin, MenuEvent};
use network::NetworkPlugin;
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use shared::app_state::AppState;
use shared::world_setup::BALL_RADIUS;
use things::{ThingRegistry, ThingsPlugin};
use tiles::TilesPlugin;
use ui::UiPlugin;

mod client;
mod main_menu;

const BALL_COLOR: (f32, f32, f32) = (1.0, 0.8, 0.0); // Bright yellow

fn parse_log_level(s: &str) -> Level {
    match s.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" | "warning" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

fn main() {
    let app_config = shared::config::load_config();
    let log_level = parse_log_level(&app_config.debug.log_level);

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: app_config.window.title.clone(),
                    ..default()
                }),
                ..default()
            })
            .set(LogPlugin {
                level: log_level,
                ..default()
            }),
    )
    .add_plugins(UiPlugin::new().with_event::<MenuEvent>().with_event::<ContextMenuAction>())
    .add_plugins(MainMenuPlugin)
    .add_plugins(NetworkPlugin)
    .add_plugins(PhysicsPlugin);

    if app_config.debug.physics_debug {
        app.add_plugins(PhysicsDebugPlugin);
    }

    app.add_plugins(TilesPlugin)
        .add_plugins(ThingsPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(atmospherics::AtmosphericsPlugin)
        .add_plugins(creatures::CreaturesPlugin)
        .add_plugins(souls::SoulsPlugin)
        .add_plugins(player::PlayerPlugin)
        .add_plugins(camera::CameraPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(shared::world_setup::WorldSetupPlugin)
        .add_plugins(client::ClientPlugin)
        .add_plugins(shared::server::ServerPlugin)
        .add_plugins(InputPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(InteractionsPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(ItemsPlugin)
        .insert_resource(InteractionRange(app_config.items.interaction_range))
        .init_state::<AppState>();

    // Pre-load visual assets once at startup so every spawned thing reuses the same handles.
    // This must come after ThingsPlugin + WorldSetupPlugin so ThingRegistry exists.
    let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
    let creature_mesh = meshes.add(Capsule3d::new(0.3, 1.0));
    let ball_mesh = meshes.add(Sphere::new(BALL_RADIUS));
    let can_mesh = meshes.add(Cylinder::new(0.15, 0.1));
    let toolbox_mesh = meshes.add(Cuboid::new(0.6, 0.3, 0.4));

    let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
    let creature_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.5, 0.2),
        ..default()
    });
    let ball_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(BALL_COLOR.0, BALL_COLOR.1, BALL_COLOR.2),
        ..default()
    });
    let can_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.7, 0.7, 0.75),
        metallic: 0.8,
        ..default()
    });
    let toolbox_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.2, 0.2),
        ..default()
    });

    // Override shared physics-only registrations with mesh + material.
    // Each template is responsible for its own collider, RigidBody, and visual.
    let mut registry = app.world_mut().resource_mut::<ThingRegistry>();
    registry.register(0, move |entity, _event, commands| {
        commands.entity(entity).insert((
            Mesh3d(creature_mesh.clone()),
            MeshMaterial3d(creature_mat.clone()),
            creatures::Creature,
            creatures::MovementSpeed::default(),
            things::InputDirection::default(),
            physics::RigidBody::Dynamic,
            physics::Collider::capsule(0.3, 1.0),
            physics::LockedAxes::ROTATION_LOCKED.lock_translation_y(),
            physics::GravityScale(0.0),
        ));
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                things::HandSlot { side: things::HandSide::Right },
                Transform::from_translation(things::HAND_OFFSET),
            ));
        });
    });
    registry.register(1, move |entity, _event, commands| {
        commands.entity(entity).insert((
            Mesh3d(ball_mesh.clone()),
            MeshMaterial3d(ball_mat.clone()),
            physics::Collider::sphere(BALL_RADIUS),
            physics::RigidBody::Dynamic,
            physics::GravityScale(1.0),
            physics::Restitution::new(0.8),
        ));
    });
    registry.register(2, move |entity, _event, commands| {
        commands.entity(entity).insert((
            Mesh3d(can_mesh.clone()),
            MeshMaterial3d(can_mat.clone()),
            physics::Collider::cylinder(0.15, 0.1),
            physics::RigidBody::Dynamic,
            physics::GravityScale(1.0),
            items::Item,
            Name::new("Can"),
        ));
    });
    registry.register(3, move |entity, _event, commands| {
        commands.entity(entity).insert((
            Mesh3d(toolbox_mesh.clone()),
            MeshMaterial3d(toolbox_mat.clone()),
            physics::Collider::cuboid(0.3, 0.15, 0.2),
            physics::RigidBody::Dynamic,
            physics::GravityScale(1.0),
            items::Item,
            Name::new("Toolbox"),
            items::Container::with_capacity(6),
        ));
    });

    app.run();
}
