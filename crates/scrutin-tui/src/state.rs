//! TUI state types and AppState.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crossterm::event::{KeyEvent, MouseEvent};

use scrutin_core::engine::pool::{BusyCounter, CancelHandle};
use scrutin_core::engine::run_events::FileResult;
use scrutin_core::git::GitAvailability;
use scrutin_core::logbuf::LogBuffer;
use scrutin_core::project::package::Package;

use super::build_reverse_dep_map;

// ── Responsive layout constants ──
//
// Single source of truth for the breakpoints the view layer uses to decide
// when to split panes, drop chrome, or refuse to render at all.

/// Minimum usable width for the file/test list pane.
pub(super) const MIN_LIST_COLS: u16 = 28;
/// Minimum usable width for the source/main pane (gutter + a few words).
pub(super) const MIN_MAIN_COLS: u16 = 40;
/// Hide the right-aligned per-file detail column below this width.
pub(super) const FILE_DETAIL_MIN_COLS: u16 = 50;
/// Drop the bottom hints bar when total terminal height is below this.
pub(super) const HINTS_BAR_MIN_ROWS: u16 = 20;
/// Drop the counts bar when total terminal height is below this.
pub(super) const COUNTS_BAR_MIN_ROWS: u16 = 15;
/// Below this size the TUI renders a "terminal too small" notice instead.
pub(super) const MIN_TERMINAL_COLS: u16 = 30;
pub(super) const MIN_TERMINAL_ROWS: u16 = 10;

// ── Data types ──

#[derive(Clone, PartialEq)]
pub(super) enum FileStatus {
    Pending,
    Running,
    Passed {
        passed: u32,
        warned: u32,
        ms: u64,
    },
    Failed {
        passed: u32,
        failed: u32,
        errored: u32,
        warned: u32,
        ms: u64,
    },
    Skipped {
        skipped: u32,
        ms: u64,
    },
    Cancelled,
}

impl FileStatus {
    /// Extract the counts embedded in this status as a `Counts` struct
    /// (for subtracting from run totals on rerun).
    pub(super) fn counts(&self) -> scrutin_core::engine::protocol::Counts {
        use scrutin_core::engine::protocol::Counts;
        match *self {
            FileStatus::Failed { passed, failed, errored, warned, .. } => Counts {
                pass: passed, fail: failed, error: errored, warn: warned,
                ..Default::default()
            },
            FileStatus::Passed { passed, warned, .. } => Counts {
                pass: passed, warn: warned,
                ..Default::default()
            },
            FileStatus::Skipped { skipped, .. } => Counts {
                skip: skipped,
                ..Default::default()
            },
            _ => Counts::default(),
        }
    }
}

pub(super) struct FileEntry {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) status: FileStatus,
    pub(super) tests: Vec<TestEntry>,
    /// Plugin/suite name (e.g. "testthat", "pytest", "pointblank") that owns
    /// this file. Set once at construction by `Package::suite_for` so the
    /// plugin filter doesn't have to re-walk the suite list every frame.
    /// Empty if no suite owned the path (shouldn't happen in practice).
    pub(super) suite: String,
    /// Per-file rerun attempt counter. 0 means "this file has never been
    /// retried"; 1+ means "this file is on attempt N+1 of the rerun loop."
    /// Cleared on a fresh run; set by `reset_for_run` when `attempt > 0`.
    pub(super) attempt: u32,
    /// True once a passing run on attempt > 1 has cleared a previous
    /// failure for this file. Surfaces in the file row as a yellow flake
    /// marker so users can see *which* files are flaky at a glance.
    pub(super) flaky: bool,
}

pub(super) type TestEntry = scrutin_core::engine::protocol::ProcessedEvent;

pub(super) type FailureInfo = scrutin_core::engine::protocol::Finding;

// ── Modes ──
//
// `Mode` historically conflated two orthogonal concepts:
//   1. drill *level* (Normal/Detail/Failure)         \u2014 "where am I"
//   2. transient *overlay* (Help/Log/ActionOutput/Palette) \u2014 "what's on top"
//
// `Level` and `Overlay` separate these so dispatch sites can target the
// axis they actually care about. `Mode` is kept as a derived projection
// of `(level, overlay)` so existing dispatch keeps working; new code
// should prefer `state.level()` / `state.overlay_kind()` accessors.

/// The three drill-down navigation levels. Mutually exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Level {
    Normal,
    Detail,
    Failure,
}

/// Transient overlay sitting on top of the current `Level`. Optional.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Overlay {
    Help,
    Log,
    Palette(super::keymap::PaletteKind),
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum Mode {
    Normal,
    Detail,       // viewing tests within a file
    Failure,      // full failure view with source
    Help,         // keybinding overlay
    Log,          // subprocess stderr + internal messages
    /// Unified command palette (filter input, run menu, config menu).
    Palette(super::keymap::PaletteKind),
}

impl Mode {
    /// Project a `Mode` onto the two-axis (level, overlay) decomposition.
    /// Inverse of the projection: `Mode::Help.level() == Level::Normal`
    /// because Help is an overlay, not a level.
    pub(super) fn level(&self) -> Level {
        match self {
            Mode::Normal | Mode::Help | Mode::Log | Mode::Palette(_) => Level::Normal,
            Mode::Detail  => Level::Detail,
            Mode::Failure => Level::Failure,
        }
    }

    /// Returns Some if this mode is an overlay layer rather than a drill
    /// level. None for Normal/Detail/Failure.
    pub(super) fn overlay_kind(&self) -> Option<Overlay> {
        match self {
            Mode::Help       => Some(Overlay::Help),
            Mode::Log        => Some(Overlay::Log),
            Mode::Palette(k) => Some(Overlay::Palette(*k)),
            _ => None,
        }
    }
}

/// Shared state for overlay modes (Help + Run/Sort palettes). Content
/// is rendered directly from `AppState` (the help overlay walks the
/// default keymap; run/sort palettes derive their rows from run_groups
/// / sort-mode tables), so this struct only tracks cursor / scroll /
/// viewport bookkeeping.
pub(super) struct OverlayState {
    /// Scroll offset for text overlays.
    pub scroll: usize,
    /// Viewport height (set each frame by the draw function).
    pub view_height: usize,
    /// Menu cursor position. `None` for read-only text overlays.
    pub cursor: Option<usize>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            scroll: 0,
            view_height: 20,
            cursor: None,
        }
    }
}

impl OverlayState {
    /// Reset for a new text overlay (scrollable, no cursor).
    pub fn text() -> Self {
        Self::default()
    }

    /// Reset for a new menu overlay (cursor, no scroll).
    pub fn menu() -> Self {
        Self {
            scroll: 0,
            view_height: 20,
            cursor: Some(0),
        }
    }

    /// Scroll text overlay up/down.
    pub fn scroll_by(&mut self, down: bool, step: usize) {
        if down {
            self.scroll = self.scroll.saturating_add(step);
        } else {
            self.scroll = self.scroll.saturating_sub(step);
        }
    }

    /// Move menu cursor up/down, clamped to item count.
    pub fn move_cursor(&mut self, down: bool, step: usize, n_items: usize) {
        if n_items == 0 {
            return;
        }
        if let Some(ref mut c) = self.cursor {
            if down {
                *c = (*c + step).min(n_items - 1);
            } else {
                *c = c.saturating_sub(step);
            }
        }
    }

    /// Current cursor position (for menus).
    pub fn cursor_pos(&self) -> usize {
        self.cursor.unwrap_or(0)
    }

    /// Mutable cursor position (for menus). Panics if not a menu overlay.
    pub fn cursor_mut(&mut self) -> &mut usize {
        self.cursor.as_mut().expect("overlay is not a menu")
    }
}

/// A named filter group exposed in the run menu (mirrors `[filter.groups.*]`).
#[derive(Clone)]
pub struct RunGroup {
    pub name: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Three preset list-pane percentages cycled with `+` / `_`. Continuous
/// adjustment with `(` / `)` overrides these. `Full` (= 0%) hides the
/// list pane entirely and shows only the main pane.
pub(super) const LIST_PCT_NORMAL: u16 = 45;
pub(super) const LIST_PCT_STEP: u16 = 5;
pub(super) const LIST_PCT_MAX: u16 = 95;

/// Lazygit-style status filter on the file list, cycled with [ and ].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusFilter {
    All,
    Failed,
    Passed,
    Skipped,
    Xfailed,
    Warned,
    Running,
}

impl StatusFilter {
    /// Every variant in display order. `cycle_next`/`cycle_prev` and the
    /// view-layer chip rendering walk this in lockstep with `supported`.
    const ALL: &'static [Self] = &[
        Self::All,
        Self::Failed,
        Self::Passed,
        Self::Skipped,
        Self::Xfailed,
        Self::Warned,
        Self::Running,
    ];

    pub(super) fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
    pub(super) fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// Cycle forward, skipping any variant whose backing outcome isn't
    /// supported by any plugin in the current package. `All` and `Running`
    /// are always supported (they're meta-filters, not outcome-bound).
    pub(super) fn next_supported(
        self,
        supported: &std::collections::HashSet<scrutin_core::engine::protocol::Outcome>,
    ) -> Self {
        let mut cur = self.next();
        for _ in 0..Self::ALL.len() {
            if cur.is_supported(supported) {
                return cur;
            }
            cur = cur.next();
        }
        Self::All
    }
    pub(super) fn prev_supported(
        self,
        supported: &std::collections::HashSet<scrutin_core::engine::protocol::Outcome>,
    ) -> Self {
        let mut cur = self.prev();
        for _ in 0..Self::ALL.len() {
            if cur.is_supported(supported) {
                return cur;
            }
            cur = cur.prev();
        }
        Self::All
    }
    pub(super) fn is_supported(
        self,
        supported: &std::collections::HashSet<scrutin_core::engine::protocol::Outcome>,
    ) -> bool {
        use scrutin_core::engine::protocol::Outcome;
        match self {
            Self::All | Self::Running => true,
            Self::Failed => supported.contains(&Outcome::Fail) || supported.contains(&Outcome::Error),
            Self::Passed => supported.contains(&Outcome::Pass),
            Self::Skipped => supported.contains(&Outcome::Skip),
            Self::Xfailed => supported.contains(&Outcome::Xfail),
            Self::Warned => supported.contains(&Outcome::Warn),
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Failed => "failed",
            Self::Passed => "passed",
            Self::Skipped => "skipped",
            Self::Xfailed => "xfail",
            Self::Warned => "warn",
            Self::Running => "running",
        }
    }
}

/// How to sort the file list. Matches the web's sort modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SortMode {
    Sequential, // original order (default)
    Status,     // failures first (warns separated from clean passes)
    Name,       // alphabetical
    Suite,      // by suite name
    Time,       // slowest first
}

impl SortMode {
    pub(super) const ALL: &[SortMode] = &[
        Self::Sequential, Self::Status, Self::Name, Self::Suite, Self::Time,
    ];

    /// Short identifier shown in palette rows.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Status     => "status",
            Self::Name       => "name",
            Self::Suite      => "suite",
            Self::Time       => "time",
        }
    }

    /// One-line human-friendly description of what the sort does.
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Sequential => "original order",
            Self::Status     => "failures first",
            Self::Name       => "alphabetical",
            Self::Suite      => "by suite",
            Self::Time       => "slowest first",
        }
    }
}

/// Filter the file list by which plugin/suite owns each file. The list of
/// suite names is captured once at startup from `Package::test_suites`; the
/// `current` index walks `[All, suite[0], suite[1], …]` with `All` at -1
/// modeled as `None`.
#[derive(Clone, Debug)]
pub(super) struct SuiteFilter {
    /// Suite names in detection order (e.g. ["tinytest", "testthat", "pytest"]).
    pub(super) suites: Vec<String>,
    /// `None` = All; `Some(i)` = `suites[i]`.
    pub(super) current: Option<usize>,
}

impl SuiteFilter {
    pub(super) fn new(suites: Vec<String>) -> Self {
        Self { suites, current: None }
    }
    pub(super) fn label(&self) -> &str {
        match self.current {
            None => "all",
            Some(i) => self.suites.get(i).map(|s| s.as_str()).unwrap_or("all"),
        }
    }
    /// Cycle: All → 0 → 1 → … → All
    pub(super) fn cycle_next(&mut self) {
        if self.suites.is_empty() {
            return;
        }
        self.current = match self.current {
            None => Some(0),
            Some(i) if i + 1 < self.suites.len() => Some(i + 1),
            Some(_) => None,
        };
    }
    pub(super) fn cycle_prev(&mut self) {
        if self.suites.is_empty() {
            return;
        }
        self.current = match self.current {
            None => Some(self.suites.len() - 1),
            Some(0) => None,
            Some(i) => Some(i - 1),
        };
    }
    /// True iff the filter has multiple values to cycle through (i.e. the
    /// project has more than one active suite). The view layer hides the
    /// chip and the cycler is a no-op when this is false.
    pub(super) fn is_meaningful(&self) -> bool {
        self.suites.len() > 1
    }
    /// Does this filter accept a file owned by `suite_name`?
    pub(super) fn accepts(&self, suite_name: &str) -> bool {
        match self.current {
            None => true,
            Some(i) => self.suites.get(i).map(|s| s == suite_name).unwrap_or(true),
        }
    }
}

// ── App State ──

/// File-list multi-selection. Keyed by absolute path so the set survives
/// filter changes and snapshot reloads (the `file_cursor` index doesn't).
#[derive(Default)]
pub(super) struct MultiSelectState {
    pub selected: HashSet<PathBuf>,
    /// Visible-list index where visual-mode selection started. `None` when
    /// not in visual mode.
    pub visual_anchor: Option<usize>,
    /// Snapshot of `selected` at visual-mode entry, so a visual-mode drag
    /// is computed as anchor-to-cursor on top of this base.
    pub visual_base: HashSet<PathBuf>,
}

/// All file-list filtering state. The four `*_filter` fields are
/// orthogonal: a file shows up only if every filter accepts it.
pub(super) struct FilterState {
    /// Live filter palette text input (only relevant while typing).
    pub input: String,
    /// Confirmed filter pattern; None = no filter applied.
    pub active: Option<String>,
    /// Saved filter pattern before entering filter mode; restored on Esc.
    pub pre_filter: Option<String>,
    /// Status chip cycler (Failed/Errored/Warned/Passed/...).
    pub status: StatusFilter,
    /// Suite chip cycler (testthat/pytest/...).
    pub suite: SuiteFilter,
    /// Union of `Plugin::supported_outcomes()` across every active suite.
    /// Lets the status-filter cycler skip variants no plugin can produce.
    pub supported_outcomes: std::collections::HashSet<scrutin_core::engine::protocol::Outcome>,
}

impl FilterState {
    pub(super) fn new(suites: Vec<String>, supported_outcomes: std::collections::HashSet<scrutin_core::engine::protocol::Outcome>) -> Self {
        Self {
            input: String::new(),
            active: None,
            pre_filter: None,
            status: StatusFilter::All,
            suite: SuiteFilter::new(suites),
            supported_outcomes,
        }
    }
}

/// Display options that survive across runs (sort, watch, layout pct).
pub(super) struct DisplayState {
    pub sort_mode: SortMode,
    pub sort_reversed: bool,
    pub test_sort_mode: SortMode,
    pub test_sort_reversed: bool,
    pub show_duration_bars: bool,
    pub watch_active: bool,
    pub watch_paused: bool,
    /// List-pane percentage per drill level. Normal and Detail keep
    /// separate values so resizing one doesn't bleed into the other.
    pub normal_list_pct: u16,
    pub detail_list_pct: u16,
    /// Per-mode split orientation: false = vertical (panes side by side,
    /// the default), true = horizontal (panes stacked top/bottom).
    pub normal_horizontal: bool,
    pub detail_horizontal: bool,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self {
            sort_mode: SortMode::Status,
            sort_reversed: false,
            test_sort_mode: SortMode::Status,
            test_sort_reversed: false,
            show_duration_bars: true,
            watch_active: false,
            watch_paused: false,
            normal_list_pct: LIST_PCT_NORMAL,
            detail_list_pct: LIST_PCT_NORMAL,
            normal_horizontal: false,
            detail_horizontal: false,
        }
    }
}

/// All navigation state: drill-mode stack, per-mode cursor + scroll
/// positions, and viewport heights captured each frame for paging math.
pub(super) struct NavState {
    /// Drill-mode stack. Invariant: never empty; bottom is `Mode::Normal`.
    pub mode_stack: Vec<Mode>,
    pub file_cursor: usize,
    pub file_scroll: usize,
    pub test_cursor: usize,
    pub test_scroll: usize,
    pub failure_cursor: usize,
    pub failure_scroll: usize,
    /// Vertical line offset into the source pane (Normal/Detail).
    pub source_scroll: usize,
    /// Maximum valid value for `source_scroll`, recomputed each frame.
    /// Mouse-scroll handlers clamp against this so the wheel can't push
    /// past EOF.
    pub source_scroll_max: usize,
    /// Horizontal column offset for the source pane.
    pub source_hscroll: usize,
    /// Requested visible window for `load_source_context`. 0 = auto
    /// (use available height).
    pub source_context_lines: usize,
    pub log_scroll: usize,
    pub log_view_height: usize,
    /// Last-drawn viewport heights (rows), captured each frame so vim
    /// paging keys (Ctrl-d/u/f/b) scale with actual screen size.
    pub file_list_height: usize,
    pub test_list_height: usize,
    pub failure_view_height: usize,
}

impl Default for NavState {
    fn default() -> Self {
        Self {
            mode_stack: vec![Mode::Normal],
            file_cursor: 0,
            file_scroll: 0,
            test_cursor: 0,
            test_scroll: 0,
            failure_cursor: 0,
            failure_scroll: 0,
            source_scroll: 0,
            source_scroll_max: 0,
            source_hscroll: 0,
            source_context_lines: 0,
            log_scroll: 0,
            log_view_height: 20,
            file_list_height: 20,
            test_list_height: 20,
            failure_view_height: 20,
        }
    }
}

/// Live run state: in-flight flag, accumulated totals, timing, busy/cancel
/// handles. Re-initialized at the start of each run.
#[derive(Default)]
pub(super) struct RunState {
    pub running: bool,
    pub run_totals: scrutin_core::engine::protocol::Counts,
    /// Per-file durations from the last run; powers the "slowest file"
    /// summary in the counts bar.
    pub file_durations: Vec<(String, u64)>,
    pub last_run: Option<Instant>,
    pub last_duration: Option<Duration>,
    /// Rerun-loop attempt counter (0 = initial attempt only).
    pub current_attempt: u32,
    /// Tracks whether the most recently started run was an unfiltered
    /// full suite (decides whether to kick a background dep-map rebuild
    /// at completion).
    pub last_run_full_suite: bool,
    /// True while a background dep-map rebuild is in progress; prevents
    /// double-triggering.
    pub depmap_rebuilding: bool,
    /// Live busy-worker count from the pool; None when idle.
    pub busy_counter: Option<BusyCounter>,
    /// Cancel handle for the in-flight run; cleared when the run finishes.
    pub cancel: Option<CancelHandle>,
}

pub(super) struct AppState {
    pub(super) files: Vec<FileEntry>,
    pub(super) failures: Vec<FailureInfo>,
    pub(super) run: RunState,
    pub(super) pkg_name: String,
    pub(super) n_workers: usize,

    // Navigation — mode stack + cursors + scrolls + viewport heights.
    pub(super) nav: NavState,

    // Multi-selection (file list, by path so it survives filter changes/reloads)
    pub(super) multi: MultiSelectState,

    // Filter
    pub(super) filter: FilterState,

    // Display options (sort, watch, layout pct/orientation).
    pub(super) display: DisplayState,
    /// Default bindings generated from the shared keymap at startup.
    pub(super) default_bindings:
        std::collections::HashMap<Mode, Vec<super::keymap::Binding>>,
    /// Per-mode runtime keymap built from `[keymap.<mode>]` in .scrutin/config.toml.
    /// When a mode has an entry here, its bindings fully replace the defaults.
    pub(super) user_keymap:
        std::collections::HashMap<Mode, Vec<super::keymap::Binding>>,

    // Run / Config menus (navigable lists)
    pub(super) run_groups: Vec<RunGroup>,

    // Pauses the background stdin poller; set true while $EDITOR runs so vim
    // gets the keystrokes instead of scrutin's poll loop.
    pub(super) poll_paused: Option<Arc<AtomicBool>>,

    /// Reruns config from `[run]`. `rerun_max = 0` disables the loop.
    pub(super) rerun_max: u32,
    pub(super) rerun_delay_ms: u64,

    /// Per-file and whole-run timeouts (from config). Stored here so
    /// `start_test_run_inner` can read them without threading config
    /// through every call site.
    pub(super) timeout_file_ms: u64,
    pub(super) timeout_run_ms: u64,
    pub(super) fork_workers: bool,

    // Dep map for source function lookup (test_file → [source_files])
    pub(super) reverse_dep_map: std::collections::HashMap<String, Vec<String>>,
    /// Forward dep map (source_file → [test_files]) used by `resolve_tests`
    /// when running uncommitted-changes from the run menu. Mirrors the
    /// `dep_map` held by the watcher loop in `tui::mod`; updated together.
    pub(super) dep_map: Option<HashMap<String, Vec<String>>>,
    /// Cached git availability for the project root. Probed once at TUI
    /// startup; never re-probed (running `git init` mid-session is rare
    /// enough to require a TUI restart).
    pub(super) git: GitAvailability,
    pub(super) pkg_root: PathBuf,

    // Shared log buffer (subprocess stderr, pool/runner events).
    pub(super) log: LogBuffer,
    /// Shared overlay state for all overlay modes (Help, ActionOutput,
    /// Palette menus). Reset when pushing a new overlay mode.
    pub(super) overlay: OverlayState,

    /// Bounds of the scrollable panes drawn on the **last** frame, used by
    /// `handle_mouse` to route the wheel to whichever pane the cursor is
    /// over (rather than always driving the focused pane). Cleared at the
    /// top of each `draw()` and repopulated by the per-mode draw functions.
    pub(super) pane_rects: PaneRects,

    /// Plugin-defined actions keyed by suite name. Populated at startup
    /// from `Plugin::actions()`. The TUI checks the selected file's suite
    /// to decide which plugin keys are active.
    pub(super) suite_actions: HashMap<String, Vec<scrutin_core::project::plugin::PluginAction>>,

    /// Per-suite working directory, keyed by suite name. Used as the CWD
    /// when a plugin action is spawned so tool-specific configs
    /// (`ruff.toml`, `jarl.toml`, `pyproject.toml`) resolve against the
    /// right subtree in a monorepo.
    ///
    /// Denormalized from `Package.test_suites[_].root` so `AppState`
    /// doesn't need to hold an `Arc<Package>`. n_suites is small (at
    /// most a handful), so the map stays tiny.
    pub(super) suite_roots: HashMap<String, PathBuf>,
}

/// Per-frame snapshot of which pane occupies which screen rectangle. All
/// fields are `Option<Rect>` because not every pane is visible in every
/// mode (or at every terminal size).
#[derive(Default, Clone, Copy)]
pub(super) struct PaneRects {
    /// File list (Normal) or test list (Detail).
    pub list: Option<ratatui::layout::Rect>,
    /// Source / failure view (Normal, Detail).
    pub main: Option<ratatui::layout::Rect>,
    /// Full-screen log pane (Log mode).
    pub log: Option<ratatui::layout::Rect>,
    /// Error message pane (Failure mode).
    pub failure_error: Option<ratatui::layout::Rect>,
}

impl PaneRects {
    pub(super) fn hit(rect: Option<ratatui::layout::Rect>, col: u16, row: u16) -> bool {
        match rect {
            Some(r) => col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height,
            None => false,
        }
    }
}

impl AppState {
    pub(super) fn new(
        pkg: &Package,
        test_files: &[PathBuf],
        n_workers: usize,
        dep_map: &Option<std::collections::HashMap<String, Vec<String>>>,
        log: LogBuffer,
        rerun_max: u32,
        rerun_delay_ms: u64,
        timeout_file_ms: u64,
        timeout_run_ms: u64,
        fork_workers: bool,
        keymap_config: &std::collections::HashMap<
            String,
            std::collections::HashMap<String, String>,
        >,
    ) -> Self {
        let files = test_files
            .iter()
            .map(|p| FileEntry {
                name: scrutin_core::engine::protocol::file_display_name(p),
                path: p.clone(),
                status: FileStatus::Pending,
                tests: Vec::new(),
                suite: pkg
                    .suite_for(p)
                    .map(|s| s.plugin.name().to_string())
                    .unwrap_or_default(),
                attempt: 0,
                flaky: false,
            })
            .collect();

        let suite_names: Vec<String> = pkg
            .test_suites
            .iter()
            .map(|s| s.plugin.name().to_string())
            .collect();

        let mut supported_outcomes: std::collections::HashSet<
            scrutin_core::engine::protocol::Outcome,
        > = std::collections::HashSet::new();
        let mut suite_actions: HashMap<String, Vec<scrutin_core::project::plugin::PluginAction>> =
            HashMap::new();
        let mut suite_roots: HashMap<String, PathBuf> = HashMap::new();
        for suite in &pkg.test_suites {
            for o in suite.plugin.supported_outcomes() {
                supported_outcomes.insert(*o);
            }
            let actions = suite.plugin.actions();
            if !actions.is_empty() {
                suite_actions.insert(suite.plugin.name().to_string(), actions);
            }
            suite_roots.insert(suite.plugin.name().to_string(), suite.root.clone());
        }

        AppState {
            files,
            failures: Vec::new(),
            run: RunState::default(),
            pkg_name: pkg.name.clone(),
            n_workers,
            nav: NavState::default(),
            multi: MultiSelectState::default(),
            filter: FilterState::new(suite_names, supported_outcomes),
            display: DisplayState::default(),
            default_bindings: super::keymap::build_default_bindings(),
            user_keymap: super::keymap::build_user_keymap(keymap_config, |w| {
                log.push("scrutin", &format!("{w}\n"));
            }),
            run_groups: Vec::new(),
            poll_paused: None,
            rerun_max,
            rerun_delay_ms,
            timeout_file_ms,
            timeout_run_ms,
            fork_workers,
            reverse_dep_map: build_reverse_dep_map(dep_map),
            dep_map: dep_map.clone(),
            git: scrutin_core::git::detect_git(&pkg.root),
            pkg_root: pkg.root.clone(),
            log,
            overlay: OverlayState::default(),
            pane_rects: PaneRects::default(),
            suite_actions,
            suite_roots,
        }
    }

    /// Top of the mode stack. Stack invariant: never empty.
    pub(super) fn mode(&self) -> &Mode {
        self.nav.mode_stack.last().expect("mode_stack invariant: non-empty")
    }

    /// The active drill level, ignoring any overlay sitting on top.
    /// Walks the stack to find the topmost non-overlay frame so that
    /// e.g. opening Help on top of Detail still reports `Level::Detail`.
    pub(super) fn level(&self) -> Level {
        for m in self.nav.mode_stack.iter().rev() {
            if m.overlay_kind().is_none() {
                return m.level();
            }
        }
        Level::Normal
    }

    /// Push a new mode on top of the stack (drill-down). No-op if the
    /// requested mode already sits on top — prevents Help↔Log ping-pong
    /// from growing the stack unbounded.
    pub(super) fn push_mode(&mut self, m: Mode) {
        if self.nav.mode_stack.last() == Some(&m) {
            return;
        }
        self.nav.mode_stack.push(m);
    }

    /// Pop the top mode, restoring the previous one. Never leaves the
    /// stack empty — if only one frame remains, it's left in place.
    pub(super) fn pop_mode(&mut self) {
        if self.nav.mode_stack.len() > 1 {
            self.nav.mode_stack.pop();
        }
    }

    /// List-pane percentage for the active TUI mode (0 hides the list).
    pub(super) fn current_list_pct(&self) -> u16 {
        match self.mode() {
            Mode::Detail => self.display.detail_list_pct,
            _ => self.display.normal_list_pct,
        }
    }

    /// Set the list-pane percentage for the active TUI mode, clamped.
    pub(super) fn set_current_list_pct(&mut self, p: u16) {
        let p = p.min(LIST_PCT_MAX);
        match self.mode() {
            Mode::Detail => self.display.detail_list_pct = p,
            _ => self.display.normal_list_pct = p,
        }
    }

    /// Split orientation for the active TUI mode (false = vertical).
    pub(super) fn current_horizontal(&self) -> bool {
        match self.mode() {
            Mode::Detail => self.display.detail_horizontal,
            _ => self.display.normal_horizontal,
        }
    }

    /// Per-mode effective bindings: user override if present, else the
    /// static default. Returned as a slice so callers iterate without
    /// allocating.
    pub(super) fn effective_bindings(&self, mode: &Mode) -> &[super::keymap::Binding] {
        if let Some(v) = self.user_keymap.get(mode) {
            return v.as_slice();
        }
        self.default_bindings.get(mode).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Toggle the pane orientation (vertical/horizontal split) for whichever
    /// mode is currently on top of the stack.
    pub(super) fn toggle_current_horizontal(&mut self) {
        match self.mode() {
            Mode::Detail => self.display.detail_horizontal = !self.display.detail_horizontal,
            _ => self.display.normal_horizontal = !self.display.normal_horizontal,
        }
    }
    /// Full relevant viewport minus one line for context (at least 1).
    pub(super) fn full_page(h: usize) -> usize {
        h.saturating_sub(1).max(1)
    }

    /// Move the cursor appropriate to the current mode by `delta` lines.
    /// Positive = forward, negative = back. `isize::MAX` jumps to the
    /// bottom; `isize::MIN` jumps to the top. Each mode targets its own
    /// cursor (file_cursor / test_cursor / failure_cursor / log_scroll /
    /// overlay.scroll) so the four CursorDown/Up/Top/Bottom and two
    /// FullPage arms in `apply_action` no longer have to enumerate them.
    pub(super) fn move_cursor(&mut self, mode: &Mode, delta: isize) {
        // Helper closures for the bounded-arithmetic patterns.
        let bottom = |max: usize| max;
        let bounded = |cur: usize, max: usize| -> usize {
            if delta == isize::MAX { bottom(max) }
            else if delta == isize::MIN { 0 }
            else if delta >= 0 { cur.saturating_add(delta as usize).min(max) }
            else { cur.saturating_sub(delta.unsigned_abs()) }
        };
        match mode {
            Mode::Failure => {
                let max = self.failures.len().saturating_sub(1);
                self.nav.failure_cursor = bounded(self.nav.failure_cursor, max);
                self.nav.failure_scroll = 0;
            }
            Mode::Log => {
                let n = self.log.len();
                let max = n.saturating_sub(self.nav.log_view_height.max(1));
                self.nav.log_scroll = bounded(self.nav.log_scroll, max);
            }
            Mode::Help => {
                // Overlay scroll is clamped against actual content at draw
                // time (via OverlayState::view_height), so we just bump the
                // raw scroll counter here. usize::MAX/2 means "bottom".
                self.overlay.scroll = if delta == isize::MAX { usize::MAX / 2 }
                    else if delta == isize::MIN { 0 }
                    else if delta >= 0 { self.overlay.scroll.saturating_add(delta as usize) }
                    else { self.overlay.scroll.saturating_sub(delta.unsigned_abs()) };
            }
            _ => {
                let in_detail = matches!(mode, Mode::Detail);
                let n = if in_detail {
                    self.selected_file().map(|f| f.tests.len()).unwrap_or(0)
                } else {
                    self.visible_files().len()
                };
                let max = n.saturating_sub(1);
                let cur = if in_detail { self.nav.test_cursor } else { self.nav.file_cursor };
                let next = bounded(cur, max);
                if in_detail {
                    // Spill across files on unit steps only (j/k, arrow keys).
                    // At the last test of the current file, pressing down
                    // advances to the first test of the next visible file;
                    // symmetric for pressing up past the first test. Page
                    // moves and top/bottom jumps stay file-scoped so the
                    // user doesn't accidentally blow past a whole file.
                    if delta == 1 && cur == max && n > 0 {
                        let visible = self.visible_files();
                        if self.nav.file_cursor + 1 < visible.len() {
                            self.nav.file_cursor += 1;
                            self.nav.test_cursor = 0;
                            self.nav.test_scroll = 0;
                            return;
                        }
                    } else if delta == -1 && cur == 0 && n > 0 && self.nav.file_cursor > 0 {
                        self.nav.file_cursor -= 1;
                        let prev_max = self
                            .selected_file()
                            .map(|f| f.tests.len().saturating_sub(1))
                            .unwrap_or(0);
                        self.nav.test_cursor = prev_max;
                        self.nav.test_scroll = 0;
                        return;
                    }
                    self.nav.test_cursor = next;
                } else {
                    self.nav.file_cursor = next;
                    if self.multi.visual_anchor.is_some() {
                        self.apply_visual();
                    }
                }
            }
        }
    }

    /// Reset state ahead of running `files_to_run`.
    ///
    /// `attempt = 0` means a fresh run cycle: counters and per-file
    /// attempt/flake markers are cleared. `attempt > 0` means a rerun
    /// continuation: only the *files about to be re-run* get their
    /// `attempt` bumped to the new value, and the global tallies are
    /// rolled back so the merged final state still accumulates correctly.
    pub(super) fn reset_for_run(
        &mut self,
        files_to_run: &[PathBuf],
        is_full_suite: bool,
        attempt: u32,
    ) {
        self.run.last_run_full_suite = is_full_suite;
        self.run.running = true;
        self.run.last_run = Some(Instant::now());

        if attempt == 0 {
            // Fresh run: blow everything away.
            self.failures.clear();
            self.run.run_totals = scrutin_core::engine::protocol::Counts::default();
            self.run.file_durations.clear();
            self.nav.failure_cursor = 0;
            self.nav.failure_scroll = 0;
            self.run.current_attempt = 0;
            for entry in &mut self.files {
                entry.attempt = 0;
                entry.flaky = false;
                if files_to_run.contains(&entry.path) {
                    entry.status = FileStatus::Running;
                    entry.tests.clear();
                }
            }
        } else {
            // Rerun continuation: roll back the previous-attempt totals
            // for the files we're about to re-run, then mark them
            // running with the new attempt counter. Failures from prior
            // attempts on these files are evicted from `failures` so the
            // failure-list view doesn't show stale entries.
            self.run.current_attempt = attempt;
            self.failures
                .retain(|f| !files_to_run.contains(&f.file_path));
            for entry in &mut self.files {
                if !files_to_run.contains(&entry.path) {
                    continue;
                }
                // Subtract this file's previous-attempt tally from the
                // global counters before clearing it.
                self.run.run_totals.saturating_sub(&entry.status.counts());
                entry.attempt = attempt;
                entry.status = FileStatus::Running;
                entry.tests.clear();
            }
        }
    }

    pub(super) fn apply_result(&mut self, result: &FileResult) {
        use scrutin_core::engine::protocol;

        let file_name = protocol::file_display_name(&result.file);

        // Shared tally for counts, duration, and status.
        let tally = protocol::tally_messages(&result.messages, result.cancelled);
        let c = &tally.counts;

        // Accumulate run-level totals from the shared counts.
        self.run.run_totals.merge(c);

        // Collect failures/warnings via shared core function.
        let (failures, _warnings) = protocol::collect_findings(&result.messages, &file_name, &result.file);
        self.failures.extend(failures);

        let tests = protocol::process_events(&result.messages);

        self.run.file_durations.push((file_name.clone(), tally.duration_ms));

        if let Some(entry) = self.files.iter_mut().find(|e| e.name == file_name) {
            entry.tests = tests;
            entry.status = match tally.status {
                protocol::FileStatus::Cancelled => FileStatus::Cancelled,
                protocol::FileStatus::Failed => FileStatus::Failed {
                    passed: c.pass,
                    failed: c.fail,
                    errored: c.error,
                    warned: c.warn,
                    ms: tally.duration_ms,
                },
                protocol::FileStatus::Passed => FileStatus::Passed {
                    passed: c.pass,
                    warned: c.warn,
                    ms: tally.duration_ms,
                },
                protocol::FileStatus::Skipped => FileStatus::Skipped {
                    skipped: c.skip,
                    ms: tally.duration_ms,
                },
                protocol::FileStatus::Pending => FileStatus::Passed {
                    passed: 0,
                    warned: 0,
                    ms: tally.duration_ms,
                },
            };
        }
    }

    pub(super) fn finish_run(&mut self) {
        self.run.running = false;
        self.run.cancel = None;
        if let Some(start) = self.run.last_run {
            self.run.last_duration = Some(start.elapsed());
        }
    }

    pub(super) fn visible_files(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.files.len())
            .filter(|&i| {
                let f = &self.files[i];
                if let Some(ref pat) = self.filter.active
                    && !scrutin_core::filter::matches_name(pat, &f.name)
                {
                    return false;
                }
                if !self.filter.suite.accepts(&f.suite) {
                    return false;
                }
                use scrutin_core::engine::protocol::Outcome;
                match self.filter.status {
                    StatusFilter::All => true,
                    StatusFilter::Failed => matches!(f.status, FileStatus::Failed { .. }),
                    StatusFilter::Passed => matches!(f.status, FileStatus::Passed { .. }),
                    StatusFilter::Running => matches!(f.status, FileStatus::Running),
                    StatusFilter::Skipped => f.tests.iter().any(|t| matches!(t.outcome, Outcome::Skip)),
                    StatusFilter::Xfailed => f.tests.iter().any(|t| matches!(t.outcome, Outcome::Xfail)),
                    StatusFilter::Warned => f.tests.iter().any(|t| matches!(t.outcome, Outcome::Warn)),
                }
            })
            .collect();

        match self.display.sort_mode {
            SortMode::Sequential => {} // preserve original order
            SortMode::Status => {
                indices.sort_by_key(|&i| {
                    let f = &self.files[i];
                    match f.status {
                        FileStatus::Failed { .. } => 0,
                        FileStatus::Running => 1,
                        FileStatus::Cancelled => 2,
                        FileStatus::Pending => 3,
                        FileStatus::Passed { warned, .. } if warned > 0 => 4,
                        FileStatus::Passed { .. } => 5,
                        FileStatus::Skipped { .. } => 6,
                    }
                });
            }
            SortMode::Name => {
                indices.sort_by(|&a, &b| self.files[a].name.cmp(&self.files[b].name));
            }
            SortMode::Suite => {
                indices.sort_by(|&a, &b| self.files[a].suite.cmp(&self.files[b].suite));
            }
            SortMode::Time => {
                let file_ms = |f: &FileEntry| -> u64 {
                    match f.status {
                        FileStatus::Passed { ms, .. } | FileStatus::Failed { ms, .. } | FileStatus::Skipped { ms, .. } => ms,
                        _ => 0,
                    }
                };
                indices.sort_by(|&a, &b| file_ms(&self.files[b]).cmp(&file_ms(&self.files[a])));
            }
        }

        if self.display.sort_reversed {
            indices.reverse();
        }

        indices
    }

    /// Recompute `selected` from `visual_base` plus the inclusive range
    /// [anchor, file_cursor] in the current visible list. No-op if not in
    /// visual mode.
    pub(super) fn apply_visual(&mut self) {
        let Some(anchor) = self.multi.visual_anchor else {
            return;
        };
        let visible = self.visible_files();
        if visible.is_empty() {
            return;
        }
        let cur = self.nav.file_cursor.min(visible.len() - 1);
        let (lo, hi) = if anchor <= cur {
            (anchor, cur)
        } else {
            (cur, anchor)
        };
        let hi = hi.min(visible.len() - 1);
        let mut sel = self.multi.visual_base.clone();
        for &i in &visible[lo..=hi] {
            sel.insert(self.files[i].path.clone());
        }
        self.multi.selected = sel;
    }

    pub(super) fn selected_file(&self) -> Option<&FileEntry> {
        let visible = self.visible_files();
        visible.get(self.nav.file_cursor).map(|&i| &self.files[i])
    }

    /// Return the selected file's tests sorted for display (same order the
    /// Detail view renders). `test_cursor` indexes into this list, so any
    /// caller that wants "the test under the cursor" must go through here,
    /// not `file.tests[cursor]` which reads the unsorted emission order.
    pub(super) fn sorted_selected_tests(&self) -> Vec<TestEntry> {
        let Some(file) = self.selected_file() else { return Vec::new() };
        let mut tests = file.tests.clone();
        super::view::sort_tests(&mut tests, self.display.test_sort_mode, self.display.test_sort_reversed);
        tests
    }

    pub(super) fn selected_file_mut_idx(&self) -> Option<usize> {
        let visible = self.visible_files();
        visible.get(self.nav.file_cursor).copied()
    }

    /// Plugin actions available for the currently selected file's suite.
    pub(super) fn selected_plugin_actions(
        &self,
    ) -> Option<&[scrutin_core::project::plugin::PluginAction]> {
        let file = self.selected_file()?;
        self.suite_actions.get(&file.suite).map(|v| v.as_slice())
    }

    /// Working directory for the given suite name. Falls back to the
    /// project root when a suite has no registered root (shouldn't
    /// happen; every suite contributes to `suite_roots` at init time).
    pub(super) fn suite_root(&self, suite: &str) -> PathBuf {
        self.suite_roots
            .get(suite)
            .cloned()
            .unwrap_or_else(|| self.pkg_root.clone())
    }
}

// ── Events ──

pub(super) enum TuiEvent {
    Run(scrutin_core::engine::run_events::RunEvent),
    WatchEvent(Vec<PathBuf>),
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    DepMapRebuilt(HashMap<String, Vec<String>>),
}

// ── Tests (spec §3.14) ──

#[cfg(test)]
mod tests {
    use super::*;
    use scrutin_core::engine::protocol::Outcome;
    use std::collections::HashSet;

    // ── Mode ↔ (Level, Overlay) projection ──────────────────────────────────
    //
    // The spec pins that `Mode` is kept as a derived projection of
    // `(Level, Overlay)`. New dispatch code prefers `state.level()` /
    // `state.overlay_kind()`. These tests lock the projection table so a
    // renamed variant or reordered arm can't silently break it.

    #[test]
    fn mode_levels_are_correct() {
        assert_eq!(Mode::Normal.level(), Level::Normal);
        assert_eq!(Mode::Detail.level(), Level::Detail);
        assert_eq!(Mode::Failure.level(), Level::Failure);
        // Overlays always project to Normal level (they sit on top of it).
        assert_eq!(Mode::Help.level(), Level::Normal);
        assert_eq!(Mode::Log.level(), Level::Normal);
        assert_eq!(
            Mode::Palette(super::super::keymap::PaletteKind::Sort).level(),
            Level::Normal
        );
    }

    #[test]
    fn mode_overlay_kind_distinguishes_levels_from_overlays() {
        // Level modes have no overlay.
        assert!(Mode::Normal.overlay_kind().is_none());
        assert!(Mode::Detail.overlay_kind().is_none());
        assert!(Mode::Failure.overlay_kind().is_none());
        // Overlays return Some.
        assert!(matches!(Mode::Help.overlay_kind(), Some(Overlay::Help)));
        assert!(matches!(Mode::Log.overlay_kind(), Some(Overlay::Log)));
        let palette = Mode::Palette(super::super::keymap::PaletteKind::Sort);
        assert!(matches!(
            palette.overlay_kind(),
            Some(Overlay::Palette(_))
        ));
    }

    // ── OverlayState: scroll/cursor boundaries ──────────────────────────────

    #[test]
    fn overlay_scroll_by_saturates_at_zero() {
        let mut o = OverlayState::text();
        assert_eq!(o.scroll, 0);
        o.scroll_by(false, 5); // scroll up from 0 must not underflow
        assert_eq!(o.scroll, 0);
        o.scroll_by(true, 3);
        assert_eq!(o.scroll, 3);
        o.scroll_by(false, 100);
        assert_eq!(o.scroll, 0, "saturating_sub must clamp, not underflow");
    }

    #[test]
    fn overlay_move_cursor_clamps_to_item_count() {
        let mut o = OverlayState::menu();
        assert_eq!(o.cursor_pos(), 0);
        o.move_cursor(true, 1, 3);
        assert_eq!(o.cursor_pos(), 1);
        o.move_cursor(true, 100, 3);
        assert_eq!(o.cursor_pos(), 2, "cursor must clamp at n-1, not overflow");
        o.move_cursor(false, 100, 3);
        assert_eq!(o.cursor_pos(), 0, "cursor must clamp at 0, not underflow");
    }

    #[test]
    fn overlay_move_cursor_no_op_on_empty_menu() {
        let mut o = OverlayState::menu();
        // cursor was initialized to Some(0) by ::menu even when empty.
        o.move_cursor(true, 1, 0);
        // n_items=0 must early-return without panicking.
        assert_eq!(o.cursor_pos(), 0);
    }

    #[test]
    fn overlay_text_has_no_cursor() {
        let o = OverlayState::text();
        assert!(o.cursor.is_none());
        // cursor_pos() falls back to 0 for text overlays.
        assert_eq!(o.cursor_pos(), 0);
    }

    // ── StatusFilter cycling ────────────────────────────────────────────────

    #[test]
    fn status_filter_next_prev_inverse() {
        for f in StatusFilter::ALL {
            assert!(
                f.next().prev() == *f,
                "next/prev are inverses for {}",
                f.label()
            );
        }
    }

    #[test]
    fn status_filter_cycle_covers_all_variants_once() {
        let mut seen = HashSet::new();
        let mut cur = StatusFilter::All;
        for _ in 0..StatusFilter::ALL.len() {
            seen.insert(cur.label());
            cur = cur.next();
        }
        assert_eq!(seen.len(), StatusFilter::ALL.len());
        assert_eq!(cur.label(), StatusFilter::All.label(),
            "cycle returns to start after len() steps");
    }

    #[test]
    fn status_filter_next_supported_skips_unsupported() {
        // Only Pass + Fail outcomes supported: next_supported from All
        // should skip right past Skipped/Xfailed/Warned.
        let supported: HashSet<Outcome> = [Outcome::Pass, Outcome::Fail].into_iter().collect();
        let after_all = StatusFilter::All.next_supported(&supported);
        assert_eq!(after_all.label(), "failed");
        let after_failed = after_all.next_supported(&supported);
        assert_eq!(after_failed.label(), "passed");
        let after_passed = after_failed.next_supported(&supported);
        assert_eq!(
            after_passed.label(),
            "running",
            "after passed, next supported skips skipped/xfail/warn and lands on running"
        );
    }

    #[test]
    fn status_filter_supports_bucket_failed_on_fail_or_error() {
        // The Failed chip covers both Fail and Error outcomes: tests that
        // error out should still show under "failed" when filtered.
        let error_only: HashSet<Outcome> = [Outcome::Error].into_iter().collect();
        let fail_only: HashSet<Outcome> = [Outcome::Fail].into_iter().collect();
        let neither: HashSet<Outcome> = HashSet::new();
        assert!(StatusFilter::Failed.is_supported(&error_only));
        assert!(StatusFilter::Failed.is_supported(&fail_only));
        assert!(!StatusFilter::Failed.is_supported(&neither));
    }

    #[test]
    fn status_filter_all_and_running_always_supported() {
        let empty: HashSet<Outcome> = HashSet::new();
        assert!(StatusFilter::All.is_supported(&empty));
        assert!(StatusFilter::Running.is_supported(&empty));
    }

    // ── SuiteFilter cycling ────────────────────────────────────────────────

    #[test]
    fn suite_filter_cycle_visits_all_then_each_suite() {
        let mut f = SuiteFilter::new(vec!["testthat".into(), "pytest".into()]);
        // Starts at All.
        assert_eq!(f.label(), "all");
        f.cycle_next();
        assert_eq!(f.label(), "testthat");
        f.cycle_next();
        assert_eq!(f.label(), "pytest");
        f.cycle_next();
        assert_eq!(f.label(), "all", "cycles back to All after last suite");
    }

    #[test]
    fn suite_filter_cycle_prev_is_reverse() {
        let mut f = SuiteFilter::new(vec!["testthat".into(), "pytest".into()]);
        f.cycle_prev();
        assert_eq!(f.label(), "pytest");
        f.cycle_prev();
        assert_eq!(f.label(), "testthat");
        f.cycle_prev();
        assert_eq!(f.label(), "all");
    }

    #[test]
    fn suite_filter_single_suite_not_meaningful() {
        // Only one suite: the chip is hidden and cycling is a no-op.
        let solo = SuiteFilter::new(vec!["pytest".into()]);
        assert!(!solo.is_meaningful());
        let mut f = solo;
        f.cycle_next();
        // Single suite still cycles (All ↔ pytest) but the chip won't render.
        assert_eq!(f.label(), "pytest");
    }

    #[test]
    fn suite_filter_empty_suites_is_safe_noop() {
        let mut f = SuiteFilter::new(Vec::new());
        assert!(!f.is_meaningful());
        f.cycle_next(); // must not panic
        f.cycle_prev();
        assert_eq!(f.label(), "all");
    }

    #[test]
    fn suite_filter_accepts_gate() {
        let f = SuiteFilter {
            suites: vec!["testthat".into(), "pytest".into()],
            current: Some(0),
        };
        assert!(f.accepts("testthat"));
        assert!(!f.accepts("pytest"));
        // `All` state accepts everything.
        let all = SuiteFilter::new(vec!["testthat".into()]);
        assert!(all.accepts("anything"));
    }

    // ── SortMode labels ────────────────────────────────────────────────────
    //
    // Locks the five sort modes the TUI and web share. If a sort mode is
    // added or removed, the label/description table here needs explicit
    // attention (the web mirrors this list).

    #[test]
    fn sort_mode_has_five_variants() {
        assert_eq!(SortMode::ALL.len(), 5);
    }

    #[test]
    fn sort_mode_labels_are_unique() {
        let labels: HashSet<_> = SortMode::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(labels.len(), SortMode::ALL.len(), "labels must be unique");
    }

    #[test]
    fn sort_mode_sequential_is_default_first() {
        // The web and TUI docs both describe Sequential as the default
        // and first mode. Lock that ordering.
        assert_eq!(SortMode::ALL[0], SortMode::Sequential);
    }
}
