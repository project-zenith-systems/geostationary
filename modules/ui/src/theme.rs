use bevy::prelude::*;

#[derive(Resource, Clone, Debug)]
pub struct UiTheme {
    pub background: Color,
    pub surface: Color,
    pub primary: Color,
    pub primary_hover: Color,
    pub primary_press: Color,
    pub text: Color,
    pub text_muted: Color,

    pub font_size_heading: f32,
    pub font_size_body: f32,
    pub font_size_small: f32,

    pub button_padding: UiRect,
    pub panel_padding: UiRect,
    pub gap: Val,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            background: Color::srgb(0.08, 0.08, 0.12),
            surface: Color::srgb(0.14, 0.14, 0.20),
            primary: Color::srgb(0.25, 0.25, 0.35),
            primary_hover: Color::srgb(0.35, 0.35, 0.50),
            primary_press: Color::srgb(0.15, 0.15, 0.22),
            text: Color::srgb(0.90, 0.90, 0.92),
            text_muted: Color::srgb(0.55, 0.55, 0.60),

            font_size_heading: 60.0,
            font_size_body: 28.0,
            font_size_small: 18.0,

            button_padding: UiRect::axes(Val::Px(40.0), Val::Px(12.0)),
            panel_padding: UiRect::all(Val::Px(24.0)),
            gap: Val::Px(20.0),
        }
    }
}
