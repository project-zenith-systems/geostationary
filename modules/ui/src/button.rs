use bevy::prelude::*;

use crate::theme::UiTheme;

pub struct ButtonBuilder {
    text: Option<String>,
    colors: ButtonColors,
    node: Node,
    font_size: f32,
    text_color: Color,
}

pub struct ButtonBuilderWithMessage<T: Message + Clone> {
    inner: ButtonBuilder,
    message: ButtonMessage<T>,
}

pub fn build_button(theme: &UiTheme) -> ButtonBuilder {
    ButtonBuilder {
        text: None,
        colors: ButtonColors {
            normal: theme.primary,
            hovered: theme.primary_hover,
            pressed: theme.primary_press,
        },
        node: Node {
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            padding: theme.button_padding,
            ..default()
        },
        font_size: theme.font_size_body,
        text_color: theme.text,
    }
}

impl ButtonBuilder {
    pub fn with_text(mut self, text: &str) -> Self {
        self.text = Some(text.to_string());
        self
    }

    pub fn with_colors(mut self, normal: Color, hovered: Color, pressed: Color) -> Self {
        self.colors = ButtonColors {
            normal,
            hovered,
            pressed,
        };
        self
    }

    pub fn with_event<T: Message + Clone>(self, message: T) -> ButtonBuilderWithMessage<T> {
        ButtonBuilderWithMessage {
            inner: self,
            message: ButtonMessage { on_press: message },
        }
    }

    pub fn build(self, commands: &mut Commands) -> Entity {
        let mut entity = commands.spawn((
            Button,
            self.node,
            BackgroundColor(self.colors.normal),
            self.colors,
        ));

        if let Some(text) = self.text {
            entity.with_children(|parent| {
                parent.spawn((
                    Text::new(text),
                    TextFont::from_font_size(self.font_size),
                    TextColor(self.text_color),
                ));
            });
        }

        entity.id()
    }

    fn build_with_message<T: Message + Clone>(
        self,
        commands: &mut Commands,
        message: ButtonMessage<T>,
    ) -> Entity {
        let mut entity = commands.spawn((
            Button,
            self.node,
            BackgroundColor(self.colors.normal),
            self.colors,
            message,
        ));

        if let Some(text) = self.text {
            entity.with_children(|parent| {
                parent.spawn((
                    Text::new(text),
                    TextFont::from_font_size(self.font_size),
                    TextColor(self.text_color),
                ));
            });
        }

        entity.id()
    }
}

impl<T: Message + Clone> ButtonBuilderWithMessage<T> {
    pub fn build(self, commands: &mut Commands) -> Entity {
        self.inner.build_with_message(commands, self.message)
    }
}

#[derive(Component, Clone, Debug)]
pub struct ButtonColors {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
}

#[derive(Component, Clone)]
pub(crate) struct ButtonMessage<T: Message + Clone> {
    pub(crate) on_press: T,
}

pub(crate) fn change_button_colors(
    mut query: Query<(&Interaction, &mut BackgroundColor, &ButtonColors), Changed<Interaction>>,
) {
    for (interaction, mut background_color, colors) in &mut query {
        match *interaction {
            Interaction::Pressed => background_color.0 = colors.pressed,
            Interaction::Hovered => background_color.0 = colors.hovered,
            Interaction::None => background_color.0 = colors.normal,
        }
    }
}

pub(crate) fn process_button_messages<T: Message + Clone>(
    query: Query<(&Interaction, &ButtonMessage<T>), Changed<Interaction>>,
    mut writer: MessageWriter<T>,
) {
    for (interaction, button_msg) in &query {
        if *interaction == Interaction::Pressed {
            writer.write(button_msg.on_press.clone());
        }
    }
}
