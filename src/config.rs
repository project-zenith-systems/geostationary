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
    }

    #[test]
    fn test_debug_config_default() {
        let config = DebugConfig { physics_debug: false };
        assert_eq!(config.physics_debug, false);
    }

    #[test]
    fn test_debug_config_enabled() {
        let config = DebugConfig { physics_debug: true };
        assert_eq!(config.physics_debug, true);
    }
}
