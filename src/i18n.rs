use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    En,
    Zh,
}

impl Language {
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale.map(str::trim).map(str::to_ascii_lowercase) {
            Some(locale) if locale.starts_with("zh") => Self::Zh,
            _ => Self::En,
        }
    }

    pub fn detect() -> Self {
        Self::from_locale(sys_locale::get_locale().as_deref())
    }

    pub fn text(self, key: Text) -> &'static str {
        let (zh, en) = key.translations();
        if self == Self::Zh { zh } else { en }
    }
}

static LANGUAGE: OnceLock<Language> = OnceLock::new();

pub fn language() -> Language {
    *LANGUAGE.get_or_init(Language::detect)
}

pub fn text(key: Text) -> &'static str {
    language().text(key)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Text {
    AppTitle,
    OpenFolder,
    OpenCsvFolder,
    Shortcuts,
    Close,
    NoWorkspace,
    SearchConfigurations,
    FileView,
    List,
    Tree,
    Scanning,
    ParseError,
    ResizeSidebar,
    CloseTab,
    ReloadFromDisk,
    KeepEditing,
    RetryReload,
    DelimiterDefaulted,
    Analyzing,
    CommandPalette,
    SearchFilesOrLine,
    SearchFiles,
    GoToLine,
    LineNumber,
    PositiveLineNumber,
    PositiveLineAfterColon,
    SearchCurrentFile,
    Find,
    Previous,
    Next,
    PreviousMatch,
    NextMatch,
    CloseSearch,
    MatchCase,
    SearchWorkspace,
    SearchWorkspaceContents,
    Stop,
    NoMatches,
    Loading,
    EmptyPreviewTitle,
    EmptyPreviewBody,
    LoadingPreview,
    PreviewFailed,
    RevealInFileManager,
    HeaderRows,
    EditorView,
    Delimiter,
    CsvDelimiter,
    Comma,
    Tab,
    Semicolon,
    Pipe,
    Table,
    Text,
    ReadOnlyPreview,
    EditableText,
    EditableTable,
    TypeLabel,
    Save,
    SaveFile,
    CsvTextEditor,
    InvalidHeaderTitle,
    InvalidHeaderBody,
    ResizeColumn,
    JsonCellEditor,
    JsonSyntaxError,
    JsonValueMustRemain,
    JsonObject,
    JsonArray,
    JsonArray2d,
    ProblemReasons,
    CellValue,
    ProblemRealLineBreak,
    ProblemDangerousInvisibleCharacter,
    ProblemUnescapedQuote,
    ProblemInvalidBackslashEscape,
    MixedTypeReason,
    UnsavedFiles,
    SaveAll,
    DontSave,
    ReturnToEditing,
    UnsavedChanges,
    Cancel,
    SaveAnyway,
    SaveProblemsTitle,
    FileChangedTitle,
    OverwriteDiskFile,
    ReloadDiskFile,
    DiscardLocalTitle,
    DiscardAndReload,
    ConvertEncodingTitle,
    ConvertToUtf8,
    ShortcutGlobal,
    ShortcutFileSearch,
    ShortcutTableNavigation,
    ShortcutEditing,
    ShortcutShowList,
    ShortcutToggleSidebar,
    ShortcutNextTab,
    ShortcutPreviousTab,
    ShortcutCloseOverlay,
    ShortcutFilePalette,
    ShortcutGoToLine,
    ShortcutCurrentSearch,
    ShortcutGlobalSearch,
    ShortcutSave,
    ShortcutCloseDocument,
    ShortcutMoveCell,
    ShortcutEditCell,
    ShortcutToggleFocus,
    ShortcutMoveFocus,
    ShortcutNextProblem,
    ShortcutPreviousProblem,
    ShortcutCopy,
    ShortcutPaste,
    ShortcutUndo,
    ShortcutRedo,
    ShortcutCommitDown,
    ShortcutCommitAcross,
    ShortcutInsertLineBreak,
    ShortcutIndentJson,
    ShortcutCancelEdit,
    TabLimit,
    OpenTabMissing,
    PreviewClosed,
    NoWorkspaceOpen,
    OutsideWorkspace,
    EditedCellMissing,
    SelectCellBeforePaste,
    PastedCell,
    ClipboardSame,
    CopiedCell,
    NothingSelectedCopy,
    RedidEdit,
    UndidEdit,
    NothingRedo,
    NothingUndo,
    AnalysisRunning,
    AnalysisRunningSave,
    NoProblems,
    NoSearchableFile,
    LineStartsAtOne,
    TableNoColumns,
    TextChangedParsing,
    DiscardDescription,
    SettingsError,
    ScanError,
    OpenError,
    SaveError,
    ReloadError,
    StartupError,
}

impl Text {
    fn translations(self) -> (&'static str, &'static str) {
        match self {
            Self::AppTitle => ("游戏配置编辑器", "Game Config Editor"),
            Self::OpenFolder => ("打开文件夹", "Open folder"),
            Self::OpenCsvFolder => ("打开 CSV 配置文件夹", "Open CSV configuration folder"),
            Self::Shortcuts => ("快捷键", "Shortcuts"),
            Self::Close => ("关闭", "Close"),
            Self::NoWorkspace => ("未选择工作区", "No workspace selected"),
            Self::SearchConfigurations => ("搜索配置", "Search configurations"),
            Self::FileView => ("文件视图", "File view"),
            Self::List => ("列表", "List"),
            Self::Tree => ("树状", "Tree"),
            Self::Scanning => ("正在扫描...", "Scanning..."),
            Self::ParseError => ("解析错误", "Parse error"),
            Self::ResizeSidebar => ("调整侧栏宽度", "Resize sidebar"),
            Self::CloseTab => ("关闭标签页", "Close tab"),
            Self::ReloadFromDisk => ("从磁盘重新加载", "Reload from disk"),
            Self::KeepEditing => ("保留本地编辑", "Keep editing"),
            Self::RetryReload => ("重试加载", "Retry reload"),
            Self::DelimiterDefaulted => (
                "未识别分隔符，已默认使用逗号",
                "Delimiter defaulted to comma",
            ),
            Self::Analyzing => ("正在分析", "Analyzing"),
            Self::CommandPalette => ("文件快速打开", "Command palette"),
            Self::SearchFilesOrLine => ("搜索文件或输入行号", "Search files or enter a line"),
            Self::SearchFiles => ("搜索文件", "Search files"),
            Self::GoToLine => ("跳转到行", "Go to line"),
            Self::LineNumber => ("行号", "Line number"),
            Self::PositiveLineNumber => ("请输入正整数行号", "Enter a positive line number"),
            Self::PositiveLineAfterColon => (
                "请在 ':' 后输入正整数行号",
                "Enter a positive line number after ':'",
            ),
            Self::SearchCurrentFile => ("搜索当前文件", "Search current file"),
            Self::Find => ("查找", "Find"),
            Self::Previous => ("上一个", "Previous"),
            Self::Next => ("下一个", "Next"),
            Self::PreviousMatch => ("上一个匹配项", "Previous match"),
            Self::NextMatch => ("下一个匹配项", "Next match"),
            Self::CloseSearch => ("关闭搜索", "Close search"),
            Self::MatchCase => ("区分大小写", "Match case"),
            Self::SearchWorkspace => ("搜索工作区", "Search workspace"),
            Self::SearchWorkspaceContents => ("搜索工作区内容", "Search workspace contents"),
            Self::Stop => ("停止", "Stop"),
            Self::NoMatches => ("无匹配项", "No matches"),
            Self::Loading => ("正在加载", "Loading"),
            Self::EmptyPreviewTitle => ("未选择文件", "No file selected"),
            Self::EmptyPreviewBody => (
                "从工作区选择文件以打开只读预览。",
                "Choose a file from the workspace to open a read-only preview.",
            ),
            Self::LoadingPreview => ("正在加载预览", "Loading preview"),
            Self::PreviewFailed => ("预览失败", "Preview failed"),
            Self::RevealInFileManager => ("在文件管理器中显示", "Reveal in file manager"),
            Self::HeaderRows => ("表头行数", "Header rows"),
            Self::EditorView => ("编辑器视图", "Editor view"),
            Self::Delimiter => ("分隔符", "Delimiter"),
            Self::CsvDelimiter => ("CSV 分隔符", "CSV delimiter"),
            Self::Comma => ("逗号", "Comma"),
            Self::Tab => ("制表符", "Tab"),
            Self::Semicolon => ("分号", "Semicolon"),
            Self::Pipe => ("竖线", "Pipe"),
            Self::Table => ("表格", "Table"),
            Self::Text => ("文本", "Text"),
            Self::ReadOnlyPreview => ("只读预览", "Read-only preview"),
            Self::EditableText => ("可编辑文本", "Editable text"),
            Self::EditableTable => ("可编辑表格", "Editable table"),
            Self::TypeLabel => ("类型", "Type"),
            Self::Save => ("保存", "Save"),
            Self::SaveFile => ("保存文件", "Save file"),
            Self::CsvTextEditor => ("CSV 文本编辑器", "CSV text editor"),
            Self::InvalidHeaderTitle => ("表头配置无效", "Invalid header configuration"),
            Self::InvalidHeaderBody => (
                "文件行数少于配置的表头行数。",
                "This file has fewer than the configured header records.",
            ),
            Self::ResizeColumn => ("调整列宽", "Resize column"),
            Self::JsonCellEditor => ("JSON 单元格编辑器", "JSON cell editor"),
            Self::JsonSyntaxError => ("JSON 语法错误", "JSON syntax error"),
            Self::JsonValueMustRemain => ("JSON 值必须保持为", "JSON value must remain a"),
            Self::JsonObject => ("JSON 对象", "JSON object"),
            Self::JsonArray => ("一维 JSON 数组", "one-dimensional JSON array"),
            Self::JsonArray2d => ("二维 JSON 数组", "two-dimensional JSON array"),
            Self::ProblemReasons => ("问题原因", "Problem reasons"),
            Self::CellValue => ("单元格值", "Cell value"),
            Self::ProblemRealLineBreak => (
                "包含真实换行控制字符（CR/LF）",
                "Contains a real line-break control character (CR/LF)",
            ),
            Self::ProblemDangerousInvisibleCharacter => (
                "包含危险的不可见字符",
                "Contains a dangerous invisible character",
            ),
            Self::ProblemUnescapedQuote => (
                "普通字符串包含未转义的双引号",
                "Plain string contains an unescaped double quote",
            ),
            Self::ProblemInvalidBackslashEscape => (
                "普通字符串包含非法反斜杠转义",
                "Plain string contains an invalid backslash escape",
            ),
            Self::MixedTypeReason => (
                "该列包含不兼容的基础类型，已判定为 mixed",
                "This column contains incompatible primitive types and is classified as mixed",
            ),
            Self::UnsavedFiles => ("未保存的文件", "Unsaved files"),
            Self::SaveAll => ("全部保存", "Save all"),
            Self::DontSave => ("不保存", "Don't save"),
            Self::ReturnToEditing => ("返回编辑", "Return to editing"),
            Self::UnsavedChanges => ("未保存的更改", "Unsaved changes"),
            Self::Cancel => ("取消", "Cancel"),
            Self::SaveAnyway => ("仍然保存", "Save anyway"),
            Self::SaveProblemsTitle => ("保存存在问题的 CSV？", "Save CSV with problems?"),
            Self::FileChangedTitle => ("文件已在磁盘上更改", "File changed on disk"),
            Self::OverwriteDiskFile => ("覆盖磁盘文件", "Overwrite disk file"),
            Self::ReloadDiskFile => ("重新加载磁盘文件", "Reload disk file"),
            Self::DiscardLocalTitle => ("放弃本地更改？", "Discard local changes?"),
            Self::DiscardAndReload => ("放弃并重新加载", "Discard and reload"),
            Self::ConvertEncodingTitle => ("转换 CSV 编码？", "Convert CSV encoding?"),
            Self::ConvertToUtf8 => ("转换为 UTF-8", "Convert to UTF-8"),
            Self::ShortcutGlobal => ("全局", "Global"),
            Self::ShortcutFileSearch => ("文件与搜索", "Files and search"),
            Self::ShortcutTableNavigation => ("表格导航", "Table navigation"),
            Self::ShortcutEditing => ("编辑", "Editing"),
            Self::ShortcutShowList => ("显示快捷键", "Show shortcuts"),
            Self::ShortcutToggleSidebar => ("显示或隐藏侧栏", "Show or hide sidebar"),
            Self::ShortcutNextTab => ("切换到下一个标签页", "Switch to next tab"),
            Self::ShortcutPreviousTab => ("切换到上一个标签页", "Switch to previous tab"),
            Self::ShortcutCloseOverlay => (
                "关闭面板、取消编辑或退出聚焦",
                "Close panel, cancel edit, or exit focus",
            ),
            Self::ShortcutFilePalette => ("快速打开文件", "Quick open file"),
            Self::ShortcutGoToLine => ("跳转到行", "Go to line"),
            Self::ShortcutCurrentSearch => ("搜索当前文件", "Search current file"),
            Self::ShortcutGlobalSearch => ("搜索工作区", "Search workspace"),
            Self::ShortcutSave => ("保存当前文件", "Save current file"),
            Self::ShortcutCloseDocument => ("关闭标签页或预览", "Close tab or preview"),
            Self::ShortcutMoveCell => ("移动单元格选择", "Move cell selection"),
            Self::ShortcutEditCell => ("编辑选中单元格", "Edit selected cell"),
            Self::ShortcutToggleFocus => ("进入或退出列聚焦", "Enter or exit column focus"),
            Self::ShortcutMoveFocus => ("切换聚焦列", "Move focused column"),
            Self::ShortcutNextProblem => ("下一个问题", "Next problem"),
            Self::ShortcutPreviousProblem => ("上一个问题", "Previous problem"),
            Self::ShortcutCopy => ("复制单元格值", "Copy cell value"),
            Self::ShortcutPaste => ("粘贴单元格值", "Paste cell value"),
            Self::ShortcutUndo => ("撤销", "Undo"),
            Self::ShortcutRedo => ("重做", "Redo"),
            Self::ShortcutCommitDown => ("提交并移动到下一行", "Commit and move down"),
            Self::ShortcutCommitAcross => ("提交并横向移动", "Commit and move across"),
            Self::ShortcutInsertLineBreak => ("插入换行", "Insert a line break"),
            Self::ShortcutIndentJson => ("在 JSON 中缩进", "Indent JSON"),
            Self::ShortcutCancelEdit => ("取消编辑", "Cancel edit"),
            Self::TabLimit => (
                "已达到 20 个标签页的上限。请先关闭一个标签页再打开其他文件。",
                "The 20-tab limit has been reached. Close a tab before opening another file.",
            ),
            Self::OpenTabMissing => ("打开的标签页已不存在", "Open tab no longer exists"),
            Self::PreviewClosed => ("CSV 预览已关闭", "CSV preview is no longer open"),
            Self::NoWorkspaceOpen => ("当前没有打开工作区", "No workspace is open"),
            Self::OutsideWorkspace => (
                "CSV 文件不在当前工作区内",
                "CSV file is outside the current workspace",
            ),
            Self::EditedCellMissing => ("编辑的单元格已不存在", "Edited cell no longer exists"),
            Self::SelectCellBeforePaste => (
                "请先选择表格单元格再粘贴",
                "Select a table cell before pasting",
            ),
            Self::PastedCell => ("已粘贴单元格值", "Pasted cell value"),
            Self::ClipboardSame => (
                "剪贴板内容与选中单元格相同",
                "Clipboard value matches the selected cell",
            ),
            Self::CopiedCell => ("已复制单元格值", "Copied cell value"),
            Self::NothingSelectedCopy => ("没有可复制的选中内容", "Nothing selected to copy"),
            Self::RedidEdit => ("已重做上一次编辑", "Redid the last edit"),
            Self::UndidEdit => ("已撤销上一次编辑", "Undid the last edit"),
            Self::NothingRedo => ("没有可重做的操作", "Nothing to redo"),
            Self::NothingUndo => ("没有可撤销的操作", "Nothing to undo"),
            Self::AnalysisRunning => ("表格分析仍在进行", "Table analysis is still running"),
            Self::AnalysisRunningSave => (
                "表格分析仍在进行；请在完成后再次保存",
                "Table analysis is still running; save again when it finishes",
            ),
            Self::NoProblems => (
                "没有单元格问题或混合类型列",
                "No cell problems or mixed columns",
            ),
            Self::NoSearchableFile => ("当前没有可搜索的文件", "No searchable file is open"),
            Self::LineStartsAtOne => ("行号从 1 开始", "Line numbers start at 1"),
            Self::TableNoColumns => ("此表格没有列", "This table has no columns"),
            Self::TextChangedParsing => (
                "解析过程中内容已更改；请再次切换到表格视图",
                "Text changed while parsing; switch to Table again",
            ),
            Self::DiscardDescription => (
                "重新加载将永久放弃此标签页中所有未保存的编辑。",
                "Reloading will permanently discard all unsaved edits in this tab.",
            ),
            Self::SettingsError => ("设置错误", "Settings error"),
            Self::ScanError => ("扫描失败", "Scan failed"),
            Self::OpenError => ("打开失败", "Open failed"),
            Self::SaveError => ("保存失败", "Save failed"),
            Self::ReloadError => ("重新加载失败", "Reload failed"),
            Self::StartupError => ("启动失败", "Startup failed"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseAction {
    Window,
    Workspace,
}

pub enum Message<'a> {
    Opening(&'a str),
    Parsing(&'a str),
    Saved(&'a str),
    Reloaded(&'a str),
    Overwrote(&'a str),
    KeptLocal(&'a str),
    ExternalConflict(&'a str),
    ExternalReloaded(&'a str),
    ReloadFailed {
        file: &'a str,
        detail: &'a str,
    },
    DiskParseFailed(&'a str),
    Revealed(&'a str),
    RevealFailed {
        file: &'a str,
        detail: &'a str,
    },
    TableAnalysisFailed {
        file: &'a str,
        detail: &'a str,
    },
    MatchPosition {
        current: usize,
        total: usize,
    },
    LineOutOfFile {
        line: usize,
        lines: usize,
    },
    RowOutOfTable {
        row: usize,
        rows: usize,
    },
    RowOutOfPreview {
        row: usize,
        rows: usize,
    },
    ProblemAt {
        row: usize,
        column: usize,
    },
    MixedColumn(usize),
    FocusMode {
        column: usize,
        field: Option<&'a str>,
    },
    FocusModeTooltip {
        column: usize,
        field: Option<&'a str>,
    },
    HeaderRange {
        minimum: usize,
        maximum: usize,
    },
    InvalidHeaderRecords(usize),
    RowsColumns {
        rows: usize,
        columns: usize,
    },
    RequiresHeaderRows(usize),
    TablePosition {
        row: usize,
        column: usize,
    },
    TextPosition {
        line: usize,
        column: usize,
    },
    UnsavedFiles {
        count: usize,
        action: CloseAction,
    },
    SaveChanges(&'a str),
    ReloadDiscard(&'a str),
    ConvertGb18030(&'a str),
    FileChanged(&'a str),
    SaveParseProblems {
        count: usize,
        detail: &'a str,
    },
    SaveCellProblems(usize),
    StructuralMismatch(&'a str),
    Technical {
        prefix: Text,
        detail: &'a str,
    },
}

pub fn message(value: Message<'_>) -> String {
    message_for(language(), value)
}

pub fn message_for(language: Language, value: Message<'_>) -> String {
    let zh = language == Language::Zh;
    match value {
        Message::Opening(file) => {
            if zh {
                format!("正在打开 {file}...")
            } else {
                format!("Opening {file}...")
            }
        }
        Message::Parsing(file) => {
            if zh {
                format!("正在解析 {file}...")
            } else {
                format!("Parsing {file}...")
            }
        }
        Message::Saved(file) => {
            if zh {
                format!("已保存 {file}")
            } else {
                format!("Saved {file}")
            }
        }
        Message::Reloaded(file) => {
            if zh {
                format!("已重新加载 {file}")
            } else {
                format!("Reloaded {file}")
            }
        }
        Message::Overwrote(file) => {
            if zh {
                format!("已覆盖 {file}")
            } else {
                format!("Overwrote {file}")
            }
        }
        Message::KeptLocal(file) => {
            if zh {
                format!("已保留 {file} 的本地编辑")
            } else {
                format!("Kept local edits for {file}")
            }
        }
        Message::ExternalConflict(file) => {
            if zh {
                format!("{file} 已在磁盘上更改，同时存在未保存的编辑")
            } else {
                format!("{file} changed on disk while it has unsaved edits")
            }
        }
        Message::ExternalReloaded(file) => {
            if zh {
                format!("检测到外部更改，已重新加载 {file}")
            } else {
                format!("Reloaded {file} after an external change")
            }
        }
        Message::ReloadFailed { file, detail } => {
            if zh {
                format!("无法重新加载 {file}: {detail}")
            } else {
                format!("Could not reload {file}: {detail}")
            }
        }
        Message::DiskParseFailed(file) => {
            if zh {
                format!("无法解析 {file} 的磁盘版本。")
            } else {
                format!("Disk version of {file} could not be parsed.")
            }
        }
        Message::Revealed(file) => {
            if zh {
                format!("已在系统文件管理器中显示 {file}")
            } else {
                format!("Opened {file} in the system file manager")
            }
        }
        Message::RevealFailed { file, detail } => {
            if zh {
                format!("无法在系统文件管理器中显示 {file}: {detail}")
            } else {
                format!("Could not show {file} in the system file manager: {detail}")
            }
        }
        Message::TableAnalysisFailed { file, detail } => {
            if zh {
                format!("{file} 的表格分析失败: {detail}")
            } else {
                format!("Table analysis failed for {file}: {detail}")
            }
        }
        Message::MatchPosition { current, total } => {
            if zh {
                format!("第 {current} 个匹配项，共 {total} 个")
            } else {
                format!("Match {current} of {total}")
            }
        }
        Message::LineOutOfFile { line, lines } => {
            if zh {
                format!("第 {line} 行超出文件范围（共 {lines} 个物理行）")
            } else {
                format!("Line {line} is outside this file ({lines} physical lines)")
            }
        }
        Message::RowOutOfTable { row, rows } => {
            if zh {
                format!("第 {row} 行超出表格范围（共 {rows} 个数据行）")
            } else {
                format!("Row {row} is outside this table ({rows} data rows)")
            }
        }
        Message::RowOutOfPreview { row, rows } => {
            if zh {
                format!("第 {row} 行超出预览范围（共 {rows} 个数据行）")
            } else {
                format!("Row {row} is outside this preview ({rows} data rows)")
            }
        }
        Message::ProblemAt { row, column } => {
            if zh {
                format!("问题位于第 {row} 行、第 {column} 列")
            } else {
                format!("Problem at row {row}, column {column}")
            }
        }
        Message::MixedColumn(column) => {
            if zh {
                format!("第 {column} 列存在混合类型")
            } else {
                format!("Mixed types in column {column}")
            }
        }
        Message::FocusMode { column, field } => match (zh, field) {
            (true, Some(field)) => format!("聚焦模式 · 第 {column} 列 · {field}"),
            (true, None) => format!("聚焦模式 · 第 {column} 列"),
            (false, Some(field)) => format!("Focus mode · Column {column} · {field}"),
            (false, None) => format!("Focus mode · Column {column}"),
        },
        Message::FocusModeTooltip { column, field } => match (zh, field) {
            (true, Some(field)) => {
                format!("聚焦模式已开启：第 {column} 列（字段：{field}）。按 T 或 Esc 退出。")
            }
            (true, None) => format!("聚焦模式已开启：第 {column} 列。按 T 或 Esc 退出。"),
            (false, Some(field)) => format!(
                "Focus mode is active on column {column} (field: {field}). Press T or Esc to exit."
            ),
            (false, None) => {
                format!("Focus mode is active on column {column}. Press T or Esc to exit.")
            }
        },
        Message::HeaderRange { minimum, maximum } => {
            if zh {
                format!("表头行数必须在 {minimum} 到 {maximum} 之间")
            } else {
                format!("Header rows must be between {minimum} and {maximum}")
            }
        }
        Message::InvalidHeaderRecords(rows) => {
            if zh {
                format!("文件少于配置的 {rows} 个表头记录")
            } else {
                format!("file has fewer than the configured {rows} header records")
            }
        }
        Message::RowsColumns { rows, columns } => {
            if zh {
                format!("{rows} 行 · {columns} 列")
            } else {
                format!("{rows} rows · {columns} columns")
            }
        }
        Message::RequiresHeaderRows(rows) => {
            if zh {
                format!("需要 {rows} 个表头行")
            } else {
                format!("requires {rows} header rows")
            }
        }
        Message::TablePosition { row, column } => {
            if zh {
                format!("第 {row} 行，第 {column} 列")
            } else {
                format!("Row {row}, Col {column}")
            }
        }
        Message::TextPosition { line, column } => {
            if zh {
                format!("第 {line} 行，第 {column} 列")
            } else {
                format!("Ln {line}, Col {column}")
            }
        }
        Message::UnsavedFiles { count, action } => match (zh, action) {
            (true, CloseAction::Window) => {
                format!("{count} 个打开的文件有未保存更改。关闭窗口前是否全部保存？")
            }
            (true, CloseAction::Workspace) => {
                format!("{count} 个打开的文件有未保存更改。切换工作区前是否全部保存？")
            }
            (false, CloseAction::Window) => format!(
                "{count} open files have unsaved changes. Save all before closing the window?"
            ),
            (false, CloseAction::Workspace) => format!(
                "{count} open files have unsaved changes. Save all before switching workspaces?"
            ),
        },
        Message::SaveChanges(file) => {
            if zh {
                format!("关闭前是否保存对 {file} 的更改？")
            } else {
                format!("Save changes to {file} before closing?")
            }
        }
        Message::ReloadDiscard(file) => {
            if zh {
                format!("重新加载 {file} 将永久放弃此标签页中所有未保存的编辑。")
            } else {
                format!("Reloading {file} will permanently discard all unsaved edits in this tab.")
            }
        }
        Message::ConvertGb18030(file) => {
            if zh {
                format!(
                    "{file} 使用 GB18030 编码。编辑前需要在内存中转换为 UTF-8；文件会在保存时才写为无 BOM UTF-8。是否继续？"
                )
            } else {
                format!(
                    "{file} uses GB18030. Editing requires an in-memory UTF-8 conversion; the file will only be written as UTF-8 without BOM when you save. Continue?"
                )
            }
        }
        Message::FileChanged(file) => {
            if zh {
                format!("{file} 在打开后已被更改。要覆盖磁盘文件、重新加载还是取消？")
            } else {
                format!(
                    "{file} changed after it was opened. Overwrite the disk file, reload it, or cancel?"
                )
            }
        }
        Message::SaveParseProblems { count, detail } => {
            if zh {
                format!(
                    "文本包含 {count} 个 CSV 解析错误。第一个错误: {detail}\n\n仍然保存将保留无效的 CSV 文本。"
                )
            } else {
                format!(
                    "This text contains {count} CSV parse error(s). First error: {detail}\n\nSaving anyway will preserve the invalid CSV text."
                )
            }
        }
        Message::SaveCellProblems(count) => {
            if zh {
                format!("文件包含 {count} 个存在红色兼容性或结构问题的单元格。")
            } else {
                format!(
                    "This file contains {count} cells with red compatibility or structure problems."
                )
            }
        }
        Message::StructuralMismatch(expected) => {
            if zh {
                format!("与该列推断出的 {expected} 结构不匹配")
            } else {
                format!("Does not match the column's inferred {expected} structure")
            }
        }
        Message::Technical { prefix, detail } => format!("{}: {detail}", language.text(prefix)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutGroup {
    Global,
    FileSearch,
    TableNavigation,
    Editing,
}

impl ShortcutGroup {
    pub fn title(self) -> &'static str {
        text(match self {
            Self::Global => Text::ShortcutGlobal,
            Self::FileSearch => Text::ShortcutFileSearch,
            Self::TableNavigation => Text::ShortcutTableNavigation,
            Self::Editing => Text::ShortcutEditing,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutEntry {
    pub group: ShortcutGroup,
    pub keys: &'static str,
    pub description: Text,
}

pub const SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        group: ShortcutGroup::Global,
        keys: "F1",
        description: Text::ShortcutShowList,
    },
    ShortcutEntry {
        group: ShortcutGroup::Global,
        keys: "PRIMARY+B",
        description: Text::ShortcutToggleSidebar,
    },
    ShortcutEntry {
        group: ShortcutGroup::Global,
        keys: "Ctrl+Tab",
        description: Text::ShortcutNextTab,
    },
    ShortcutEntry {
        group: ShortcutGroup::Global,
        keys: "Ctrl+Shift+Tab",
        description: Text::ShortcutPreviousTab,
    },
    ShortcutEntry {
        group: ShortcutGroup::Global,
        keys: "Esc",
        description: Text::ShortcutCloseOverlay,
    },
    ShortcutEntry {
        group: ShortcutGroup::FileSearch,
        keys: "PRIMARY+P",
        description: Text::ShortcutFilePalette,
    },
    ShortcutEntry {
        group: ShortcutGroup::FileSearch,
        keys: "PRIMARY+G",
        description: Text::ShortcutGoToLine,
    },
    ShortcutEntry {
        group: ShortcutGroup::FileSearch,
        keys: "PRIMARY+F",
        description: Text::ShortcutCurrentSearch,
    },
    ShortcutEntry {
        group: ShortcutGroup::FileSearch,
        keys: "PRIMARY+Shift+F",
        description: Text::ShortcutGlobalSearch,
    },
    ShortcutEntry {
        group: ShortcutGroup::FileSearch,
        keys: "PRIMARY+S",
        description: Text::ShortcutSave,
    },
    ShortcutEntry {
        group: ShortcutGroup::FileSearch,
        keys: "PRIMARY+W",
        description: Text::ShortcutCloseDocument,
    },
    ShortcutEntry {
        group: ShortcutGroup::TableNavigation,
        keys: "Arrow keys",
        description: Text::ShortcutMoveCell,
    },
    ShortcutEntry {
        group: ShortcutGroup::TableNavigation,
        keys: "Enter / F2",
        description: Text::ShortcutEditCell,
    },
    ShortcutEntry {
        group: ShortcutGroup::TableNavigation,
        keys: "T",
        description: Text::ShortcutToggleFocus,
    },
    ShortcutEntry {
        group: ShortcutGroup::TableNavigation,
        keys: "A / D / Left / Right",
        description: Text::ShortcutMoveFocus,
    },
    ShortcutEntry {
        group: ShortcutGroup::TableNavigation,
        keys: "F8",
        description: Text::ShortcutNextProblem,
    },
    ShortcutEntry {
        group: ShortcutGroup::TableNavigation,
        keys: "Shift+F8",
        description: Text::ShortcutPreviousProblem,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "PRIMARY+C",
        description: Text::ShortcutCopy,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "PRIMARY+V",
        description: Text::ShortcutPaste,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "PRIMARY+Z",
        description: Text::ShortcutUndo,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "REDO",
        description: Text::ShortcutRedo,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "Enter",
        description: Text::ShortcutCommitDown,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "Tab / Shift+Tab",
        description: Text::ShortcutCommitAcross,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "PRIMARY+Enter",
        description: Text::ShortcutInsertLineBreak,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "Tab",
        description: Text::ShortcutIndentJson,
    },
    ShortcutEntry {
        group: ShortcutGroup::Editing,
        keys: "Esc",
        description: Text::ShortcutCancelEdit,
    },
];

pub fn shortcut_keys(keys: &str, macos: bool) -> String {
    let primary = if macos { "Command" } else { "Ctrl" };
    let redo = if macos { "Command+Shift+Z" } else { "Ctrl+Y" };
    keys.replace("PRIMARY", primary).replace("REDO", redo)
}

pub fn count(label: Count, value: usize) -> String {
    count_for(language(), label, value)
}

pub fn count_for(language: Language, label: Count, value: usize) -> String {
    match (language, label) {
        (Language::Zh, Count::Files) => format!("{value} 个文件"),
        (Language::Zh, Count::CsvFiles) => format!("{value} 个 CSV 文件"),
        (Language::Zh, Count::ScanWarnings) => format!("{value} 个扫描警告"),
        (Language::Zh, Count::CsvErrors) => format!("{value} 个 CSV 错误"),
        (Language::Zh, Count::RedCells) => format!("{value} 个红色单元格"),
        (Language::Zh, Count::YellowColumns) => format!("{value} 个黄色列"),
        (Language::Zh, Count::Matches) => format!("{value} 个匹配项"),
        (Language::En, Count::Files) => format!("{value} files"),
        (Language::En, Count::CsvFiles) => format!("{value} CSV files"),
        (Language::En, Count::ScanWarnings) => format!("{value} scan warnings"),
        (Language::En, Count::CsvErrors) => format!("{value} CSV errors"),
        (Language::En, Count::RedCells) => format!("{value} red cells"),
        (Language::En, Count::YellowColumns) => format!("{value} yellow columns"),
        (Language::En, Count::Matches) => format!("{value} matches"),
    }
}

pub fn search_summary(loading: bool, truncated: bool, value: usize) -> String {
    match (language(), loading, truncated) {
        (Language::Zh, true, _) => format!("正在搜索 · {value}"),
        (Language::Zh, false, true) => format!("{value}+ 个匹配项"),
        (Language::Zh, false, false) => format!("{value} 个匹配项"),
        (Language::En, true, _) => format!("Searching · {value}"),
        (Language::En, false, true) => format!("{value}+ matches"),
        (Language::En, false, false) => format!("{value} matches"),
    }
}

pub fn physical_lines(value: usize) -> String {
    match language() {
        Language::Zh => format!("{value} 个物理行"),
        Language::En => format!("{value} physical lines"),
    }
}

pub fn records_columns(records: usize, columns: usize) -> String {
    match language() {
        Language::Zh => format!("{records} 条记录 · {columns} 列"),
        Language::En => format!("{records} records · {columns} columns"),
    }
}

pub fn header_requirement(header_rows: usize) -> String {
    match language() {
        Language::Zh => format!("表头配置需要 {header_rows} 行"),
        Language::En => format!("Header configuration requires {header_rows} records"),
    }
}

pub fn csv_parse_failed(error_count: usize) -> String {
    match language() {
        Language::Zh => format!("CSV 解析失败（{error_count} 个错误）"),
        Language::En => format!("CSV parse failed ({error_count})"),
    }
}

pub fn unsearchable_files(value: usize) -> String {
    match language() {
        Language::Zh => format!("{value} 个文件无法搜索"),
        Language::En => format!("{value} files could not be searched"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Count {
    Files,
    CsvFiles,
    ScanWarnings,
    CsvErrors,
    RedCells,
    YellowColumns,
    Matches,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_mapping_uses_chinese_only_for_zh_locales() {
        assert_eq!(Language::from_locale(Some("zh-CN")), Language::Zh);
        assert_eq!(Language::from_locale(Some("ZH_Hant_TW")), Language::Zh);
        assert_eq!(Language::from_locale(Some("en-US")), Language::En);
        assert_eq!(Language::from_locale(None), Language::En);
    }

    #[test]
    fn key_texts_are_available_in_both_languages() {
        assert_eq!(Language::Zh.text(Text::AppTitle), "游戏配置编辑器");
        assert_eq!(Language::En.text(Text::Shortcuts), "Shortcuts");
        assert_eq!(Language::Zh.text(Text::Comma), "逗号");
        assert_eq!(Language::Zh.text(Text::TypeLabel), "类型");
        assert_eq!(Language::Zh.text(Text::JsonArray2d), "二维 JSON 数组");
        assert_eq!(
            Language::Zh.text(Text::ProblemUnescapedQuote),
            "普通字符串包含未转义的双引号"
        );
        assert_eq!(
            Language::En.text(Text::ShortcutInsertLineBreak),
            "Insert a line break"
        );
        assert_eq!(
            Language::En.text(Text::PositiveLineAfterColon),
            "Enter a positive line number after ':'"
        );
    }

    #[test]
    fn shortcut_labels_are_platform_specific_except_tab_switching() {
        assert_eq!(shortcut_keys("PRIMARY+S", false), "Ctrl+S");
        assert_eq!(shortcut_keys("PRIMARY+S", true), "Command+S");
        assert_eq!(shortcut_keys("Ctrl+Tab", true), "Ctrl+Tab");
        assert_eq!(shortcut_keys("REDO", true), "Command+Shift+Z");
    }

    #[test]
    fn shortcut_catalog_covers_every_group() {
        for group in [
            ShortcutGroup::Global,
            ShortcutGroup::FileSearch,
            ShortcutGroup::TableNavigation,
            ShortcutGroup::Editing,
        ] {
            assert!(SHORTCUTS.iter().any(|entry| entry.group == group));
        }
        assert!(SHORTCUTS.len() >= 26);
    }

    #[test]
    fn dynamic_counts_are_localized() {
        assert_eq!(count_for(Language::Zh, Count::Files, 12), "12 个文件");
        assert_eq!(count_for(Language::En, Count::CsvErrors, 3), "3 CSV errors");
        assert_eq!(
            message_for(Language::Zh, Message::TablePosition { row: 2, column: 4 }),
            "第 2 行，第 4 列"
        );
        assert_eq!(
            message_for(Language::En, Message::StructuralMismatch("json")),
            "Does not match the column's inferred json structure"
        );
        assert_eq!(
            message_for(
                Language::Zh,
                Message::FocusMode {
                    column: 3,
                    field: Some("name")
                }
            ),
            "聚焦模式 · 第 3 列 · name"
        );
        assert_eq!(
            message_for(
                Language::En,
                Message::FocusMode {
                    column: 2,
                    field: None
                }
            ),
            "Focus mode · Column 2"
        );
        assert!(
            message_for(
                Language::Zh,
                Message::FocusModeTooltip {
                    column: 1,
                    field: Some("id")
                }
            )
            .contains("按 T 或 Esc 退出")
        );
    }
}
