use bevy::prelude::*;
use ui::UiTheme;

pub fn spawn(commands: &mut Commands, theme: &UiTheme) -> Vec<Entity> {
    let text = commands
        .spawn((
            Text::new("Loading..."),
            TextFont::from_font_size(theme.font_size_heading),
            TextColor(theme.text_muted),
        ))
        .id();

    vec![text]
}
