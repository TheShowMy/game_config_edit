use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartupDecision {
    OpenWorkspace(PathBuf),
    ChooseWorkspace,
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("workspace path does not exist: {0}")]
    NotFound(PathBuf),
    #[error("workspace path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("workspace path cannot be read: {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub fn resolve_startup(
    explicit_path: Option<&Path>,
    recent_path: Option<&Path>,
) -> Result<StartupDecision, StartupError> {
    if let Some(path) = explicit_path {
        return validate_workspace(path).map(StartupDecision::OpenWorkspace);
    }

    match recent_path.and_then(|path| validate_workspace(path).ok()) {
        Some(path) => Ok(StartupDecision::OpenWorkspace(path)),
        None => Ok(StartupDecision::ChooseWorkspace),
    }
}

pub fn validate_workspace(path: &Path) -> Result<PathBuf, StartupError> {
    if !path.exists() {
        return Err(StartupError::NotFound(path.to_path_buf()));
    }
    if !path.is_dir() {
        return Err(StartupError::NotDirectory(path.to_path_buf()));
    }

    fs::read_dir(path).map_err(|source| StartupError::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;

    path.canonicalize()
        .map_err(|source| StartupError::Unreadable {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_workspace_takes_precedence() {
        let explicit = tempfile::tempdir().unwrap();
        let recent = tempfile::tempdir().unwrap();

        let decision = resolve_startup(Some(explicit.path()), Some(recent.path())).unwrap();

        assert_eq!(
            decision,
            StartupDecision::OpenWorkspace(explicit.path().canonicalize().unwrap())
        );
    }

    #[test]
    fn valid_recent_workspace_is_reopened_without_explicit_path() {
        let recent = tempfile::tempdir().unwrap();

        let decision = resolve_startup(None, Some(recent.path())).unwrap();

        assert_eq!(
            decision,
            StartupDecision::OpenWorkspace(recent.path().canonicalize().unwrap())
        );
    }

    #[test]
    fn missing_recent_workspace_falls_back_to_picker() {
        let decision = resolve_startup(None, Some(Path::new("missing-workspace"))).unwrap();

        assert_eq!(decision, StartupDecision::ChooseWorkspace);
    }

    #[test]
    fn missing_explicit_workspace_is_an_error() {
        let error = resolve_startup(Some(Path::new("missing-workspace")), None).unwrap_err();

        assert!(matches!(error, StartupError::NotFound(_)));
    }

    #[test]
    fn explicit_file_is_rejected() {
        let file = tempfile::NamedTempFile::new().unwrap();

        let error = resolve_startup(Some(file.path()), None).unwrap_err();

        assert!(matches!(error, StartupError::NotDirectory(_)));
    }
}
