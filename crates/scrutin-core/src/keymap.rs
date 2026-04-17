//! Shared keymap definition: the single source of truth for keybindings
//! across TUI, web, and editor frontends.
//!
//! Each `KeyBinding` describes a key, an action name, which navigation
//! levels it's available at, and a human-readable hint. Frontends consume
//! this table to dispatch keys, render help overlays, and generate hints.

use serde::Serialize;

/// Navigation level (analogous to TUI "modes" but simpler).
/// "overlay" covers Help, Log, and Palette modes which aren't
/// navigation levels but popup layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Files,
    Detail,
    Failure,
    Overlay,
}

/// When a binding's hint should be visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum When {
    Always,
    WhenIdle,
    WhenRunning,
}

/// A single keybinding entry.
#[derive(Clone, Debug, Serialize)]
pub struct KeyBinding {
    /// Key string: "j", "k", "Enter", "Esc", "Ctrl-d", "Space", etc.
    pub key: &'static str,
    /// Action name: "cursor_down", "open_edit_menu", etc.
    pub action: &'static str,
    /// Navigation levels where this binding is active.
    pub levels: &'static [Level],
    /// Description for the help overlay (`?`). Format: "key description".
    /// Every binding should have one; duplicates are deduped by action.
    pub help: &'static str,
    /// Short hint for the footer bar. Empty = not shown in footer.
    /// Only essential bindings get a bar hint.
    pub bar: &'static str,
    /// Visibility condition for the footer bar.
    pub when: When,
}

/// All navigation levels (files, detail, failure).
const ALL: &[Level] = &[Level::Files, Level::Detail, Level::Failure];
/// Files and detail only.
const FD: &[Level] = &[Level::Files, Level::Detail];
/// Files only.
const F: &[Level] = &[Level::Files];
/// Detail and failure.
const DF: &[Level] = &[Level::Detail, Level::Failure];
/// Overlay (help, log).
const OV: &[Level] = &[Level::Overlay];

/// The default keymap. This is the single source of truth.
pub static DEFAULT_KEYMAP: &[KeyBinding] = &[
    // ── Navigation ──────────────────────────────────────────────
    // Help overlay lists bindings in array order. Put the four core
    // navigation actions (down/up/drill-in/drill-out) first so they
    // head the help card. Enter and Esc are aliases for drill-in and
    // drill-out respectively but omitted from the help text since
    // they're expected to work by default everywhere.
    KeyBinding { key: "j",       action: "cursor_down",     levels: ALL, help: "j/\u{2193} move down",           bar: "\u{2191}\u{2193} navigate", when: When::Always },
    KeyBinding { key: "k",       action: "cursor_up",       levels: ALL, help: "k/\u{2191} move up",             bar: "",                          when: When::Always },
    KeyBinding { key: "Down",    action: "cursor_down",     levels: ALL, help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Up",      action: "cursor_up",       levels: ALL, help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Right",   action: "enter",           levels: FD,  help: "\u{2192} details",                bar: "\u{2192} details",          when: When::Always },
    KeyBinding { key: "Enter",   action: "enter",           levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "l",       action: "enter",           levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Left",    action: "pop",             levels: ALL, help: "\u{2190} back",                   bar: "\u{2190} back",             when: When::Always },
    KeyBinding { key: "Esc",     action: "pop",             levels: ALL, help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "h",       action: "pop",             levels: DF,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "q",       action: "quit",            levels: ALL, help: "q quit",                          bar: "",                          when: When::Always },

    // ── Larger navigation jumps ────────────────────────────────
    KeyBinding { key: "g",       action: "cursor_top",      levels: FD,  help: "g jump to top",                   bar: "",                          when: When::Always },
    KeyBinding { key: "G",       action: "cursor_bottom",   levels: FD,  help: "G jump to bottom",               bar: "",                          when: When::Always },
    KeyBinding { key: "Home",    action: "cursor_top",      levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "End",     action: "cursor_bottom",   levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "PageUp",  action: "full_page_up",    levels: FD,  help: "PgUp page up",                    bar: "",                          when: When::Always },
    KeyBinding { key: "PageDown",action: "full_page_down",  levels: FD,  help: "PgDn page down",                  bar: "",                          when: When::Always },
    KeyBinding { key: "Ctrl-f",  action: "full_page_down",  levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Ctrl-b",  action: "full_page_up",    levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "J",       action: "source_scroll_down", levels: FD, help: "Shift-J scroll right pane down", bar: "",                        when: When::Always },
    KeyBinding { key: "K",       action: "source_scroll_up",   levels: FD, help: "Shift-K scroll right pane up",   bar: "",                        when: When::Always },

    // ── Run control ─────────────────────────────────────────────
    KeyBinding { key: "r",       action: "open_run_menu",   levels: F,   help: "r run menu / run file",           bar: "r run",                     when: When::WhenIdle },
    KeyBinding { key: "r",       action: "run_current_file",levels: DF,  help: "",                                bar: "r run file",                when: When::WhenIdle },
    KeyBinding { key: "R",       action: "open_run_menu",   levels: DF,  help: "R run menu (from detail)",        bar: "",                          when: When::WhenIdle },
    KeyBinding { key: "x",       action: "cancel_file",     levels: ALL, help: "x/X cancel file / entire run",   bar: "x cancel",                  when: When::WhenRunning },
    KeyBinding { key: "X",       action: "cancel_all",      levels: ALL, help: "",                                bar: "",                          when: When::WhenRunning },

    // ── Filters ─────────────────────────────────────────────────
    // Three filters grouped together with parallel naming
    // (filter_{name,status,tool}). Displayed adjacent in the help.
    KeyBinding { key: "/",       action: "filter_name",     levels: ALL, help: "/ filter by name",                bar: "",                          when: When::Always },
    KeyBinding { key: "o",       action: "filter_status",   levels: FD,  help: "o/O filter by status",            bar: "",                          when: When::Always },
    KeyBinding { key: "O",       action: "filter_status_back", levels: FD, help: "",                              bar: "",                          when: When::Always },
    KeyBinding { key: "t",       action: "filter_tool",     levels: FD,  help: "t/T filter by tool",              bar: "",                          when: When::Always },
    KeyBinding { key: "T",       action: "filter_tool_back",levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "f",       action: "filter_group",    levels: FD,  help: "f/F filter by group",             bar: "",                          when: When::Always },
    KeyBinding { key: "F",       action: "filter_group_back", levels: FD, help: "",                               bar: "",                          when: When::Always },

    // ── Display / layout ────────────────────────────────────────
    KeyBinding { key: "s",       action: "open_sort_menu",  levels: FD,  help: "s sort menu",                     bar: "",                          when: When::Always },
    KeyBinding { key: "\\",      action: "toggle_orientation", levels: FD, help: "\\ toggle split orientation",   bar: "",                          when: When::Always },
    KeyBinding { key: "(",       action: "shrink_list",     levels: FD,  help: "(/) shrink/grow list pane",       bar: "",                          when: When::Always },
    KeyBinding { key: ")",       action: "grow_list",       levels: FD,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Space",   action: "toggle_select",   levels: F,   help: "Space toggle selection",          bar: "",                          when: When::Always },
    KeyBinding { key: "v",       action: "toggle_visual",   levels: F,   help: "v visual select mode",            bar: "",                          when: When::Always },

    // ── Editor integration ──────────────────────────────────────
    KeyBinding { key: "e",       action: "edit_test",       levels: ALL, help: "e/E edit test/source file",       bar: "",                          when: When::Always },
    KeyBinding { key: "E",       action: "edit_source",     levels: DF,  help: "",                                bar: "",                          when: When::Always },

    // ── Actions ─────────────────────────────────────────────────
    KeyBinding { key: "y",       action: "yank_message",    levels: ALL, help: "y yank error message to clipboard", bar: "",                        when: When::Always },
    KeyBinding { key: "a",       action: "diagnose_with_agent", levels: DF, help: "a send selected failure to LLM agent (see [agent] config)", bar: "", when: When::Always },
    KeyBinding { key: "L",       action: "enter_log",       levels: ALL, help: "L open log",                      bar: "",                          when: When::Always },
    KeyBinding { key: "?",       action: "enter_help",      levels: ALL, help: "? keyboard shortcuts",            bar: "? help",                    when: When::Always },

    // ── Overlay navigation (Help, Log) ──────────────────────────
    KeyBinding { key: "j",       action: "scroll_down",     levels: OV,  help: "j/\u{2193} scroll down",         bar: "",                          when: When::Always },
    KeyBinding { key: "k",       action: "scroll_up",       levels: OV,  help: "k/\u{2191} scroll up",           bar: "",                          when: When::Always },
    KeyBinding { key: "Down",    action: "scroll_down",     levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Up",      action: "scroll_up",       levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "g",       action: "scroll_top",      levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "G",       action: "scroll_bottom",   levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "PageDown",action: "scroll_page_down",levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "PageUp",  action: "scroll_page_up",  levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "Esc",     action: "pop",             levels: OV,  help: "Esc close",                       bar: "",                          when: When::Always },
    KeyBinding { key: "q",       action: "pop",             levels: OV,  help: "",                                bar: "",                          when: When::Always },
    KeyBinding { key: "?",       action: "pop",             levels: OV,  help: "",                                bar: "",                          when: When::Always },
];

/// Return bindings active at a given level.
pub fn bindings_for_level(level: Level) -> impl Iterator<Item = &'static KeyBinding> {
    DEFAULT_KEYMAP.iter().filter(move |b| b.levels.contains(&level))
}

/// Resolve the first matching action for a key string at a given level.
pub fn resolve(key: &str, level: Level) -> Option<&'static str> {
    DEFAULT_KEYMAP.iter()
        .find(|b| b.key == key && b.levels.contains(&level))
        .map(|b| b.action)
}

/// All known action names (for config validation).
pub fn all_action_names() -> impl Iterator<Item = &'static str> {
    let mut names: Vec<&str> = DEFAULT_KEYMAP.iter().map(|b| b.action).collect();
    names.sort_unstable();
    names.dedup();
    names.into_iter()
}

/// Serialize the keymap as JSON (for the web frontend).
pub fn keymap_json() -> String {
    serde_json::to_string(DEFAULT_KEYMAP).unwrap_or_default()
}
