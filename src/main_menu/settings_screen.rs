use bevy::prelude::*;
use ui::{UiTheme, build_button};

use super::MenuEvent;

pub fn spawn(commands: &mut Commands, theme: &UiTheme) -> Vec<Entity> {
    let heading = commands
        .spawn((
            Text::new("Settings"),
            TextFont::from_font_size(theme.font_size_heading),
            TextColor(theme.text),
        ))
        .id();

    let audio_label = commands
        .spawn((
            Text::new("Audio — coming soon"),
            TextFont::from_font_size(theme.font_size_body),
            TextColor(theme.text_muted),
        ))
        .id();

    let video_label = commands
        .spawn((
            Text::new("Video — coming soon"),
            TextFont::from_font_size(theme.font_size_body),
            TextColor(theme.text_muted),
        ))
        .id();

    let controls_label = commands
        .spawn((
            Text::new("Controls — coming soon"),
            TextFont::from_font_size(theme.font_size_body),
            TextColor(theme.text_muted),
        ))
        .id();

    let back_button = build_button(theme)
        .with_text("Back")
        .with_event(MenuEvent::Title)
        .build(commands);

    vec![
        heading,
        audio_label,
        video_label,
        controls_label,
        back_button,
    ]
}
