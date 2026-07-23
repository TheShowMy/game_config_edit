#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use clap::Parser;
use dioxus::desktop::WindowCloseBehaviour;
use dioxus::desktop::tao::event::{Event as WryEvent, WindowEvent};
use dioxus::prelude::*;
use game_config_edit::csv_document::{CsvDelimiter, CsvDocument, CsvEncoding, DelimiterSource};
use game_config_edit::diagnostics::{CellProblemKind, ColumnAnalysis, analyze_table};
use game_config_edit::document_session::{
    DocumentSession, DocumentSessionError, DocumentView, TextParseIssue,
};
use game_config_edit::editor_navigation::{
    DiagnosticTarget, GridMovement, GridPosition, cycle_diagnostic, diagnostic_targets,
    move_in_grid,
};
use game_config_edit::file_monitor::WorkspaceMonitor;
use game_config_edit::i18n::{
    CloseAction, Count, Message, SHORTCUTS, ShortcutGroup, Text, count, csv_parse_failed,
    header_requirement, message as l10n, physical_lines, records_columns, search_summary,
    shortcut_keys, text as tr, unsearchable_files,
};
use game_config_edit::platform::reveal_in_file_manager;
use game_config_edit::search::{
    CellSearchMatch, GlobalSearchEvent, GlobalSearchMatch, TextSearchMatch, find_cell_matches,
    find_text_matches, rank_files, stream_workspace_search,
};
use game_config_edit::settings::{
    DEFAULT_HEADER_ROWS, FilePreferences, MAX_HEADER_ROWS, MIN_HEADER_ROWS, Settings, SettingsStore,
};
use game_config_edit::startup::{StartupDecision, resolve_startup, validate_workspace};
use game_config_edit::table_virtualization::{
    DATA_ROW_HEIGHT, ExpandedRow, FocusColumnRole, FocusLayout, TableViewport, spacer_heights,
    visible_row_range,
};
use game_config_edit::workspace::{
    CsvFileEntry, CsvFileStats, WorkspaceSnapshot, WorkspaceTreeRow, inspect_csv_file,
    scan_workspace, visible_tree_rows,
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::Deserialize;

const APP_CSS: &str = include_str!("app.css");
const HEADER_ROW_HEIGHT: usize = 28;
const STATS_BATCH_SIZE: usize = 32;
static BOOTSTRAP: OnceLock<Bootstrap> = OnceLock::new();

#[derive(Debug, Parser)]
#[command(name = "gconf", version, about)]
struct Cli {
    /// Workspace directory containing CSV configuration files.
    workspace: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
struct Bootstrap {
    workspace: Option<PathBuf>,
    warning: Option<String>,
    settings_store: Option<SettingsStore>,
    settings: Settings,
}

#[derive(Clone, Debug, Default)]
struct ScanView {
    snapshot: WorkspaceSnapshot,
    loading: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SidebarMode {
    #[default]
    List,
    Tree,
}

#[derive(Clone, Debug, Default)]
enum Preview {
    #[default]
    Empty,
    Loading {
        path: PathBuf,
        file_name: String,
    },
    Document {
        document: CsvDocument,
        header_rows: usize,
    },
    Error {
        path: PathBuf,
        file_name: String,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CellLocation {
    path: PathBuf,
    row_index: usize,
    column_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CellDraft {
    location: CellLocation,
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FocusedColumn {
    path: PathBuf,
    column_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum ResizeDrag {
    Sidebar {
        start_x: f64,
        start_width: usize,
    },
    Column {
        path: PathBuf,
        column_index: usize,
        start_x: f64,
        start_width: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalChangeAction {
    None,
    Reload,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellClickAction {
    Select,
    SwitchFocus,
    Edit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EditIntent {
    BeginDraft,
    Paste(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlayPanel {
    CommandPalette,
    GoToLine,
    CurrentSearch,
    GlobalSearch,
    Shortcuts,
}

#[derive(Clone, Debug, Default)]
struct CommandPaletteState {
    query: String,
    selected_index: usize,
}

#[derive(Clone, Debug, Default)]
struct CurrentSearchState {
    query: String,
    case_sensitive: bool,
    active_index: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct GlobalSearchState {
    query: String,
    case_sensitive: bool,
    results: Vec<GlobalSearchMatch>,
    loading: bool,
    truncated: bool,
    warning_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct TextCursorPosition {
    path: PathBuf,
    line: usize,
    column: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct TableViewportSize {
    path: PathBuf,
    width: f64,
    height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct FocusedRowMeasurement {
    path: PathBuf,
    row_index: usize,
    column_index: usize,
    content_height: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DocumentStatus {
    file_name: String,
    dimensions: String,
    encoding: &'static str,
    position: Option<String>,
    diagnostic: Option<String>,
    red_cells: Option<usize>,
    yellow_columns: Option<usize>,
    parse_errors: Option<usize>,
    analysis_loading: bool,
    delimiter_defaulted: bool,
}

#[derive(Clone, Debug)]
enum TableAnalysisState {
    Loading {
        document_version: u64,
        header_rows: usize,
    },
    Ready {
        document_version: u64,
        header_rows: usize,
        columns: Arc<Vec<ColumnAnalysis>>,
    },
}

impl TableAnalysisState {
    fn matches(&self, document_version: u64, header_rows: usize) -> bool {
        match self {
            Self::Loading {
                document_version: current_version,
                header_rows: current_header_rows,
            }
            | Self::Ready {
                document_version: current_version,
                header_rows: current_header_rows,
                ..
            } => *current_version == document_version && *current_header_rows == header_rows,
        }
    }

    fn ready_columns(
        &self,
        document_version: u64,
        header_rows: usize,
    ) -> Option<Arc<Vec<ColumnAnalysis>>> {
        match self {
            Self::Ready {
                document_version: current_version,
                header_rows: current_header_rows,
                columns,
            } if *current_version == document_version && *current_header_rows == header_rows => {
                Some(columns.clone())
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonStructure {
    Object,
    Array,
    Array2d,
}

#[derive(Clone)]
struct PreferenceContext {
    workspace: Option<PathBuf>,
    settings_store: Option<SettingsStore>,
    settings: Signal<Settings>,
    file_stats: Signal<HashMap<PathBuf, CsvFileStats>>,
    preview: Signal<Preview>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
}

#[derive(Clone)]
struct OverlayContext {
    workspace: Option<PathBuf>,
    files: Vec<CsvFileEntry>,
    settings: Signal<Settings>,
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    preview: Signal<Preview>,
    preview_return_tab: Signal<Option<PathBuf>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    focused_column: Signal<Option<FocusedColumn>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    panel: Signal<Option<OverlayPanel>>,
    command_palette: Signal<CommandPaletteState>,
    go_to_line: Signal<String>,
    current_search: Signal<CurrentSearchState>,
    global_search: Signal<GlobalSearchState>,
    global_search_cancel: Signal<Option<Arc<AtomicBool>>>,
    notice: Signal<Option<String>>,
}

#[derive(Clone)]
struct PreviewEditContext {
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    preview: Signal<Preview>,
    preview_return_tab: Signal<Option<PathBuf>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    focused_column: Signal<Option<FocusedColumn>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    notice: Signal<Option<String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum WindowShortcutCommand {
    CommandPalette,
    GoToLine,
    CurrentSearch,
    GlobalSearch,
    Shortcuts,
    Save,
    Close,
    CloseReleased,
    NextTab,
    PreviousTab,
    ToggleSidebar,
    Undo,
    Redo,
    NextDiagnostic,
    PreviousDiagnostic,
    Copied,
    Paste(String),
    TextCursor(TextCursorPosition),
    TableViewport(TableViewportSize),
    FocusedRowHeight(FocusedRowMeasurement),
}

fn main() {
    let cli = Cli::parse();
    let bootstrap = build_bootstrap(cli.workspace);
    let _ = BOOTSTRAP.set(bootstrap);
    dioxus::LaunchBuilder::new()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_menu(None::<dioxus::desktop::muda::Menu>)
                .with_close_behaviour(WindowCloseBehaviour::WindowHides),
        )
        .launch(App);
}

fn build_bootstrap(explicit_workspace: Option<PathBuf>) -> Bootstrap {
    let (settings_store, discovery_warning) = match SettingsStore::discover() {
        Ok(store) => (Some(store), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let (mut settings, warning) = match settings_store.as_ref() {
        Some(store) => match store.load() {
            Ok(settings) => (settings, discovery_warning),
            Err(error) => (Settings::default(), Some(error.to_string())),
        },
        None => (Settings::default(), discovery_warning),
    };
    let recent_workspace = settings.recent_workspace.clone();

    let decision = match resolve_startup(explicit_workspace.as_deref(), recent_workspace.as_deref())
    {
        Ok(decision) => decision,
        Err(error) => {
            show_startup_error(&error.to_string());
            std::process::exit(2);
        }
    };

    let workspace = match decision {
        StartupDecision::OpenWorkspace(path) => Some(path),
        StartupDecision::ChooseWorkspace => choose_workspace(),
    };
    if let Some(path) = workspace.as_deref()
        && let Some(store) = settings_store.as_ref()
        && let Err(error) = store.save_recent_workspace(path)
    {
        return Bootstrap {
            workspace,
            warning: Some(error.to_string()),
            settings_store,
            settings,
        };
    }
    settings.recent_workspace = workspace.clone();

    Bootstrap {
        workspace,
        warning,
        settings_store,
        settings,
    }
}

fn choose_workspace() -> Option<PathBuf> {
    FileDialog::new()
        .set_title(tr(Text::OpenCsvFolder))
        .pick_folder()
        .and_then(|path| validate_workspace(&path).ok())
}

fn show_startup_error(message: &str) {
    eprintln!("gconf: {message}");
    let _ = MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(tr(Text::AppTitle))
        .set_description(l10n(Message::Technical {
            prefix: Text::StartupError,
            detail: message,
        }))
        .show();
}

#[allow(non_snake_case)]
fn App() -> Element {
    let bootstrap = BOOTSTRAP.get().cloned().unwrap_or_default();
    let initial_workspace = bootstrap.workspace.clone();
    let settings_store = bootstrap.settings_store.clone();
    let initial_settings = bootstrap.settings;
    let mut workspace = use_signal(move || initial_workspace);
    let mut app_settings = use_signal(move || initial_settings);
    let mut scan = use_signal(ScanView::default);
    let mut file_stats = use_signal(HashMap::<PathBuf, CsvFileStats>::new);
    let mut filter = use_signal(String::new);
    let mut sidebar_mode = use_signal(SidebarMode::default);
    let mut expanded_directories = use_signal(HashSet::<PathBuf>::new);
    let mut preview = use_signal(Preview::default);
    let mut tabs = use_signal(Vec::<DocumentSession>::new);
    let mut active_tab = use_signal(|| None::<PathBuf>);
    let mut preview_return_tab = use_signal(|| None::<PathBuf>);
    let mut selected_cell = use_signal(|| None::<CellLocation>);
    let mut cell_draft = use_signal(|| None::<CellDraft>);
    let focused_column = use_signal(|| None::<FocusedColumn>);
    let mut column_widths = use_signal(HashMap::<(PathBuf, usize), usize>::new);
    let mut sidebar_width = use_signal(|| 280_usize);
    let mut resize_drag = use_signal(|| None::<ResizeDrag>);
    let mut diagnostic_target = use_signal(|| None::<DiagnosticTarget>);
    let table_viewports = use_signal(HashMap::<PathBuf, TableViewport>::new);
    let sidebar_visible = use_signal(|| true);
    let mut external_conflicts = use_signal(HashSet::<PathBuf>::new);
    let mut external_reload_errors = use_signal(HashMap::<PathBuf, String>::new);
    let mut overlay_panel = use_signal(|| None::<OverlayPanel>);
    let command_palette = use_signal(CommandPaletteState::default);
    let go_to_line = use_signal(String::new);
    let current_search = use_signal(CurrentSearchState::default);
    let global_search = use_signal(GlobalSearchState::default);
    let global_search_cancel = use_signal(|| None::<Arc<AtomicBool>>);
    let mut table_analyses = use_signal(HashMap::<PathBuf, TableAnalysisState>::new);
    let text_cursor = use_signal(|| None::<TextCursorPosition>);
    let mut notice = use_signal(|| None::<String>);
    let mut shortcut_close_in_progress = use_signal(|| false);
    let desktop = dioxus::desktop::use_window();
    let warning = bootstrap.warning;

    dioxus::desktop::use_wry_event_handler({
        let desktop = desktop.clone();
        move |event, _| {
            let WryEvent::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } = event
            else {
                return;
            };

            if *shortcut_close_in_progress.read() {
                shortcut_close_in_progress.set(false);
                restore_hidden_window(desktop.clone());
                return;
            }

            if confirm_close_all_tabs(
                tabs,
                selected_cell,
                cell_draft,
                diagnostic_target,
                table_analyses,
                notice,
                CloseAction::Window,
            ) {
                desktop.set_close_behavior(WindowCloseBehaviour::WindowCloses);
            } else {
                desktop.set_close_behavior(WindowCloseBehaviour::WindowHides);
                restore_hidden_window(desktop.clone());
            }
        }
    });

    use_effect(move || {
        let Some(root) = workspace.read().clone() else {
            scan.set(ScanView::default());
            file_stats.set(HashMap::new());
            return;
        };
        file_stats.set(HashMap::new());
        expanded_directories.set(HashSet::new());
        scan.write().loading = true;
        scan.write().error = None;
        spawn(async move {
            let scan_root = root.clone();
            let result = tokio::task::spawn_blocking(move || scan_workspace(&scan_root)).await;
            if workspace.read().as_ref() != Some(&root) {
                return;
            }
            match result {
                Ok(snapshot) => {
                    let files = snapshot.files.clone();
                    scan.set(ScanView {
                        snapshot,
                        loading: false,
                        error: None,
                    });
                    let mut stats_batch = Vec::with_capacity(STATS_BATCH_SIZE);
                    for entry in files {
                        let preferences = app_settings
                            .read()
                            .file_preferences(&root, &entry.relative_path);
                        let path = entry.absolute_path;
                        let inspect_path = path.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            inspect_csv_file(
                                &inspect_path,
                                preferences.header_rows,
                                preferences.delimiter.map(CsvDelimiter::byte),
                            )
                        })
                        .await;
                        if workspace.read().as_ref() != Some(&root) {
                            return;
                        }
                        let stats = result.unwrap_or_else(|error| CsvFileStats::Error {
                            message: error.to_string(),
                        });
                        stats_batch.push((path, stats));
                        if stats_batch.len() == STATS_BATCH_SIZE {
                            file_stats.write().extend(stats_batch.drain(..));
                            tokio::task::yield_now().await;
                        }
                    }
                    if !stats_batch.is_empty() {
                        file_stats.write().extend(stats_batch);
                    }
                }
                Err(error) => scan.set(ScanView {
                    snapshot: WorkspaceSnapshot::default(),
                    loading: false,
                    error: Some(error.to_string()),
                }),
            }
        });
    });

    use_effect(move || {
        let target = if let Some(path) = active_tab.read().clone() {
            let tabs_read = tabs.read();
            tabs_read
                .iter()
                .find(|tab| tab.document.path == path && tab.text_parse_issue().is_none())
                .map(|tab| {
                    (
                        path,
                        tab.document.analysis_version(),
                        tab.header_rows,
                        tab.document.records.clone(),
                    )
                })
        } else {
            let preview_read = preview.read();
            match &*preview_read {
                Preview::Document {
                    document,
                    header_rows,
                } => Some((
                    document.path.clone(),
                    document.analysis_version(),
                    *header_rows,
                    document.records.clone(),
                )),
                _ => None,
            }
        };
        let Some((path, document_version, header_rows, records)) = target else {
            return;
        };
        if records.len() < header_rows
            || table_analyses
                .read()
                .get(&path)
                .is_some_and(|state| state.matches(document_version, header_rows))
        {
            return;
        }

        table_analyses.write().insert(
            path.clone(),
            TableAnalysisState::Loading {
                document_version,
                header_rows,
            },
        );
        spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || analyze_table(records.as_ref(), header_rows))
                    .await;
            let columns = match result {
                Ok(columns) => columns,
                Err(error) => {
                    notice.set(Some(l10n(Message::TableAnalysisFailed {
                        file: &file_name(&path),
                        detail: &error.to_string(),
                    })));
                    table_analyses.write().remove(&path);
                    return;
                }
            };
            let still_requested = table_analyses
                .peek()
                .get(&path)
                .is_some_and(|state| state.matches(document_version, header_rows));
            if still_requested {
                table_analyses.write().insert(
                    path,
                    TableAnalysisState::Ready {
                        document_version,
                        header_rows,
                        columns: Arc::new(columns),
                    },
                );
            }
        });
    });

    use_future(move || {
        let watched_workspace = workspace.read().clone();
        async move {
            let Some(root) = watched_workspace else {
                return;
            };
            let monitor_root = root.clone();
            let mut monitor =
                match tokio::task::spawn_blocking(move || WorkspaceMonitor::new(&monitor_root))
                    .await
                {
                    Ok(Ok(monitor)) => monitor,
                    Ok(Err(error)) => {
                        notice.set(Some(error.to_string()));
                        return;
                    }
                    Err(error) => {
                        notice.set(Some(error.to_string()));
                        return;
                    }
                };

            while let Some(batch) = monitor.next_batch().await {
                if workspace.peek().as_ref() != Some(&root) {
                    return;
                }
                if !batch.errors.is_empty() {
                    notice.set(Some(format!(
                        "File monitoring warning: {}",
                        batch.errors.join("; ")
                    )));
                }
                if !batch.refresh_required() {
                    continue;
                }

                let preview_target = {
                    let preview_read = preview.peek();
                    match &*preview_read {
                        Preview::Document {
                            document,
                            header_rows,
                        } if path_was_affected(&document.path, &batch.paths) => {
                            Some((document.path.clone(), *header_rows))
                        }
                        Preview::Error { path, .. } if path_was_affected(path, &batch.paths) => {
                            let header_rows = path
                                .strip_prefix(&root)
                                .ok()
                                .map(|relative| {
                                    app_settings
                                        .peek()
                                        .file_preferences(&root, relative)
                                        .header_rows
                                })
                                .unwrap_or(DEFAULT_HEADER_ROWS);
                            Some((path.clone(), header_rows))
                        }
                        _ => None,
                    }
                };
                if let Some((path, header_rows)) = preview_target {
                    let relative_path = path.strip_prefix(&root).ok().map(Path::to_path_buf);
                    let delimiter = relative_path.as_deref().and_then(|relative| {
                        app_settings
                            .peek()
                            .file_preferences(&root, relative)
                            .delimiter
                            .map(CsvDelimiter::byte)
                    });
                    let reload_path = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        CsvDocument::open(&reload_path, delimiter)
                    })
                    .await;
                    if preview_path(&preview.peek()) == Some(path.as_path()) {
                        let file_name = file_name(&path);
                        preview.set(match result {
                            Ok(Ok(document)) => Preview::Document {
                                document,
                                header_rows,
                            },
                            Ok(Err(error)) => Preview::Error {
                                path,
                                file_name,
                                message: error.to_string(),
                            },
                            Err(error) => Preview::Error {
                                path,
                                file_name,
                                message: error.to_string(),
                            },
                        });
                    }
                }

                let baselines = tabs
                    .peek()
                    .iter()
                    .map(|tab| {
                        (
                            tab.document.path.clone(),
                            tab.saved_hash(),
                            tab.delimiter_override(),
                            tab.header_rows,
                            tab.view(),
                            tab.gb18030_conversion_allowed(),
                        )
                    })
                    .collect::<Vec<_>>();
                for (path, saved_hash, delimiter, header_rows, previous_view, conversion_allowed) in
                    baselines
                {
                    let check_path = path.clone();
                    let disk_hash = tokio::task::spawn_blocking(move || {
                        fs::read(&check_path).map(|bytes| blake3::hash(&bytes))
                    })
                    .await;
                    let is_dirty = {
                        let tabs_read = tabs.peek();
                        let draft_read = cell_draft.peek();
                        tabs_read
                            .iter()
                            .find(|tab| tab.document.path == path)
                            .is_some_and(|tab| tab_has_unsaved_changes(tab, draft_read.as_ref()))
                    };
                    let disk_hash = match disk_hash {
                        Ok(Ok(hash)) => Some(hash),
                        Ok(Err(_)) | Err(_) => None,
                    };
                    match external_change_action(saved_hash, disk_hash, is_dirty) {
                        ExternalChangeAction::None => continue,
                        ExternalChangeAction::Reload => {}
                        ExternalChangeAction::Conflict => {
                            external_conflicts.write().insert(path.clone());
                            notice.set(Some(l10n(Message::ExternalConflict(&file_name(&path)))));
                            continue;
                        }
                    }

                    let reloaded = load_session_for_edit(
                        path.clone(),
                        delimiter,
                        header_rows,
                        conversion_allowed,
                    )
                    .await;
                    let mut replacement = match reloaded {
                        Ok(Some(session)) => session,
                        Ok(None) => continue,
                        Err(error) => {
                            external_reload_errors
                                .write()
                                .insert(path.clone(), error.clone());
                            notice.set(Some(l10n(Message::ReloadFailed {
                                file: &file_name(&path),
                                detail: &error,
                            })));
                            continue;
                        }
                    };
                    if previous_view == DocumentView::Text
                        && replacement.text_parse_issue().is_none()
                    {
                        replacement.show_text();
                    }

                    let still_clean = {
                        let tabs_read = tabs.peek();
                        let draft_read = cell_draft.peek();
                        tabs_read
                            .iter()
                            .find(|tab| tab.document.path == path)
                            .is_some_and(|tab| {
                                tab.saved_hash() == saved_hash
                                    && !tab_has_unsaved_changes(tab, draft_read.as_ref())
                            })
                    };
                    if !still_clean {
                        external_conflicts.write().insert(path.clone());
                        continue;
                    }
                    if let Some(tab) = tabs
                        .write()
                        .iter_mut()
                        .find(|tab| tab.document.path == path)
                    {
                        *tab = replacement;
                    }
                    if cell_draft
                        .peek()
                        .as_ref()
                        .is_some_and(|draft| draft.location.path == path)
                    {
                        cell_draft.set(None);
                    }
                    if active_tab.peek().as_ref() == Some(&path) {
                        diagnostic_target.set(None);
                    }
                    external_conflicts.write().remove(&path);
                    external_reload_errors.write().remove(&path);
                    notice.set(Some(l10n(Message::ExternalReloaded(&file_name(&path)))));
                }

                let previous_paths = scan
                    .peek()
                    .snapshot
                    .files
                    .iter()
                    .map(|entry| entry.absolute_path.clone())
                    .collect::<HashSet<_>>();
                let scan_root = root.clone();
                let scan_result =
                    tokio::task::spawn_blocking(move || scan_workspace(&scan_root)).await;
                if workspace.peek().as_ref() != Some(&root) {
                    return;
                }
                match scan_result {
                    Ok(snapshot) => {
                        let current_paths = snapshot
                            .files
                            .iter()
                            .map(|entry| entry.absolute_path.clone())
                            .collect::<HashSet<_>>();
                        file_stats
                            .write()
                            .retain(|path, _| current_paths.contains(path));
                        let files_to_inspect = snapshot
                            .files
                            .iter()
                            .filter(|entry| {
                                !previous_paths.contains(&entry.absolute_path)
                                    || path_was_affected(&entry.absolute_path, &batch.paths)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        scan.set(ScanView {
                            snapshot,
                            loading: false,
                            error: None,
                        });

                        for entry in files_to_inspect {
                            let preferences = app_settings
                                .peek()
                                .file_preferences(&root, &entry.relative_path);
                            let path = entry.absolute_path;
                            let inspect_path = path.clone();
                            let stats = tokio::task::spawn_blocking(move || {
                                inspect_csv_file(
                                    &inspect_path,
                                    preferences.header_rows,
                                    preferences.delimiter.map(CsvDelimiter::byte),
                                )
                            })
                            .await
                            .unwrap_or_else(|error| {
                                CsvFileStats::Error {
                                    message: error.to_string(),
                                }
                            });
                            file_stats.write().insert(path, stats);
                        }
                    }
                    Err(error) => scan.write().error = Some(error.to_string()),
                }
            }
        }
    });

    use_future(move || async move {
        let mut eval = document::eval(
            r#"
            const isMac = /Mac|iPhone|iPad|iPod/.test(navigator.platform);
            const handler = (event) => {
                const key = event.key.toLowerCase();
                const primary = isMac ? event.metaKey : event.ctrlKey;
                const editingText = event.target instanceof HTMLElement
                    && (event.target.matches("input, textarea") || event.target.isContentEditable);
                let command = null;

                if (event.ctrlKey && event.key === "Tab") {
                    command = event.shiftKey ? "previous_tab" : "next_tab";
                } else if (primary && event.shiftKey && key === "f") {
                    command = "global_search";
                } else if (primary && key === "p") {
                    command = "command_palette";
                } else if (primary && key === "g") {
                    command = "go_to_line";
                } else if (primary && key === "f") {
                    command = "current_search";
                } else if (primary && key === "s") {
                    command = "save";
                } else if (primary && key === "w") {
                    command = "close";
                } else if (primary && key === "b") {
                    command = "toggle_sidebar";
                } else if (!editingText && primary && key === "z") {
                    command = isMac && event.shiftKey ? "redo" : "undo";
                } else if (!editingText && !isMac && primary && key === "y") {
                    command = "redo";
                } else if (event.key === "F8") {
                    command = event.shiftKey ? "previous_diagnostic" : "next_diagnostic";
                } else if (!editingText && event.key === "F1") {
                    command = "shortcuts";
                }

                if (command !== null) {
                    event.preventDefault();
                    event.stopPropagation();
                    dioxus.send(command);
                }
            };
            const releaseHandler = (event) => {
                const key = event.key.toLowerCase();
                const primary = isMac ? event.metaKey : event.ctrlKey;
                if (primary && key === "w") {
                    dioxus.send("close_released");
                }
            };
            const copyHandler = (event) => {
                const editingText = event.target instanceof HTMLElement
                    && (event.target.matches("input, textarea") || event.target.isContentEditable);
                if (editingText) return;
                const selected = document.querySelector(".cell-selected .cell-button");
                if (!(selected instanceof HTMLElement) || event.clipboardData === null) return;
                event.preventDefault();
                event.clipboardData.setData("text/plain", selected.dataset.cellValue ?? "");
                dioxus.send("copied");
            };
            const pasteHandler = (event) => {
                const editingText = event.target instanceof HTMLElement
                    && (event.target.matches("input, textarea") || event.target.isContentEditable);
                if (editingText) return;
                const selected = document.querySelector(".cell-selected .cell-button");
                if (!(selected instanceof HTMLElement) || event.clipboardData === null) return;
                event.preventDefault();
                dioxus.send({paste: event.clipboardData.getData("text/plain")});
            };
            const cursorHandler = () => {
                const editor = document.activeElement;
                if (!(editor instanceof HTMLTextAreaElement)
                    || !editor.classList.contains("text-editor-input")) return;
                const position = editor.selectionStart ?? 0;
                const before = editor.value.slice(0, position);
                const lines = before.split(/\r\n|\r|\n/);
                dioxus.send({text_cursor: {
                    path: editor.dataset.path ?? "",
                    line: lines.length,
                    column: Array.from(lines.at(-1) ?? "").length + 1,
                }});
            };
            const observedTables = new Set();
            const measureFocusedRow = (table) => {
                const cell = table.querySelector("td.focus-column.cell-selected[data-data-row-index]");
                const content = cell?.querySelector(".cell-button, .cell-input");
                if (!(cell instanceof HTMLElement) || !(content instanceof HTMLElement)) return;
                const rowIndex = Number.parseInt(cell.dataset.dataRowIndex ?? "", 10);
                const columnIndex = Number.parseInt(cell.dataset.columnIndex ?? "", 10);
                if (!Number.isFinite(rowIndex) || !Number.isFinite(columnIndex)) return;
                const previousHeight = content.style.height;
                const previousMaxHeight = content.style.maxHeight;
                content.style.height = "0px";
                content.style.maxHeight = "none";
                const contentHeight = Math.max(30, content.scrollHeight + 10);
                content.style.height = previousHeight;
                content.style.maxHeight = previousMaxHeight;
                dioxus.send({focused_row_height: {
                    path: table.dataset.path ?? "",
                    row_index: rowIndex,
                    column_index: columnIndex,
                    content_height: contentHeight,
                }});
            };
            const sendTableSize = (table) => {
                dioxus.send({table_viewport: {
                    path: table.dataset.path ?? "",
                    width: table.clientWidth,
                    height: table.clientHeight,
                }});
                window.requestAnimationFrame(() => measureFocusedRow(table));
            };
            const resizeObserver = new ResizeObserver((entries) => {
                for (const entry of entries) sendTableSize(entry.target);
            });
            const observeTables = (root) => {
                const tables = [];
                if (root instanceof HTMLElement && root.matches(".table-scroll[data-path]")) {
                    tables.push(root);
                }
                if (root instanceof Element || root instanceof Document) {
                    tables.push(...root.querySelectorAll(".table-scroll[data-path]"));
                }
                for (const table of tables) {
                    if (observedTables.has(table)) continue;
                    observedTables.add(table);
                    resizeObserver.observe(table);
                    sendTableSize(table);
                }
            };
            const unobserveTables = (root) => {
                const tables = [];
                if (root instanceof HTMLElement && root.matches(".table-scroll[data-path]")) {
                    tables.push(root);
                }
                if (root instanceof Element) {
                    tables.push(...root.querySelectorAll(".table-scroll[data-path]"));
                }
                for (const table of tables) {
                    resizeObserver.unobserve(table);
                    observedTables.delete(table);
                }
            };
            const mutationObserver = new MutationObserver((mutations) => {
                for (const mutation of mutations) {
                    for (const node of mutation.addedNodes) observeTables(node);
                    for (const node of mutation.removedNodes) unobserveTables(node);
                }
                window.requestAnimationFrame(() => {
                    for (const table of observedTables) measureFocusedRow(table);
                });
            });
            const inputHandler = (event) => {
                cursorHandler();
                const target = event.target;
                if (!(target instanceof Element) || !target.matches(".cell-input")) return;
                const table = target.closest(".table-scroll[data-path]");
                if (table instanceof HTMLElement) {
                    window.requestAnimationFrame(() => measureFocusedRow(table));
                }
            };
            window.addEventListener("keydown", handler, true);
            window.addEventListener("keyup", releaseHandler, true);
            window.addEventListener("copy", copyHandler, true);
            window.addEventListener("paste", pasteHandler, true);
            document.addEventListener("selectionchange", cursorHandler, true);
            document.addEventListener("input", inputHandler, true);
            observeTables(document);
            mutationObserver.observe(document.body, {
                childList: true,
                subtree: true,
                attributes: true,
                attributeFilter: ["class"],
            });
            await new Promise(() => {});
            "#,
        );
        while let Ok(command) = eval.recv::<WindowShortcutCommand>().await {
            handle_window_shortcut(
                command,
                tabs,
                active_tab,
                preview,
                preview_return_tab,
                selected_cell,
                cell_draft,
                focused_column,
                diagnostic_target,
                table_analyses,
                table_viewports,
                text_cursor,
                sidebar_visible,
                overlay_panel,
                notice,
                shortcut_close_in_progress,
            );
        }
    });

    let root_label = workspace
        .read()
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| tr(Text::NoWorkspace).to_owned());
    let normalized_filter = filter.read().trim().to_lowercase();
    let visible_files = scan
        .read()
        .snapshot
        .files
        .iter()
        .filter(|file| {
            normalized_filter.is_empty()
                || file
                    .relative_path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&normalized_filter)
        })
        .cloned()
        .collect::<Vec<_>>();
    let sidebar_rows = if *sidebar_mode.read() == SidebarMode::Tree {
        visible_tree_rows(
            &visible_files,
            &expanded_directories.read(),
            !normalized_filter.is_empty(),
        )
    } else {
        visible_files
            .into_iter()
            .map(|entry| WorkspaceTreeRow::File { entry, depth: 0 })
            .collect()
    };
    let file_count = scan.read().snapshot.files.len();
    let warning_count = scan.read().snapshot.warnings.len();
    let active_external_conflict = active_tab
        .read()
        .as_ref()
        .filter(|path| external_conflicts.read().contains(*path))
        .cloned();
    let active_reload_error = active_tab.read().as_ref().and_then(|path| {
        external_reload_errors
            .read()
            .get(path)
            .map(|message| (path.clone(), message.clone()))
    });
    let conflict_reload_path = active_external_conflict.clone();
    let conflict_keep_path = active_external_conflict.clone();
    let error_retry_path = active_reload_error.as_ref().map(|(path, _)| path.clone());
    let show_general_notice = active_external_conflict.is_none() && active_reload_error.is_none();
    let preference_context = PreferenceContext {
        workspace: workspace.read().clone(),
        settings_store: settings_store.clone(),
        settings: app_settings,
        file_stats,
        preview,
        table_analyses,
    };
    let overlay_context = OverlayContext {
        workspace: workspace.read().clone(),
        files: scan.read().snapshot.files.clone(),
        settings: app_settings,
        tabs,
        active_tab,
        preview,
        preview_return_tab,
        selected_cell,
        cell_draft,
        focused_column,
        diagnostic_target,
        panel: overlay_panel,
        command_palette,
        go_to_line,
        current_search,
        global_search,
        global_search_cancel,
        notice,
    };
    let preview_edit_context = PreviewEditContext {
        tabs,
        active_tab,
        preview,
        preview_return_tab,
        selected_cell,
        cell_draft,
        focused_column,
        diagnostic_target,
        notice,
    };
    let current_status = {
        let analysis_read = table_analyses.read();
        let selected_read = selected_cell.read();
        let cursor_read = text_cursor.read();
        if let Some(path) = active_tab.read().as_ref() {
            tabs.read()
                .iter()
                .find(|tab| &tab.document.path == path)
                .map(|tab| {
                    document_status(
                        &tab.document,
                        tab.text(),
                        tab.view(),
                        tab.text_parse_issue(),
                        tab.header_rows,
                        selected_read.as_ref(),
                        cursor_read.as_ref(),
                        analysis_read.get(path),
                    )
                })
        } else {
            match &*preview.read() {
                Preview::Document {
                    document,
                    header_rows,
                } => Some(document_status(
                    document,
                    &document.raw_text,
                    DocumentView::Table,
                    None,
                    *header_rows,
                    selected_read.as_ref(),
                    None,
                    analysis_read.get(&document.path),
                )),
                _ => None,
            }
        }
    };

    rsx! {
        document::Title { {tr(Text::AppTitle)} }
        style { {APP_CSS} }
        div {
            class: "app-shell",
            tabindex: "0",
            autofocus: true,
            onmounted: move |event| async move {
                let _ = event.set_focus(true).await;
            },
            onkeydown: move |event| handle_app_keydown(
                event,
                tabs,
                active_tab,
                preview,
                preview_return_tab,
                selected_cell,
                cell_draft,
                diagnostic_target,
                table_analyses,
                sidebar_visible,
                overlay_panel,
                notice,
                shortcut_close_in_progress,
                preview_edit_context.clone(),
            ),
            onmousemove: move |event| {
                let Some(drag) = resize_drag.read().clone() else {
                    return;
                };
                let current_x = event.client_coordinates().x;
                match drag {
                    ResizeDrag::Sidebar {
                        start_x,
                        start_width,
                    } => sidebar_width.set(resized_dimension(
                        start_width,
                        current_x - start_x,
                        220,
                        520,
                    )),
                    ResizeDrag::Column {
                        path,
                        column_index,
                        start_x,
                        start_width,
                    } => {
                        let width = resized_dimension(
                            start_width,
                            current_x - start_x,
                            80,
                            720,
                        );
                        column_widths.write().insert((path, column_index), width);
                    }
                }
            },
            onmouseup: move |_| resize_drag.set(None),
            header { class: "titlebar",
                div { class: "brand",
                    span { class: "brand-mark", aria_hidden: "true" }
                    {tr(Text::AppTitle)}
                }
                div { class: "workspace-path", title: "{root_label}", "{root_label}" }
                div { class: "titlebar-actions",
                    button {
                        class: "open-button",
                        title: tr(Text::Shortcuts),
                        onclick: move |_| overlay_panel.set(Some(OverlayPanel::Shortcuts)),
                        {tr(Text::Shortcuts)}
                    }
                    button {
                        class: "open-button primary",
                        title: tr(Text::OpenFolder),
                        onclick: move |_| {
                            if let Some(path) = choose_workspace() {
                                if !confirm_close_all_tabs(
                                    tabs,
                                    selected_cell,
                                    cell_draft,
                                    diagnostic_target,
                                    table_analyses,
                                    notice,
                                    CloseAction::Workspace,
                                ) {
                                    return;
                                }
                                if let Some(store) = settings_store.as_ref()
                                    && let Err(error) = store.save_recent_workspace(&path)
                                {
                                    scan.write().error = Some(error.to_string());
                                }
                                app_settings.write().recent_workspace = Some(path.clone());
                                preview.set(Preview::Empty);
                                tabs.set(Vec::new());
                                active_tab.set(None);
                                preview_return_tab.set(None);
                                selected_cell.set(None);
                                cell_draft.set(None);
                                diagnostic_target.set(None);
                                workspace.set(Some(path));
                            }
                        },
                        {tr(Text::OpenFolder)}
                    }
                }
            }
            if let Some(message) = warning.as_deref() {
                div { class: "banner warning", {l10n(Message::Technical { prefix: Text::SettingsError, detail: message })} }
            }
            if let Some(message) = scan.read().error.as_deref() {
                div { class: "banner error", {l10n(Message::Technical { prefix: Text::ScanError, detail: message })} }
            }
            if show_general_notice && let Some(message) = notice.read().as_deref() {
                div { class: "banner warning", "{message}" }
            }
            if let Some(path) = active_external_conflict {
                div { class: "banner error conflict-banner",
                    span { {l10n(Message::ExternalConflict(&file_name(&path)))} }
                    div { class: "banner-actions",
                        button {
                            onclick: move |_| reload_external_tab(
                                conflict_reload_path.clone().expect("conflict path exists"),
                                true,
                                tabs,
                                cell_draft,
                                diagnostic_target,
                                external_conflicts,
                                external_reload_errors,
                                notice,
                            ),
                            {tr(Text::ReloadFromDisk)}
                        }
                        button {
                            onclick: move |_| {
                                let path = conflict_keep_path
                                    .as_ref()
                                    .expect("conflict path exists");
                                external_conflicts.write().remove(path);
                                notice.set(Some(l10n(Message::KeptLocal(&file_name(path)))));
                            },
                            {tr(Text::KeepEditing)}
                        }
                    }
                }
            }
            if let Some((path, message)) = active_reload_error {
                div { class: "banner error conflict-banner",
                    span { title: "{message}", {l10n(Message::DiskParseFailed(&file_name(&path)))} }
                    div { class: "banner-actions",
                        button {
                            onclick: move |_| reload_external_tab(
                                error_retry_path.clone().expect("reload error path exists"),
                                false,
                                tabs,
                                cell_draft,
                                diagnostic_target,
                                external_conflicts,
                                external_reload_errors,
                                notice,
                            ),
                            {tr(Text::RetryReload)}
                        }
                    }
                }
            }
            main {
                class: if *sidebar_visible.read() { "workspace" } else { "workspace sidebar-hidden" },
                style: "--sidebar-width: {sidebar_width}px",
                if *sidebar_visible.read() {
                    aside { class: "sidebar",
                    div { class: "sidebar-tools",
                        input {
                            class: "filter-input",
                            r#type: "search",
                            placeholder: tr(Text::SearchConfigurations),
                            value: "{filter}",
                            oninput: move |event| filter.set(event.value()),
                        }
                        div { class: "sidebar-mode", role: "group", aria_label: tr(Text::FileView),
                            button {
                                class: if *sidebar_mode.read() == SidebarMode::List { "mode-button active" } else { "mode-button" },
                                aria_pressed: if *sidebar_mode.read() == SidebarMode::List { "true" } else { "false" },
                                onclick: move |_| sidebar_mode.set(SidebarMode::List),
                                {tr(Text::List)}
                            }
                            button {
                                class: if *sidebar_mode.read() == SidebarMode::Tree { "mode-button active" } else { "mode-button" },
                                aria_pressed: if *sidebar_mode.read() == SidebarMode::Tree { "true" } else { "false" },
                                onclick: move |_| sidebar_mode.set(SidebarMode::Tree),
                                {tr(Text::Tree)}
                            }
                        }
                        div { class: "scan-summary",
                            if scan.read().loading { {tr(Text::Scanning)} } else { {count(Count::CsvFiles, file_count)} }
                        }
                    }
                    div { class: "file-list",
                        for row in sidebar_rows {
                            {
                                match row {
                                    WorkspaceTreeRow::Directory {
                                        relative_path,
                                        name,
                                        depth,
                                        expanded,
                                    } => {
                                        let directory_key = relative_path.to_string_lossy().into_owned();
                                        let toggle_path = relative_path.clone();
                                        let indent = 12 + depth * 14;
                                        rsx! {
                                            button {
                                                class: "tree-directory",
                                                key: "{directory_key}",
                                                style: "padding-left: {indent}px",
                                                title: "{directory_key}",
                                                onclick: move |_| {
                                                    let mut directories = expanded_directories.write();
                                                    if expanded {
                                                        directories.remove(&toggle_path);
                                                    } else {
                                                        directories.insert(toggle_path.clone());
                                                    }
                                                },
                                                span { class: "tree-chevron", if expanded { "▾" } else { "▸" } }
                                                span { class: "tree-directory-name", "{name}" }
                                            }
                                        }
                                    }
                                    WorkspaceTreeRow::File { entry, depth } => {
                                let indent = 12 + depth * 14;
                                let workspace_root = workspace.read().clone();
                                let preferences = workspace_root
                                    .as_deref()
                                    .map(|root| {
                                        app_settings
                                            .read()
                                            .file_preferences(root, &entry.relative_path)
                                    })
                                    .unwrap_or_default();
                                let path = entry.absolute_path.clone();
                                let relative = entry.relative_path.to_string_lossy().into_owned();
                                let stats = file_stats.read().get(&entry.absolute_path).cloned();
                                let is_dirty = {
                                    let tabs_read = tabs.read();
                                    let draft_read = cell_draft.read();
                                    tabs_read
                                        .iter()
                                        .find(|tab| tab.document.path == entry.absolute_path)
                                        .is_some_and(|tab| {
                                            tab_has_unsaved_changes(tab, draft_read.as_ref())
                                        })
                                };
                                let row_title = match &stats {
                                    Some(CsvFileStats::Error { message }) => {
                                        format!("{relative}: {message}")
                                    }
                                    _ => relative.clone(),
                                };
                                let file_name = entry.file_name.clone();
                                let open_path = entry.absolute_path.clone();
                                let open_file_name = entry.file_name.clone();
                                rsx! {
                                    button {
                                        class: "file-row",
                                        key: "{relative}",
                                        style: "padding-left: {indent}px",
                                        title: "{row_title}",
                                        onclick: move |_| {
                                            preview_return_tab.set(active_tab.read().clone());
                                            active_tab.set(None);
                                            selected_cell.set(None);
                                            cell_draft.set(None);
                                            diagnostic_target.set(None);
                                            preview.set(Preview::Loading {
                                                path: path.clone(),
                                                file_name: file_name.clone(),
                                            });
                                            let path = path.clone();
                                            let file_name = file_name.clone();
                                            let preferences = preferences;
                                            spawn(async move {
                                                let open_path = path.clone();
                                                let result = tokio::task::spawn_blocking(move || {
                                                    CsvDocument::open(
                                                        &open_path,
                                                        preferences.delimiter.map(CsvDelimiter::byte),
                                                    )
                                                }).await;
                                                if preview_path(&preview.peek()) != Some(path.as_path()) {
                                                    return;
                                                }
                                                preview.set(match result {
                                                    Ok(Ok(document)) => Preview::Document {
                                                        document,
                                                        header_rows: preferences.header_rows,
                                                    },
                                                    Ok(Err(error)) => Preview::Error {
                                                        path: path.clone(),
                                                        file_name,
                                                        message: error.to_string(),
                                                    },
                                                    Err(error) => Preview::Error {
                                                        path: path.clone(),
                                                        file_name,
                                                        message: error.to_string(),
                                                    },
                                                });
                                            });
                                        },
                                        ondoubleclick: move |event| {
                                            event.stop_propagation();
                                            let path = open_path.clone();
                                            if tabs.read().iter().any(|tab| tab.document.path == path) {
                                                preview.set(Preview::Empty);
                                                active_tab.set(Some(path));
                                                preview_return_tab.set(None);
                                                return;
                                            }
                                            if tabs.read().len() >= 20 {
                                                notice.set(Some(tr(Text::TabLimit).to_owned()));
                                                return;
                                            }
                                            let file_name = open_file_name.clone();
                                            let preferences = preferences;
                                            notice.set(Some(l10n(Message::Opening(&file_name))));
                                            spawn(async move {
                                                let result = load_session_for_edit(
                                                    path.clone(),
                                                    preferences.delimiter.map(CsvDelimiter::byte),
                                                    preferences.header_rows,
                                                    false,
                                                ).await;
                                                match result {
                                                    Ok(Some(session)) => {
                                                        if !tabs.read().iter().any(|tab| tab.document.path == path) {
                                                            tabs.write().push(session);
                                                        }
                                                        preview.set(Preview::Empty);
                                                        active_tab.set(Some(path));
                                                        preview_return_tab.set(None);
                                                        selected_cell.set(None);
                                                        cell_draft.set(None);
                                                        diagnostic_target.set(None);
                                                        notice.set(None);
                                                    }
                                                    Ok(None) => notice.set(None),
                                                    Err(error) => notice.set(Some(l10n(Message::Technical {
                                                        prefix: Text::OpenError,
                                                        detail: &error,
                                                    }))),
                                                }
                                            });
                                        },
                                        span { class: "file-title-line",
                                            if is_dirty {
                                                span { class: "dirty-dot", "●" }
                                            }
                                            span { class: "file-icon", aria_hidden: "true" }
                                            span { class: "file-name", "{entry.file_name}" }
                                            span { class: "file-stats",
                                                match stats {
                                                    None => "...".to_owned(),
                                                    Some(CsvFileStats::Ready { data_rows, columns }) => {
                                                        format!("{data_rows} × {columns}")
                                                    }
                                                    Some(CsvFileStats::Error { .. }) => {
                                                        tr(Text::ParseError).to_owned()
                                                    }
                                                }
                                            }
                                        }
                                        if *sidebar_mode.read() == SidebarMode::List {
                                            span { class: "file-path", "{relative}" }
                                        }
                                    }
                                }
                                    }
                                }
                            }
                        }
                    }
                }
                }
                if *sidebar_visible.read() {
                    div {
                        class: "sidebar-resizer",
                        title: tr(Text::ResizeSidebar),
                        onmousedown: move |event| {
                            event.prevent_default();
                            resize_drag.set(Some(ResizeDrag::Sidebar {
                                start_x: event.client_coordinates().x,
                                start_width: *sidebar_width.read(),
                            }));
                        },
                    }
                }
                section { class: "editor",
                    if !tabs.read().is_empty() {
                        div { class: "tabbar",
                            for session in tabs.read().iter() {
                                {
                                    let path = session.document.path.clone();
                                    let close_path = path.clone();
                                    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("CSV").to_owned();
                                    let is_active = active_tab.read().as_ref() == Some(&path);
                                    let is_dirty = session.is_dirty();
                                    rsx! {
                                        div { class: if is_active { "tab active" } else { "tab" }, key: "{path.to_string_lossy()}",
                                            button {
                                                class: "tab-label",
                                                title: "{path.to_string_lossy()}",
                                                onclick: move |_| {
                                                    preview.set(Preview::Empty);
                                                    active_tab.set(Some(path.clone()));
                                                    preview_return_tab.set(None);
                                                    selected_cell.set(None);
                                                    cell_draft.set(None);
                                                    diagnostic_target.set(None);
                                                },
                                                if is_dirty { span { class: "dirty-dot", "●" } }
                                                "{file_name}"
                                            }
                                            button {
                                                class: "tab-close",
                                                title: tr(Text::CloseTab),
                                                onclick: move |_| {
                                                    request_close_tab(
                                                        close_path.clone(),
                                                        tabs,
                                                        active_tab,
                                                        selected_cell,
                                                        cell_draft,
                                                        diagnostic_target,
                                                        table_analyses,
                                                        notice,
                                                    );
                                                },
                                                "×"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(path) = active_tab.read().as_ref() {
                        if let Some(session) = tabs.read().iter().find(|tab| &tab.document.path == path) {
                            {render_csv_document(
                                &session.document,
                                session.text(),
                                session.view(),
                                session.text_parse_issue(),
                                false,
                                session.header_rows,
                                tabs,
                                selected_cell,
                                cell_draft,
                                focused_column,
                                column_widths,
                                resize_drag,
                                diagnostic_target,
                                table_viewports,
                                preference_context.clone(),
                                notice,
                                None,
                            )}
                        }
                    } else {
                        {render_preview(
                            &preview.read(),
                            tabs,
                            selected_cell,
                            cell_draft,
                            focused_column,
                            column_widths,
                            resize_drag,
                            diagnostic_target,
                            table_viewports,
                            preference_context.clone(),
                            notice,
                            preview_edit_context.clone(),
                        )}
                    }
                }
            }
            footer { class: "statusbar",
                span { {count(Count::Files, file_count)} }
                if warning_count > 0 {
                    span { class: "status-warning", {count(Count::ScanWarnings, warning_count)} }
                }
                if let Some(status) = current_status {
                    span { "{status.file_name}" }
                    span { "{status.dimensions}" }
                    if status.delimiter_defaulted {
                        span { class: "status-warning", {tr(Text::DelimiterDefaulted)} }
                    }
                    if let Some(parse_errors) = status.parse_errors {
                        span { class: "status-error", {count(Count::CsvErrors, parse_errors)} }
                    } else if status.analysis_loading {
                        span { {tr(Text::Analyzing)} }
                    } else {
                        if let Some(red_cells) = status.red_cells {
                            span { class: if red_cells > 0 { "status-error" } else { "" }, {count(Count::RedCells, red_cells)} }
                        }
                        if let Some(yellow_columns) = status.yellow_columns {
                            span { class: if yellow_columns > 0 { "status-warning" } else { "" }, {count(Count::YellowColumns, yellow_columns)} }
                        }
                    }
                    if let Some(diagnostic) = status.diagnostic {
                        span {
                            class: "status-diagnostic status-error",
                            title: "{diagnostic}",
                            "{diagnostic}"
                        }
                    }
                    span { class: "status-spacer" }
                    if let Some(position) = status.position {
                        span { "{position}" }
                    }
                    span { "{status.encoding}" }
                } else {
                    span { class: "status-spacer" }
                    span { "UTF-8" }
                }
            }
            if let Some(panel) = *overlay_panel.read() {
                {render_overlay_panel(panel, overlay_context.clone())}
            }
        }
    }
}

fn render_overlay_panel(panel: OverlayPanel, context: OverlayContext) -> Element {
    match panel {
        OverlayPanel::CommandPalette => render_command_palette(context),
        OverlayPanel::GoToLine => render_go_to_line(context),
        OverlayPanel::CurrentSearch => render_current_search(context),
        OverlayPanel::GlobalSearch => render_global_search(context),
        OverlayPanel::Shortcuts => render_shortcuts(context),
    }
}

fn render_shortcuts(mut context: OverlayContext) -> Element {
    let macos = cfg!(target_os = "macos");
    rsx! {
        div { class: "overlay-backdrop",
            section { class: "overlay-panel shortcuts-panel", role: "dialog", aria_label: tr(Text::Shortcuts),
                header { class: "shortcuts-header",
                    h1 { {tr(Text::Shortcuts)} }
                    button {
                        class: "panel-close-button",
                        title: tr(Text::Close),
                        aria_label: tr(Text::Close),
                        onclick: move |_| context.panel.set(None),
                        "×"
                    }
                }
                div { class: "shortcuts-content",
                    for group in [
                        ShortcutGroup::Global,
                        ShortcutGroup::FileSearch,
                        ShortcutGroup::TableNavigation,
                        ShortcutGroup::Editing,
                    ] {
                        section { class: "shortcut-group", key: "{group:?}",
                            h2 { {group.title()} }
                            dl {
                                for entry in SHORTCUTS.iter().filter(|entry| entry.group == group) {
                                    div { class: "shortcut-row", key: "{entry.keys}",
                                        dt { "{shortcut_keys(entry.keys, macos)}" }
                                        dd { {tr(entry.description)} }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_command_palette(context: OverlayContext) -> Element {
    let state = context.command_palette.read().clone();
    let results = if state.query.trim_start().starts_with(':') {
        Vec::new()
    } else {
        rank_files(&context.files, &state.query, 20)
    };
    let selected_index = state.selected_index.min(results.len().saturating_sub(1));
    let mut command_state = context.command_palette;
    let mut close_panel = context.panel;
    let key_context = context.clone();
    let key_results = results.clone();

    rsx! {
        div { class: "overlay-backdrop",
            section { class: "overlay-panel command-panel", role: "dialog", aria_label: tr(Text::CommandPalette),
                input {
                    class: "overlay-search-input",
                    r#type: "search",
                    aria_label: tr(Text::SearchFilesOrLine),
                    placeholder: tr(Text::SearchFiles),
                    autofocus: true,
                    value: "{state.query}",
                    onmounted: move |event| async move {
                        let _ = event.set_focus(true).await;
                    },
                    oninput: move |event| {
                        let mut state = command_state.write();
                        state.query = event.value();
                        state.selected_index = 0;
                    },
                    onkeydown: move |event| {
                        let key = event.key();
                        match key {
                            Key::Escape => {
                                event.prevent_default();
                                event.stop_propagation();
                                close_panel.set(None);
                            }
                            Key::ArrowDown => {
                                event.prevent_default();
                                event.stop_propagation();
                                let mut state = command_state.write();
                                if !key_results.is_empty() {
                                    state.selected_index = (state.selected_index + 1) % key_results.len();
                                }
                            }
                            Key::ArrowUp => {
                                event.prevent_default();
                                event.stop_propagation();
                                let mut state = command_state.write();
                                if !key_results.is_empty() {
                                    state.selected_index = state
                                        .selected_index
                                        .checked_sub(1)
                                        .unwrap_or(key_results.len() - 1);
                                }
                            }
                            Key::Enter => {
                                event.prevent_default();
                                event.stop_propagation();
                                execute_command_palette(key_context.clone(), &key_results);
                            }
                            _ => event.stop_propagation(),
                        }
                    },
                }
                div { class: "overlay-results",
                    if state.query.trim_start().starts_with(':') {
                        button {
                            class: "overlay-result active",
                            onclick: move |_| execute_command_palette(context.clone(), &[]),
                            span { class: "result-icon", "#" }
                            span { class: "result-main",
                                strong { "{state.query.trim()}" }
                            }
                        }
                    } else {
                        for (index, entry) in results.into_iter().enumerate() {
                            {
                                let open_context = context.clone();
                                let open_entry = entry.clone();
                                rsx! {
                                    button {
                                        class: if index == selected_index { "overlay-result active" } else { "overlay-result" },
                                        key: "{entry.absolute_path.to_string_lossy()}",
                                        onclick: move |_| {
                                            open_csv_tab(open_entry.clone(), None, open_context.clone());
                                        },
                                        span { class: "result-icon", "CSV" }
                                        span { class: "result-main",
                                            strong { "{entry.file_name}" }
                                            small { "{entry.relative_path.to_string_lossy()}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_go_to_line(context: OverlayContext) -> Element {
    let value = context.go_to_line.read().clone();
    let mut line_input = context.go_to_line;
    let mut close_panel = context.panel;
    let mut jump_context = context.clone();
    rsx! {
        div { class: "overlay-backdrop",
            section { class: "overlay-panel line-panel", role: "dialog", aria_label: tr(Text::GoToLine),
                input {
                    class: "overlay-search-input",
                    r#type: "number",
                    min: "1",
                    aria_label: tr(Text::LineNumber),
                    placeholder: tr(Text::LineNumber),
                    autofocus: true,
                    value: "{value}",
                    onmounted: move |event| async move {
                        let _ = event.set_focus(true).await;
                    },
                    oninput: move |event| line_input.set(event.value()),
                    onkeydown: move |event| match event.key() {
                        Key::Escape => {
                            event.prevent_default();
                            event.stop_propagation();
                            close_panel.set(None);
                        }
                        Key::Enter => {
                            event.prevent_default();
                            event.stop_propagation();
                            let line = line_input.read().trim().parse::<usize>();
                            match line {
                                Ok(line) if jump_to_line(line, jump_context.clone()) => {
                                    close_panel.set(None);
                                }
                                Ok(_) => {}
                                Err(_) => {
                                    jump_context.notice.set(Some(tr(Text::PositiveLineNumber).to_owned()));
                                }
                            }
                        }
                        _ => event.stop_propagation(),
                    },
                }
            }
        }
    }
}

fn render_current_search(context: OverlayContext) -> Element {
    let state = context.current_search.read().clone();
    let match_count = collect_current_matches(&context, &state).map_or(0, |matches| matches.len());
    let counter = state
        .active_index
        .filter(|index| *index < match_count)
        .map(|index| format!("{} / {match_count}", index + 1))
        .unwrap_or_else(|| format!("{match_count}"));
    let mut search_state = context.current_search;
    let mut close_panel = context.panel;
    let next_context = context.clone();
    let previous_context = context.clone();
    let key_context = context.clone();
    let mut case_context = context.clone();

    rsx! {
        div { class: "overlay-backdrop overlay-top",
            section { class: "overlay-panel current-search-panel", role: "dialog", aria_label: tr(Text::SearchCurrentFile),
                input {
                    class: "overlay-search-input",
                    r#type: "search",
                    aria_label: tr(Text::SearchCurrentFile),
                    placeholder: tr(Text::Find),
                    autofocus: true,
                    value: "{state.query}",
                    onmounted: move |event| async move {
                        let _ = event.set_focus(true).await;
                    },
                    oninput: move |event| {
                        let mut state = search_state.write();
                        state.query = event.value();
                        state.active_index = None;
                    },
                    onkeydown: move |event| match event.key() {
                        Key::Escape => {
                            event.prevent_default();
                            event.stop_propagation();
                            close_panel.set(None);
                        }
                        Key::Enter => {
                            event.prevent_default();
                            event.stop_propagation();
                            navigate_current_search(
                                key_context.clone(),
                                if event.modifiers().contains(Modifiers::SHIFT) { -1 } else { 1 },
                            );
                        }
                        _ => event.stop_propagation(),
                    },
                }
                label { class: "case-toggle", title: tr(Text::MatchCase),
                    input {
                        r#type: "checkbox",
                        aria_label: tr(Text::MatchCase),
                        checked: state.case_sensitive,
                        onchange: move |event| {
                            let mut state = case_context.current_search.write();
                            state.case_sensitive = event.checked();
                            state.active_index = None;
                        },
                    }
                    span { "Aa" }
                }
                span { class: "search-counter", "{counter}" }
                button {
                    class: "panel-tool-button",
                    title: tr(Text::PreviousMatch),
                    aria_label: tr(Text::PreviousMatch),
                    onclick: move |_| navigate_current_search(previous_context.clone(), -1),
                    "‹"
                }
                button {
                    class: "panel-tool-button",
                    title: tr(Text::NextMatch),
                    aria_label: tr(Text::NextMatch),
                    onclick: move |_| navigate_current_search(next_context.clone(), 1),
                    "›"
                }
                button {
                    class: "panel-close-button",
                    title: tr(Text::CloseSearch),
                    aria_label: tr(Text::CloseSearch),
                    onclick: move |_| close_panel.set(None),
                    "×"
                }
            }
        }
    }
}

fn render_global_search(mut context: OverlayContext) -> Element {
    let state = context.global_search.read().clone();
    let mut close_panel = context.panel;
    let cancel_signal = context.global_search_cancel;
    let input_context = context.clone();
    let case_context = context.clone();
    let close_cancel = context.global_search_cancel;
    let result_count = state.results.len();
    let summary = search_summary(state.loading, state.truncated, result_count);

    rsx! {
        div { class: "overlay-backdrop",
            section { class: "overlay-panel global-search-panel", role: "dialog", aria_label: tr(Text::SearchWorkspace),
                div { class: "global-search-header",
                    input {
                        class: "overlay-search-input",
                        r#type: "search",
                        aria_label: tr(Text::SearchWorkspaceContents),
                        placeholder: tr(Text::SearchWorkspace),
                        autofocus: true,
                        value: "{state.query}",
                        onmounted: move |event| async move {
                            let _ = event.set_focus(true).await;
                        },
                        oninput: move |event| {
                            start_global_search(
                                event.value(),
                                state.case_sensitive,
                                input_context.clone(),
                            );
                        },
                        onkeydown: move |event| {
                            if event.key() == Key::Escape {
                                event.prevent_default();
                                event.stop_propagation();
                                if let Some(cancel) = close_cancel.read().as_ref() {
                                    cancel.store(true, Ordering::Relaxed);
                                }
                                close_panel.set(None);
                            } else {
                                event.stop_propagation();
                            }
                        },
                    }
                    label { class: "case-toggle", title: tr(Text::MatchCase),
                        input {
                            r#type: "checkbox",
                            aria_label: tr(Text::MatchCase),
                            checked: state.case_sensitive,
                            onchange: move |event| {
                                start_global_search(
                                    state.query.clone(),
                                    event.checked(),
                                    case_context.clone(),
                                );
                            },
                        }
                        span { "Aa" }
                    }
                    span { class: "search-counter", "{summary}" }
                    if state.loading {
                        button {
                            class: "panel-tool-button",
                            onclick: move |_| {
                                if let Some(cancel) = cancel_signal.read().as_ref() {
                                    cancel.store(true, Ordering::Relaxed);
                                }
                                context.global_search.write().loading = false;
                            },
                            {tr(Text::Stop)}
                        }
                    }
                    button {
                        class: "panel-close-button",
                        title: tr(Text::CloseSearch),
                        aria_label: tr(Text::CloseSearch),
                        onclick: move |_| {
                            if let Some(cancel) = cancel_signal.read().as_ref() {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            close_panel.set(None);
                        },
                        "×"
                    }
                }
                if state.warning_count > 0 {
                    div { class: "search-warning", {unsearchable_files(state.warning_count)} }
                }
                div { class: "overlay-results global-results",
                    for result in state.results {
                        {
                            let open_context = context.clone();
                            let open_entry = CsvFileEntry {
                                absolute_path: result.absolute_path.clone(),
                                relative_path: result.relative_path.clone(),
                                file_name: file_name(&result.absolute_path),
                            };
                            let line_number = result.line_number;
                            rsx! {
                                button {
                                    class: "overlay-result",
                                    key: "{result.absolute_path.to_string_lossy()}:{line_number}",
                                    onclick: move |_| open_csv_tab(
                                        open_entry.clone(),
                                        Some(line_number),
                                        open_context.clone(),
                                    ),
                                    span { class: "result-icon", "{line_number}" }
                                    span { class: "result-main",
                                        strong { "{result.relative_path.to_string_lossy()}" }
                                        small { "{result.snippet}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum CurrentMatchSet {
    Cells {
        path: PathBuf,
        header_rows: usize,
        matches: Vec<CellSearchMatch>,
    },
    Text {
        path: PathBuf,
        matches: Vec<TextSearchMatch>,
    },
}

impl CurrentMatchSet {
    fn len(&self) -> usize {
        match self {
            Self::Cells { matches, .. } => matches.len(),
            Self::Text { matches, .. } => matches.len(),
        }
    }
}

fn execute_command_palette(mut context: OverlayContext, results: &[CsvFileEntry]) {
    let state = context.command_palette.read().clone();
    if let Some(line) = state.query.trim().strip_prefix(':') {
        match line.trim().parse::<usize>() {
            Ok(line) if jump_to_line(line, context.clone()) => context.panel.set(None),
            Ok(_) => {}
            Err(_) => context
                .notice
                .set(Some(tr(Text::PositiveLineAfterColon).to_owned())),
        }
        return;
    }
    let Some(entry) = results.get(state.selected_index.min(results.len().saturating_sub(1))) else {
        return;
    };
    open_csv_tab(entry.clone(), None, context);
}

async fn load_session_for_edit(
    path: PathBuf,
    delimiter_override: Option<u8>,
    header_rows: usize,
    conversion_already_allowed: bool,
) -> Result<Option<DocumentSession>, String> {
    let open_path = path.clone();
    let first = tokio::task::spawn_blocking(move || {
        DocumentSession::open_with_conversion(
            &open_path,
            delimiter_override,
            header_rows,
            conversion_already_allowed,
        )
    })
    .await
    .map_err(|error| error.to_string())?;

    match first {
        Ok(session) => Ok(Some(session)),
        Err(DocumentSessionError::Gb18030RequiresConversion { .. }) => {
            if !confirm_gb18030_conversion(&path) {
                return Ok(None);
            }
            let open_path = path;
            tokio::task::spawn_blocking(move || {
                DocumentSession::open_with_conversion(
                    &open_path,
                    delimiter_override,
                    header_rows,
                    true,
                )
            })
            .await
            .map_err(|error| error.to_string())?
            .map(Some)
            .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn confirm_gb18030_conversion(path: &Path) -> bool {
    let convert_label = tr(Text::ConvertToUtf8);
    let choice = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(tr(Text::ConvertEncodingTitle))
        .set_description(l10n(Message::ConvertGb18030(&file_name(path))))
        .set_buttons(MessageButtons::OkCancelCustom(
            convert_label.to_owned(),
            tr(Text::Cancel).to_owned(),
        ))
        .show();
    matches!(choice, MessageDialogResult::Ok)
        || matches!(&choice, MessageDialogResult::Custom(label) if label == convert_label)
}

fn open_csv_tab(entry: CsvFileEntry, text_line: Option<usize>, mut context: OverlayContext) {
    let path = entry.absolute_path.clone();
    let existing_text = {
        let mut tabs = context.tabs;
        let mut tabs_write = tabs.write();
        tabs_write
            .iter_mut()
            .find(|tab| tab.document.path == path)
            .map(|tab| {
                if text_line.is_some() {
                    tab.show_text();
                }
                tab.text().to_owned()
            })
    };
    if let Some(text) = existing_text {
        context.preview.set(Preview::Empty);
        context.active_tab.set(Some(path.clone()));
        context.preview_return_tab.set(None);
        context.selected_cell.set(None);
        context.cell_draft.set(None);
        context.focused_column.set(None);
        context.diagnostic_target.set(None);
        context.panel.set(None);
        if let Some(line) = text_line {
            schedule_text_line_jump(path, text, line);
        }
        return;
    }

    if context.tabs.read().len() >= 20 {
        context.notice.set(Some(tr(Text::TabLimit).to_owned()));
        return;
    }
    let preferences = context
        .workspace
        .as_deref()
        .map(|root| {
            context
                .settings
                .read()
                .file_preferences(root, &entry.relative_path)
        })
        .unwrap_or_default();
    context
        .notice
        .set(Some(l10n(Message::Opening(&entry.file_name))));
    context.panel.set(None);
    spawn(async move {
        let result = load_session_for_edit(
            path.clone(),
            preferences.delimiter.map(CsvDelimiter::byte),
            preferences.header_rows,
            false,
        )
        .await;
        match result {
            Ok(Some(mut session)) => {
                let jump_text = text_line.map(|_| {
                    session.show_text();
                    session.text().to_owned()
                });
                if !context
                    .tabs
                    .read()
                    .iter()
                    .any(|tab| tab.document.path == path)
                {
                    context.tabs.write().push(session);
                }
                context.preview.set(Preview::Empty);
                context.active_tab.set(Some(path.clone()));
                context.preview_return_tab.set(None);
                context.selected_cell.set(None);
                context.cell_draft.set(None);
                context.focused_column.set(None);
                context.diagnostic_target.set(None);
                context.notice.set(None);
                if let (Some(line), Some(text)) = (text_line, jump_text) {
                    schedule_text_line_jump(path, text, line);
                }
            }
            Ok(None) => context.notice.set(None),
            Err(error) => context.notice.set(Some(l10n(Message::Technical {
                prefix: Text::OpenError,
                detail: &error,
            }))),
        }
    });
}

fn collect_current_matches(
    context: &OverlayContext,
    state: &CurrentSearchState,
) -> Option<CurrentMatchSet> {
    if state.query.is_empty() {
        return None;
    }
    if let Some(path) = context.active_tab.read().clone() {
        let tabs = context.tabs.read();
        let tab = tabs.iter().find(|tab| tab.document.path == path)?;
        return Some(if tab.view() == DocumentView::Text {
            CurrentMatchSet::Text {
                path,
                matches: find_text_matches(tab.text(), &state.query, state.case_sensitive),
            }
        } else {
            CurrentMatchSet::Cells {
                path,
                header_rows: tab.header_rows,
                matches: find_cell_matches(
                    &tab.document.records,
                    tab.header_rows,
                    &state.query,
                    state.case_sensitive,
                ),
            }
        });
    }
    match &*context.preview.read() {
        Preview::Document {
            document,
            header_rows,
        } => Some(CurrentMatchSet::Cells {
            path: document.path.clone(),
            header_rows: *header_rows,
            matches: find_cell_matches(
                &document.records,
                *header_rows,
                &state.query,
                state.case_sensitive,
            ),
        }),
        _ => None,
    }
}

fn navigate_current_search(mut context: OverlayContext, direction: isize) {
    let state = context.current_search.read().clone();
    let Some(matches) = collect_current_matches(&context, &state) else {
        context
            .notice
            .set(Some(tr(Text::NoSearchableFile).to_owned()));
        return;
    };
    let match_count = matches.len();
    if match_count == 0 {
        context.notice.set(Some(tr(Text::NoMatches).to_owned()));
        return;
    }
    let next_index = match (state.active_index, direction.is_negative()) {
        (None, false) => 0,
        (None, true) => match_count - 1,
        (Some(0), true) => match_count - 1,
        (Some(index), true) => index - 1,
        (Some(index), false) => (index + 1) % match_count,
    };
    context.current_search.write().active_index = Some(next_index);

    match matches {
        CurrentMatchSet::Cells {
            path,
            header_rows,
            matches,
        } => {
            let matched = matches[next_index];
            context.selected_cell.set(Some(CellLocation {
                path,
                row_index: matched.row_index,
                column_index: matched.column_index,
            }));
            context.diagnostic_target.set(None);
            scroll_to_target(
                DiagnosticTarget::Cell(GridPosition {
                    row_index: matched.row_index,
                    column_index: matched.column_index,
                }),
                header_rows,
            );
        }
        CurrentMatchSet::Text { path, matches } => {
            select_text_match(&path, &matches[next_index]);
        }
    }
    context.notice.set(Some(l10n(Message::MatchPosition {
        current: next_index + 1,
        total: match_count,
    })));
}

fn jump_to_line(line: usize, mut context: OverlayContext) -> bool {
    if line == 0 {
        context
            .notice
            .set(Some(tr(Text::LineStartsAtOne).to_owned()));
        return false;
    }
    if let Some(path) = context.active_tab.read().clone() {
        let tabs = context.tabs.read();
        let Some(tab) = tabs.iter().find(|tab| tab.document.path == path) else {
            return false;
        };
        if tab.view() == DocumentView::Text {
            let line_count = physical_line_count(tab.text());
            if line > line_count {
                context.notice.set(Some(l10n(Message::LineOutOfFile {
                    line,
                    lines: line_count,
                })));
                return false;
            }
            let text = tab.text().to_owned();
            drop(tabs);
            schedule_text_line_jump(path, text, line);
            context
                .notice
                .set(Some(l10n(Message::TextPosition { line, column: 1 })));
            return true;
        }
        let data_rows = tab.document.records.len().saturating_sub(tab.header_rows);
        if line > data_rows {
            context.notice.set(Some(l10n(Message::RowOutOfTable {
                row: line,
                rows: data_rows,
            })));
            return false;
        }
        let row_index = tab.header_rows + line - 1;
        let column_count = tab.document.records.first().map_or(0, Vec::len);
        if column_count == 0 {
            context
                .notice
                .set(Some(tr(Text::TableNoColumns).to_owned()));
            return false;
        }
        let column_index = context
            .selected_cell
            .read()
            .as_ref()
            .filter(|location| location.path == path)
            .map(|location| location.column_index.min(column_count - 1))
            .unwrap_or(0);
        let header_rows = tab.header_rows;
        drop(tabs);
        context.selected_cell.set(Some(CellLocation {
            path,
            row_index,
            column_index,
        }));
        context.diagnostic_target.set(None);
        scroll_to_target(
            DiagnosticTarget::Cell(GridPosition {
                row_index,
                column_index,
            }),
            header_rows,
        );
        context.notice.set(Some(l10n(Message::TablePosition {
            row: line,
            column: 1,
        })));
        return true;
    }

    if let Preview::Document {
        document,
        header_rows,
    } = &*context.preview.read()
    {
        let data_rows = document.records.len().saturating_sub(*header_rows);
        let column_count = document.records.first().map_or(0, Vec::len);
        if line == 0 || line > data_rows || column_count == 0 {
            context.notice.set(Some(l10n(Message::RowOutOfPreview {
                row: line,
                rows: data_rows,
            })));
            return false;
        }
        let row_index = *header_rows + line - 1;
        context.selected_cell.set(Some(CellLocation {
            path: document.path.clone(),
            row_index,
            column_index: 0,
        }));
        scroll_to_target(
            DiagnosticTarget::Cell(GridPosition {
                row_index,
                column_index: 0,
            }),
            *header_rows,
        );
        context.notice.set(Some(l10n(Message::TablePosition {
            row: line,
            column: 1,
        })));
        return true;
    }
    context
        .notice
        .set(Some(tr(Text::NoSearchableFile).to_owned()));
    false
}

fn start_global_search(query: String, case_sensitive: bool, mut context: OverlayContext) {
    if let Some(previous) = context.global_search_cancel.read().as_ref() {
        previous.store(true, Ordering::Relaxed);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    context.global_search_cancel.set(Some(cancel.clone()));
    context.global_search.set(GlobalSearchState {
        query: query.clone(),
        case_sensitive,
        results: Vec::new(),
        loading: !query.is_empty(),
        truncated: false,
        warning_count: 0,
    });
    if query.is_empty() {
        return;
    }

    let files = context.files.clone();
    spawn(async move {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let worker_cancel = cancel.clone();
        let worker = tokio::task::spawn_blocking(move || {
            stream_workspace_search(files, query, case_sensitive, worker_cancel, sender)
        });
        while let Some(event) = receiver.recv().await {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            match event {
                GlobalSearchEvent::Batch(batch) => {
                    context.global_search.write().results.extend(batch);
                }
                GlobalSearchEvent::Finished {
                    cancelled,
                    truncated,
                    warning_count,
                } => {
                    let mut state = context.global_search.write();
                    state.loading = false;
                    state.truncated = truncated;
                    state.warning_count = warning_count;
                    if cancelled {
                        break;
                    }
                }
            }
        }
        let _ = worker.await;
    });
}

fn select_text_match(path: &Path, matched: &TextSearchMatch) {
    let editor_id = text_editor_id(path);
    let start = matched.start_utf16;
    let end = matched.end_utf16;
    let script = format!(
        r#"
        const editor = document.getElementById('{editor_id}');
        if (editor instanceof HTMLTextAreaElement) {{
            editor.focus();
            editor.setSelectionRange({start}, {end});
            const lineHeight = 20;
            editor.scrollTop = Math.max(0, ({line} - 2) * lineHeight);
        }}
        "#,
        line = matched.line_number,
    );
    let _ = document::eval(&script);
}

fn schedule_text_line_jump(path: PathBuf, text: String, line: usize) {
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        select_text_line(&path, &text, line);
    });
}

fn select_text_line(path: &Path, text: &str, line: usize) {
    let start_byte = if line == 1 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 2)
            .map_or(text.len(), |(index, _)| index + 1)
    };
    let end_byte = text[start_byte..]
        .find('\n')
        .map_or(text.len(), |offset| start_byte + offset);
    let start = text[..start_byte].encode_utf16().count();
    let end = text[..end_byte].encode_utf16().count();
    let editor_id = text_editor_id(path);
    let script = format!(
        r#"
        const editor = document.getElementById('{editor_id}');
        if (editor instanceof HTMLTextAreaElement) {{
            editor.focus();
            editor.setSelectionRange({start}, {end});
            editor.scrollTop = Math.max(0, ({line} - 2) * 20);
            window.setTimeout(() => {{
                if (document.activeElement === editor) editor.setSelectionRange({end}, {end});
            }}, 2000);
        }}
        "#,
    );
    let _ = document::eval(&script);
}

#[allow(clippy::too_many_arguments)]
fn render_preview(
    preview: &Preview,
    tabs: Signal<Vec<DocumentSession>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    focused_column: Signal<Option<FocusedColumn>>,
    column_widths: Signal<HashMap<(PathBuf, usize), usize>>,
    resize_drag: Signal<Option<ResizeDrag>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_viewports: Signal<HashMap<PathBuf, TableViewport>>,
    preference_context: PreferenceContext,
    notice: Signal<Option<String>>,
    preview_edit_context: PreviewEditContext,
) -> Element {
    match preview {
        Preview::Empty => rsx! {
            div { class: "empty-editor",
                h1 { {tr(Text::EmptyPreviewTitle)} }
                p { {tr(Text::EmptyPreviewBody)} }
            }
        },
        Preview::Loading { file_name, .. } => rsx! {
            div { class: "empty-editor",
                h1 { "{file_name}" }
                p { {tr(Text::LoadingPreview)} }
            }
        },
        Preview::Error {
            file_name, message, ..
        } => rsx! {
            div { class: "preview-error",
                h1 { "{file_name}" }
                p { strong { {tr(Text::PreviewFailed)} ": " } "{message}" }
            }
        },
        Preview::Document {
            document,
            header_rows,
        } => render_csv_document(
            document,
            &document.raw_text,
            DocumentView::Table,
            None,
            true,
            *header_rows,
            tabs,
            selected_cell,
            cell_draft,
            focused_column,
            column_widths,
            resize_drag,
            diagnostic_target,
            table_viewports,
            preference_context,
            notice,
            Some(preview_edit_context),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_csv_document(
    document: &CsvDocument,
    text: &str,
    view: DocumentView,
    text_parse_issue: Option<&TextParseIssue>,
    read_only: bool,
    header_rows: usize,
    tabs: Signal<Vec<DocumentSession>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    mut focused_column: Signal<Option<FocusedColumn>>,
    column_widths: Signal<HashMap<(PathBuf, usize), usize>>,
    mut resize_drag: Signal<Option<ResizeDrag>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut table_viewports: Signal<HashMap<PathBuf, TableViewport>>,
    preference_context: PreferenceContext,
    notice: Signal<Option<String>>,
    preview_edit_context: Option<PreviewEditContext>,
) -> Element {
    let path = document.path.clone();
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| tr(Text::ReadOnlyPreview))
        .to_owned();
    let delimiter = CsvDelimiter::from_byte(document.delimiter).unwrap_or(CsvDelimiter::Comma);
    let text_view = !read_only && view == DocumentView::Text;
    let analyses = if !text_view && document.records.len() >= header_rows {
        preference_context
            .table_analyses
            .read()
            .get(&path)
            .and_then(|state| state.ready_columns(document.analysis_version(), header_rows))
    } else {
        None
    };
    let line_count = if text_view {
        physical_line_count(text)
    } else {
        0
    };
    let summary = if text_view {
        physical_lines(line_count)
    } else {
        document
            .dimensions(header_rows)
            .map(|(rows, columns)| records_columns(rows, columns))
            .unwrap_or_else(|| header_requirement(header_rows))
    };
    let mode_label = if read_only {
        tr(Text::ReadOnlyPreview)
    } else if text_view {
        tr(Text::EditableText)
    } else {
        tr(Text::EditableTable)
    };
    let save_path = path.clone();
    let data_row_count = document.records.len().saturating_sub(header_rows);
    let data_column_count = document.records.first().map_or(0, Vec::len);
    let focused_index = focused_column
        .read()
        .as_ref()
        .filter(|focused| focused.path == path && focused.column_index < data_column_count)
        .map(|focused| focused.column_index);
    let focus_status = focused_index.map(|column_index| {
        let field = focused_column_field(document, header_rows, column_index);
        (
            l10n(Message::FocusMode {
                column: column_index + 1,
                field,
            }),
            l10n(Message::FocusModeTooltip {
                column: column_index + 1,
                field,
            }),
        )
    });
    let mut viewport = table_viewports
        .read()
        .get(&path)
        .copied()
        .unwrap_or_default();
    let selected_focus_cell = focused_index.and_then(|focused_index| {
        selected_cell
            .read()
            .as_ref()
            .filter(|location| {
                location.path == path
                    && location.column_index == focused_index
                    && location.row_index >= header_rows
            })
            .map(|location| (location.row_index - header_rows, location.column_index))
    });
    viewport.expanded_row = viewport.expanded_row.filter(|row| {
        selected_focus_cell
            .is_some_and(|(index, column_index)| row.matches_cell(index, column_index))
    });
    let visible_range = visible_row_range(data_row_count, viewport);
    let (top_spacer_height, bottom_spacer_height) =
        spacer_heights(data_row_count, &visible_range, viewport.expanded_row);
    let visible_start = header_rows + visible_range.start;
    let visible_count = visible_range.len();
    let column_count = data_column_count + 1;
    let scroll_path = path.clone();
    let table_mode_path = path.clone();
    let table_class = if focused_index.is_some() {
        "csv-table focus-table"
    } else {
        "csv-table"
    };
    let requested_focus_width = focused_index
        .and_then(|column_index| {
            analyses
                .as_deref()
                .and_then(|columns| columns.get(column_index))
        })
        .map(|analysis| focus_column_width(analysis.max_content_chars))
        .unwrap_or(320);
    let focus_layout = focused_index.and_then(|column_index| {
        FocusLayout::calculate(
            data_column_count,
            column_index,
            requested_focus_width,
            viewport.width,
        )
    });
    let focus_width = focus_layout
        .as_ref()
        .map_or(requested_focus_width, |layout| layout.focused_width);
    let configured_column_widths = {
        let widths = column_widths.read();
        (0..data_column_count)
            .map(|column_index| {
                widths
                    .get(&(path.clone(), column_index))
                    .copied()
                    .unwrap_or(180)
            })
            .collect::<Vec<_>>()
    };
    let table_width = if let Some(layout) = focus_layout.as_ref() {
        layout.table_width
    } else {
        58 + configured_column_widths.iter().sum::<usize>()
    };
    let left_spacer_width = focus_layout
        .as_ref()
        .map_or(0, |layout| layout.left_spacer_width);
    let right_spacer_width = focus_layout
        .as_ref()
        .map_or(0, |layout| layout.right_spacer_width);
    let rendered_column_count =
        column_count + usize::from(left_spacer_width > 0) + usize::from(right_spacer_width > 0);
    let type_row_top = header_rows * HEADER_ROW_HEIGHT;
    let header_path = path.clone();
    let header_context = preference_context.clone();
    let delimiter_path = path.clone();
    let delimiter_context = preference_context.clone();
    let table_view_path = path.clone();
    let table_view_context = preference_context.clone();
    let text_view_path = path.clone();
    let text_input_path = path.clone();
    let mut text_tabs = tabs;
    let mut text_notice = notice;
    let text_value = if text_view {
        text.to_owned()
    } else {
        String::new()
    };
    let line_numbers = if text_view {
        physical_line_numbers(line_count)
    } else {
        String::new()
    };
    let editor_id = text_editor_id(&path);
    let gutter_id = format!("{editor_id}-lines");
    let scroll_editor_id = editor_id.clone();
    let scroll_gutter_id = gutter_id.clone();
    let parse_issue = text_parse_issue.cloned();
    let table_analyses = preference_context.table_analyses;
    let reveal_path = path.clone();
    let mut reveal_notice = notice;
    let reveal_button_label = tr(Text::RevealInFileManager);

    rsx! {
        div { class: "preview-header",
            div {
                h1 { "{title}" }
                p { "{mode_label} · {summary}" }
                if let Some((focus_label, focus_tooltip)) = focus_status.as_ref() {
                    p {
                        class: "focus-mode-status",
                        title: "{focus_tooltip}",
                        "{focus_label}"
                    }
                }
            }
            div { class: "preview-actions",
                if !read_only {
                    div { class: "view-switch", role: "group", aria_label: tr(Text::EditorView),
                        button {
                            class: if view == DocumentView::Table { "active" } else { "" },
                            aria_pressed: if view == DocumentView::Table { "true" } else { "false" },
                            onclick: move |_| request_document_view(
                                &table_view_path,
                                DocumentView::Table,
                                tabs,
                                cell_draft,
                                selected_cell,
                                focused_column,
                                diagnostic_target,
                                table_viewports,
                                table_view_context.clone(),
                                notice,
                            ),
                            {tr(Text::Table)}
                        }
                        button {
                            class: if view == DocumentView::Text { "active" } else { "" },
                            aria_pressed: if view == DocumentView::Text { "true" } else { "false" },
                            onclick: move |_| request_document_view(
                                &text_view_path,
                                DocumentView::Text,
                                tabs,
                                cell_draft,
                                selected_cell,
                                focused_column,
                                diagnostic_target,
                                table_viewports,
                                preference_context.clone(),
                                notice,
                            ),
                            {tr(Text::Text)}
                        }
                    }
                }
                if !text_view {
                    label { class: "document-setting",
                    span { {tr(Text::HeaderRows)} }
                    select {
                        aria_label: tr(Text::HeaderRows),
                        value: "{header_rows}",
                        onchange: move |event| {
                            if let Ok(rows) = event.value().parse::<usize>() {
                                change_header_rows(
                                    &header_path,
                                    rows,
                                    read_only,
                                    tabs,
                                    cell_draft,
                                    selected_cell,
                                    diagnostic_target,
                                    table_viewports,
                                    header_context.clone(),
                                    notice,
                                );
                            }
                        },
                        for rows in MIN_HEADER_ROWS..=MAX_HEADER_ROWS {
                            option {
                                value: "{rows}",
                                selected: rows == header_rows,
                                "{rows}"
                            }
                        }
                    }
                }
                    label { class: "document-setting",
                    span { {tr(Text::Delimiter)} }
                    select {
                        aria_label: tr(Text::CsvDelimiter),
                        value: "{delimiter.setting_value()}",
                        onchange: move |event| {
                            if let Some(delimiter) = CsvDelimiter::from_setting_value(&event.value()) {
                                change_delimiter(
                                    &delimiter_path,
                                    delimiter,
                                    read_only,
                                    tabs,
                                    cell_draft,
                                    selected_cell,
                                    diagnostic_target,
                                    table_viewports,
                                    delimiter_context.clone(),
                                    notice,
                                );
                            }
                        },
                        for option_delimiter in CsvDelimiter::ALL {
                            option {
                                value: "{option_delimiter.setting_value()}",
                                selected: option_delimiter == delimiter,
                                {localized_delimiter_label(option_delimiter)}
                            }
                        }
                    }
                }
                }
                button {
                    class: "reveal-button",
                    title: "{reveal_button_label}",
                    onclick: move |_| match reveal_in_file_manager(&reveal_path) {
                        Ok(()) => reveal_notice.set(Some(l10n(Message::Revealed(
                            &file_name(&reveal_path),
                        )))),
                        Err(error) => reveal_notice.set(Some(l10n(Message::RevealFailed {
                            file: &file_name(&reveal_path),
                            detail: &error.to_string(),
                        }))),
                    },
                    "{reveal_button_label}"
                }
                if !read_only {
                    button {
                        class: "save-button",
                        title: tr(Text::SaveFile),
                        onclick: move |_| {
                            attempt_save_tab(
                                &save_path,
                                tabs,
                                selected_cell,
                                cell_draft,
                                diagnostic_target,
                                table_analyses,
                                notice,
                            );
                        },
                        {tr(Text::Save)}
                    }
                }
            }
        }
        if text_view {
            div { class: "text-view",
                if let Some(issue) = parse_issue {
                    div { class: "text-parse-error", title: "{issue.message}",
                        strong { {csv_parse_failed(issue.count)} }
                        span { "{issue.message}" }
                    }
                }
                div { class: "text-editor-shell",
                    pre {
                        id: "{gutter_id}",
                        class: "text-line-numbers",
                        aria_hidden: "true",
                        "{line_numbers}"
                    }
                    textarea {
                        id: "{editor_id}",
                        class: "text-editor-input",
                        "data-path": "{path.to_string_lossy()}",
                        aria_label: tr(Text::CsvTextEditor),
                        spellcheck: "false",
                        wrap: "off",
                        value: "{text_value}",
                        oninput: move |event| {
                            if let Some(tab) = text_tabs
                                .write()
                                .iter_mut()
                                .find(|tab| tab.document.path == text_input_path)
                                && tab.set_text(event.value())
                            {
                                text_notice.set(None);
                            }
                        },
                        onkeydown: move |event| event.stop_propagation(),
                        onscroll: move |_| sync_text_editor_scroll(
                            &scroll_editor_id,
                            &scroll_gutter_id,
                        ),
                    }
                }
            }
        } else if document.records.len() < header_rows {
            div { class: "preview-error",
                h1 { {tr(Text::InvalidHeaderTitle)} }
                p { {tr(Text::InvalidHeaderBody)} " ({header_rows})" }
            }
        } else {
            div {
                class: "table-scroll",
                tabindex: "0",
                "data-path": "{path.to_string_lossy()}",
                "data-row-height": "{DATA_ROW_HEIGHT}",
                "data-expanded-row-index": viewport.expanded_row.map(|row| row.index.to_string()).unwrap_or_default(),
                "data-expanded-row-height": viewport.expanded_row.map(|row| row.height.to_string()).unwrap_or_default(),
                onkeydown: move |event| handle_table_mode_keydown(
                    event,
                    &table_mode_path,
                    data_column_count,
                    header_rows,
                    selected_cell,
                    cell_draft,
                    focused_column,
                    diagnostic_target,
                ),
                onscroll: move |event| {
                    let current = table_viewports
                        .read()
                        .get(&scroll_path)
                        .copied()
                        .unwrap_or_default();
                    let next = TableViewport {
                        scroll_top: event.data().scroll_top(),
                        height: f64::from(event.data().client_height()),
                        width: current.width,
                        expanded_row: current.expanded_row,
                    };
                    if visible_row_range(data_row_count, current)
                        != visible_row_range(data_row_count, next)
                    {
                        table_viewports.write().insert(scroll_path.clone(), next);
                    }
                },
                table {
                    class: "{table_class}",
                    style: "--focus-column-width: {focus_width}px; width: {table_width}px",
                    colgroup {
                        col { class: "row-number-column" }
                        if left_spacer_width > 0 {
                            col {
                                class: "focus-side-spacer",
                                style: "width: {left_spacer_width}px",
                            }
                        }
                        for (column_index, configured_width) in configured_column_widths.iter().copied().enumerate() {
                            {
                                let focus_class = column_focus_class(column_index, focus_layout.as_ref());
                                let width = focus_layout
                                    .as_ref()
                                    .and_then(|layout| layout.column_width(column_index))
                                    .unwrap_or(configured_width);
                                rsx! {
                                    col {
                                        class: "{focus_class}",
                                        key: "{column_index}",
                                        style: "width: {width}px",
                                    }
                                }
                            }
                        }
                        if right_spacer_width > 0 {
                            col {
                                class: "focus-side-spacer",
                                style: "width: {right_spacer_width}px",
                            }
                        }
                    }
                    thead {
                        for (header_index, record) in document.records.iter().take(header_rows).enumerate() {
                            {
                                let header_top = header_index * HEADER_ROW_HEIGHT;
                                rsx! {
                                    tr {
                                        class: if header_index + 1 == header_rows { "field-header" } else { "description-header" },
                                        key: "header-{header_index}",
                                        style: "--header-top: {header_top}px",
                                        th { class: "row-number", "" }
                                        if left_spacer_width > 0 {
                                            th { class: "focus-side-spacer", "" }
                                        }
                                        for (column_index, value) in record.iter().enumerate() {
                                            {
                                                let column_class = with_column_focus_class(
                                                    "",
                                                    column_index,
                                                    focus_layout.as_ref(),
                                                );
                                                let resize_path = path.clone();
                                                let configured_width = configured_column_widths
                                                    .get(column_index)
                                                    .copied()
                                                    .unwrap_or(180);
                                                rsx! {
                                                    th {
                                                        class: "{column_class}",
                                                        key: "{column_index}",
                                                        title: "{value}",
                                                        "{value}"
                                                        if !read_only
                                                            && focused_index.is_none()
                                                            && header_index + 1 == header_rows
                                                        {
                                                            button {
                                                                class: "column-resizer",
                                                                aria_label: format!("{} {column_index}", tr(Text::ResizeColumn)),
                                                                title: tr(Text::ResizeColumn),
                                                                onmousedown: move |event| {
                                                                    event.prevent_default();
                                                                    event.stop_propagation();
                                                                    resize_drag.set(Some(ResizeDrag::Column {
                                                                        path: resize_path.clone(),
                                                                        column_index,
                                                                        start_x: event.client_coordinates().x,
                                                                        start_width: configured_width,
                                                                    }));
                                                                },
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        if right_spacer_width > 0 {
                                            th { class: "focus-side-spacer", "" }
                                        }
                                    }
                                }
                            }
                        }
                        tr { class: "type-row", style: "--type-top: {type_row_top}px",
                            th { class: "row-number", {tr(Text::TypeLabel)} }
                            if left_spacer_width > 0 {
                                th { class: "focus-side-spacer", "" }
                            }
                            if let Some(analyses) = analyses.as_deref() {
                                for (column_index, analysis) in analyses.iter().enumerate() {
                                    {
                                        let structural_error = column_has_structural_error(analysis);
                                        let label = column_type_label(analysis);
                                        let is_diagnostic_target = diagnostic_target.read().as_ref()
                                            == Some(&DiagnosticTarget::Column(column_index));
                                        let diagnostic_class = if structural_error || !analysis.problems.is_empty() {
                                            if is_diagnostic_target { "type-error type-selected" } else { "type-error" }
                                        } else if analysis.has_mixed_warning {
                                            if is_diagnostic_target { "type-warning type-selected" } else { "type-warning" }
                                        } else if is_diagnostic_target {
                                            "type-selected"
                                        } else {
                                            ""
                                        };
                                        let class = with_column_focus_class(
                                            diagnostic_class,
                                            column_index,
                                            focus_layout.as_ref(),
                                        );
                                        let first_problem_row = analysis.problems.first().map(|problem| problem.row_index);
                                        let diagnostic_title = column_type_tooltip(analysis);
                                        let type_path = path.clone();
                                        rsx! {
                                            th {
                                                class,
                                                id: "type-col-{column_index}",
                                                key: "{column_index}",
                                                title: "{diagnostic_title}",
                                                onclick: move |_| {
                                                    if let Some(row_index) = first_problem_row {
                                                        let location = CellLocation {
                                                            path: type_path.clone(),
                                                            row_index,
                                                            column_index,
                                                        };
                                                        selected_cell.set(Some(location));
                                                        diagnostic_target.set(Some(DiagnosticTarget::Cell(GridPosition {
                                                            row_index,
                                                            column_index,
                                                        })));
                                                        scroll_to_target(
                                                            DiagnosticTarget::Cell(GridPosition {
                                                                row_index,
                                                                column_index,
                                                            }),
                                                            header_rows,
                                                        );
                                                    }
                                                },
                                                "{label}"
                                            },
                                        }
                                    }
                                }
                            } else {
                                for column_index in 0..data_column_count {
                                    th {
                                        class: with_column_focus_class("type-loading", column_index, focus_layout.as_ref()),
                                        key: "loading-{column_index}",
                                        {tr(Text::Analyzing)}
                                    }
                                }
                            }
                            if right_spacer_width > 0 {
                                th { class: "focus-side-spacer", "" }
                            }
                        }
                    }
                    tbody {
                        if top_spacer_height > 0.0 {
                            tr { class: "virtual-spacer",
                                td {
                                    colspan: "{rendered_column_count}",
                                    style: "height: {top_spacer_height}px",
                                }
                            }
                        }
                        for (source_row_index, record) in document.records.iter().enumerate().skip(visible_start).take(visible_count) {
                            tr {
                                key: "{source_row_index}",
                                class: if viewport.expanded_row.is_some_and(|row| row.index == source_row_index - header_rows) {
                                    "focus-expanded-row"
                                } else {
                                    ""
                                },
                                style: if viewport.expanded_row.is_some_and(|row| row.index == source_row_index - header_rows) {
                                    format!(
                                        "--focused-row-height: {}px; height: {}px; max-height: {}px",
                                        viewport.expanded_row.unwrap().height,
                                        viewport.expanded_row.unwrap().height,
                                        viewport.expanded_row.unwrap().height,
                                    )
                                } else {
                                    format!("height: {DATA_ROW_HEIGHT}px")
                                },
                                th { class: "row-number", "{source_row_index - header_rows + 1}" }
                                if left_spacer_width > 0 {
                                    td { class: "focus-side-spacer", "" }
                                }
                                for (column_index, value) in record.iter().enumerate() {
                                    {
                                        let location = CellLocation {
                                            path: path.clone(),
                                            row_index: source_row_index,
                                            column_index,
                                        };
                                        let problem_reasons = analyses
                                            .as_deref()
                                            .and_then(|columns| columns.get(column_index))
                                            .and_then(|analysis| {
                                                analysis
                                                    .problems
                                                    .iter()
                                                    .find(|problem| problem.row_index == source_row_index)
                                                    .map(|problem| cell_problem_reasons(&problem.kinds, analysis))
                                            })
                                            .unwrap_or_default();
                                        let has_problem = !problem_reasons.is_empty();
                                        let is_selected = selected_cell.read().as_ref() == Some(&location);
                                        let editing_value = if read_only {
                                            None
                                        } else {
                                            cell_draft
                                                .read()
                                                .as_ref()
                                                .filter(|draft| draft.location == location)
                                                .map(|draft| draft.value.clone())
                                        };
                                        let is_json_cell = json_structure(value).is_some();
                                        let use_multiline_editor = cell_uses_multiline_editor(
                                            is_json_cell,
                                            focused_index,
                                            column_index,
                                        );
                                        let display_value = editable_cell_value(value);
                                        let highlighted_json = if is_json_cell {
                                            editing_value.as_deref().map(syntax_highlight_json)
                                        } else {
                                            None
                                        };
                                        let cell_state_class = match (has_problem, is_selected) {
                                            (true, true) => "cell-error cell-selected",
                                            (true, false) => "cell-error",
                                            (false, true) => "cell-selected",
                                            (false, false) => "",
                                        };
                                        let class = with_column_focus_class(
                                            cell_state_class,
                                            column_index,
                                            focus_layout.as_ref(),
                                        );
                                        let select_location = location.clone();
                                        let input_location = location.clone();
                                        let focus_input_location = location.clone();
                                        let json_input_location = location.clone();
                                        let input_keyboard_path = location.path.clone();
                                        let focus_keyboard_path = location.path.clone();
                                        let json_keyboard_path = location.path.clone();
                                        let draft_value = value.clone();
                                        let promotion_context = preview_edit_context.clone();
                                        let cell_title = cell_problem_tooltip(value, &problem_reasons);
                                        let (json_highlight_id, json_input_id) = json_editor_ids(
                                            &path,
                                            source_row_index,
                                            column_index,
                                        );
                                        let scroll_json_highlight_id = json_highlight_id.clone();
                                        let scroll_json_input_id = json_input_id.clone();
                                        rsx! {
                                            td {
                                                class,
                                                id: "cell-{source_row_index}-{column_index}",
                                                "data-data-row-index": "{source_row_index - header_rows}",
                                                "data-column-index": "{column_index}",
                                                key: "{column_index}",
                                                title: "{cell_title}",
                                                if let Some(editing_value) = editing_value {
                                                    if let Some(highlighted_json) = highlighted_json {
                                                        div { class: "json-editor",
                                                            pre {
                                                                id: "{json_highlight_id}",
                                                                class: "json-highlight",
                                                                aria_hidden: "true",
                                                                dangerous_inner_html: "{highlighted_json}",
                                                            }
                                                            textarea {
                                                                id: "{json_input_id}",
                                                                class: "cell-input json-input",
                                                                aria_label: tr(Text::JsonCellEditor),
                                                                autofocus: true,
                                                                spellcheck: "false",
                                                                value: "{editing_value}",
                                                                onmounted: move |event| async move {
                                                                    let _ = event.set_focus(true).await;
                                                                },
                                                                oninput: move |event| {
                                                                    if let Some(draft) = cell_draft.write().as_mut()
                                                                        && draft.location == json_input_location
                                                                    {
                                                                        draft.value = event.value();
                                                                    }
                                                                },
                                                                onscroll: move |_| sync_json_editor_scroll(
                                                                    &scroll_json_input_id,
                                                                    &scroll_json_highlight_id,
                                                                ),
                                                                onkeydown: move |event| handle_cell_editor_keydown(
                                                                    event,
                                                                    &json_keyboard_path,
                                                                    true,
                                                                    true,
                                                                    tabs,
                                                                    cell_draft,
                                                                    selected_cell,
                                                                    diagnostic_target,
                                                                    notice,
                                                                ),
                                                                onblur: move |_| {
                                                                    commit_cell_draft(tabs, cell_draft, notice);
                                                                },
                                                            }
                                                        }
                                                    } else if use_multiline_editor {
                                                        textarea {
                                                            class: "cell-input focus-textarea",
                                                            autofocus: true,
                                                            spellcheck: "false",
                                                            value: "{editing_value}",
                                                            onmounted: move |event| async move {
                                                                let _ = event.set_focus(true).await;
                                                            },
                                                            oninput: move |event| {
                                                                if let Some(draft) = cell_draft.write().as_mut()
                                                                    && draft.location == focus_input_location
                                                                {
                                                                    draft.value = event.value();
                                                                }
                                                            },
                                                            onkeydown: move |event| handle_cell_editor_keydown(
                                                                event,
                                                                &focus_keyboard_path,
                                                                true,
                                                                false,
                                                                tabs,
                                                                cell_draft,
                                                                selected_cell,
                                                                diagnostic_target,
                                                                notice,
                                                            ),
                                                            onblur: move |_| {
                                                                commit_cell_draft(tabs, cell_draft, notice);
                                                            },
                                                        }
                                                    } else {
                                                        input {
                                                            class: "cell-input",
                                                            autofocus: true,
                                                            value: "{editing_value}",
                                                            onmounted: move |event| async move {
                                                                let _ = event.set_focus(true).await;
                                                            },
                                                            oninput: move |event| {
                                                                if let Some(draft) = cell_draft.write().as_mut()
                                                                    && draft.location == input_location
                                                                {
                                                                    draft.value = event.value();
                                                                }
                                                            },
                                                            onkeydown: move |event| handle_cell_editor_keydown(
                                                                event,
                                                                &input_keyboard_path,
                                                                false,
                                                                false,
                                                                tabs,
                                                                cell_draft,
                                                                selected_cell,
                                                                diagnostic_target,
                                                                notice,
                                                            ),
                                                            onblur: move |_| {
                                                                commit_cell_draft(tabs, cell_draft, notice);
                                                            },
                                                        }
                                                    }
                                                } else {
                                                    button {
                                                        class: "cell-button",
                                                        "data-cell-value": "{value}",
                                                        onclick: move |_| {
                                                            let is_selected = selected_cell.read().as_ref()
                                                                == Some(&select_location);
                                                            match cell_click_action(
                                                                read_only,
                                                                is_selected,
                                                                focused_index,
                                                                select_location.column_index,
                                                            ) {
                                                                CellClickAction::Select => {
                                                                    selected_cell.set(Some(select_location.clone()));
                                                                    diagnostic_target.set(None);
                                                                }
                                                                CellClickAction::SwitchFocus => {
                                                                    focused_column.set(Some(FocusedColumn {
                                                                        path: select_location.path.clone(),
                                                                        column_index: select_location.column_index,
                                                                    }));
                                                                    selected_cell.set(Some(select_location.clone()));
                                                                    diagnostic_target.set(None);
                                                                }
                                                                CellClickAction::Edit => {
                                                                    if read_only {
                                                                        if let Some(context) = promotion_context.clone() {
                                                                            promote_preview_for_edit(
                                                                                EditIntent::BeginDraft,
                                                                                context,
                                                                            );
                                                                        }
                                                                    } else {
                                                                        cell_draft.set(Some(CellDraft {
                                                                            location: select_location.clone(),
                                                                            value: editable_cell_value(&draft_value),
                                                                        }));
                                                                    }
                                                                }
                                                            }
                                                        },
                                                        "{display_value}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if right_spacer_width > 0 {
                                    td { class: "focus-side-spacer", "" }
                                }
                            }
                        }
                        if bottom_spacer_height > 0.0 {
                            tr { class: "virtual-spacer",
                                td {
                                    colspan: "{rendered_column_count}",
                                    style: "height: {bottom_spacer_height}px",
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn request_document_view(
    path: &PathBuf,
    view: DocumentView,
    mut tabs: Signal<Vec<DocumentSession>>,
    cell_draft: Signal<Option<CellDraft>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut focused_column: Signal<Option<FocusedColumn>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut table_viewports: Signal<HashMap<PathBuf, TableViewport>>,
    mut context: PreferenceContext,
    mut notice: Signal<Option<String>>,
) {
    if cell_draft
        .read()
        .as_ref()
        .is_some_and(|draft| &draft.location.path == path)
        && !commit_cell_draft(tabs, cell_draft, notice)
    {
        return;
    }

    selected_cell.set(None);
    focused_column.set(None);
    diagnostic_target.set(None);
    table_viewports.write().remove(path);

    if view == DocumentView::Text {
        if let Some(tab) = tabs
            .write()
            .iter_mut()
            .find(|tab| &tab.document.path == path)
        {
            tab.show_text();
        }
        notice.set(None);
        return;
    }

    let request = tabs
        .read()
        .iter()
        .find(|tab| &tab.document.path == path)
        .map(|tab| (tab.text_bytes(), tab.text_hash(), tab.delimiter_override()));
    let Some((bytes, source_hash, delimiter)) = request else {
        notice.set(Some(tr(Text::OpenTabMissing).to_owned()));
        return;
    };

    let parse_path = path.clone();
    notice.set(Some(l10n(Message::Parsing(&file_name(path)))));
    spawn(async move {
        let worker_path = parse_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            CsvDocument::from_bytes(&worker_path, &bytes, delimiter)
        })
        .await;

        let mut tabs_write = tabs.write();
        let Some(tab) = tabs_write
            .iter_mut()
            .find(|tab| tab.document.path == parse_path)
        else {
            return;
        };
        if tab.text_hash() != source_hash {
            notice.set(Some(tr(Text::TextChangedParsing).to_owned()));
            return;
        }

        match result {
            Ok(Ok(document)) => {
                if !tab.show_parsed_table(document) {
                    notice.set(Some(tr(Text::TextChangedParsing).to_owned()));
                    return;
                }
                let stats = stats_for_document(&tab.document, tab.header_rows);
                context.file_stats.write().insert(parse_path.clone(), stats);
                notice.set(None);
            }
            Ok(Err(error)) => {
                let count = error.parse_error_count().unwrap_or(1);
                tab.reject_parsed_text(error.to_string(), count);
                notice.set(Some(l10n(Message::Technical {
                    prefix: Text::OpenError,
                    detail: &error.to_string(),
                })));
            }
            Err(error) => {
                tab.reject_parsed_text(error.to_string(), 1);
                notice.set(Some(l10n(Message::Technical {
                    prefix: Text::OpenError,
                    detail: &error.to_string(),
                })));
            }
        }
    });
}

fn physical_line_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

fn physical_line_numbers(line_count: usize) -> String {
    let mut output = String::new();
    for line in 1..=line_count {
        if line > 1 {
            output.push('\n');
        }
        output.push_str(&line.to_string());
    }
    output
}

fn text_editor_id(path: &Path) -> String {
    let digest = blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    format!("text-editor-{}", &digest[..12])
}

fn json_editor_ids(path: &Path, row_index: usize, column_index: usize) -> (String, String) {
    let digest = blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let base = format!("json-cell-{}-{row_index}-{column_index}", &digest[..12]);
    (format!("{base}-highlight"), format!("{base}-input"))
}

fn json_scroll_sync_script(input_id: &str, highlight_id: &str) -> String {
    format!(
        r#"
        const input = document.getElementById('{input_id}');
        const highlight = document.getElementById('{highlight_id}');
        if (input && highlight) {{
            highlight.scrollTop = input.scrollTop;
            highlight.scrollLeft = input.scrollLeft;
        }}
        "#,
    )
}

fn sync_json_editor_scroll(input_id: &str, highlight_id: &str) {
    let _ = document::eval(&json_scroll_sync_script(input_id, highlight_id));
}

fn sync_text_editor_scroll(editor_id: &str, gutter_id: &str) {
    let script = format!(
        r#"
        const editor = document.getElementById('{editor_id}');
        const gutter = document.getElementById('{gutter_id}');
        if (editor && gutter) gutter.scrollTop = editor.scrollTop;
        "#,
    );
    let _ = document::eval(&script);
}

#[allow(clippy::too_many_arguments)]
fn change_header_rows(
    path: &PathBuf,
    header_rows: usize,
    read_only: bool,
    mut tabs: Signal<Vec<DocumentSession>>,
    cell_draft: Signal<Option<CellDraft>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut table_viewports: Signal<HashMap<PathBuf, TableViewport>>,
    context: PreferenceContext,
    mut notice: Signal<Option<String>>,
) {
    if !(MIN_HEADER_ROWS..=MAX_HEADER_ROWS).contains(&header_rows) {
        notice.set(Some(l10n(Message::HeaderRange {
            minimum: MIN_HEADER_ROWS,
            maximum: MAX_HEADER_ROWS,
        })));
        return;
    }
    if !read_only
        && cell_draft
            .read()
            .as_ref()
            .is_some_and(|draft| &draft.location.path == path)
        && !commit_cell_draft(tabs, cell_draft, notice)
    {
        return;
    }

    let result = if read_only {
        let mut preview = context.preview;
        let mut preview_write = preview.write();
        match &mut *preview_write {
            Preview::Document {
                document,
                header_rows: current_header_rows,
            } if &document.path == path => {
                *current_header_rows = header_rows;
                Ok(stats_for_document(document, header_rows))
            }
            _ => Err(tr(Text::PreviewClosed).to_owned()),
        }
    } else {
        let mut tabs_write = tabs.write();
        match tabs_write.iter_mut().find(|tab| &tab.document.path == path) {
            Some(tab) => tab
                .set_header_rows(header_rows)
                .map(|()| stats_for_document(&tab.document, header_rows))
                .map_err(|error| error.to_string()),
            None => Err(tr(Text::OpenTabMissing).to_owned()),
        }
    };

    match result {
        Ok(stats) => {
            selected_cell.set(None);
            diagnostic_target.set(None);
            table_viewports.write().remove(path);
            let mut file_stats = context.file_stats;
            file_stats.write().insert(path.clone(), stats);
            let Some(mut preferences) = preferences_for_path(&context, path, notice) else {
                return;
            };
            preferences.header_rows = header_rows;
            persist_file_preferences(&context, path, preferences, notice);
        }
        Err(error) => notice.set(Some(l10n(Message::Technical {
            prefix: Text::SettingsError,
            detail: &error,
        }))),
    }
}

#[allow(clippy::too_many_arguments)]
fn change_delimiter(
    path: &PathBuf,
    delimiter: CsvDelimiter,
    read_only: bool,
    mut tabs: Signal<Vec<DocumentSession>>,
    cell_draft: Signal<Option<CellDraft>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut table_viewports: Signal<HashMap<PathBuf, TableViewport>>,
    context: PreferenceContext,
    mut notice: Signal<Option<String>>,
) {
    if !read_only
        && cell_draft
            .read()
            .as_ref()
            .is_some_and(|draft| &draft.location.path == path)
        && !commit_cell_draft(tabs, cell_draft, notice)
    {
        return;
    }

    let result = if read_only {
        let mut preview = context.preview;
        let mut preview_write = preview.write();
        match &mut *preview_write {
            Preview::Document {
                document,
                header_rows,
            } if &document.path == path => {
                let bytes = document.to_bytes();
                CsvDocument::from_bytes(path, &bytes, Some(delimiter.byte()))
                    .map(|reparsed| {
                        *document = reparsed;
                        (*header_rows, stats_for_document(document, *header_rows))
                    })
                    .map_err(|error| error.to_string())
            }
            _ => Err(tr(Text::PreviewClosed).to_owned()),
        }
    } else {
        let mut tabs_write = tabs.write();
        match tabs_write.iter_mut().find(|tab| &tab.document.path == path) {
            Some(tab) => tab
                .set_delimiter(delimiter.byte())
                .map(|()| {
                    (
                        tab.header_rows,
                        stats_for_document(&tab.document, tab.header_rows),
                    )
                })
                .map_err(|error| error.to_string()),
            None => Err(tr(Text::OpenTabMissing).to_owned()),
        }
    };

    match result {
        Ok((header_rows, stats)) => {
            selected_cell.set(None);
            diagnostic_target.set(None);
            table_viewports.write().remove(path);
            let mut file_stats = context.file_stats;
            file_stats.write().insert(path.clone(), stats);
            let Some(mut preferences) = preferences_for_path(&context, path, notice) else {
                return;
            };
            preferences.header_rows = header_rows;
            preferences.delimiter = Some(delimiter);
            persist_file_preferences(&context, path, preferences, notice);
        }
        Err(error) => notice.set(Some(l10n(Message::Technical {
            prefix: Text::SettingsError,
            detail: &error,
        }))),
    }
}

fn stats_for_document(document: &CsvDocument, header_rows: usize) -> CsvFileStats {
    match document.dimensions(header_rows) {
        Some((data_rows, columns)) => CsvFileStats::Ready { data_rows, columns },
        None => CsvFileStats::Error {
            message: l10n(Message::InvalidHeaderRecords(header_rows)),
        },
    }
}

fn preferences_for_path(
    context: &PreferenceContext,
    path: &Path,
    mut notice: Signal<Option<String>>,
) -> Option<FilePreferences> {
    let Some(workspace) = context.workspace.as_deref() else {
        notice.set(Some(tr(Text::NoWorkspaceOpen).to_owned()));
        return None;
    };
    let Ok(relative_path) = path.strip_prefix(workspace) else {
        notice.set(Some(tr(Text::OutsideWorkspace).to_owned()));
        return None;
    };
    Some(
        context
            .settings
            .read()
            .file_preferences(workspace, relative_path),
    )
}

fn persist_file_preferences(
    context: &PreferenceContext,
    path: &Path,
    preferences: FilePreferences,
    mut notice: Signal<Option<String>>,
) {
    let Some(workspace) = context.workspace.as_deref() else {
        notice.set(Some(tr(Text::NoWorkspaceOpen).to_owned()));
        return;
    };
    let Ok(relative_path) = path.strip_prefix(workspace) else {
        notice.set(Some(tr(Text::OutsideWorkspace).to_owned()));
        return;
    };

    let mut settings = context.settings;
    settings
        .write()
        .set_file_preferences(workspace, relative_path, preferences);
    if let Some(store) = context.settings_store.as_ref()
        && let Err(error) = store.save_file_preferences(workspace, relative_path, preferences)
    {
        notice.set(Some(error.to_string()));
        return;
    }
    notice.set(None);
}

fn commit_cell_draft(
    mut tabs: Signal<Vec<DocumentSession>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    mut notice: Signal<Option<String>>,
) -> bool {
    let Some(draft) = cell_draft.read().clone() else {
        return false;
    };
    let result = tabs
        .write()
        .iter_mut()
        .find(|tab| tab.document.path == draft.location.path)
        .ok_or_else(|| tr(Text::OpenTabMissing).to_owned())
        .and_then(|tab| {
            let original = tab
                .document
                .records
                .get(draft.location.row_index)
                .and_then(|row| row.get(draft.location.column_index))
                .ok_or_else(|| tr(Text::EditedCellMissing).to_owned())?;
            let value = normalize_cell_edit(original, &draft.value)?;
            tab.edit_cell(draft.location.row_index, draft.location.column_index, value)
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(_) => {
            cell_draft.set(None);
            notice.set(None);
            true
        }
        Err(error) => {
            notice.set(Some(l10n(Message::Technical {
                prefix: Text::SaveError,
                detail: &error,
            })));
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paste_selected_cell(
    value: String,
    mut tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut notice: Signal<Option<String>>,
) {
    if cell_draft.read().is_some() {
        return;
    }
    let Some(active_path) = active_tab.read().clone() else {
        return;
    };
    let Some(location) = selected_cell
        .read()
        .clone()
        .filter(|location| location.path == active_path)
    else {
        notice.set(Some(tr(Text::SelectCellBeforePaste).to_owned()));
        return;
    };

    let result = tabs
        .write()
        .iter_mut()
        .find(|tab| tab.document.path == active_path)
        .ok_or_else(|| tr(Text::OpenTabMissing).to_owned())
        .and_then(|tab| {
            tab.edit_cell(location.row_index, location.column_index, value)
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(true) => {
            diagnostic_target.set(None);
            notice.set(Some(tr(Text::PastedCell).to_owned()));
        }
        Ok(false) => notice.set(Some(tr(Text::ClipboardSame).to_owned())),
        Err(error) => notice.set(Some(l10n(Message::Technical {
            prefix: Text::SaveError,
            detail: &error,
        }))),
    }
}

#[allow(clippy::too_many_arguments)]
fn close_active_or_preview(
    tabs: Signal<Vec<DocumentSession>>,
    mut active_tab: Signal<Option<PathBuf>>,
    mut preview: Signal<Preview>,
    mut preview_return_tab: Signal<Option<PathBuf>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    notice: Signal<Option<String>>,
) {
    let active_path = active_tab.read().clone();
    let has_preview = !matches!(&*preview.read(), Preview::Empty);
    if let Some(path) = active_path {
        request_close_tab(
            path,
            tabs,
            active_tab,
            selected_cell,
            cell_draft,
            diagnostic_target,
            table_analyses,
            notice,
        );
    } else if has_preview {
        preview.set(Preview::Empty);
        let return_path = preview_return_tab.read().clone();
        active_tab.set(return_path);
        preview_return_tab.set(None);
        selected_cell.set(None);
        diagnostic_target.set(None);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_window_shortcut(
    command: WindowShortcutCommand,
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    mut preview: Signal<Preview>,
    mut preview_return_tab: Signal<Option<PathBuf>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    focused_column: Signal<Option<FocusedColumn>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    mut table_viewports: Signal<HashMap<PathBuf, TableViewport>>,
    mut text_cursor: Signal<Option<TextCursorPosition>>,
    mut sidebar_visible: Signal<bool>,
    mut overlay_panel: Signal<Option<OverlayPanel>>,
    notice: Signal<Option<String>>,
    mut shortcut_close_in_progress: Signal<bool>,
) {
    match command {
        WindowShortcutCommand::CommandPalette => {
            overlay_panel.set(Some(OverlayPanel::CommandPalette));
        }
        WindowShortcutCommand::GoToLine => {
            overlay_panel.set(Some(OverlayPanel::GoToLine));
        }
        WindowShortcutCommand::CurrentSearch => {
            overlay_panel.set(Some(OverlayPanel::CurrentSearch));
        }
        WindowShortcutCommand::GlobalSearch => {
            overlay_panel.set(Some(OverlayPanel::GlobalSearch));
        }
        WindowShortcutCommand::Shortcuts => {
            overlay_panel.set(Some(OverlayPanel::Shortcuts));
        }
        WindowShortcutCommand::Save => {
            if let Some(path) = active_tab.read().clone() {
                attempt_save_tab(
                    &path,
                    tabs,
                    selected_cell,
                    cell_draft,
                    diagnostic_target,
                    table_analyses,
                    notice,
                );
            }
        }
        WindowShortcutCommand::Close => {
            shortcut_close_in_progress.set(true);
            close_active_or_preview(
                tabs,
                active_tab,
                preview,
                preview_return_tab,
                selected_cell,
                cell_draft,
                diagnostic_target,
                table_analyses,
                notice,
            );
        }
        WindowShortcutCommand::CloseReleased => shortcut_close_in_progress.set(false),
        WindowShortcutCommand::NextTab | WindowShortcutCommand::PreviousTab => {
            preview.set(Preview::Empty);
            preview_return_tab.set(None);
            cycle_active_tab(
                tabs,
                active_tab,
                matches!(command, WindowShortcutCommand::PreviousTab),
            );
            selected_cell.set(None);
            cell_draft.set(None);
            diagnostic_target.set(None);
        }
        WindowShortcutCommand::ToggleSidebar => {
            let visible = *sidebar_visible.read();
            sidebar_visible.set(!visible);
        }
        WindowShortcutCommand::Undo => {
            run_history_action(false, tabs, active_tab, diagnostic_target, notice)
        }
        WindowShortcutCommand::Redo => {
            run_history_action(true, tabs, active_tab, diagnostic_target, notice)
        }
        WindowShortcutCommand::NextDiagnostic | WindowShortcutCommand::PreviousDiagnostic => {
            navigate_diagnostic(
                tabs,
                active_tab,
                selected_cell,
                diagnostic_target,
                table_analyses,
                notice,
                matches!(command, WindowShortcutCommand::PreviousDiagnostic),
            );
        }
        WindowShortcutCommand::Copied => {
            let value_label = selected_cell
                .read()
                .as_ref()
                .and_then(|location| {
                    tabs.read()
                        .iter()
                        .find(|tab| tab.document.path == location.path)
                        .and_then(|tab| tab.document.records.get(location.row_index))
                        .and_then(|row| row.get(location.column_index))
                        .map(|_| tr(Text::CopiedCell))
                })
                .unwrap_or_else(|| tr(Text::NothingSelectedCopy));
            let mut notice = notice;
            notice.set(Some(value_label.to_owned()));
        }
        WindowShortcutCommand::Paste(value) => {
            let context = PreviewEditContext {
                tabs,
                active_tab,
                preview,
                preview_return_tab,
                selected_cell,
                cell_draft,
                focused_column,
                diagnostic_target,
                notice,
            };
            if active_tab.read().is_none()
                && promote_preview_for_edit(EditIntent::Paste(value.clone()), context)
            {
                return;
            }
            paste_selected_cell(
                value,
                tabs,
                active_tab,
                selected_cell,
                cell_draft,
                diagnostic_target,
                notice,
            );
        }
        WindowShortcutCommand::TextCursor(position) => text_cursor.set(Some(position)),
        WindowShortcutCommand::TableViewport(size) => {
            let mut viewports = table_viewports.write();
            let viewport = viewports.entry(size.path).or_default();
            if size.width.is_finite() && size.width > 0.0 {
                viewport.width = size.width;
            }
            if size.height.is_finite() && size.height > 0.0 {
                viewport.height = size.height;
                if let Some(expanded) = viewport.expanded_row {
                    viewport.expanded_row = Some(ExpandedRow::measured(
                        expanded.index,
                        expanded.column_index,
                        expanded.height,
                        size.height,
                    ));
                }
            }
        }
        WindowShortcutCommand::FocusedRowHeight(measurement) => {
            let mut viewports = table_viewports.write();
            let viewport = viewports.entry(measurement.path).or_default();
            let measured = ExpandedRow::measured(
                measurement.row_index,
                measurement.column_index,
                measurement.content_height,
                viewport.height,
            );
            if viewport.expanded_row != Some(measured) {
                viewport.expanded_row = Some(measured);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_app_keydown(
    event: KeyboardEvent,
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    mut preview: Signal<Preview>,
    mut preview_return_tab: Signal<Option<PathBuf>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    mut sidebar_visible: Signal<bool>,
    mut overlay_panel: Signal<Option<OverlayPanel>>,
    notice: Signal<Option<String>>,
    mut shortcut_close_in_progress: Signal<bool>,
    preview_edit_context: PreviewEditContext,
) {
    let key = event.key();
    let modifiers = event.modifiers();
    let primary = primary_modifier(modifiers);
    let control = modifiers.contains(Modifiers::CONTROL);
    let shift = modifiers.contains(Modifiers::SHIFT);

    if control && key == Key::Tab {
        event.prevent_default();
        preview.set(Preview::Empty);
        preview_return_tab.set(None);
        cycle_active_tab(tabs, active_tab, shift);
        selected_cell.set(None);
        cell_draft.set(None);
        diagnostic_target.set(None);
        return;
    }

    if primary && key_character_is(&key, "b") {
        event.prevent_default();
        let visible = *sidebar_visible.read();
        sidebar_visible.set(!visible);
        return;
    }

    if primary && key_character_is(&key, "s") {
        event.prevent_default();
        if let Some(path) = active_tab.read().clone() {
            attempt_save_tab(
                &path,
                tabs,
                selected_cell,
                cell_draft,
                diagnostic_target,
                table_analyses,
                notice,
            );
        }
        return;
    }

    if primary && key_character_is(&key, "w") {
        event.prevent_default();
        shortcut_close_in_progress.set(true);
        close_active_or_preview(
            tabs,
            active_tab,
            preview,
            preview_return_tab,
            selected_cell,
            cell_draft,
            diagnostic_target,
            table_analyses,
            notice,
        );
        return;
    }

    let editing = cell_draft.read().is_some();
    if primary && key_character_is(&key, "z") {
        if editing {
            return;
        }
        event.prevent_default();
        if redo_uses_shift_z(modifiers) {
            run_history_action(true, tabs, active_tab, diagnostic_target, notice);
        } else {
            run_history_action(false, tabs, active_tab, diagnostic_target, notice);
        }
        return;
    }

    if primary && key_character_is(&key, "y") && !cfg!(target_os = "macos") {
        if editing {
            return;
        }
        event.prevent_default();
        run_history_action(true, tabs, active_tab, diagnostic_target, notice);
        return;
    }

    if key == Key::Escape {
        if overlay_panel.read().is_some() {
            event.prevent_default();
            overlay_panel.set(None);
        } else if editing {
            event.prevent_default();
            cell_draft.set(None);
        } else {
            diagnostic_target.set(None);
        }
        return;
    }

    if editing || primary || control {
        return;
    }

    if key == Key::F8 {
        event.prevent_default();
        navigate_diagnostic(
            tabs,
            active_tab,
            selected_cell,
            diagnostic_target,
            table_analyses,
            notice,
            shift,
        );
        return;
    }

    if key == Key::Enter || key == Key::F2 {
        event.prevent_default();
        if active_tab.read().is_none()
            && promote_preview_for_edit(EditIntent::BeginDraft, preview_edit_context)
        {
            return;
        }
        start_selected_cell_edit(tabs, active_tab, selected_cell, cell_draft);
        return;
    }

    let movement = match key {
        Key::ArrowUp => Some(GridMovement::Up),
        Key::ArrowDown => Some(GridMovement::Down),
        Key::ArrowLeft => Some(GridMovement::Left),
        Key::ArrowRight => Some(GridMovement::Right),
        _ => None,
    };
    if let Some(movement) = movement
        && let Some(path) = active_tab.read().clone()
    {
        event.prevent_default();
        move_selected_cell(&path, movement, tabs, selected_cell, diagnostic_target);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_table_mode_keydown(
    event: KeyboardEvent,
    path: &PathBuf,
    column_count: usize,
    header_rows: usize,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    mut focused_column: Signal<Option<FocusedColumn>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
) {
    if cell_draft.read().is_some() {
        return;
    }
    let key = event.key();
    let modifiers = event.modifiers();
    let current_focus = focused_column
        .read()
        .clone()
        .filter(|focused| &focused.path == path);

    if key == Key::Escape && current_focus.is_some() {
        event.prevent_default();
        event.stop_propagation();
        focused_column.set(None);
        return;
    }

    if modifiers.is_empty() && key_character_is(&key, "t") {
        let selected = selected_cell
            .read()
            .clone()
            .filter(|location| &location.path == path);
        if let Some(selected) = selected {
            event.prevent_default();
            event.stop_propagation();
            if current_focus.is_some() {
                focused_column.set(None);
            } else {
                focused_column.set(Some(FocusedColumn {
                    path: path.clone(),
                    column_index: selected.column_index,
                }));
            }
        }
        return;
    }

    let direction =
        if modifiers.is_empty() && (key == Key::ArrowLeft || key_character_is(&key, "a")) {
            Some(-1)
        } else if modifiers.is_empty() && (key == Key::ArrowRight || key_character_is(&key, "d")) {
            Some(1)
        } else {
            None
        };
    if current_focus.is_some()
        && let Some(direction) = direction
    {
        event.prevent_default();
        event.stop_propagation();
        move_focused_column(
            path,
            direction,
            column_count,
            header_rows,
            selected_cell,
            focused_column,
            diagnostic_target,
        );
    }
}

fn move_focused_column(
    path: &PathBuf,
    direction: isize,
    column_count: usize,
    header_rows: usize,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut focused_column: Signal<Option<FocusedColumn>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
) {
    if column_count == 0 {
        return;
    }
    let current_column = focused_column
        .read()
        .as_ref()
        .filter(|focused| &focused.path == path)
        .map(|focused| focused.column_index)
        .or_else(|| {
            selected_cell
                .read()
                .as_ref()
                .filter(|location| &location.path == path)
                .map(|location| location.column_index)
        })
        .unwrap_or(0);
    let next_column = next_focused_column(current_column, direction, column_count);
    let row_index = selected_cell
        .read()
        .as_ref()
        .filter(|location| &location.path == path)
        .map(|location| location.row_index)
        .unwrap_or(header_rows);

    focused_column.set(Some(FocusedColumn {
        path: path.clone(),
        column_index: next_column,
    }));
    selected_cell.set(Some(CellLocation {
        path: path.clone(),
        row_index,
        column_index: next_column,
    }));
    diagnostic_target.set(None);
    scroll_to_target(
        DiagnosticTarget::Cell(GridPosition {
            row_index,
            column_index: next_column,
        }),
        header_rows,
    );
}

fn next_focused_column(current: usize, direction: isize, column_count: usize) -> usize {
    if column_count == 0 {
        return 0;
    }
    current
        .saturating_add_signed(direction)
        .min(column_count - 1)
}

fn focus_column_width(max_content_chars: usize) -> usize {
    max_content_chars
        .saturating_mul(7)
        .saturating_add(36)
        .clamp(320, 720)
}

fn cell_uses_multiline_editor(
    is_json: bool,
    focused_column: Option<usize>,
    column_index: usize,
) -> bool {
    is_json || focused_column == Some(column_index)
}

fn cell_click_action(
    read_only: bool,
    is_selected: bool,
    focused_column: Option<usize>,
    clicked_column: usize,
) -> CellClickAction {
    if focused_column.is_some_and(|focused| focused != clicked_column) {
        CellClickAction::SwitchFocus
    } else if is_selected && (!read_only || focused_column == Some(clicked_column)) {
        CellClickAction::Edit
    } else {
        CellClickAction::Select
    }
}

fn localized_delimiter_label(delimiter: CsvDelimiter) -> &'static str {
    tr(match delimiter {
        CsvDelimiter::Comma => Text::Comma,
        CsvDelimiter::Tab => Text::Tab,
        CsvDelimiter::Semicolon => Text::Semicolon,
        CsvDelimiter::Pipe => Text::Pipe,
    })
}

fn resized_dimension(start_width: usize, delta: f64, minimum: usize, maximum: usize) -> usize {
    let delta = if delta.is_finite() { delta } else { 0.0 };
    (start_width as f64 + delta)
        .round()
        .clamp(minimum as f64, maximum as f64) as usize
}

fn column_focus_class(column_index: usize, layout: Option<&FocusLayout>) -> &'static str {
    match layout.map(|layout| layout.column_role(column_index)) {
        None => "",
        Some(FocusColumnRole::LeftNeighbor) => "focus-left-neighbor",
        Some(FocusColumnRole::Focused) => "focus-column",
        Some(FocusColumnRole::RightNeighbor) => "focus-right-neighbor",
        Some(FocusColumnRole::Hidden) => "column-hidden",
    }
}

fn with_column_focus_class(
    base_class: &str,
    column_index: usize,
    layout: Option<&FocusLayout>,
) -> String {
    let focus_class = column_focus_class(column_index, layout);
    match (base_class.is_empty(), focus_class.is_empty()) {
        (true, true) => String::new(),
        (false, true) => base_class.to_owned(),
        (true, false) => focus_class.to_owned(),
        (false, false) => format!("{base_class} {focus_class}"),
    }
}

fn focused_column_field(
    document: &CsvDocument,
    header_rows: usize,
    column_index: usize,
) -> Option<&str> {
    document
        .records
        .get(header_rows.checked_sub(1)?)?
        .get(column_index)
        .map(|field| field.trim())
        .filter(|field| !field.is_empty())
}

fn json_structure(value: &str) -> Option<JsonStructure> {
    match serde_json::from_str::<serde_json::Value>(value).ok()? {
        serde_json::Value::Object(_) => Some(JsonStructure::Object),
        serde_json::Value::Array(items)
            if !items.is_empty() && items.iter().all(serde_json::Value::is_array) =>
        {
            Some(JsonStructure::Array2d)
        }
        serde_json::Value::Array(_) => Some(JsonStructure::Array),
        _ => None,
    }
}

fn editable_cell_value(value: &str) -> String {
    if json_structure(value).is_none() {
        return value.to_owned();
    }
    serde_json::from_str::<serde_json::Value>(value)
        .and_then(|json| serde_json::to_string_pretty(&json))
        .unwrap_or_else(|_| value.to_owned())
}

fn normalize_cell_edit(original: &str, draft: &str) -> Result<String, String> {
    let Some(expected) = json_structure(original) else {
        return Ok(draft.to_owned());
    };
    let parsed = serde_json::from_str::<serde_json::Value>(draft).map_err(|error| {
        l10n(Message::Technical {
            prefix: Text::JsonSyntaxError,
            detail: &error.to_string(),
        })
    })?;
    let actual = match &parsed {
        serde_json::Value::Object(_) => Some(JsonStructure::Object),
        serde_json::Value::Array(items)
            if !items.is_empty() && items.iter().all(serde_json::Value::is_array) =>
        {
            Some(JsonStructure::Array2d)
        }
        serde_json::Value::Array(_) => Some(JsonStructure::Array),
        _ => None,
    };
    if actual != Some(expected) {
        return Err(format!(
            "{} {}",
            tr(Text::JsonValueMustRemain),
            json_structure_label(expected)
        ));
    }
    serde_json::to_string(&parsed).map_err(|error| error.to_string())
}

fn json_structure_label(structure: JsonStructure) -> &'static str {
    tr(match structure {
        JsonStructure::Object => Text::JsonObject,
        JsonStructure::Array => Text::JsonArray,
        JsonStructure::Array2d => Text::JsonArray2d,
    })
}

fn syntax_highlight_json(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len() + value.len() / 2);
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == '"' {
            let start = index;
            index += 1;
            while index < chars.len() {
                match chars[index] {
                    '\\' => index = (index + 2).min(chars.len()),
                    '"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            let mut lookahead = index;
            while chars
                .get(lookahead)
                .is_some_and(|character| character.is_whitespace())
            {
                lookahead += 1;
            }
            let class = if chars.get(lookahead) == Some(&':') {
                "json-key"
            } else {
                "json-string"
            };
            push_highlighted_token(&mut output, class, &chars[start..index]);
            continue;
        }
        if current == '-' || current.is_ascii_digit() {
            let start = index;
            index += 1;
            while chars.get(index).is_some_and(|character| {
                character.is_ascii_digit() || matches!(character, '.' | 'e' | 'E' | '+' | '-')
            }) {
                index += 1;
            }
            push_highlighted_token(&mut output, "json-number", &chars[start..index]);
            continue;
        }
        if let Some(keyword) = ["true", "false", "null"]
            .into_iter()
            .find(|keyword| chars_start_with(&chars[index..], keyword))
        {
            let end = index + keyword.len();
            push_highlighted_token(&mut output, "json-literal", &chars[index..end]);
            index = end;
            continue;
        }
        if matches!(current, '{' | '}' | '[' | ']' | ':' | ',') {
            push_highlighted_token(&mut output, "json-punctuation", &chars[index..=index]);
        } else {
            push_html_escaped(&mut output, current);
        }
        index += 1;
    }
    output
}

fn chars_start_with(chars: &[char], prefix: &str) -> bool {
    let mut prefix = prefix.chars();
    for character in chars {
        let Some(expected) = prefix.next() else {
            return true;
        };
        if *character != expected {
            return false;
        }
    }
    prefix.next().is_none()
}

fn push_highlighted_token(output: &mut String, class: &str, token: &[char]) {
    output.push_str("<span class=\"");
    output.push_str(class);
    output.push_str("\">");
    for character in token {
        push_html_escaped(output, *character);
    }
    output.push_str("</span>");
}

fn push_html_escaped(output: &mut String, character: char) {
    match character {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        '"' => output.push_str("&quot;"),
        '\'' => output.push_str("&#39;"),
        _ => output.push(character),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_cell_editor_keydown(
    event: KeyboardEvent,
    path: &PathBuf,
    multiline: bool,
    indent_with_tab: bool,
    tabs: Signal<Vec<DocumentSession>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    selected_cell: Signal<Option<CellLocation>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    notice: Signal<Option<String>>,
) {
    match event.key() {
        Key::Enter if multiline && primary_modifier(event.modifiers()) => {
            event.prevent_default();
            event.stop_propagation();
            insert_editor_text("\n");
        }
        Key::Enter => {
            event.prevent_default();
            event.stop_propagation();
            if commit_cell_draft(tabs, cell_draft, notice) {
                move_selected_cell(
                    path,
                    GridMovement::Down,
                    tabs,
                    selected_cell,
                    diagnostic_target,
                );
            }
        }
        Key::Tab if indent_with_tab => {
            event.prevent_default();
            event.stop_propagation();
            insert_editor_text("  ");
        }
        Key::Tab => {
            event.prevent_default();
            event.stop_propagation();
            let movement = if event.modifiers().contains(Modifiers::SHIFT) {
                GridMovement::Left
            } else {
                GridMovement::Right
            };
            if commit_cell_draft(tabs, cell_draft, notice) {
                move_selected_cell(path, movement, tabs, selected_cell, diagnostic_target);
            }
        }
        Key::Escape => {
            event.stop_propagation();
            cell_draft.set(None);
        }
        _ => {}
    }
}

fn insert_editor_text(text: &str) {
    let _ = document::eval(&editor_insert_script(text));
}

fn editor_insert_script(text: &str) -> String {
    let encoded = serde_json::to_string(text).expect("editor text is serializable");
    format!(
        r#"
        const editor = document.activeElement;
        if (editor instanceof HTMLTextAreaElement) {{
            const start = editor.selectionStart;
            const end = editor.selectionEnd;
            editor.setRangeText({encoded}, start, end, "end");
            editor.dispatchEvent(new Event("input", {{bubbles: true}}));
        }}
        "#,
    )
}

fn primary_modifier(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.contains(Modifiers::META)
    } else {
        modifiers.contains(Modifiers::CONTROL)
    }
}

fn redo_uses_shift_z(modifiers: Modifiers) -> bool {
    cfg!(target_os = "macos") && modifiers.contains(Modifiers::SHIFT)
}

fn key_character_is(key: &Key, expected: &str) -> bool {
    matches!(key, Key::Character(value) if value.eq_ignore_ascii_case(expected))
}

fn cycle_active_tab(
    tabs: Signal<Vec<DocumentSession>>,
    mut active_tab: Signal<Option<PathBuf>>,
    backwards: bool,
) {
    let tabs = tabs.read();
    if tabs.is_empty() {
        active_tab.set(None);
        return;
    }
    let current_index = active_tab
        .read()
        .as_ref()
        .and_then(|path| tabs.iter().position(|tab| &tab.document.path == path));
    let next_index = match (current_index, backwards) {
        (Some(0), true) | (None, true) => tabs.len() - 1,
        (Some(index), true) => index - 1,
        (Some(index), false) => (index + 1) % tabs.len(),
        (None, false) => 0,
    };
    active_tab.set(Some(tabs[next_index].document.path.clone()));
}

fn run_history_action(
    redo: bool,
    mut tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut notice: Signal<Option<String>>,
) {
    let Some(path) = active_tab.read().clone() else {
        return;
    };
    let result = tabs
        .write()
        .iter_mut()
        .find(|tab| tab.document.path == path)
        .ok_or_else(|| tr(Text::OpenTabMissing).to_owned())
        .and_then(|tab| {
            if redo { tab.redo() } else { tab.undo() }.map_err(|error| error.to_string())
        });
    match result {
        Ok(true) => {
            diagnostic_target.set(None);
            notice.set(Some(if redo {
                tr(Text::RedidEdit).to_owned()
            } else {
                tr(Text::UndidEdit).to_owned()
            }));
        }
        Ok(false) => notice.set(Some(if redo {
            tr(Text::NothingRedo).to_owned()
        } else {
            tr(Text::NothingUndo).to_owned()
        })),
        Err(error) => notice.set(Some(l10n(Message::Technical {
            prefix: Text::SaveError,
            detail: &error,
        }))),
    }
}

fn promote_preview_for_edit(intent: EditIntent, mut context: PreviewEditContext) -> bool {
    let (document, header_rows) = match &*context.preview.read() {
        Preview::Document {
            document,
            header_rows,
        } => (document.clone(), *header_rows),
        _ => return false,
    };
    let path = document.path.clone();
    if !preview_edit_is_eligible(
        &path,
        context.selected_cell.read().as_ref(),
        context.focused_column.read().as_ref(),
    ) {
        return false;
    }

    let already_open = context
        .tabs
        .read()
        .iter()
        .any(|tab| tab.document.path == path);
    if !already_open {
        if context.tabs.read().len() >= 20 {
            context.notice.set(Some(tr(Text::TabLimit).to_owned()));
            return true;
        }
        let convert_gb18030 = if document.encoding == CsvEncoding::Gb18030 {
            if !confirm_gb18030_conversion(&path) {
                return true;
            }
            true
        } else {
            false
        };
        match DocumentSession::from_loaded_document(document, header_rows, convert_gb18030) {
            Ok(session) => context.tabs.write().push(session),
            Err(error) => {
                context.notice.set(Some(l10n(Message::Technical {
                    prefix: Text::OpenError,
                    detail: &error.to_string(),
                })));
                return true;
            }
        }
    }

    context.preview.set(Preview::Empty);
    context.active_tab.set(Some(path));
    context.preview_return_tab.set(None);
    context.diagnostic_target.set(None);
    match intent {
        EditIntent::BeginDraft => start_selected_cell_edit(
            context.tabs,
            context.active_tab,
            context.selected_cell,
            context.cell_draft,
        ),
        EditIntent::Paste(value) => paste_selected_cell(
            value,
            context.tabs,
            context.active_tab,
            context.selected_cell,
            context.cell_draft,
            context.diagnostic_target,
            context.notice,
        ),
    }
    true
}

fn preview_edit_is_eligible(
    path: &Path,
    selected_cell: Option<&CellLocation>,
    focused_column: Option<&FocusedColumn>,
) -> bool {
    selected_cell.is_some_and(|location| location.path == path)
        && focused_column.is_some_and(|focused| focused.path == path)
}

fn start_selected_cell_edit(
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    selected_cell: Signal<Option<CellLocation>>,
    mut cell_draft: Signal<Option<CellDraft>>,
) {
    let Some(path) = active_tab.read().clone() else {
        return;
    };
    let Some(location) = selected_cell
        .read()
        .clone()
        .filter(|location| location.path == path)
    else {
        return;
    };
    let value = tabs.read().iter().find_map(|tab| {
        (tab.document.path == path).then(|| {
            tab.document
                .records
                .get(location.row_index)
                .and_then(|row| row.get(location.column_index))
                .cloned()
        })?
    });
    if let Some(value) = value {
        cell_draft.set(Some(CellDraft {
            location,
            value: editable_cell_value(&value),
        }));
    }
}

fn move_selected_cell(
    path: &PathBuf,
    movement: GridMovement,
    tabs: Signal<Vec<DocumentSession>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
) {
    let tabs = tabs.read();
    let Some(session) = tabs.iter().find(|tab| &tab.document.path == path) else {
        return;
    };
    let document = &session.document;
    let header_rows = session.header_rows;
    let current = selected_cell
        .read()
        .as_ref()
        .filter(|location| &location.path == path)
        .map(|location| GridPosition {
            row_index: location.row_index,
            column_index: location.column_index,
        });
    let column_count = document.records.first().map_or(0, Vec::len);
    let Some(position) = move_in_grid(
        current,
        header_rows,
        document.records.len(),
        column_count,
        movement,
    ) else {
        return;
    };
    selected_cell.set(Some(CellLocation {
        path: path.clone(),
        row_index: position.row_index,
        column_index: position.column_index,
    }));
    diagnostic_target.set(None);
    scroll_to_target(DiagnosticTarget::Cell(position), header_rows);
}

fn navigate_diagnostic(
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    mut notice: Signal<Option<String>>,
    backwards: bool,
) {
    let Some(path) = active_tab.read().clone() else {
        return;
    };
    let tabs_read = tabs.read();
    let Some(session) = tabs_read.iter().find(|tab| tab.document.path == path) else {
        return;
    };
    let header_rows = session.header_rows;
    let analyses = table_analyses.read().get(&path).and_then(|state| {
        state.ready_columns(session.document.analysis_version(), session.header_rows)
    });
    let Some(analyses) = analyses else {
        notice.set(Some(tr(Text::AnalysisRunning).to_owned()));
        return;
    };
    let targets = diagnostic_targets(&analyses);
    let target = cycle_diagnostic(&targets, *diagnostic_target.read(), backwards);
    drop(tabs_read);

    let Some(target) = target else {
        notice.set(Some(tr(Text::NoProblems).to_owned()));
        return;
    };
    diagnostic_target.set(Some(target));
    match target {
        DiagnosticTarget::Cell(position) => {
            selected_cell.set(Some(CellLocation {
                path,
                row_index: position.row_index,
                column_index: position.column_index,
            }));
            notice.set(Some(l10n(Message::ProblemAt {
                row: position.row_index.saturating_sub(header_rows) + 1,
                column: position.column_index + 1,
            })));
        }
        DiagnosticTarget::Column(column_index) => {
            selected_cell.set(None);
            notice.set(Some(l10n(Message::MixedColumn(column_index + 1))));
        }
    }
    scroll_to_target(target, header_rows);
}

fn scroll_to_target(target: DiagnosticTarget, header_rows: usize) {
    let element_id = match target {
        DiagnosticTarget::Cell(position) => {
            format!("cell-{}-{}", position.row_index, position.column_index)
        }
        DiagnosticTarget::Column(column_index) => format!("type-col-{column_index}"),
    };
    let script = format!(
        r#"
        const id = '{element_id}';
        const focusTarget = () => {{
            const element = document.getElementById(id);
            element?.scrollIntoView({{block: 'nearest', inline: 'center'}});
            (element?.querySelector('button') ?? element)?.focus();
            return element !== null;
        }};
        if (!focusTarget() && id.startsWith('cell-')) {{
            const sourceRow = Number.parseInt(id.split('-')[1], 10);
            const scroller = document.querySelector('.table-scroll');
            if (scroller && Number.isFinite(sourceRow)) {{
                const rowHeight = Number.parseFloat(scroller.dataset.rowHeight) || {DATA_ROW_HEIGHT};
                const dataRow = Math.max(0, sourceRow - {header_rows});
                const expandedRow = Number.parseInt(scroller.dataset.expandedRowIndex ?? "", 10);
                const expandedHeight = Number.parseFloat(scroller.dataset.expandedRowHeight ?? "");
                const expandedOffset = Number.isFinite(expandedRow)
                    && Number.isFinite(expandedHeight)
                    && expandedRow < dataRow
                    ? Math.max(0, expandedHeight - rowHeight)
                    : 0;
                scroller.scrollTop = dataRow * rowHeight + expandedOffset;
                window.setTimeout(focusTarget, 60);
            }}
        }}
        window.setTimeout(focusTarget, 80);
        "#,
    );
    let _ = document::eval(&script);
}

fn attempt_save_tab(
    path: &PathBuf,
    mut tabs: Signal<Vec<DocumentSession>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    mut notice: Signal<Option<String>>,
) -> bool {
    if cell_draft
        .read()
        .as_ref()
        .is_some_and(|draft| &draft.location.path == path)
        && !commit_cell_draft(tabs, cell_draft, notice)
    {
        return false;
    }

    let text_parse_issue = {
        let mut tabs_write = tabs.write();
        tabs_write
            .iter_mut()
            .find(|tab| &tab.document.path == path)
            .and_then(|tab| {
                (tab.view() == DocumentView::Text)
                    .then(|| tab.validate_text().err())
                    .flatten()
            })
    };
    let problem_count = if text_parse_issue.is_some() {
        0
    } else {
        let tabs_read = tabs.read();
        let Some(tab) = tabs_read.iter().find(|tab| &tab.document.path == path) else {
            notice.set(Some(tr(Text::OpenTabMissing).to_owned()));
            return false;
        };
        let analyses = table_analyses.read().get(path).and_then(|state| {
            state.ready_columns(tab.document.analysis_version(), tab.header_rows)
        });
        let Some(analyses) = analyses else {
            notice.set(Some(tr(Text::AnalysisRunningSave).to_owned()));
            return false;
        };
        analyses
            .iter()
            .map(|analysis| analysis.problems.len())
            .sum::<usize>()
    };
    if text_parse_issue.is_some() || problem_count > 0 {
        let description = match text_parse_issue.as_ref() {
            Some(issue) => l10n(Message::SaveParseProblems {
                count: issue.count,
                detail: &issue.message,
            }),
            None => l10n(Message::SaveCellProblems(problem_count)),
        };
        let save_anyway = tr(Text::SaveAnyway);
        let choice = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(tr(Text::SaveProblemsTitle))
            .set_description(description)
            .set_buttons(MessageButtons::OkCancelCustom(
                save_anyway.to_owned(),
                tr(Text::Cancel).to_owned(),
            ))
            .show();
        let confirmed = matches!(choice, MessageDialogResult::Ok)
            || matches!(&choice, MessageDialogResult::Custom(label) if label == save_anyway);
        if !confirmed {
            return false;
        }
    }

    let result = {
        let mut tabs_write = tabs.write();
        let Some(tab) = tabs_write.iter_mut().find(|tab| &tab.document.path == path) else {
            notice.set(Some(tr(Text::OpenTabMissing).to_owned()));
            return false;
        };
        tab.save(false)
    };
    match result {
        Ok(()) => {
            notice.set(Some(l10n(Message::Saved(&file_name(path)))));
            true
        }
        Err(DocumentSessionError::ExternalModification { .. }) => resolve_external_conflict(
            path,
            tabs,
            selected_cell,
            cell_draft,
            diagnostic_target,
            notice,
        ),
        Err(error) => {
            notice.set(Some(l10n(Message::Technical {
                prefix: Text::SaveError,
                detail: &error.to_string(),
            })));
            false
        }
    }
}

fn resolve_external_conflict(
    path: &PathBuf,
    mut tabs: Signal<Vec<DocumentSession>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut notice: Signal<Option<String>>,
) -> bool {
    let overwrite_label = tr(Text::OverwriteDiskFile);
    let reload_label = tr(Text::ReloadDiskFile);
    let choice = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(tr(Text::FileChangedTitle))
        .set_description(l10n(Message::FileChanged(&file_name(path))))
        .set_buttons(MessageButtons::YesNoCancelCustom(
            overwrite_label.to_owned(),
            reload_label.to_owned(),
            tr(Text::Cancel).to_owned(),
        ))
        .show();

    if matches!(choice, MessageDialogResult::Yes)
        || matches!(&choice, MessageDialogResult::Custom(label) if label == overwrite_label)
    {
        let result = tabs
            .write()
            .iter_mut()
            .find(|tab| &tab.document.path == path)
            .ok_or_else(|| tr(Text::OpenTabMissing).to_owned())
            .and_then(|tab| tab.save(true).map_err(|error| error.to_string()));
        return match result {
            Ok(()) => {
                notice.set(Some(l10n(Message::Overwrote(&file_name(path)))));
                true
            }
            Err(error) => {
                notice.set(Some(error));
                false
            }
        };
    }

    if matches!(choice, MessageDialogResult::No)
        || matches!(&choice, MessageDialogResult::Custom(label) if label == reload_label)
    {
        let discard_label = tr(Text::DiscardAndReload);
        let confirm = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(tr(Text::DiscardLocalTitle))
            .set_description(tr(Text::DiscardDescription))
            .set_buttons(MessageButtons::OkCancelCustom(
                discard_label.to_owned(),
                tr(Text::Cancel).to_owned(),
            ))
            .show();
        if !matches!(confirm, MessageDialogResult::Ok)
            && !matches!(&confirm, MessageDialogResult::Custom(label) if label == discard_label)
        {
            return false;
        }
        let result = tabs
            .write()
            .iter_mut()
            .find(|tab| &tab.document.path == path)
            .ok_or_else(|| tr(Text::OpenTabMissing).to_owned())
            .and_then(|tab| tab.reload().map_err(|error| error.to_string()));
        return match result {
            Ok(()) => {
                cell_draft.set(None);
                selected_cell.set(None);
                diagnostic_target.set(None);
                notice.set(Some(l10n(Message::Reloaded(&file_name(path)))));
                true
            }
            Err(error) => {
                notice.set(Some(error));
                false
            }
        };
    }
    false
}

fn restore_hidden_window(desktop: dioxus::desktop::DesktopContext) {
    spawn(async move {
        tokio::task::yield_now().await;
        desktop.set_visible(true);
        desktop.set_focus();
    });
}

fn tab_has_unsaved_changes(tab: &DocumentSession, draft: Option<&CellDraft>) -> bool {
    if tab.is_dirty() {
        return true;
    }
    let Some(draft) = draft.filter(|draft| draft.location.path == tab.document.path) else {
        return false;
    };
    let original = tab
        .document
        .records
        .get(draft.location.row_index)
        .and_then(|row| row.get(draft.location.column_index));
    let Some(original) = original else {
        return true;
    };
    normalize_cell_edit(original, &draft.value)
        .map(|normalized| normalized != *original)
        .unwrap_or(true)
}

fn unsaved_tab_paths(tabs: &[DocumentSession], draft: Option<&CellDraft>) -> Vec<PathBuf> {
    tabs.iter()
        .filter(|tab| tab_has_unsaved_changes(tab, draft))
        .map(|tab| tab.document.path.clone())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn confirm_close_all_tabs(
    tabs: Signal<Vec<DocumentSession>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    notice: Signal<Option<String>>,
    action: CloseAction,
) -> bool {
    let paths = {
        let tabs_read = tabs.read();
        let draft_read = cell_draft.read();
        unsaved_tab_paths(&tabs_read, draft_read.as_ref())
    };
    if paths.is_empty() {
        return true;
    }

    let save_all_label = tr(Text::SaveAll);
    let dont_save_label = tr(Text::DontSave);
    let choice = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(tr(Text::UnsavedFiles))
        .set_description(l10n(Message::UnsavedFiles {
            count: paths.len(),
            action,
        }))
        .set_buttons(MessageButtons::YesNoCancelCustom(
            save_all_label.to_owned(),
            dont_save_label.to_owned(),
            tr(Text::ReturnToEditing).to_owned(),
        ))
        .show();

    if matches!(choice, MessageDialogResult::Yes)
        || matches!(&choice, MessageDialogResult::Custom(label) if label == save_all_label)
    {
        for path in paths {
            if !attempt_save_tab(
                &path,
                tabs,
                selected_cell,
                cell_draft,
                diagnostic_target,
                table_analyses,
                notice,
            ) {
                return false;
            }
        }
        return true;
    }

    matches!(choice, MessageDialogResult::No)
        || matches!(&choice, MessageDialogResult::Custom(label) if label == dont_save_label)
}

#[allow(clippy::too_many_arguments)]
fn request_close_tab(
    path: PathBuf,
    tabs: Signal<Vec<DocumentSession>>,
    active_tab: Signal<Option<PathBuf>>,
    selected_cell: Signal<Option<CellLocation>>,
    cell_draft: Signal<Option<CellDraft>>,
    diagnostic_target: Signal<Option<DiagnosticTarget>>,
    table_analyses: Signal<HashMap<PathBuf, TableAnalysisState>>,
    notice: Signal<Option<String>>,
) {
    let is_dirty = {
        let tabs_read = tabs.read();
        let draft_read = cell_draft.read();
        tabs_read
            .iter()
            .find(|tab| tab.document.path == path)
            .is_some_and(|tab| tab_has_unsaved_changes(tab, draft_read.as_ref()))
    };
    if !is_dirty {
        close_tab_now(
            &path,
            tabs,
            active_tab,
            selected_cell,
            cell_draft,
            diagnostic_target,
        );
        return;
    }

    let save_label = tr(Text::Save);
    let dont_save_label = tr(Text::DontSave);
    let choice = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(tr(Text::UnsavedChanges))
        .set_description(l10n(Message::SaveChanges(&file_name(&path))))
        .set_buttons(MessageButtons::YesNoCancelCustom(
            save_label.to_owned(),
            dont_save_label.to_owned(),
            tr(Text::Cancel).to_owned(),
        ))
        .show();
    let should_close = if matches!(choice, MessageDialogResult::Yes)
        || matches!(&choice, MessageDialogResult::Custom(label) if label == save_label)
    {
        attempt_save_tab(
            &path,
            tabs,
            selected_cell,
            cell_draft,
            diagnostic_target,
            table_analyses,
            notice,
        )
    } else {
        matches!(choice, MessageDialogResult::No)
            || matches!(&choice, MessageDialogResult::Custom(label) if label == dont_save_label)
    };
    if should_close {
        close_tab_now(
            &path,
            tabs,
            active_tab,
            selected_cell,
            cell_draft,
            diagnostic_target,
        );
    }
}

fn close_tab_now(
    path: &PathBuf,
    mut tabs: Signal<Vec<DocumentSession>>,
    mut active_tab: Signal<Option<PathBuf>>,
    mut selected_cell: Signal<Option<CellLocation>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
) {
    tabs.write().retain(|tab| &tab.document.path != path);
    let was_active = active_tab.read().as_ref() == Some(path);
    if was_active {
        let next_path = tabs.read().last().map(|tab| tab.document.path.clone());
        active_tab.set(next_path);
        selected_cell.set(None);
        cell_draft.set(None);
        diagnostic_target.set(None);
    }
}

#[allow(clippy::too_many_arguments)]
fn reload_external_tab(
    path: PathBuf,
    confirm_discard: bool,
    mut tabs: Signal<Vec<DocumentSession>>,
    mut cell_draft: Signal<Option<CellDraft>>,
    mut diagnostic_target: Signal<Option<DiagnosticTarget>>,
    mut external_conflicts: Signal<HashSet<PathBuf>>,
    mut external_reload_errors: Signal<HashMap<PathBuf, String>>,
    mut notice: Signal<Option<String>>,
) {
    if confirm_discard {
        let discard_label = tr(Text::DiscardAndReload);
        let choice = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(tr(Text::DiscardLocalTitle))
            .set_description(l10n(Message::ReloadDiscard(&file_name(&path))))
            .set_buttons(MessageButtons::OkCancelCustom(
                discard_label.to_owned(),
                tr(Text::Cancel).to_owned(),
            ))
            .show();
        if !matches!(choice, MessageDialogResult::Ok)
            && !matches!(&choice, MessageDialogResult::Custom(label) if label == discard_label)
        {
            return;
        }
    }

    let options = tabs
        .read()
        .iter()
        .find(|tab| tab.document.path == path)
        .map(|tab| {
            (
                tab.delimiter_override(),
                tab.header_rows,
                tab.view(),
                tab.gb18030_conversion_allowed(),
            )
        });
    let Some((delimiter, header_rows, previous_view, conversion_allowed)) = options else {
        notice.set(Some(tr(Text::OpenTabMissing).to_owned()));
        return;
    };
    spawn(async move {
        let result =
            load_session_for_edit(path.clone(), delimiter, header_rows, conversion_allowed).await;
        match result {
            Ok(Some(mut replacement)) => {
                if previous_view == DocumentView::Text && replacement.text_parse_issue().is_none() {
                    replacement.show_text();
                }
                if let Some(tab) = tabs
                    .write()
                    .iter_mut()
                    .find(|tab| tab.document.path == path)
                {
                    *tab = replacement;
                }
                if cell_draft
                    .read()
                    .as_ref()
                    .is_some_and(|draft| draft.location.path == path)
                {
                    cell_draft.set(None);
                }
                diagnostic_target.set(None);
                external_conflicts.write().remove(&path);
                external_reload_errors.write().remove(&path);
                notice.set(Some(l10n(Message::Reloaded(&file_name(&path)))));
            }
            Ok(None) => {}
            Err(error) => {
                external_reload_errors
                    .write()
                    .insert(path.clone(), error.clone());
                notice.set(Some(l10n(Message::Technical {
                    prefix: Text::ReloadError,
                    detail: &error,
                })));
            }
        }
    });
}

fn path_was_affected(path: &Path, changed_paths: &HashSet<PathBuf>) -> bool {
    changed_paths
        .iter()
        .any(|changed| changed == path || path.starts_with(changed))
}

fn preview_path(preview: &Preview) -> Option<&Path> {
    match preview {
        Preview::Loading { path, .. }
        | Preview::Error { path, .. }
        | Preview::Document {
            document: CsvDocument { path, .. },
            ..
        } => Some(path),
        Preview::Empty => None,
    }
}

fn external_change_action(
    saved_hash: blake3::Hash,
    disk_hash: Option<blake3::Hash>,
    is_dirty: bool,
) -> ExternalChangeAction {
    if disk_hash == Some(saved_hash) {
        ExternalChangeAction::None
    } else if is_dirty {
        ExternalChangeAction::Conflict
    } else {
        ExternalChangeAction::Reload
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("CSV")
        .to_owned()
}

const CELL_PROBLEM_KINDS: [CellProblemKind; 5] = [
    CellProblemKind::StructuralMismatch,
    CellProblemKind::RealLineBreak,
    CellProblemKind::DangerousInvisibleCharacter,
    CellProblemKind::UnescapedQuote,
    CellProblemKind::InvalidBackslashEscape,
];

fn cell_problem_reason(kind: CellProblemKind, analysis: &ColumnAnalysis) -> String {
    match kind {
        CellProblemKind::StructuralMismatch => {
            l10n(Message::StructuralMismatch(&analysis.type_expression))
        }
        CellProblemKind::RealLineBreak => tr(Text::ProblemRealLineBreak).to_owned(),
        CellProblemKind::DangerousInvisibleCharacter => {
            tr(Text::ProblemDangerousInvisibleCharacter).to_owned()
        }
        CellProblemKind::UnescapedQuote => tr(Text::ProblemUnescapedQuote).to_owned(),
        CellProblemKind::InvalidBackslashEscape => {
            tr(Text::ProblemInvalidBackslashEscape).to_owned()
        }
    }
}

fn cell_problem_reasons(kinds: &[CellProblemKind], analysis: &ColumnAnalysis) -> Vec<String> {
    kinds
        .iter()
        .map(|kind| cell_problem_reason(*kind, analysis))
        .collect()
}

fn cell_problem_tooltip(value: &str, reasons: &[String]) -> String {
    if reasons.is_empty() {
        return value.to_owned();
    }
    let reasons = reasons
        .iter()
        .map(|reason| format!("- {reason}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}:\n{reasons}\n\n{}:\n{value}",
        tr(Text::ProblemReasons),
        tr(Text::CellValue),
    )
}

fn column_problem_summary(analysis: &ColumnAnalysis) -> String {
    let lines = CELL_PROBLEM_KINDS
        .iter()
        .filter_map(|kind| {
            let count = analysis
                .problems
                .iter()
                .filter(|problem| problem.kinds.contains(kind))
                .count();
            (count > 0).then(|| format!("- {} ({count})", cell_problem_reason(*kind, analysis)))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}:\n{lines}", tr(Text::ProblemReasons))
}

fn column_has_structural_error(analysis: &ColumnAnalysis) -> bool {
    analysis
        .problems
        .iter()
        .any(|problem| problem.kinds.contains(&CellProblemKind::StructuralMismatch))
}

fn column_type_label(analysis: &ColumnAnalysis) -> String {
    if column_has_structural_error(analysis) {
        format!("{}*", analysis.type_expression)
    } else {
        analysis.type_expression.clone()
    }
}

fn column_type_tooltip(analysis: &ColumnAnalysis) -> String {
    let mut sections = vec![format!(
        "{}: {}",
        tr(Text::TypeLabel),
        analysis.type_expression
    )];
    if !analysis.problems.is_empty() {
        sections.push(column_problem_summary(analysis));
    } else if analysis.has_mixed_warning {
        sections.push(tr(Text::MixedTypeReason).to_owned());
    }
    sections.join("\n\n")
}

#[allow(clippy::too_many_arguments)]
fn document_status(
    document: &CsvDocument,
    text: &str,
    view: DocumentView,
    text_parse_issue: Option<&TextParseIssue>,
    header_rows: usize,
    selected_cell: Option<&CellLocation>,
    text_cursor: Option<&TextCursorPosition>,
    analysis_state: Option<&TableAnalysisState>,
) -> DocumentStatus {
    let table_view = view == DocumentView::Table;
    let dimensions = if table_view {
        document
            .dimensions(header_rows)
            .map(|(rows, columns)| l10n(Message::RowsColumns { rows, columns }))
            .unwrap_or_else(|| l10n(Message::RequiresHeaderRows(header_rows)))
    } else {
        physical_lines(physical_line_count(text))
    };
    let position = if table_view {
        selected_cell
            .filter(|location| location.path == document.path && location.row_index >= header_rows)
            .map(|location| {
                l10n(Message::TablePosition {
                    row: location.row_index - header_rows + 1,
                    column: location.column_index + 1,
                })
            })
    } else {
        let cursor = text_cursor.filter(|cursor| cursor.path == document.path);
        Some(match cursor {
            Some(cursor) => l10n(Message::TextPosition {
                line: cursor.line,
                column: cursor.column,
            }),
            None => l10n(Message::TextPosition { line: 1, column: 1 }),
        })
    };
    let analyses = table_view
        .then(|| {
            analysis_state
                .and_then(|state| state.ready_columns(document.analysis_version(), header_rows))
        })
        .flatten();
    let (red_cells, yellow_columns) = analyses
        .as_deref()
        .map(|columns| {
            (
                columns.iter().map(|analysis| analysis.problems.len()).sum(),
                columns
                    .iter()
                    .filter(|analysis| analysis.has_mixed_warning)
                    .count(),
            )
        })
        .map_or((None, None), |(red, yellow)| (Some(red), Some(yellow)));
    let diagnostic = analyses.as_deref().and_then(|columns| {
        let location = selected_cell.filter(|location| {
            location.path == document.path && location.row_index >= header_rows
        })?;
        let analysis = columns.get(location.column_index)?;
        let problem = analysis
            .problems
            .iter()
            .find(|problem| problem.row_index == location.row_index)?;
        let reasons = cell_problem_reasons(&problem.kinds, analysis);
        (!reasons.is_empty())
            .then(|| format!("{}: {}", tr(Text::ProblemReasons), reasons.join(" · ")))
    });

    DocumentStatus {
        file_name: file_name(&document.path),
        dimensions,
        encoding: document.encoding.label(),
        position,
        diagnostic,
        red_cells,
        yellow_columns,
        parse_errors: text_parse_issue.map(|issue| issue.count),
        analysis_loading: table_view && document.records.len() >= header_rows && analyses.is_none(),
        delimiter_defaulted: document.delimiter_source == DelimiterSource::Default,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn unsaved_paths_include_changed_drafts_without_counting_no_op_drafts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(&path, b"id,name\n1,Arthur\n").unwrap();
        let mut session = DocumentSession::open(&path, Some(b',')).unwrap();

        let no_op_draft = CellDraft {
            location: CellLocation {
                path: path.clone(),
                row_index: 1,
                column_index: 1,
            },
            value: "Arthur".to_owned(),
        };
        assert!(unsaved_tab_paths(&[session.clone()], None).is_empty());
        assert!(unsaved_tab_paths(&[session.clone()], Some(&no_op_draft)).is_empty());

        let changed_draft = CellDraft {
            value: "Merlin".to_owned(),
            ..no_op_draft
        };
        assert_eq!(
            unsaved_tab_paths(&[session.clone()], Some(&changed_draft)),
            vec![path.clone()]
        );

        session.edit_cell(1, 1, "Lancelot".to_owned()).unwrap();
        assert_eq!(unsaved_tab_paths(&[session], None), vec![path]);
    }

    #[test]
    fn pretty_json_drafts_are_not_dirty_until_the_value_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("json.csv");
        fs::write(&path, "meta\n\"{\"\"id\"\":1}\"\n").unwrap();
        let session = DocumentSession::open_with_options(&path, Some(b','), 1).unwrap();
        let draft = CellDraft {
            location: CellLocation {
                path: path.clone(),
                row_index: 1,
                column_index: 0,
            },
            value: "{\n  \"id\": 1\n}".to_owned(),
        };

        assert!(!tab_has_unsaved_changes(&session, Some(&draft)));
    }

    #[test]
    fn changed_directory_paths_cover_descendant_csv_files() {
        let changes = HashSet::from([PathBuf::from("C:/configs/heroes")]);

        assert!(path_was_affected(
            Path::new("C:/configs/heroes/basic.csv"),
            &changes
        ));
        assert!(!path_was_affected(
            Path::new("C:/configs/items.csv"),
            &changes
        ));
    }

    #[test]
    fn external_changes_reload_clean_tabs_and_flag_dirty_tabs() {
        let saved = blake3::hash(b"saved");
        let changed = blake3::hash(b"changed");

        assert_eq!(
            external_change_action(saved, Some(saved), false),
            ExternalChangeAction::None
        );
        assert_eq!(
            external_change_action(saved, Some(changed), false),
            ExternalChangeAction::Reload
        );
        assert_eq!(
            external_change_action(saved, Some(changed), true),
            ExternalChangeAction::Conflict
        );
        assert_eq!(
            external_change_action(saved, None, true),
            ExternalChangeAction::Conflict
        );
    }

    #[test]
    fn clipboard_bridge_deserializes_pasted_text_without_normalizing_it() {
        let command: WindowShortcutCommand =
            serde_json::from_value(serde_json::json!({ "paste": "a\r\nb" })).unwrap();

        assert_eq!(command, WindowShortcutCommand::Paste("a\r\nb".to_owned()));
    }

    #[test]
    fn focused_row_measurement_command_preserves_horizontal_and_vertical_inputs() {
        let command: WindowShortcutCommand = serde_json::from_value(serde_json::json!({
            "focused_row_height": {
                "path": "heroes.csv",
                "row_index": 4,
                "column_index": 2,
                "content_height": 180.5
            }
        }))
        .unwrap();

        assert_eq!(
            command,
            WindowShortcutCommand::FocusedRowHeight(FocusedRowMeasurement {
                path: PathBuf::from("heroes.csv"),
                row_index: 4,
                column_index: 2,
                content_height: 180.5,
            })
        );
    }

    #[test]
    fn focused_plain_text_and_json_use_multiline_editors() {
        assert!(cell_uses_multiline_editor(false, Some(2), 2));
        assert!(!cell_uses_multiline_editor(false, Some(2), 1));
        assert!(cell_uses_multiline_editor(true, None, 1));
    }

    #[test]
    fn focused_row_css_hard_caps_the_row_and_inner_scroller() {
        assert!(APP_CSS.contains("max-height: var(--focused-row-height)"));
        assert!(APP_CSS.contains("max-height: calc(var(--focused-row-height, 30px) - 10px)"));
        assert!(APP_CSS.contains(".focus-textarea"));
    }

    #[test]
    fn editor_text_insertion_preserves_a_real_line_break() {
        let script = editor_insert_script("\n");

        assert!(script.contains(r#"setRangeText("\n", start, end, "end")"#));
        assert!(script.contains("dispatchEvent(new Event(\"input\""));
    }

    #[test]
    fn focus_mode_maps_dynamic_layout_roles_to_css_classes() {
        let layout = FocusLayout::calculate(7, 3, 320, 1_100.0).unwrap();

        assert_eq!(column_focus_class(0, Some(&layout)), "column-hidden");
        assert_eq!(column_focus_class(2, Some(&layout)), "focus-left-neighbor");
        assert_eq!(column_focus_class(3, Some(&layout)), "focus-column");
        assert_eq!(column_focus_class(5, Some(&layout)), "focus-right-neighbor");
        assert_eq!(column_focus_class(6, Some(&layout)), "column-hidden");
        assert_eq!(column_focus_class(4, None), "");
    }

    #[test]
    fn focus_status_uses_the_last_header_row_and_falls_back_for_empty_fields() {
        let document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"description,value\nid,\n1,Arthur\n",
            Some(b','),
        )
        .unwrap();

        assert_eq!(focused_column_field(&document, 2, 0), Some("id"));
        assert_eq!(focused_column_field(&document, 2, 1), None);
        assert_eq!(focused_column_field(&document, 1, 1), Some("value"));
    }

    #[test]
    fn focused_header_and_type_row_have_persistent_accent_styles() {
        assert!(APP_CSS.contains(".field-header th.focus-column"));
        assert!(APP_CSS.contains(".type-row th.focus-column"));
        assert!(APP_CSS.contains(".focus-mode-status"));
    }

    #[test]
    fn focused_column_width_is_bounded_for_short_and_long_content() {
        assert_eq!(focus_column_width(5), 320);
        assert_eq!(focus_column_width(1_000), 720);
    }

    #[test]
    fn read_only_cells_select_and_switch_focus_without_entering_edit_mode() {
        assert_eq!(
            cell_click_action(true, false, None, 2),
            CellClickAction::Select
        );
        assert_eq!(
            cell_click_action(true, true, Some(2), 2),
            CellClickAction::Edit
        );
        assert_eq!(
            cell_click_action(true, true, None, 2),
            CellClickAction::Select
        );
        assert_eq!(
            cell_click_action(true, false, Some(2), 3),
            CellClickAction::SwitchFocus
        );
        assert_eq!(
            cell_click_action(false, true, None, 2),
            CellClickAction::Edit
        );
    }

    #[test]
    fn preview_edit_intents_require_a_selected_cell_in_focus_mode() {
        let path = PathBuf::from("heroes.csv");
        let selected = CellLocation {
            path: path.clone(),
            row_index: 2,
            column_index: 1,
        };
        let focused = FocusedColumn {
            path: path.clone(),
            column_index: 1,
        };

        assert!(preview_edit_is_eligible(
            &path,
            Some(&selected),
            Some(&focused)
        ));
        assert!(!preview_edit_is_eligible(&path, Some(&selected), None));
        assert!(!preview_edit_is_eligible(
            Path::new("other.csv"),
            Some(&selected),
            Some(&focused)
        ));
    }

    #[test]
    fn focused_column_navigation_uses_the_rendered_document_column_count() {
        assert_eq!(next_focused_column(0, -1, 5), 0);
        assert_eq!(next_focused_column(2, 1, 5), 3);
        assert_eq!(next_focused_column(4, 1, 5), 4);
        assert_eq!(next_focused_column(0, 1, 0), 0);
    }

    #[test]
    fn f1_shortcut_command_deserializes_to_the_shortcuts_panel() {
        let command: WindowShortcutCommand = serde_json::from_str("\"shortcuts\"").unwrap();
        assert_eq!(command, WindowShortcutCommand::Shortcuts);
    }

    #[test]
    fn resize_dimensions_are_clamped_to_the_control_limits() {
        assert_eq!(resized_dimension(280, 40.0, 220, 520), 320);
        assert_eq!(resized_dimension(280, -500.0, 220, 520), 220);
        assert_eq!(resized_dimension(280, 500.0, 220, 520), 520);
    }

    #[test]
    fn text_view_uses_physical_lines_including_a_trailing_empty_line() {
        assert_eq!(physical_line_count(""), 1);
        assert_eq!(physical_line_count("id,name\n1,Arthur\n"), 3);
        assert_eq!(physical_line_count("id,name\r\n1,Arthur"), 2);
        assert_eq!(physical_line_numbers(3), "1\n2\n3");
    }

    #[test]
    fn status_reports_table_position_and_problem_counts() {
        let document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"description,value\nid,value\n1,true\n2,2\n",
            Some(b','),
        )
        .unwrap();
        let analysis = TableAnalysisState::Ready {
            document_version: document.analysis_version(),
            header_rows: 2,
            columns: Arc::new(analyze_table(document.records.as_ref(), 2)),
        };
        let selected = CellLocation {
            path: document.path.clone(),
            row_index: 3,
            column_index: 1,
        };

        let status = document_status(
            &document,
            &document.raw_text,
            DocumentView::Table,
            None,
            2,
            Some(&selected),
            None,
            Some(&analysis),
        );

        assert_eq!(
            status.position.as_deref(),
            Some(l10n(Message::TablePosition { row: 2, column: 2 }).as_str())
        );
        assert_eq!(status.red_cells, Some(0));
        assert_eq!(status.yellow_columns, Some(1));
        assert_eq!(status.diagnostic, None);
        assert!(!status.analysis_loading);
    }

    #[test]
    fn red_cell_reasons_are_available_in_tooltips_and_status() {
        let document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"id,payload\n1,{\"id\":1}\n2,broken\n",
            Some(b','),
        )
        .unwrap();
        let columns = Arc::new(analyze_table(document.records.as_ref(), 1));
        let analysis = &columns[1];
        let problem = analysis
            .problems
            .iter()
            .find(|problem| problem.row_index == 2)
            .unwrap();
        let reasons = cell_problem_reasons(&problem.kinds, analysis);
        let all_reasons = cell_problem_reasons(&CELL_PROBLEM_KINDS, analysis);
        let tooltip = cell_problem_tooltip("broken", &reasons);
        let summary = column_problem_summary(analysis);
        let type_tooltip = column_type_tooltip(analysis);
        assert_eq!(column_type_label(analysis), "{id:number}*");
        let state = TableAnalysisState::Ready {
            document_version: document.analysis_version(),
            header_rows: 1,
            columns,
        };
        let selected = CellLocation {
            path: document.path.clone(),
            row_index: 2,
            column_index: 1,
        };
        let status = document_status(
            &document,
            &document.raw_text,
            DocumentView::Table,
            None,
            1,
            Some(&selected),
            None,
            Some(&state),
        );

        assert_eq!(reasons.len(), 1);
        assert_eq!(all_reasons.len(), 5);
        assert!(all_reasons.iter().all(|reason| !reason.is_empty()));
        assert!(tooltip.contains(tr(Text::ProblemReasons)));
        assert!(tooltip.contains("broken"));
        assert!(summary.contains("(1)"));
        assert!(type_tooltip.contains("{id:number}"));
        assert!(type_tooltip.contains(summary.as_str()));
        assert!(
            status
                .diagnostic
                .as_deref()
                .is_some_and(|detail| detail.contains("{id:number}"))
        );
        assert_eq!(cell_problem_tooltip("plain", &[]), "plain");
    }

    #[test]
    fn status_uses_physical_text_cursor_coordinates() {
        let document =
            CsvDocument::from_bytes(Path::new("heroes.csv"), b"id,name\n1,Arthur\n", Some(b','))
                .unwrap();
        let cursor = TextCursorPosition {
            path: document.path.clone(),
            line: 2,
            column: 4,
        };

        let status = document_status(
            &document,
            &document.raw_text,
            DocumentView::Text,
            None,
            1,
            None,
            Some(&cursor),
            None,
        );

        assert_eq!(
            status.position.as_deref(),
            Some(l10n(Message::TextPosition { line: 2, column: 4 }).as_str())
        );
        assert_eq!(status.red_cells, None);
        assert!(!status.analysis_loading);
    }

    #[test]
    fn json_edits_are_pretty_in_the_draft_and_compact_on_commit() {
        let original = r#"{"name":"Arthur","hp":500}"#;
        let draft = editable_cell_value(original);

        assert!(draft.contains('\n'));
        assert_eq!(normalize_cell_edit(original, &draft).unwrap(), original);
    }

    #[test]
    fn json_edits_reject_invalid_syntax_and_structure_changes() {
        assert!(normalize_cell_edit(r#"{"id":1}"#, "{invalid").is_err());
        assert!(normalize_cell_edit("[[1],[2]]", "[1,2]").is_err());
        assert_eq!(
            normalize_cell_edit("plain", "still plain").unwrap(),
            "still plain"
        );
    }

    #[test]
    fn json_highlighting_escapes_cell_content_before_inserting_markup() {
        let highlighted = syntax_highlight_json(r#"{"html":"<script>"}"#);

        assert!(highlighted.contains("json-key"));
        assert!(highlighted.contains("&lt;script&gt;"));
        assert!(!highlighted.contains("<script>"));
    }

    #[test]
    fn json_editor_ids_are_stable_and_scroll_syncs_both_axes() {
        let first = json_editor_ids(Path::new("heroes.csv"), 3, 2);
        let second = json_editor_ids(Path::new("heroes.csv"), 3, 2);
        let other = json_editor_ids(Path::new("heroes.csv"), 4, 2);
        let script = json_scroll_sync_script(&first.1, &first.0);

        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(script.contains("highlight.scrollTop = input.scrollTop"));
        assert!(script.contains("highlight.scrollLeft = input.scrollLeft"));
    }
}
