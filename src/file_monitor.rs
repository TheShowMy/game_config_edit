use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::timeout;

const EVENT_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Debug, Default, Eq, PartialEq)]
pub struct WorkspaceChangeBatch {
    pub paths: HashSet<PathBuf>,
    pub errors: Vec<String>,
    refresh_required: bool,
}

impl WorkspaceChangeBatch {
    pub fn refresh_required(&self) -> bool {
        self.refresh_required
    }

    fn record(&mut self, result: notify::Result<Event>) {
        match result {
            Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                self.refresh_required = true;
                self.paths.extend(event.paths);
            }
            Ok(_) => {}
            Err(error) => self.errors.push(error.to_string()),
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceMonitorError {
    #[error("failed to create workspace file monitor: {0}")]
    Create(#[source] notify::Error),
    #[error("failed to monitor {path}: {source}")]
    Watch {
        path: PathBuf,
        #[source]
        source: notify::Error,
    },
}

pub struct WorkspaceMonitor {
    _watcher: RecommendedWatcher,
    receiver: UnboundedReceiver<notify::Result<Event>>,
}

impl WorkspaceMonitor {
    pub fn new(root: &Path) -> Result<Self, WorkspaceMonitorError> {
        let (sender, receiver) = unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .map_err(WorkspaceMonitorError::Create)?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|source| WorkspaceMonitorError::Watch {
                path: root.to_path_buf(),
                source,
            })?;
        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }

    pub async fn next_batch(&mut self) -> Option<WorkspaceChangeBatch> {
        let first = self.receiver.recv().await?;
        let mut batch = WorkspaceChangeBatch::default();
        batch.record(first);

        while let Ok(Some(event)) = timeout(EVENT_DEBOUNCE, self.receiver.recv()).await {
            batch.record(event);
        }
        Some(batch)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use notify::event::{AccessKind, CreateKind, ModifyKind};

    use super::*;

    #[test]
    fn change_events_request_refresh_and_collect_paths() {
        let mut batch = WorkspaceChangeBatch::default();
        batch.record(Ok(Event::new(EventKind::Create(CreateKind::File))
            .add_path(PathBuf::from("configs/new.csv"))));
        batch.record(Ok(Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(PathBuf::from("configs/heroes.csv"))));

        assert!(batch.refresh_required());
        assert_eq!(
            batch.paths,
            HashSet::from([
                PathBuf::from("configs/new.csv"),
                PathBuf::from("configs/heroes.csv"),
            ])
        );
        assert!(batch.errors.is_empty());
    }

    #[test]
    fn access_events_do_not_trigger_a_refresh() {
        let mut batch = WorkspaceChangeBatch::default();
        batch.record(Ok(Event::new(EventKind::Access(AccessKind::Any))
            .add_path(PathBuf::from("configs/heroes.csv"))));

        assert!(!batch.refresh_required());
        assert!(batch.paths.is_empty());
    }

    #[test]
    fn observes_a_real_csv_creation() {
        let directory = tempfile::tempdir().unwrap();
        let mut monitor = WorkspaceMonitor::new(directory.path()).unwrap();
        let created_path = directory.path().join("heroes.csv");
        fs::write(&created_path, "id,name\n1,Arthur\n").unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let batch = runtime.block_on(async {
            timeout(Duration::from_secs(5), monitor.next_batch())
                .await
                .expect("filesystem event timed out")
                .expect("file monitor stopped")
        });

        assert!(batch.refresh_required());
        assert!(
            batch
                .paths
                .iter()
                .any(|path| path == &created_path || created_path.starts_with(path)),
            "filesystem event paths do not cover {}: {:?}",
            created_path.display(),
            batch.paths
        );
    }
}
