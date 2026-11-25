//! Arcanum client building blocks: Bevy asset integration for the game's VFS.

pub mod actors;
pub mod animation;
pub mod pc_sheet;
pub mod picking;
pub mod art_loader;
pub mod audio;
pub mod bitmap_text;
pub mod character_editor;
pub mod character_host;
pub mod brightness;
pub mod config;
pub mod cursor;
pub mod editor_tabs;
pub mod create_character;
pub mod dialogue_ui;
pub mod vote_popup;
pub mod fate_window;
pub mod written_window;
pub mod fonts;
pub mod hotkey_bar;
pub mod hud;
pub mod hud_clock;
pub mod inventory_screen;
pub mod isometric;
pub mod load_game;
pub mod loading;
pub mod main_menu;
pub mod message_box;
pub mod menu_window;
pub mod mes_loader;
pub mod new_game;
pub mod premade;
pub mod options;
pub mod resource_pack;
pub mod rest_window;
pub mod screens;
pub mod settings;
pub mod sim_bridge;
pub mod skill_use;
pub mod spell_bar;
pub mod text_field;
pub mod vfs_asset;
pub mod widgets;
pub mod world_render;

use bevy::prelude::*;

pub use art_loader::{ArtAssetLoader, ArtFrames, ArtFramesLoader};
pub use audio::MenuAudioPlugin;
pub use bitmap_text::BitmapTextPlugin;
pub use brightness::BrightnessPlugin;
pub use character_editor::CharacterEditorPlugin;
pub use config::{ConfigFile, ConfigPlugin};
pub use create_character::CreateCharacterPlugin;
pub use fonts::FontsPlugin;
pub use load_game::LoadGamePlugin;
pub use loading::LoadingPlugin;
pub use new_game::NewGamePlugin;
pub use main_menu::{AppState, MainMenuPlugin};
pub use screens::ScreensPlugin;
pub use mes_loader::{MesAsset, MesAssetLoader};
pub use options::OptionsPlugin;
pub use resource_pack::ResourcePack;
pub use settings::{DisplayPlugin, DisplayResolution, GameSettings};
pub use vfs_asset::{SharedVfs, VfsAssetPlugin, VfsAssetReader};
pub use world_render::WorldRenderPlugin;

/// Registers the `.art` (image and frame-set) and `.mes` asset loaders. Add after `DefaultPlugins`.
pub struct ArcanumAssetsPlugin;

impl Plugin for ArcanumAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<MesAsset>()
            .init_asset::<ArtFrames>()
            .register_asset_loader(ArtAssetLoader)
            .register_asset_loader(ArtFramesLoader)
            .register_asset_loader(MesAssetLoader);
    }
}
