use std::{collections::BTreeMap, path::PathBuf};
use serde_yaml;
use serde::{Deserialize, Serialize};
use anyhow::Result;
use directories::BaseDirs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub locations: BTreeMap<String, Location>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub mode: LocationMode,
    pub cache_file: Option<String>
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationMode {
    Files,
    Folders
}

impl Default for LocationMode {
    fn default() -> Self {
        LocationMode::Files
    }
}

impl Default for Location {
    fn default() -> Self {
        Location {
            path: String::new(),
            mode: LocationMode::default(),
            cache_file: None
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            locations: BTreeMap::new()
        }
    }
}

impl Config {
    pub fn base_dir() -> PathBuf {
        BaseDirs::new().unwrap().config_dir()
            .join("blink-search")
    }

    pub fn path() -> PathBuf {
        Self::base_dir().join("blink.yml")
    }

    pub fn new() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            println!("Creating new config file: {}", path.to_string_lossy());

            std::fs::create_dir(Self::base_dir())?;

            let config = Config::default();
            let config_str = serde_yaml::to_string(&config)?;
            std::fs::write(&path, config_str)?;
            Ok(config)
        } else {
            let config_str = std::fs::read_to_string(&path)?;
            let config = serde_yaml::from_str(&config_str)?;
            Ok(config)
        }
    }
}
