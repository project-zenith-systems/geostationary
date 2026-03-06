//! Editor palette UI panels: tile selector, entity selector, and save/load buttons.

use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use bevy::ui::FocusPolicy;
use shared::app_state::AppState;
use things::ThingRegistry;
use tiles::TileKind;

/// Currently selected tile kind for painting.
#[derive(Resource, Debug, Clone, Copy)]
pub struct EditorSelectedTile(pub TileKind);

impl Default for EditorSelectedTile {
    fn default() -> Self {
        Self(TileKind::Floor)
    }
}

/// Currently selected entity template for spawn point placement, or `None`
/// when the tile brush is active.
#[derive(Resource, Debug, Clone, Default)]
pub struct EditorSelectedEntity(pub Option<EditorEntityTemplate>);

/// An entity template available in the editor entity palette.
#[derive(Debug, Clone)]
pub struct EditorEntityTemplate {
    pub name: String,
    pub kind: u16,
}

/// Editor tool mode: either painting tiles or placing entity spawn points.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTool {
    #[default]
    Tile,
    Entity,
}

/// Internal message type for editor UI interactions.
#[derive(Message, Clone, Debug)]
pub enum EditorUiEvent {
    SelectFloor,
    SelectWall,
    SelectEntity(String, u16),
    Save,
    Load,
}

/// Marker for the editor palette UI root node.
#[derive(Component)]
struct EditorPaletteRoot;

/// Spawns the editor palette UI on entering the editor.
pub fn spawn_palette_ui(mut commands: Commands, registry: Res<ThingRegistry>) {
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(10.0),
                top: Val::Px(10.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.1, 0.1, 0.15, 0.9)),
            Interaction::None,
            FocusPolicy::Block,
            DespawnOnExit(AppState::Editor),
            EditorPaletteRoot,
        ))
        .id();

    // Section: Tiles
    let tile_header = commands
        .spawn((
            Text::new("— Tiles —"),
            TextFont::from_font_size(18.0),
            TextColor(Color::srgb(0.8, 0.8, 0.85)),
        ))
        .id();

    let floor_btn = spawn_palette_button(&mut commands, "Floor", EditorUiEvent::SelectFloor);
    let wall_btn = spawn_palette_button(&mut commands, "Wall", EditorUiEvent::SelectWall);

    // Section: Entities — built from ThingRegistry::named_templates()
    let entity_header = commands
        .spawn((
            Text::new("— Entities —"),
            TextFont::from_font_size(18.0),
            TextColor(Color::srgb(0.8, 0.8, 0.85)),
        ))
        .id();

    // Collect named templates from the registry, sort by kind for stable ordering.
    let mut templates: Vec<(&str, u16)> = registry.named_templates().collect();
    templates.sort_by_key(|&(_, kind)| kind);

    let entity_btns: Vec<Entity> = templates
        .iter()
        .filter(|(name, _)| *name != "creature") // skip creature (player template)
        .map(|(name, kind)| {
            let label = capitalise(name);
            spawn_palette_button(
                &mut commands,
                &label,
                EditorUiEvent::SelectEntity(name.to_string(), *kind),
            )
        })
        .collect();

    // Section: File
    let file_header = commands
        .spawn((
            Text::new("— File —"),
            TextFont::from_font_size(18.0),
            TextColor(Color::srgb(0.8, 0.8, 0.85)),
        ))
        .id();

    let save_btn = spawn_palette_button(&mut commands, "Save", EditorUiEvent::Save);
    let load_btn = spawn_palette_button(&mut commands, "Load", EditorUiEvent::Load);

    let mut children = vec![tile_header, floor_btn, wall_btn, entity_header];
    children.extend(entity_btns);
    children.extend([file_header, save_btn, load_btn]);

    commands.entity(root).add_children(&children);
}

/// Capitalise the first letter of a string (for display labels).
fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn spawn_palette_button(commands: &mut Commands, label: &str, event: EditorUiEvent) -> Entity {
    let btn = commands
        .spawn((
            Button,
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(16.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.25, 0.25, 0.35)),
            PaletteButtonColors {
                normal: Color::srgb(0.25, 0.25, 0.35),
                hovered: Color::srgb(0.35, 0.35, 0.50),
                pressed: Color::srgb(0.15, 0.15, 0.22),
            },
            PaletteButtonEvent(event),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(label),
                TextFont::from_font_size(16.0),
                TextColor(Color::srgb(0.9, 0.9, 0.92)),
            ));
        })
        .id();
    btn
}

/// Button colors for palette buttons.
#[derive(Component, Clone)]
pub struct PaletteButtonColors {
    normal: Color,
    hovered: Color,
    pressed: Color,
}

/// The event a palette button fires on press.
#[derive(Component, Clone)]
pub struct PaletteButtonEvent(pub EditorUiEvent);

/// System: handle palette button interactions (color changes + event dispatch).
pub fn handle_palette_buttons(
    mut query: Query<
        (&Interaction, &mut BackgroundColor, &PaletteButtonColors, &PaletteButtonEvent),
        Changed<Interaction>,
    >,
    mut writer: MessageWriter<EditorUiEvent>,
) {
    for (interaction, mut bg, colors, event) in &mut query {
        match *interaction {
            Interaction::Pressed => {
                bg.0 = colors.pressed;
                writer.write(event.0.clone());
            }
            Interaction::Hovered => {
                bg.0 = colors.hovered;
            }
            Interaction::None => {
                bg.0 = colors.normal;
            }
        }
    }
}

/// System: process [`EditorUiEvent`] messages and update resources.
pub fn process_palette_events(
    mut events: MessageReader<EditorUiEvent>,
    mut selected_tile: ResMut<EditorSelectedTile>,
    mut selected_entity: ResMut<EditorSelectedEntity>,
    mut tool: ResMut<EditorTool>,
    mut save_events: MessageWriter<super::io::EditorSaveEvent>,
    mut load_events: MessageWriter<super::io::EditorLoadEvent>,
) {
    for event in events.read() {
        match event {
            EditorUiEvent::SelectFloor => {
                selected_tile.0 = TileKind::Floor;
                selected_entity.0 = None;
                *tool = EditorTool::Tile;
                info!("Editor: selected Floor tile");
            }
            EditorUiEvent::SelectWall => {
                selected_tile.0 = TileKind::Wall;
                selected_entity.0 = None;
                *tool = EditorTool::Tile;
                info!("Editor: selected Wall tile");
            }
            EditorUiEvent::SelectEntity(name, kind) => {
                selected_entity.0 = Some(EditorEntityTemplate {
                    name: name.clone(),
                    kind: *kind,
                });
                *tool = EditorTool::Entity;
                info!("Editor: selected entity template '{name}' (kind {kind})");
            }
            EditorUiEvent::Save => {
                save_events.write(super::io::EditorSaveEvent);
            }
            EditorUiEvent::Load => {
                load_events.write(super::io::EditorLoadEvent);
            }
        }
    }
}
