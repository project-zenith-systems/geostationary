use bevy::prelude::*;
use ui::{UiTheme, build_button};

use super::MenuEvent;

pub fn spawn(commands: &mut Commands, theme: &UiTheme) -> Vec<Entity> {
    let title = commands
        .spawn((
            Text::new("GEOSTATIONARY"),
            TextFont::from_font_size(theme.font_size_heading),
            TextColor(theme.text),
        ))
        .id();

    let subtitle = commands
        .spawn((
            Text::new("A space station simulation"),
            TextFont::from_font_size(theme.font_size_small),
            TextColor(theme.text_muted),
        ))
        .id();

    let spacer = commands
        .spawn(Node {
            height: Val::Px(30.0),
            ..default()
        })
        .id();

    let play_button = build_button(theme)
        .with_text("Play")
        .with_event(MenuEvent::Play)
        .build(commands);

    let join_button = build_button(theme)
        .with_text("Join")
        .with_event(MenuEvent::Join)
        .build(commands);

    let editor_button = build_button(theme)
        .with_text("Editor")
        .with_event(MenuEvent::Editor)
        .build(commands);

    let settings_button = build_button(theme)
        .with_text("Settings")
        .with_event(MenuEvent::Settings)
        .build(commands);

    let quit_button = build_button(theme)
        .with_text("Quit")
        .with_event(MenuEvent::Quit)
        .build(commands);

    vec![
        title,
        subtitle,
        spacer,
        play_button,
        join_button,
        editor_button,
        settings_button,
        quit_button,
    ]
}
