use bevy::{app::AppExit, prelude::*, state::state::FreelyMutableState, state::state_scoped::DespawnOnExit};
use network::{ClientEvent, NetCommand, NetworkSet, ServerEvent};
use ui::*;

mod loading_screen;
mod settings_screen;
mod title_screen;

/// Configuration resource injected by the binary crate.
/// Decouples the main-menu module from `AppConfig`.
#[derive(Resource, Clone)]
pub struct MainMenuConfig {
    pub port: u16,
    pub player_name: String,
}

pub struct MainMenuPlugin<S: FreelyMutableState + Copy> {
    pub state: S,
}

impl<S: FreelyMutableState + Copy> Plugin for MainMenuPlugin<S> {
    fn build(&self, app: &mut App) {
        app.insert_resource(MainMenuActiveState(self.state));
        app.add_message::<MenuEvent>();
        app.add_systems(OnEnter(self.state), (menu_setup::<S>, menu_init));
        app.add_systems(
            PreUpdate,
            (handle_network_errors, menu_message_reader::<S>)
                .chain()
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send)
                .run_if(in_state(self.state)),
        );
    }
}

#[derive(Resource, Clone, Copy)]
struct MainMenuActiveState<S: States>(S);

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

fn menu_setup<S: States + Copy>(mut commands: Commands, theme: Res<UiTheme>, active_state: Res<MainMenuActiveState<S>>) {
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
        DespawnOnExit(active_state.0),
        MenuRoot,
    ));
}

fn menu_init(mut writer: MessageWriter<MenuEvent>) {
    writer.write(MenuEvent::Title);
}

/// Resets the menu to the title screen when network errors or disconnects
/// occur while still in MainMenu state (e.g. during the loading screen).
fn handle_network_errors(
    mut client_events: MessageReader<ClientEvent>,
    mut server_events: MessageReader<ServerEvent>,
    mut menu_events: MessageWriter<MenuEvent>,
) {
    for event in client_events.read() {
        if matches!(
            event,
            ClientEvent::Error(_) | ClientEvent::Disconnected { .. }
        ) {
            menu_events.write(MenuEvent::Title);
        }
    }
    for event in server_events.read() {
        if matches!(event, ServerEvent::Error(_)) {
            menu_events.write(MenuEvent::Title);
        }
    }
}

fn menu_message_reader<S: States + Copy>(
    mut commands: Commands,
    query: Query<Entity, With<MenuRoot>>,
    theme: Res<UiTheme>,
    config: Res<MainMenuConfig>,
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
                    port: config.port,
                });
                MenuEventResult::ReplaceChildren(loading_screen::spawn(
                    &mut commands,
                    theme.as_ref(),
                ))
            }
            MenuEvent::Join => {
                net_commands.write(NetCommand::Connect {
                    addr: ([127u8, 0u8, 0u8, 1u8], config.port).into(),
                    name: config.player_name.clone(),
                });
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
