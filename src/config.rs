use bevy::log::warn;
use bevy::prelude::Resource;
use serde::Deserialize;

use ::config::{Config, Environment, File, FileFormat};

const CONFIG_BASENAME: &str = "config";

#[derive(Debug, Clone, Deserialize, Resource)]
pub struct AppConfig {
    pub network: NetworkConfig,
    pub window: WindowConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig { port: 7777 },
            window: WindowConfig {
                title: "Geostationary".to_string(),
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
        .add_source(File::new(CONFIG_BASENAME, FileFormat::Toml).required(false))
        .add_source(File::new(CONFIG_BASENAME, FileFormat::Ron).required(false))
        .add_source(Environment::with_prefix("GEOSTATIONARY").separator("__"));

    builder.build()?.try_deserialize()
}
