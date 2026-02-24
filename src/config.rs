use bevy::log::warn;
use bevy::prelude::Resource;
use serde::Deserialize;

use ::config::{Config, Environment, File, FileFormat};

const CONFIG_BASENAME: &str = "config";

#[derive(Debug, Clone, Deserialize, Resource)]
pub struct AppConfig {
    pub network: NetworkConfig,
    pub window: WindowConfig,
    pub debug: DebugConfig,
    pub atmospherics: AtmosphericsConfig,
    pub souls: SoulsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig { port: 7777 },
            window: WindowConfig {
                title: "Geostationary".to_string(),
            },
            debug: DebugConfig {
                physics_debug: false,
            },
            atmospherics: AtmosphericsConfig {
                standard_pressure: 101.325,
            },
            souls: SoulsConfig {
                player_name: "Player".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowConfig {
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DebugConfig {
    pub physics_debug: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AtmosphericsConfig {
    pub standard_pressure: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SoulsConfig {
    /// Display name shown above the player's creature.
    pub player_name: String,
}

pub fn load_config() -> AppConfig {
    match load_config_inner() {
        Ok(config) => config,
        Err(error) => {
            warn!("Failed to load config, using defaults: {error}");
            AppConfig::default()
        }
    }
}

fn load_config_inner() -> Result<AppConfig, ::config::ConfigError> {
    let defaults = AppConfig::default();

    let builder = Config::builder()
        .set_default("network.port", defaults.network.port)?
        .set_default("window.title", defaults.window.title)?
        .set_default("debug.physics_debug", defaults.debug.physics_debug)?
        .set_default(
            "atmospherics.standard_pressure",
            defaults.atmospherics.standard_pressure as f64,
        )?
        .set_default("souls.player_name", defaults.souls.player_name)?
        .add_source(File::new(CONFIG_BASENAME, FileFormat::Toml).required(false))
        .add_source(File::new(CONFIG_BASENAME, FileFormat::Ron).required(false))
        .add_source(Environment::with_prefix("GEOSTATIONARY").separator("__"));

    builder.build()?.try_deserialize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.network.port, 7777);
        assert_eq!(config.window.title, "Geostationary");
        assert_eq!(config.debug.physics_debug, false);
        assert_eq!(config.atmospherics.standard_pressure, 101.325);
        assert_eq!(config.souls.player_name, "Player");
    }
}
