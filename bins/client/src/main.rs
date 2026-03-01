use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use input::InputPlugin;
use interactions::{ContextMenuAction, InteractionsPlugin};
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
        .init_state::<AppState>();

    // Pre-load ball assets once at startup so every spawned ball reuses the same handles.
    // This must come after ThingsPlugin + WorldSetupPlugin so ThingRegistry exists.
    let ball_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(BALL_RADIUS));
    let ball_mat = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(BALL_COLOR.0, BALL_COLOR.1, BALL_COLOR.2),
            ..default()
        });

    // Override the shared physics-only ball registration with mesh + material + physics.
    app.world_mut()
        .resource_mut::<ThingRegistry>()
        .register(1, move |entity, _event, commands| {
            commands.entity(entity).insert((
                Mesh3d(ball_mesh.clone()),
                MeshMaterial3d(ball_mat.clone()),
                physics::Collider::sphere(BALL_RADIUS),
                physics::GravityScale(1.0),
                physics::Restitution::new(0.8),
            ));
        });

    app.run();
}
