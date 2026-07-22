use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::csv_document::CsvDelimiter;

pub const DEFAULT_HEADER_ROWS: usize = 2;
pub const MIN_HEADER_ROWS: usize = 1;
pub const MAX_HEADER_ROWS: usize = 5;

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Settings {
    pub recent_workspace: Option<PathBuf>,
    #[serde(default)]
    pub workspaces: BTreeMap<PathBuf, WorkspaceSettings>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceSettings {
    #[serde(default)]
    pub files: BTreeMap<PathBuf, FilePreferences>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilePreferences {
    #[serde(default = "default_header_rows")]
    pub header_rows: usize,
    #[serde(default)]
    pub delimiter: Option<CsvDelimiter>,
}

impl Default for FilePreferences {
    fn default() -> Self {
        Self {
            header_rows: DEFAULT_HEADER_ROWS,
            delimiter: None,
        }
    }
}

impl FilePreferences {
    fn normalized(mut self) -> Self {
        if !(MIN_HEADER_ROWS..=MAX_HEADER_ROWS).contains(&self.header_rows) {
            self.header_rows = DEFAULT_HEADER_ROWS;
        }
        self
    }
}

impl Settings {
    pub fn file_preferences(&self, workspace: &Path, relative_path: &Path) -> FilePreferences {
        self.workspaces
            .get(workspace)
            .and_then(|workspace| workspace.files.get(relative_path))
            .copied()
            .unwrap_or_default()
            .normalized()
    }

    pub fn set_file_preferences(
        &mut self,
        workspace: &Path,
        relative_path: &Path,
        preferences: FilePreferences,
    ) {
        self.workspaces
            .entry(workspace.to_path_buf())
            .or_default()
            .files
            .insert(relative_path.to_path_buf(), preferences.normalized());
    }
}

const fn default_header_rows() -> usize {
    DEFAULT_HEADER_ROWS
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("application configuration directory is unavailable")]
    ConfigDirectoryUnavailable,
    #[error("failed to read settings from {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("settings in {path} are invalid: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write settings to {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize settings: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl SettingsStore {
    pub fn discover() -> Result<Self, SettingsError> {
        let project_dirs = ProjectDirs::from("com", "TheShowMy", "game-config-edit")
            .ok_or(SettingsError::ConfigDirectoryUnavailable)?;
        Ok(Self::at(project_dirs.config_dir().join("settings.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Settings, SettingsError> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| SettingsError::Parse {
                path: self.path.clone(),
                source,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(source) => Err(SettingsError::Read {
                path: self.path.clone(),
                source,
            }),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let parent = self
            .path
            .parent()
            .ok_or(SettingsError::ConfigDirectoryUnavailable)?;
        fs::create_dir_all(parent).map_err(|source| SettingsError::Write {
            path: parent.to_path_buf(),
            source,
        })?;

        let bytes = serde_json::to_vec_pretty(settings)?;
        let temporary_path = self.path.with_extension("json.tmp");
        let mut file =
            fs::File::create(&temporary_path).map_err(|source| SettingsError::Write {
                path: temporary_path.clone(),
                source,
            })?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|source| SettingsError::Write {
                path: temporary_path.clone(),
                source,
            })?;
        #[cfg(windows)]
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|source| SettingsError::Write {
                path: self.path.clone(),
                source,
            })?;
        }
        fs::rename(&temporary_path, &self.path).map_err(|source| SettingsError::Write {
            path: self.path.clone(),
            source,
        })
    }

    pub fn save_recent_workspace(&self, workspace: &Path) -> Result<(), SettingsError> {
        let mut settings = self.load()?;
        settings.recent_workspace = Some(workspace.to_path_buf());
        self.save(&settings)
    }

    pub fn save_file_preferences(
        &self,
        workspace: &Path,
        relative_path: &Path,
        preferences: FilePreferences,
    ) -> Result<(), SettingsError> {
        let mut settings = self.load()?;
        settings.set_file_preferences(workspace, relative_path, preferences);
        self.save(&settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_file_loads_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));

        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("nested/settings.json"));
        let expected = Settings {
            recent_workspace: Some(PathBuf::from("C:/configs")),
            ..Settings::default()
        };

        store.save(&expected).unwrap();

        assert_eq!(store.load().unwrap(), expected);
        assert!(!store.path().with_extension("json.tmp").exists());
    }

    #[test]
    fn existing_settings_can_be_updated() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        store
            .save(&Settings {
                recent_workspace: Some(PathBuf::from("C:/first")),
                ..Settings::default()
            })
            .unwrap();

        let expected = Settings {
            recent_workspace: Some(PathBuf::from("C:/second")),
            ..Settings::default()
        };
        store.save(&expected).unwrap();

        assert_eq!(store.load().unwrap(), expected);
    }

    #[test]
    fn file_preferences_are_scoped_by_workspace_and_relative_path() {
        let mut settings = Settings::default();
        let workspace = Path::new("C:/configs");
        let relative = Path::new("heroes/basic.csv");
        let expected = FilePreferences {
            header_rows: 3,
            delimiter: Some(CsvDelimiter::Pipe),
        };

        settings.set_file_preferences(workspace, relative, expected);

        assert_eq!(settings.file_preferences(workspace, relative), expected);
        assert_eq!(
            settings.file_preferences(workspace, Path::new("items.csv")),
            FilePreferences::default()
        );
        assert_eq!(
            settings.file_preferences(Path::new("C:/other"), relative),
            FilePreferences::default()
        );
    }

    #[test]
    fn settings_load_old_files_and_normalize_invalid_header_counts() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        fs::write(
            store.path(),
            br#"{
                "recent_workspace": "C:/configs",
                "workspaces": {
                    "C:/configs": {
                        "files": {
                            "heroes.csv": { "header_rows": 9, "delimiter": "semicolon" }
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let settings = store.load().unwrap();

        assert_eq!(settings.recent_workspace, Some(PathBuf::from("C:/configs")));
        assert_eq!(
            settings.file_preferences(Path::new("C:/configs"), Path::new("heroes.csv")),
            FilePreferences {
                header_rows: DEFAULT_HEADER_ROWS,
                delimiter: Some(CsvDelimiter::Semicolon),
            }
        );
    }

    #[test]
    fn settings_without_workspace_preferences_remain_compatible() {
        let settings: Settings =
            serde_json::from_str(r#"{ "recent_workspace": "C:/configs" }"#).unwrap();

        assert_eq!(settings.recent_workspace, Some(PathBuf::from("C:/configs")));
        assert!(settings.workspaces.is_empty());
    }
}
