use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ConfigData {
    pub deepseek_api_key: String,
    pub glm_api_key: String,
}

pub struct ConfigManager;

impl ConfigManager {
    /// Loads the configuration data from the standard location.
    /// Automatically generates directories and standard config file if they do not exist.
    pub fn load_or_create() -> (Option<String>, ConfigData) {
        let mut config_path_str = None;
        let mut config_data = ConfigData::default();

        if let Some(proj_dirs) = directories::ProjectDirs::from("com", "cobolx", "rdo") {
            let config_dir = proj_dirs.config_dir();
            let config_file_path = config_dir.join("config.json");
            config_path_str = Some(config_file_path.to_string_lossy().to_string());

            if !config_dir.exists() {
                let _ = std::fs::create_dir_all(config_dir);
            }

            if !config_file_path.exists() {
                let template = ConfigData {
                    deepseek_api_key: "".to_string(),
                    glm_api_key: "".to_string(),
                };
                if let Ok(serialized) = serde_json::to_string_pretty(&template) {
                    let _ = std::fs::write(&config_file_path, serialized);
                }
            } else if let Ok(content) = std::fs::read_to_string(&config_file_path) {
                if let Ok(parsed) = serde_json::from_str::<ConfigData>(&content) {
                    config_data = parsed;
                }
            }
        }

        (config_path_str, config_data)
    }

    /// Saves the configuration data back to the file.
    pub fn save(data: &ConfigData) -> Result<(), std::io::Error> {
        if let Some(proj_dirs) = directories::ProjectDirs::from("com", "cobolx", "rdo") {
            let config_dir = proj_dirs.config_dir();
            let config_file_path = config_dir.join("config.json");
            if !config_dir.exists() {
                std::fs::create_dir_all(config_dir)?;
            }
            let serialized = serde_json::to_string_pretty(data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(config_file_path, serialized)?;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not locate project directories",
            ));
        }
        Ok(())
    }
}
