use bevy::{app::AppExit, prelude::*, state::state_scoped::DespawnOnExit};
use network::NetCommand;
use ui::*;

use crate::app_state::AppState;
use crate::config::AppConfig;

mod loading_screen;
mod settings_screen;
mod title_screen;

pub struct MainMenuPlugin;

impl Plugin for MainMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MenuEvent>();
        app.add_systems(OnEnter(AppState::MainMenu), (menu_setup, menu_init));
        app.add_systems(
            PreUpdate,
            menu_message_reader.run_if(in_state(AppState::MainMenu)),
        );
    }
}

#[derive(Message, Clone, Debug)]
pub enum MenuEvent {
    Title,
    Settings,
    Play,
    Join,
    Quit,
}

#[derive(Component)]
struct MenuRoot;

enum MenuEventResult {
    ReplaceChildren(Vec<Entity>),
    CloseMenu,
}

fn menu_setup(mut commands: Commands, theme: Res<UiTheme>) {
    commands.spawn((
        Node {
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            height: Val::Percent(100.0),
            width: Val::Percent(100.0),
            padding: UiRect::top(Val::Px(80.0)),
            row_gap: theme.gap,
            ..default()
        },
        BackgroundColor(theme.background),
        DespawnOnExit(AppState::MainMenu),
        MenuRoot,
    ));
}

fn menu_init(mut writer: MessageWriter<MenuEvent>) {
    writer.write(MenuEvent::Title);
}

fn menu_message_reader(
    mut commands: Commands,
    query: Query<Entity, With<MenuRoot>>,
    theme: Res<UiTheme>,
    config: Res<AppConfig>,
    mut messages: MessageReader<MenuEvent>,
    mut exit: MessageWriter<AppExit>,
    mut net_commands: MessageWriter<NetCommand>,
) {
    let Ok(menu_root_entity) = query.single() else {
        return;
    };

    for event in messages.read() {
        let result = match event {
            MenuEvent::Title => {
                MenuEventResult::ReplaceChildren(title_screen::spawn(&mut commands, theme.as_ref()))
            }
            MenuEvent::Settings => MenuEventResult::ReplaceChildren(settings_screen::spawn(
                &mut commands,
                theme.as_ref(),
            )),
            MenuEvent::Play => {
                net_commands.write(NetCommand::Host {
                    port: config.network.port,
                });
                MenuEventResult::ReplaceChildren(loading_screen::spawn(
                    &mut commands,
                    theme.as_ref(),
                ))
            }
            MenuEvent::Join => {
                let addr = ([127, 0, 0, 1], config.network.port).into();
                net_commands.write(NetCommand::Connect { addr });
                MenuEventResult::ReplaceChildren(loading_screen::spawn(
                    &mut commands,
                    theme.as_ref(),
                ))
            }
            MenuEvent::Quit => {
                exit.write(AppExit::Success);
                MenuEventResult::CloseMenu
            }
        };

        match result {
            MenuEventResult::ReplaceChildren(children) => {
                commands
                    .entity(menu_root_entity)
                    .despawn_children()
                    .add_children(&children);
            }
            MenuEventResult::CloseMenu => {
                commands.entity(menu_root_entity).despawn_children();
            }
        }
    }
}
