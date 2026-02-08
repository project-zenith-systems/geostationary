use bevy::{app::AppExit, prelude::*, state::state_scoped::DespawnOnExit};
use network::NetCommand;
use ui::*;

use crate::app_state::AppState;

mod settings_screen;
mod title_screen;

pub struct MainMenuPlugin;

impl Plugin for MainMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<MenuState>();
        app.add_systems(OnEnter(AppState::MainMenu), (menu_setup, menu_init));
        app.add_systems(
            PreUpdate,
            menu_message_reader.run_if(in_state(AppState::MainMenu)),
        );
    }
}

#[derive(SubStates, Clone, PartialEq, Eq, Hash, Debug, Default)]
#[source(AppState = AppState::MainMenu)]
pub enum MenuState {
    #[default]
    Title,
    Settings,
}

#[derive(Message, Clone, Debug)]
pub enum MenuEvent {
    Title,
    Settings,
    Play,
    Hide,
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
                net_commands.write(NetCommand::HostLocal { port: 7777 });
                MenuEventResult::CloseMenu
            }
            MenuEvent::Hide => MenuEventResult::CloseMenu,
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
