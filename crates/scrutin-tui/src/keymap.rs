//! Key bindings, Action enum, and per-mode binding tables.
//!
//! The TUI dispatch model is: `handle_key` looks up a `(KeyCode, KeyModifiers)`
//! in the active mode's binding table and runs the resulting `Action` via
//! `apply_action`. Keys that need complex inline state (filter text input,
//! multi-argument accumulators, terminal-suspend flows) fall through to the
//! per-mode `handle_<mode>_extras` functions in `input.rs`.
//!
//! Adding a binding: append a `Binding` row to the appropriate table and,
//! if it introduces a new verb, add an `Action` variant and handle it in
//! `apply_action`. Descriptions are shown verbatim in the hints bar and the
//! generated help overlay.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyModifiers};

use super::state::Mode;

/// Palette screen flavour. The three historical menu modes collapse into
/// `Mode::Palette(PaletteKind)` — the kind drives both rendering (filter
/// input box vs run menu vs config menu) and input handling (text capture
/// vs list navigation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum PaletteKind {
    Filter,
    Run,
    Sort,
    Action,
}

/// Verb-level actions triggered by key bindings. Pure and cheap to clone;
/// all wiring to `AppState` lives in `input::apply_action`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Action {
    // Navigation (list or test cursor, depending on mode)
    CursorUp,
    CursorDown,
    CursorTop,
    CursorBottom,
    FullPageUp,
    FullPageDown,

    // Mode transitions
    EnterDetail,
    EnterFailure,  // Detail → Failure (only if cursor test actually failed)
    EnterHelp,
    EnterLog,
    OpenPalette(PaletteKind),
    Pop, // Esc-style: pop one frame off the mode stack

    // Selection
    ToggleSelect,
    ToggleVisual,

    // Display
    CycleStatusFilter,
    CycleStatusFilterBack,
    CycleSuiteFilter,
    CycleSuiteFilterBack,
    ShrinkList,
    GrowList,
    ToggleOrientation,

    // Source view
    SourceScrollUp,
    SourceScrollDown,

    // Run control
    CancelFile,
    CancelAll,
    RunCurrentFile,

    // Editor
    EditTest,
    EditSource,

    // Misc
    Quit,
    YankMessage,
}

/// Controls whether a binding's hint appears in the hints bar. Does not
/// affect whether the key is *handled* — `apply_action` always runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Visibility {
    Always,
    WhenRunning,
    WhenIdle,
}

/// A single entry in a mode's binding table. `desc` is `Cow` so the static
/// default tables can use string literals (`Cow::Borrowed`) at compile time
/// while user-supplied bindings from scrutin.toml use owned strings
/// (`Cow::Owned`) auto-generated at parse time.
pub(super) struct Binding {
    pub key: KeyCode,
    pub mods: KeyModifiers,
    pub action: Action,
    /// Short hint for the footer bar. Empty = not shown in bar.
    pub bar_desc: Cow<'static, str>,
    pub visible: Visibility,
}

// ── Binding generation from shared keymap ──────────────────────────────

use scrutin_core::keymap::{DEFAULT_KEYMAP, Level, When};

/// Map a shared keymap `Level` to the TUI `Mode`.
fn level_to_mode(level: Level) -> Mode {
    match level {
        Level::Files => Mode::Normal,
        Level::Detail => Mode::Detail,
        Level::Failure => Mode::Failure,
        Level::Overlay => Mode::Help, // Help and Log share overlay bindings
    }
}

/// Map a shared keymap action name to the TUI `Action` enum.
/// The shared keymap uses some unified names ("enter", "scroll_down") that
/// need mode-aware mapping.
fn action_from_shared(name: &str, mode: &Mode) -> Option<Action> {
    Some(match name {
        // "enter" means drill deeper: EnterDetail in Normal, EnterFailure in Detail
        "enter" => match *mode {
            Mode::Normal => Action::EnterDetail,
            Mode::Detail => Action::EnterFailure,
            _ => return None,
        },
        // Overlay scroll actions map to cursor actions (apply_action handles the mode context)
        "scroll_down" | "scroll_half_down" | "scroll_page_down" => Action::CursorDown,
        "scroll_up" | "scroll_half_up" | "scroll_page_up" => Action::CursorUp,
        "scroll_top" => Action::CursorTop,
        "scroll_bottom" => Action::CursorBottom,
        // Everything else uses Action::from_name
        _ => Action::from_name(name)?,
    })
}

fn when_to_visibility(when: When) -> Visibility {
    match when {
        When::Always => Visibility::Always,
        When::WhenIdle => Visibility::WhenIdle,
        When::WhenRunning => Visibility::WhenRunning,
    }
}

/// Build binding tables for all modes from the shared keymap.
/// Returns a map from Mode to Vec<Binding>.
pub(super) fn build_default_bindings() -> std::collections::HashMap<Mode, Vec<Binding>> {
    let mut map: std::collections::HashMap<Mode, Vec<Binding>> = std::collections::HashMap::new();

    for entry in DEFAULT_KEYMAP {
        let Some((key, mods)) = parse_key_string(entry.key) else { continue };

        for &level in entry.levels {
            let mode = level_to_mode(level);
            // For overlay level, generate bindings for both Help and Log.
            let modes = if level == Level::Overlay {
                vec![Mode::Help, Mode::Log]
            } else {
                vec![mode]
            };
            for mode in modes {
                let Some(action) = action_from_shared(entry.action, &mode) else { continue };
                let binding = Binding {
                    key,
                    mods,
                    action,
                    bar_desc: Cow::Borrowed(entry.bar),
                    visible: when_to_visibility(entry.when),
                };
                map.entry(mode).or_default().push(binding);
            }
        }
    }

    // Log mode: also allow 'L' to close (not in shared keymap since it's TUI-specific)
    map.entry(Mode::Log).or_default().push(Binding {
        key: KeyCode::Char('L'),
        mods: KeyModifiers::NONE,
        action: Action::Pop,
        bar_desc: Cow::Borrowed(""),
        visible: Visibility::Always,
    });

    map
}

/// Label shown in the mode chip prefix of the hints bar.
pub(super) fn mode_label(mode: &Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Detail => "DETAIL",
        Mode::Failure => "FAILURE",
        Mode::Help => "HELP",
        Mode::Log => "LOG",
        Mode::ActionOutput => "ACTION",
        Mode::Palette(PaletteKind::Filter) => "PALETTE: filter",
        Mode::Palette(PaletteKind::Run) => "PALETTE: run",
        Mode::Palette(PaletteKind::Sort) => "PALETTE: sort",
        Mode::Palette(PaletteKind::Action) => "PALETTE: action",
    }
}

pub(super) fn mode_color(mode: &Mode) -> ratatui::style::Color {
    use ratatui::style::Color;
    match mode {
        Mode::Normal => Color::Cyan,
        Mode::Detail => Color::Blue,
        Mode::Failure => Color::Red,
        Mode::Palette(_) => Color::Magenta,
        Mode::Help | Mode::Log | Mode::ActionOutput => Color::Yellow,
    }
}

// ── Config-driven keymap support ────────────────────────────────────────────
//
// Users can override the default bindings via `[keymap.<mode>]` tables in
// scrutin.toml. Each subtable replaces the default bindings for that mode
// (replace semantics, not overlay): if `[keymap.normal]` exists at all, it
// fully defines normal-mode bindings, and any default key not listed there
// is unbound. `scrutin init` writes every default into the file so the
// generated config is a working starting point.

impl Action {
    /// Stable snake_case name used in `[keymap.<mode>]` tables. Adding a
    /// new Action variant requires adding an arm here AND in `from_name`
    /// below — that's intentional so accidental drift is impossible.
    pub(super) fn name(&self) -> &'static str {
        use Action::*;
        match self {
            CursorUp => "cursor_up",
            CursorDown => "cursor_down",
            CursorTop => "cursor_top",
            CursorBottom => "cursor_bottom",
            FullPageUp => "full_page_up",
            FullPageDown => "full_page_down",
            EnterDetail => "enter_detail",
            EnterFailure => "enter_failure",
            EnterHelp => "enter_help",
            EnterLog => "enter_log",
            OpenPalette(PaletteKind::Filter) => "open_filter",
            OpenPalette(PaletteKind::Run) => "open_run_menu",
            OpenPalette(PaletteKind::Sort) => "open_sort_menu",
            OpenPalette(PaletteKind::Action) => "open_action_menu",
            Pop => "pop",
            ToggleSelect => "toggle_select",
            ToggleVisual => "toggle_visual",
            CycleStatusFilter => "cycle_status_filter",
            CycleStatusFilterBack => "cycle_status_filter_back",
            CycleSuiteFilter => "cycle_suite_filter",
            CycleSuiteFilterBack => "cycle_suite_filter_back",
            ShrinkList => "shrink_list",
            GrowList => "grow_list",
            ToggleOrientation => "toggle_orientation",
            SourceScrollUp => "source_scroll_up",
            SourceScrollDown => "source_scroll_down",
            CancelFile => "cancel_file",
            CancelAll => "cancel_all",
            RunCurrentFile => "run_current_file",
            EditTest => "edit_test",
            EditSource => "edit_source",
            Quit => "quit",
            YankMessage => "yank_message",
        }
    }

    /// Inverse of `name()`. Returns `None` for unknown names so callers
    /// can log a warning and skip the binding instead of crashing.
    pub(super) fn from_name(s: &str) -> Option<Action> {
        use Action::*;
        Some(match s {
            "cursor_up" => CursorUp,
            "cursor_down" => CursorDown,
            "cursor_top" => CursorTop,
            "cursor_bottom" => CursorBottom,
            "full_page_up" => FullPageUp,
            "full_page_down" => FullPageDown,
            "enter_detail" => EnterDetail,
            "enter_failure" => EnterFailure,
            "enter_help" => EnterHelp,
            "enter_log" => EnterLog,
            "open_filter" => OpenPalette(PaletteKind::Filter),
            "open_run_menu" => OpenPalette(PaletteKind::Run),
            "open_sort_menu" => OpenPalette(PaletteKind::Sort),
            "open_action_menu" => OpenPalette(PaletteKind::Action),
            "pop" => Pop,
            "toggle_select" => ToggleSelect,
            "toggle_visual" => ToggleVisual,
            "cycle_status_filter" => CycleStatusFilter,
            "cycle_status_filter_back" => CycleStatusFilterBack,
            "cycle_suite_filter" => CycleSuiteFilter,
            "cycle_suite_filter_back" => CycleSuiteFilterBack,
            "shrink_list" => ShrinkList,
            "grow_list" => GrowList,
            "toggle_orientation" => ToggleOrientation,
            "source_scroll_up" => SourceScrollUp,
            "source_scroll_down" => SourceScrollDown,
            "cancel_file" => CancelFile,
            "cancel_all" => CancelAll,
            "run_current_file" => RunCurrentFile,
            "edit_test" => EditTest,
            "edit_source" => EditSource,
            "quit" => Quit,
            "yank_message" => YankMessage,
            _ => return None,
        })
    }
}

/// Format a `KeyCode + KeyModifiers` back into the textual form used in
/// scrutin.toml. Inverse of `parse_key_string`. Used by `scrutin init`
/// when emitting the default keymap.
pub(super) fn key_to_string(code: KeyCode, mods: KeyModifiers) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if mods.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    let base: String = match code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => "?".into(),
    };
    if parts.is_empty() {
        base
    } else {
        format!("{}+{}", parts.join("+"), base)
    }
}

/// Parse a key string from scrutin.toml into a `(KeyCode, KeyModifiers)`.
/// Accepts:
///   - Single chars: `j`, `J` (uppercase implies shift),
///     symbols: `-`, `[`, `}`, `?`, `/`
///   - Named keys: `Enter`, `Esc`, `Tab`, `Space`, `Up`, `Down`, `Left`,
///     `Right`, `Home`, `End`, `PageUp`, `PageDown`, `Backspace`, `Delete`,
///     `Insert`, `F1`..`F12`
///   - Modifiers: `ctrl+x`, `alt+x`, `shift+x` (combinable with `+`)
/// Modifier names and named keys are case-insensitive; the base char in
/// a single-char binding stays case-sensitive.
pub(super) fn parse_key_string(s: &str) -> Option<(KeyCode, KeyModifiers)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('+').collect();
    let (mod_parts, base) = parts.split_at(parts.len() - 1);
    let base = base[0];

    let mut mods = KeyModifiers::NONE;
    for m in mod_parts {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "c" => mods |= KeyModifiers::CONTROL,
            "alt" | "meta" | "a" => mods |= KeyModifiers::ALT,
            "shift" | "s" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }

    let code = match base {
        b if b.eq_ignore_ascii_case("Enter") => KeyCode::Enter,
        b if b.eq_ignore_ascii_case("Esc") || b.eq_ignore_ascii_case("Escape") => KeyCode::Esc,
        b if b.eq_ignore_ascii_case("Tab") => KeyCode::Tab,
        b if b.eq_ignore_ascii_case("BackTab") => KeyCode::BackTab,
        b if b.eq_ignore_ascii_case("Space") => KeyCode::Char(' '),
        b if b.eq_ignore_ascii_case("Backspace") => KeyCode::Backspace,
        b if b.eq_ignore_ascii_case("Delete") || b.eq_ignore_ascii_case("Del") => KeyCode::Delete,
        b if b.eq_ignore_ascii_case("Insert") || b.eq_ignore_ascii_case("Ins") => KeyCode::Insert,
        b if b.eq_ignore_ascii_case("Up") => KeyCode::Up,
        b if b.eq_ignore_ascii_case("Down") => KeyCode::Down,
        b if b.eq_ignore_ascii_case("Left") => KeyCode::Left,
        b if b.eq_ignore_ascii_case("Right") => KeyCode::Right,
        b if b.eq_ignore_ascii_case("Home") => KeyCode::Home,
        b if b.eq_ignore_ascii_case("End") => KeyCode::End,
        b if b.eq_ignore_ascii_case("PageUp") || b.eq_ignore_ascii_case("PgUp") => KeyCode::PageUp,
        b if b.eq_ignore_ascii_case("PageDown") || b.eq_ignore_ascii_case("PgDn") => {
            KeyCode::PageDown
        }
        b if b.len() >= 2 && (b.starts_with('F') || b.starts_with('f')) => {
            let n: u8 = b[1..].parse().ok()?;
            KeyCode::F(n)
        }
        b if b.chars().count() == 1 => {
            let c = b.chars().next().unwrap();
            // An uppercase letter without an explicit shift implies shift.
            if c.is_ascii_uppercase() && !mods.contains(KeyModifiers::SHIFT) {
                // We leave SHIFT off so it matches crossterm's KeyEvent for
                // typed uppercase letters, which arrive without SHIFT in mods.
            }
            KeyCode::Char(c)
        }
        _ => return None,
    };

    Some((code, mods))
}

/// Snake_case mode names accepted in `[keymap.<mode>]`. Palette modes are
/// excluded — their input handlers don't dispatch through binding tables.
pub(super) fn mode_from_name(s: &str) -> Option<Mode> {
    match s {
        "normal" => Some(Mode::Normal),
        "detail" => Some(Mode::Detail),
        "failure" => Some(Mode::Failure),
        "help" => Some(Mode::Help),
        "log" => Some(Mode::Log),
        _ => None,
    }
}

/// Public helper for `scrutin init`: returns every default binding for
/// every user-rebindable mode as a list of `(mode_name, [(key_string,
/// action_name)])` pairs, ready for serialization into a `[keymap.<mode>]`
/// section. Bindings whose key can't round-trip through `key_to_string`
/// (e.g. `KeyCode::Null`) are skipped — there are none in practice today,
/// but it keeps the dump self-consistent if someone adds an exotic key.
pub fn default_keymap_for_init() -> Vec<(&'static str, Vec<(String, String)>)> {
    let defaults = build_default_bindings();
    let mode_names: &[(&str, Mode)] = &[
        ("normal", Mode::Normal),
        ("detail", Mode::Detail),
        ("failure", Mode::Failure),
        ("help", Mode::Help),
        ("log", Mode::Log),
    ];
    mode_names
        .iter()
        .map(|(name, mode)| {
            let empty = Vec::new();
            let bindings = defaults.get(mode).unwrap_or(&empty);
            let entries: Vec<(String, String)> = bindings
                .iter()
                .map(|b| (key_to_string(b.key, b.mods), b.action.name().to_string()))
                .collect();
            (*name, entries)
        })
        .collect()
}

/// Build a runtime keymap from a `[keymap]` config table. Returns one
/// `Vec<Binding>` per mode that has a subtable in the config. Modes absent
/// from the config keep their static defaults (the lookup paths fall back
/// when a mode is missing from this map).
///
/// Bad entries (unparseable key, unknown action, unknown mode) are skipped
/// and reported via the `warn` callback so they surface in the LogBuffer
/// at startup without crashing the run.
pub(super) fn build_user_keymap(
    cfg: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    mut warn: impl FnMut(String),
) -> std::collections::HashMap<Mode, Vec<Binding>> {
    let mut out: std::collections::HashMap<Mode, Vec<Binding>> = Default::default();
    for (mode_name, entries) in cfg {
        let Some(mode) = mode_from_name(mode_name) else {
            warn(format!("[keymap] unknown mode: {mode_name}"));
            continue;
        };
        let mut bindings: Vec<Binding> = Vec::new();
        for (key_str, action_name) in entries {
            let Some((code, mods)) = parse_key_string(key_str) else {
                warn(format!(
                    "[keymap.{mode_name}] unparseable key: {key_str:?}"
                ));
                continue;
            };
            let Some(action) = Action::from_name(action_name) else {
                warn(format!(
                    "[keymap.{mode_name}] unknown action {action_name:?} for key {key_str:?}"
                ));
                continue;
            };
            bindings.push(Binding {
                key: code,
                mods,
                action,
                bar_desc: Cow::Borrowed(""),
                visible: Visibility::Always,
            });
        }
        out.insert(mode, bindings);
    }
    out
}
