use std::fs;
use std::path::PathBuf;
use std::env;
use log::{info, warn};

pub struct Config {
    pub sp_dc: String,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        let mut config = Config {
            sp_dc: String::new(),
            port: 8080,
        };
        
        // Try to load from config file first
        if let Some(sp_dc) = Config::load_from_file() {
            info!("Loaded SP_DC from config file");
            config.sp_dc = sp_dc;
        } else if let Ok(sp_dc) = env::var("SP_DC") {
            // Fall back to environment variable
            info!("Loaded SP_DC from environment variable");
            config.sp_dc = sp_dc;
        } else {
            warn!("SP_DC not found in config file or environment variables");
        }
        
        // Get port from environment variable or use default
        if let Ok(port_str) = env::var("PORT") {
            if let Ok(port) = port_str.parse::<u16>() {
                config.port = port;
            }
        }
        
        config
    }
    
    fn load_from_file() -> Option<String> {
        // Check multiple possible config file locations
        let config_paths = vec![
            // Current directory
            PathBuf::from("config.toml"),
            // User's home directory
            dirs::home_dir().map(|p| p.join(".config/spotifylyricsapi/config.toml"))?,
            // System-wide config
            PathBuf::from("/etc/spotifylyricsapi/config.toml"),
        ];
        
        for path in config_paths {
            if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        info!("Found config file at: {}", path.display());
                        return parse_config_content(&content);
                    },
                    Err(e) => {
                        warn!("Failed to read config file at {}: {}", path.display(), e);
                    }
                }
            }
        }
        
        None
    }
    
    pub fn is_valid(&self) -> bool {
        !self.sp_dc.is_empty()
    }
}

fn parse_config_content(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("sp_dc") || line.starts_with("SP_DC") {
            let parts: Vec<&str> = line.splitn(2, '=').collect();
            if parts.len() == 2 {
                // Remove quotes and whitespace
                let value = parts[1].trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim();
                
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}