//! TUI input handling: key/mouse events and editor suspension.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use scrutin_core::project::package::Package;

use super::keymap::{Action, PaletteKind};
use super::state::*;
use super::{find_source_for_test, start_test_run};

fn osc52_copy(text: &str) {
    // Try a system clipboard command first (reliable on macOS/Linux where
    // the TTY may not honor OSC 52). Fall back to OSC 52 for remote/SSH
    // sessions where no local clipboard utility is available.
    if system_clipboard_copy(text) {
        return;
    }
    use std::io::Write;
    let b64 = base64_encode(text.as_bytes());
    // Print to the same TTY ratatui is drawing on (stderr).
    let _ = write!(io::stderr(), "\x1b]52;c;{}\x07", b64);
    let _ = io::stderr().flush();
}

/// Try to pipe `text` into a platform clipboard utility. Returns true on
/// success. Tries pbcopy (macOS), wl-copy (Wayland), xclip/xsel (X11),
/// clip.exe (Windows/WSL).
fn system_clipboard_copy(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        // Linux/BSD: prefer Wayland, fall back to X11 utilities, then WSL.
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["-b", "-i"]),
            ("clip.exe", &[]),
        ]
    };

    for (bin, args) in candidates {
        let mut cmd = Command::new(bin);
        cmd.args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else { continue };
        if let Some(stdin) = child.stdin.as_mut()
            && stdin.write_all(text.as_bytes()).is_err()
        {
            let _ = child.kill();
            continue;
        }
        drop(child.stdin.take());
        if let Ok(status) = child.wait()
            && status.success()
        {
            return true;
        }
    }
    false
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(T[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub(super) fn handle_mouse(m: MouseEvent, state: &Arc<Mutex<AppState>>) {
    const STEP: usize = 3;
    let down = match m.kind {
        MouseEventKind::ScrollDown => true,
        MouseEventKind::ScrollUp => false,
        _ => return,
    };

    let mut st = state.lock().unwrap();

    // Overlay modes consume all scroll events — scrolling behind a
    // palette/help overlay is confusing.
    match st.mode() {
        Mode::Help => {
            scroll_overlay(&mut st, down, STEP);
            return;
        }
        Mode::Palette(PaletteKind::Sort) => {
            move_overlay_cursor(&mut st, down, 1, super::state::SortMode::ALL.len());
            return;
        }
        Mode::Palette(PaletteKind::Run) => {
            let n = 4 + st.run_groups.len();
            move_overlay_cursor(&mut st, down, 1, n);
            return;
        }
        _ => {}
    }

    let rects = st.pane_rects;
    let (col, row) = (m.column, m.row);

    // Route to whichever pane the cursor is over. Mode-aware: in Normal the
    // list pane scrolls the file cursor; in Detail it scrolls the test
    // cursor. The "main" pane scrolls the source view in both modes.
    if PaneRects::hit(rects.list, col, row) {
        match st.mode() {
            Mode::Detail => scroll_test_cursor(&mut st, down, STEP),
            _ => scroll_file_cursor(&mut st, down, STEP),
        }
        return;
    }
    if PaneRects::hit(rects.main, col, row) {
        scroll_source(&mut st, down, STEP);
        return;
    }
    if PaneRects::hit(rects.log, col, row) {
        scroll_log(&mut st, down, STEP);
        return;
    }
    if PaneRects::hit(rects.failure_error, col, row) {
        scroll_failure_error(&mut st, down, STEP);
        return;
    }

    // Cursor isn't over any registered pane (chrome row, gap, etc.) — fall
    // back to the previous behavior so the wheel still does *something*
    // sensible in modes where we haven't tagged a target.
    match st.mode() {
        Mode::Normal => scroll_file_cursor(&mut st, down, STEP),
        Mode::Detail => scroll_test_cursor(&mut st, down, STEP),
        Mode::Log => scroll_log(&mut st, down, STEP),
        Mode::Failure => scroll_failure_error(&mut st, down, STEP),
        Mode::Help => scroll_overlay(&mut st, down, STEP),
        _ => {}
    }
}

fn scroll_file_cursor(st: &mut AppState, down: bool, step: usize) {
    if down {
        let n = st.visible_files().len();
        if n > 0 {
            st.nav.file_cursor = (st.nav.file_cursor + step).min(n - 1);
        }
    } else {
        st.nav.file_cursor = st.nav.file_cursor.saturating_sub(step);
    }
}

fn scroll_test_cursor(st: &mut AppState, down: bool, step: usize) {
    if down {
        let n = st.selected_file().map(|f| f.tests.len()).unwrap_or(0);
        if n > 0 {
            st.nav.test_cursor = (st.nav.test_cursor + step).min(n - 1);
        }
    } else {
        st.nav.test_cursor = st.nav.test_cursor.saturating_sub(step);
    }
}

fn scroll_source(st: &mut AppState, down: bool, step: usize) {
    if down {
        // Clamp against the cap published by the renderer last frame so
        // the wheel can't push past EOF (the next render would just snap
        // it back, but that produces a "dead scroll" feel).
        st.nav.source_scroll = (st.nav.source_scroll + step).min(st.nav.source_scroll_max);
    } else {
        st.nav.source_scroll = st.nav.source_scroll.saturating_sub(step);
    }
}

fn scroll_log(st: &mut AppState, down: bool, step: usize) {
    if down {
        let n = st.log.len();
        st.nav.log_scroll = (st.nav.log_scroll + step).min(n.saturating_sub(1));
    } else {
        st.nav.log_scroll = st.nav.log_scroll.saturating_sub(step);
    }
}

fn scroll_failure_error(st: &mut AppState, down: bool, step: usize) {
    if down {
        st.nav.failure_scroll = st.nav.failure_scroll.saturating_add(step);
    } else {
        st.nav.failure_scroll = st.nav.failure_scroll.saturating_sub(step);
    }
}

fn scroll_overlay(st: &mut AppState, down: bool, step: usize) {
    st.overlay.scroll_by(down, step);
}

fn move_overlay_cursor(st: &mut AppState, down: bool, step: usize, n_items: usize) {
    st.overlay.move_cursor(down, step, n_items);
}

pub(super) fn handle_key(
    key: event::KeyEvent,
    state: &Arc<Mutex<AppState>>,
    pkg: &Package,
    test_files: &[PathBuf],
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    cli_filters_empty: bool,
) -> Result<bool> {
    // Ctrl-C quits in all modes.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }

    // Action-table dispatch first. If a binding matches, apply it and return.
    // For uppercase ASCII letter keycodes some terminals (kitty keyboard
    // protocol, certain wezterm setups) report SHIFT in modifiers; the
    // letter itself already encodes shift, so mask it out before comparing.
    let key_mods_for_match = {
        let mut m = key.modifiers;
        if let event::KeyCode::Char(c) = key.code
            && c.is_ascii_uppercase()
        {
            m.remove(KeyModifiers::SHIFT);
        }
        m
    };
    let (mode, action_match) = {
        let st = state.lock().unwrap();
        let mode = st.mode().clone();
        let action = st
            .effective_bindings(&mode)
            .iter()
            .find(|b| b.key == key.code && b.mods == key_mods_for_match)
            .map(|b| b.action.clone());
        (mode, action)
    };
    if let Some(action) = action_match {
        return apply_action(action, state, pkg, event_tx, terminal);
    }

    // Fall back to per-mode extras for keys not in the binding table
    // (filter text input, character accumulation, terminal-suspend flows).
    match mode {
        Mode::Palette(PaletteKind::Filter) => {
            handle_filter_key(key, state);
            Ok(false)
        }
        Mode::Help => {
            handle_help_key(key, state);
            Ok(false)
        }
        Mode::Log => {
            handle_log_key(key, state);
            Ok(false)
        }
        Mode::Detail => handle_detail_key(key, state, pkg, event_tx, terminal),
        Mode::Failure => handle_failure_key(key, state, terminal),
        Mode::Palette(PaletteKind::Run) => {
            handle_runmenu_key(key, state, pkg, test_files, event_tx, cli_filters_empty)?;
            Ok(false)
        }
        Mode::Palette(PaletteKind::Sort) => {
            handle_sortmenu_key(key, state);
            Ok(false)
        }
        Mode::Normal => handle_normal_key(key, state, pkg, event_tx, terminal),
    }
}

/// Apply an `Action` resolved from a key binding. Returns `Ok(true)` only
/// for `Quit`. Any action that needs to drop the state lock (subprocess
/// spawn, $EDITOR suspend) does so locally.
fn apply_action(
    action: Action,
    state: &Arc<Mutex<AppState>>,
    pkg: &Package,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    _terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) -> Result<bool> {
    use Action::*;
    // Acquire the lock for the duration of the action unless we need to drop it.
    let mut st = state.lock().unwrap();
    let mode = st.mode().clone();
    let in_detail = matches!(mode, Mode::Detail);
    let in_failure = matches!(mode, Mode::Failure);
    // Overlay-axis flags routed through the new typed accessor.
    let overlay = mode.overlay_kind();
    let in_log = matches!(overlay, Some(Overlay::Log));
    let in_help = matches!(overlay, Some(Overlay::Help));

    // Viewport height for half-page calculations. Each mode owns a
    // distinct height field; this helper picks the relevant one.
    let vheight = |st: &AppState| -> usize {
        if in_detail        { st.nav.test_list_height }
        else if in_failure  { st.nav.failure_view_height }
        else if in_log      { st.nav.log_view_height }
        else if in_help     { st.overlay.view_height }
        else                { st.nav.file_list_height }
    };

    match action {
        CursorDown   => st.move_cursor(&mode,  1),
        CursorUp     => st.move_cursor(&mode, -1),
        CursorTop    => st.move_cursor(&mode, isize::MIN),
        CursorBottom => st.move_cursor(&mode, isize::MAX),
        FullPageDown => {
            let step = AppState::full_page(vheight(&st)) as isize;
            // Failure mode special-cases scroll on the error pane (not the
            // failures list) for half-page jumps. Detail/Log/Help/Files all
            // route through move_cursor.
            if in_failure { st.nav.failure_scroll += step as usize; }
            else { st.move_cursor(&mode, step); }
        }
        FullPageUp => {
            let step = AppState::full_page(vheight(&st)) as isize;
            if in_failure { st.nav.failure_scroll = st.nav.failure_scroll.saturating_sub(step as usize); }
            else { st.move_cursor(&mode, -step); }
        }
        EnterDetail => {
            st.nav.test_cursor = 0;
            st.push_mode(Mode::Detail);
        }
        EnterFailure => {
            // test_cursor indexes the sorted-for-display list the Detail
            // view renders, not the raw emission-order file.tests vector.
            let sorted = st.sorted_selected_tests();
            if let Some(file) = st.selected_file()
                && let Some(test) = sorted.get(st.nav.test_cursor)
                    && test.is_bad() {
                        let fname = file.name.clone();
                        let tname = test.name.clone();
                        st.nav.failure_cursor = st
                            .failures
                            .iter()
                            .position(|f| f.file == fname && f.test == tname)
                            .unwrap_or(0);
                        st.push_mode(Mode::Failure);
                    }
        }
        EnterHelp => {
            st.overlay = OverlayState::text();
            st.push_mode(Mode::Help);
        }
        EnterLog => {
            st.nav.log_scroll = 0;
            st.push_mode(Mode::Log);
        }
        OpenPalette(kind) => match kind {
            PaletteKind::Filter => {
                st.filter.pre_filter = st.filter.active.clone();
                st.filter.input.clear();
                st.push_mode(Mode::Palette(PaletteKind::Filter));
            }
            PaletteKind::Run => {
                if !st.run.running {
                    st.overlay = OverlayState::menu();
                    st.push_mode(Mode::Palette(PaletteKind::Run));
                }
            }
            PaletteKind::Sort => {
                // Pre-select the current sort mode. In Detail mode we sort
                // the test list; in Normal mode we sort the file list.
                use super::state::SortMode;
                let active = if matches!(st.mode(), Mode::Detail) {
                    st.display.test_sort_mode
                } else {
                    st.display.sort_mode
                };
                let cursor = SortMode::ALL
                    .iter()
                    .position(|&m| m == active)
                    .unwrap_or(0);
                st.overlay = OverlayState::menu();
                st.overlay.cursor = Some(cursor);
                st.push_mode(Mode::Palette(PaletteKind::Sort));
            }
        },
        Pop => {
            // Help-specific reset.
            if matches!(st.mode(), Mode::Help) {
                st.overlay.scroll = 0;
            }
            st.pop_mode();
        }
        ToggleSelect => {
            if let Some(idx) = st.selected_file_mut_idx() {
                let path = st.files[idx].path.clone();
                if !st.multi.selected.remove(&path) {
                    st.multi.selected.insert(path);
                }
                if st.multi.visual_anchor.is_some() {
                    st.multi.visual_base = st.multi.selected.clone();
                    st.multi.visual_anchor = Some(st.nav.file_cursor);
                }
            }
        }
        ToggleVisual => {
            if st.multi.visual_anchor.is_some() {
                st.multi.visual_anchor = None;
                st.multi.visual_base.clear();
            } else {
                st.multi.visual_anchor = Some(st.nav.file_cursor);
                st.multi.visual_base = st.multi.selected.clone();
                st.apply_visual();
            }
        }
        CycleStatusFilter => {
            st.filter.status = st.filter.status.next_supported(&st.filter.supported_outcomes);
            st.nav.file_cursor = 0;
            st.nav.file_scroll = 0;
        }
        CycleStatusFilterBack => {
            st.filter.status = st.filter.status.prev_supported(&st.filter.supported_outcomes);
            st.nav.file_cursor = 0;
            st.nav.file_scroll = 0;
        }
        CycleToolFilter => {
            st.filter.suite.cycle_next();
            st.nav.file_cursor = 0;
            st.nav.file_scroll = 0;
        }
        CycleToolFilterBack => {
            st.filter.suite.cycle_prev();
            st.nav.file_cursor = 0;
            st.nav.file_scroll = 0;
        }
        CycleGroupFilter => {
            st.cycle_group_filter(1);
            st.nav.file_cursor = 0;
            st.nav.file_scroll = 0;
        }
        CycleGroupFilterBack => {
            st.cycle_group_filter(-1);
            st.nav.file_cursor = 0;
            st.nav.file_scroll = 0;
        }
        ShrinkList => {
            let p = st.current_list_pct().saturating_sub(LIST_PCT_STEP);
            st.set_current_list_pct(p);
        }
        GrowList => {
            let p = st.current_list_pct().saturating_add(LIST_PCT_STEP);
            st.set_current_list_pct(p);
        }
        ToggleOrientation => st.toggle_current_horizontal(),
        SourceScrollUp => st.nav.source_scroll = st.nav.source_scroll.saturating_sub(1),
        SourceScrollDown => {
            st.nav.source_scroll = (st.nav.source_scroll + 1).min(st.nav.source_scroll_max);
        }
        CancelFile => {
            if st.run.running
                && let Some(handle) = st.run.cancel.clone() {
                    let cursor_path = st.selected_file_mut_idx().map(|i| st.files[i].path.clone());
                    let targets: Vec<PathBuf> = match &cursor_path {
                        Some(p) if st.multi.selected.contains(p) && st.multi.selected.len() > 1 => {
                            st.multi.selected.iter().cloned().collect()
                        }
                        Some(p) => vec![p.clone()],
                        None => Vec::new(),
                    };
                    for p in &targets {
                        handle.cancel_file(p);
                    }
                    if !targets.is_empty() {
                        let label = if targets.len() == 1 {
                            targets[0]
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        } else {
                            format!("{} files", targets.len())
                        };
                        st.log.push("scrutin", &format!("cancel: {}\n", label));
                    }
                }
        }
        CancelAll => {
            if st.run.running
                && let Some(handle) = st.run.cancel.clone() {
                    handle.cancel_all();
                    st.log.push("scrutin", "cancel: entire run\n");
                }
        }
        RunCurrentFile => {
            if !st.run.running {
                // If a multi-file selection exists, run it. Otherwise run
                // the file under the cursor.
                let targets: Vec<PathBuf> = if !st.multi.selected.is_empty() {
                    st.multi.selected.iter().cloned().collect()
                } else if let Some(idx) = st.selected_file_mut_idx() {
                    vec![st.files[idx].path.clone()]
                } else {
                    Vec::new()
                };
                if !targets.is_empty() {
                    drop(st);
                    start_test_run(pkg, &targets, state, event_tx, false)?;
                    return Ok(false);
                }
            }
        }
        Quit => return Ok(true),
        YankMessage => {
            let msg = if in_failure {
                st.failures.get(st.nav.failure_cursor).map(|f| f.message.clone())
            } else if in_detail {
                // test_cursor indexes the sorted-for-display list.
                st.sorted_selected_tests()
                    .get(st.nav.test_cursor)
                    .map(|t| t.message.clone())
            } else {
                st.selected_file().map(|f| {
                    f.tests
                        .iter()
                        .find(|t| t.is_bad())
                        .map(|t| t.message.clone())
                        .unwrap_or_else(|| f.name.clone())
                })
            };
            if let Some(msg) = msg {
                osc52_copy(&msg);
                st.log.push("scrutin", "copied to clipboard\n");
            }
        }
        EditTest => {
            // Capture the sorted list before we borrow `file` — test_cursor
            // indexes the sorted-for-display list in Detail mode.
            let sorted_line_at_cursor = if in_detail {
                st.sorted_selected_tests()
                    .get(st.nav.test_cursor)
                    .and_then(|t| t.line)
            } else {
                None
            };
            if let Some(file) = st.selected_file() {
                let path = file.path.clone();
                let line = if in_failure {
                    st.failures.get(st.nav.failure_cursor).and_then(|f| f.line)
                } else if in_detail {
                    sorted_line_at_cursor
                } else {
                    file.tests.iter().find(|t| t.is_bad()).and_then(|t| t.line)
                };
                let pp = st.poll_paused.clone();
                drop(st);
                suspend_tui(_terminal, pp.as_ref(), || open_in_editor(&path, line))?;
                return Ok(false);
            }
        }
        EditSource => {
            if let Some(file) = st.selected_file() {
                let source = find_source_for_test(&file.name, &st.pkg_root, &st.reverse_dep_map);
                let pp = st.poll_paused.clone();
                drop(st);
                if let Some(path) = source {
                    suspend_tui(_terminal, pp.as_ref(), || open_in_editor(&path, None))?;
                }
                return Ok(false);
            }
        }
    }
    Ok(false)
}

fn handle_filter_key(key: event::KeyEvent, state: &Arc<Mutex<AppState>>) {
    let mut st = state.lock().unwrap();
    match key.code {
        KeyCode::Esc => {
            st.filter.active = st.filter.pre_filter.take();
            st.filter.input.clear();
            st.nav.file_cursor = 0;
            st.pop_mode();
        }
        KeyCode::Enter => {
            let pat = st.filter.input.clone();
            st.filter.active = make_filter_pattern(&pat);
            st.filter.input.clear();
            st.nav.file_cursor = 0;
            st.pop_mode();
        }
        KeyCode::Backspace => {
            st.filter.input.pop();
            st.filter.active = make_filter_pattern(&st.filter.input);
            st.nav.file_cursor = 0;
        }
        KeyCode::Char(c) => {
            st.filter.input.push(c);
            st.filter.active = make_filter_pattern(&st.filter.input);
            st.nav.file_cursor = 0;
        }
        _ => {}
    }
}

fn handle_help_key(_key: event::KeyEvent, _state: &Arc<Mutex<AppState>>) {
    // Everything handled via the action table / HELP_BINDINGS → CursorUp/Down etc
    // (Help mode shares the same navigation verbs as the other scroll views).
}

fn handle_log_key(_key: event::KeyEvent, _state: &Arc<Mutex<AppState>>) {
    // Fully handled via LOG_BINDINGS → apply_action.
}

fn handle_detail_key(
    key: event::KeyEvent,
    state: &Arc<Mutex<AppState>>,
    pkg: &Package,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) -> Result<bool> {
    // Most keys are dispatched via DETAIL_BINDINGS → apply_action. This
    // "extras" fn handles Ctrl-o (yank path:line) and 0-9 (accept the Nth
    // spell-check suggestion attached to the cursor test).
    let _ = terminal;
    if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let st = state.lock().unwrap();
        if let Some(file) = st.selected_file() {
            // test_cursor indexes the sorted-for-display list.
            let sorted = st.sorted_selected_tests();
            let s = match sorted.get(st.nav.test_cursor) {
                Some(t) if t.line.is_some() => {
                    format!("{}:{}", file.path.display(), t.line.unwrap_or(0))
                }
                _ => file.path.to_string_lossy().to_string(),
            };
            osc52_copy(&s);
        }
        return Ok(false);
    }

    if let KeyCode::Char(c) = key.code
        && c.is_ascii_digit()
        && key.modifiers == KeyModifiers::NONE
    {
        // Digits route to spell-check suggestions when the cursor event
        // has corrections; otherwise to the Nth plugin action (ruff/jarl
        // fix variants). This keeps the chip row's numbering meaningful
        // in both contexts without colliding.
        let has_corrections = {
            let st = state.lock().unwrap();
            st.sorted_selected_tests()
                .get(st.nav.test_cursor)
                .is_some_and(|t| !t.corrections.is_empty())
        };
        if has_corrections {
            match c {
                '0' => add_word_to_dictionary(pkg, state, event_tx),
                _ => {
                    let idx = (c as u8 - b'1') as usize;
                    accept_suggestion_in_detail(state, event_tx, idx);
                }
            }
        } else if c != '0' {
            let idx = (c as u8 - b'1') as usize;
            invoke_plugin_action_by_index(idx, state, event_tx, terminal);
        }
    }
    Ok(false)
}

/// Invoke the Nth `PluginAction` defined for the cursor file's suite,
/// matching what the `a` palette would do when row `idx` was selected.
/// Silently no-ops when the index is out of range.
fn invoke_plugin_action_by_index(
    idx: usize,
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) {
    use scrutin_core::project::plugin::ActionScope;
    let action_paths_cwd = {
        let st = state.lock().unwrap();
        st.selected_plugin_actions()
            .and_then(|actions| actions.get(idx).cloned())
            .and_then(|pa| {
                let file = st.selected_file()?;
                let suite = file.suite.clone();
                let cwd = st.suite_root(&suite);
                let paths: Vec<PathBuf> = match pa.scope {
                    ActionScope::File => vec![file.path.clone()],
                    ActionScope::All => st
                        .files
                        .iter()
                        .filter(|fe| fe.suite == suite)
                        .map(|fe| fe.path.clone())
                        .collect(),
                };
                Some((pa, paths, cwd))
            })
    };
    if let Some((pa, paths, cwd)) = action_paths_cwd {
        let _ = run_plugin_action(&pa, &paths, &cwd, state, event_tx, terminal);
    }
}

/// Apply the Nth ranked suggestion (0-based) of the cursor test's first
/// attached `Correction` to the file on disk, then trigger a rerun so the
/// refreshed test list picks up the change. No-op when the current test
/// has no corrections (non-spell-check findings) or `n` is out of range.
fn accept_suggestion_in_detail(
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    n: usize,
) {
    let (file_path, correction, replacement) = {
        let st = state.lock().unwrap();
        let Some(file) = st.selected_file() else { return };
        let sorted = st.sorted_selected_tests();
        let Some(test) = sorted.get(st.nav.test_cursor) else { return };
        let Some(correction) = test.corrections.first().cloned() else { return };
        let Some(replacement) = correction.suggestions.get(n).cloned() else { return };
        (file.path.clone(), correction, replacement)
    };

    match scrutin_core::prose::skyspell::apply_correction_to_file(
        &file_path,
        &correction,
        &replacement,
    ) {
        Ok(()) => {
            let st = state.lock().unwrap();
            st.log.push(
                "scrutin",
                &format!(
                    "spell: replaced '{}' with '{}'\n",
                    correction.word, replacement
                ),
            );
            drop(st);
            let _ = event_tx.send(TuiEvent::WatchEvent(vec![file_path]));
        }
        Err(e) => {
            let st = state.lock().unwrap();
            st.log
                .push("scrutin", &format!("spell: apply failed: {}\n", e));
        }
    }
}

/// Shell out to skyspell's `add` subcommand via the shared helper and
/// log the result. No-op when the cursor test doesn't come from the
/// skyspell suite or has no corrections attached.
fn add_word_to_dictionary(
    pkg: &Package,
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) {
    use scrutin_core::prose::skyspell::{add_word_to_dict, AddScope};

    let (file_path, word, suite_root) = {
        let st = state.lock().unwrap();
        let Some(file) = st.selected_file() else { return };
        if file.suite != "skyspell" {
            return;
        }
        let sorted = st.sorted_selected_tests();
        let Some(test) = sorted.get(st.nav.test_cursor) else { return };
        let Some(correction) = test.corrections.first() else { return };
        let Some(suite_root) = st.suite_roots.get("skyspell").cloned() else { return };
        (file.path.clone(), correction.word.clone(), suite_root)
    };

    match add_word_to_dict(
        &suite_root,
        &pkg.skyspell_extra_args,
        &pkg.skyspell_add_args,
        &word,
    ) {
        Ok(scope) => {
            let label = match scope {
                AddScope::Project(path) => path.display().to_string(),
                AddScope::Global => "skyspell global dictionary".into(),
            };
            let st = state.lock().unwrap();
            st.log.push(
                "scrutin",
                &format!("spell: added '{}' to {}\n", word, label),
            );
            drop(st);
            let _ = event_tx.send(TuiEvent::WatchEvent(vec![file_path]));
        }
        Err(e) => {
            let st = state.lock().unwrap();
            st.log
                .push("scrutin", &format!("spell: add failed: {}\n", e));
        }
    }
}

fn handle_failure_key(
    key: event::KeyEvent,
    state: &Arc<Mutex<AppState>>,
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) -> Result<bool> {
    // Navigation + Esc + edit palette handled by bindings/apply_action.
    // Nothing left in failure extras.
    let _ = (key, state, terminal);
    Ok(false)
}

fn handle_runmenu_key(
    key: event::KeyEvent,
    state: &Arc<Mutex<AppState>>,
    pkg: &Package,
    test_files: &[PathBuf],
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    cli_filters_empty: bool,
) -> Result<()> {
    let mut st = state.lock().unwrap();
    let n_items = 4 + st.run_groups.len();
    let dispatch = |sel: usize, st: std::sync::MutexGuard<AppState>| -> Result<()> {
        run_menu_dispatch(sel, st, pkg, test_files, state, event_tx, cli_filters_empty)
    };
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            st.pop_mode();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if st.overlay.cursor_pos() + 1 < n_items {
                *st.overlay.cursor_mut() += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let c = st.overlay.cursor_mut();
            *c = c.saturating_sub(1);
        }
        KeyCode::Char('g') | KeyCode::Home => {
            st.overlay.cursor = Some(0);
        }
        KeyCode::Char('G') | KeyCode::End => {
            st.overlay.cursor = Some(n_items.saturating_sub(1));
        }
        KeyCode::Char('a') => {
            st.overlay.cursor = Some(0);
            dispatch(0, st)?;
        }
        KeyCode::Char('v') => {
            st.overlay.cursor = Some(1);
            dispatch(1, st)?;
        }
        KeyCode::Char('s') => {
            st.overlay.cursor = Some(2);
            dispatch(2, st)?;
        }
        KeyCode::Char('f') => {
            st.overlay.cursor = Some(3);
            dispatch(3, st)?;
        }
        KeyCode::Char('u') => {
            st.overlay.cursor = Some(4);
            dispatch(4, st)?;
        }
        KeyCode::Enter => {
            let sel = st.overlay.cursor_pos();
            dispatch(sel, st)?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_sortmenu_key(key: event::KeyEvent, state: &Arc<Mutex<AppState>>) {
    use super::state::SortMode;
    let modes = SortMode::ALL;
    let mut st = state.lock().unwrap();
    let n = modes.len();
    if st.overlay.cursor_pos() >= n {
        st.overlay.cursor = Some(n - 1);
    }
    // Determine context: if the mode below the palette is Detail, we're
    // sorting tests within a file; otherwise sorting the file list.
    let in_detail = st.nav.mode_stack.len() >= 2
        && matches!(st.nav.mode_stack[st.nav.mode_stack.len() - 2], Mode::Detail);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('s') => {
            st.pop_mode();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if st.overlay.cursor_pos() + 1 < n {
                *st.overlay.cursor_mut() += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let c = st.overlay.cursor_mut();
            *c = c.saturating_sub(1);
        }
        KeyCode::Enter => {
            let picked = modes[st.overlay.cursor_pos()];
            if in_detail {
                if picked == st.display.test_sort_mode {
                    st.display.test_sort_reversed = !st.display.test_sort_reversed;
                } else {
                    st.display.test_sort_mode = picked;
                    st.display.test_sort_reversed = false;
                }
                st.nav.test_cursor = 0;
                st.nav.test_scroll = 0;
            } else {
                if picked == st.display.sort_mode {
                    st.display.sort_reversed = !st.display.sort_reversed;
                } else {
                    st.display.sort_mode = picked;
                    st.display.sort_reversed = false;
                }
                st.nav.file_cursor = 0;
                st.nav.file_scroll = 0;
            }
            st.pop_mode();
        }
        _ => {}
    }
}

fn handle_normal_key(
    key: event::KeyEvent,
    state: &Arc<Mutex<AppState>>,
    pkg: &Package,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) -> Result<bool> {
    // Almost every Normal-mode verb is handled via NORMAL_BINDINGS →
    // apply_action. What lives here is: Ctrl-o yank path, Shift-Down/Up
    // visual-range extend, context-sensitive Esc, and Ctrl-c quit.
    let _ = (pkg, event_tx, terminal);
    let mut st = state.lock().unwrap();
    let n_visible = st.visible_files().len();
    let mut cursor_moved = false;
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(file) = st.selected_file() {
                osc52_copy(&file.path.to_string_lossy());
                st.log
                    .push("scrutin", &format!("copied: {}\n", file.path.display()));
            }
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            if st.multi.visual_anchor.is_none() {
                st.multi.visual_anchor = Some(st.nav.file_cursor);
                st.multi.visual_base = st.multi.selected.clone();
            }
            if st.nav.file_cursor + 1 < n_visible {
                st.nav.file_cursor += 1;
            }
            cursor_moved = true;
        }
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            if st.multi.visual_anchor.is_none() {
                st.multi.visual_anchor = Some(st.nav.file_cursor);
                st.multi.visual_base = st.multi.selected.clone();
            }
            if st.nav.file_cursor > 0 {
                st.nav.file_cursor -= 1;
            }
            cursor_moved = true;
        }
        KeyCode::Esc => {
            if st.multi.visual_anchor.is_some() {
                // First Esc: leave visual mode, keep selection.
                st.multi.visual_anchor = None;
                st.multi.visual_base.clear();
            } else if !st.multi.selected.is_empty() {
                st.multi.selected.clear();
            } else if st.filter.active.is_some() {
                st.filter.active = None;
                st.nav.file_cursor = 0;
            }
        }
        _ => {}
    }
    if cursor_moved && st.multi.visual_anchor.is_some() {
        st.apply_visual();
    }
    Ok(false)
}


fn run_menu_dispatch(
    sel: usize,
    mut st: std::sync::MutexGuard<AppState>,
    pkg: &Package,
    test_files: &[PathBuf],
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    cli_filters_empty: bool,
) -> Result<()> {
    st.pop_mode();
    if st.run.running {
        return Ok(());
    }
    match sel {
        0 => {
            drop(st);
            start_test_run(pkg, test_files, state, event_tx, cli_filters_empty)?;
        }
        1 => {
            // Run the currently visible files — whatever the filter
            // dimensions (text / plugin / status) allow through right
            // now. Snapshotted at click time; edits to filters mid-run
            // don't change the running set.
            let vis: Vec<PathBuf> = st
                .visible_files()
                .into_iter()
                .map(|i| st.files[i].path.clone())
                .collect();
            if vis.is_empty() {
                st.log.push("scrutin", "run: no visible files\n");
                return Ok(());
            }
            drop(st);
            start_test_run(pkg, &vis, state, event_tx, false)?;
        }
        2 => {
            let sel: Vec<PathBuf> = st
                .visible_files()
                .into_iter()
                .map(|i| &st.files[i])
                .filter(|e| st.multi.selected.contains(&e.path))
                .map(|e| e.path.clone())
                .collect();
            if sel.is_empty() {
                st.log.push("scrutin", "run: nothing selected\n");
                return Ok(());
            }
            drop(st);
            start_test_run(pkg, &sel, state, event_tx, false)?;
        }
        3 => {
            let failed: Vec<PathBuf> = st
                .files
                .iter()
                .filter(|e| matches!(e.status, FileStatus::Failed { .. }))
                .map(|e| e.path.clone())
                .collect();
            if failed.is_empty() {
                st.log.push("scrutin", "run: nothing failed\n");
                return Ok(());
            }
            drop(st);
            start_test_run(pkg, &failed, state, event_tx, false)?;
        }
        4 => {
            // Run only test files affected by uncommitted git changes.
            // Disabled in the menu UI when git isn't available, but
            // re-check here defensively in case the entry is reached
            // some other way (Enter on a stale cursor, etc.).
            use scrutin_core::analysis::deps::{TestAction, resolve_tests};
            use scrutin_core::git::{GitAvailability, changed_paths};

            let repo_root = match &st.git {
                GitAvailability::Available { repo_root } => repo_root.clone(),
                GitAvailability::NotARepo => {
                    st.log.push("scrutin", "run: not a git repository\n");
                    return Ok(());
                }
                GitAvailability::NotInstalled => {
                    st.log.push("scrutin", "run: git not found on PATH\n");
                    return Ok(());
                }
                GitAvailability::ProbeFailed { stderr } => {
                    st.log
                        .push("scrutin", &format!("run: git probe failed: {stderr}\n"));
                    return Ok(());
                }
            };
            let pkg_root = st.pkg_root.clone();
            let dep_map = st.dep_map.clone();
            drop(st);

            let changed = match changed_paths(&repo_root, &pkg_root) {
                Ok(p) => p,
                Err(e) => {
                    let st = state.lock().unwrap();
                    st.log.push("scrutin", &format!("run: {}\n", e));
                    return Ok(());
                }
            };
            if changed.is_empty() {
                let st = state.lock().unwrap();
                st.log.push("scrutin", "run: no uncommitted changes\n");
                return Ok(());
            }

            // Only consider files any active suite recognizes as a source
            // or test file. Otherwise unrelated working-tree changes
            // (README, Cargo.toml, .gitignore, ...) hit `resolve_tests`,
            // which returns FullSuite for "I don't know what this is",
            // causing the whole suite to run on any working-tree edit.
            //
            // Also explicitly strip `.scrutin/` paths: scrutin rewrites
            // its own runner scripts there on every run, so git reports
            // them as modified, but they're generated state and must
            // never drive test selection. The shared walker already
            // ignores `.scrutin/` for discovery; this mirrors that for
            // the git-driven path.
            let is_scrutin_state = |p: &PathBuf| -> bool {
                p.strip_prefix(&pkg_root)
                    .ok()
                    .and_then(|rel| rel.components().next())
                    .and_then(|c| c.as_os_str().to_str())
                    .map(|s| s == ".scrutin")
                    .unwrap_or(false)
            };
            let relevant: Vec<&PathBuf> = changed
                .iter()
                .filter(|p| !is_scrutin_state(p))
                .filter(|p| pkg.is_any_test_file(p) || pkg.is_any_source_file(p))
                .collect();
            if relevant.is_empty() {
                let st = state.lock().unwrap();
                st.log.push(
                    "scrutin",
                    "run: no uncommitted source or test files\n",
                );
                return Ok(());
            }

            let mut tests_to_run: Vec<PathBuf> = Vec::new();
            let mut run_full = false;
            for path in &relevant {
                match resolve_tests(path, pkg, dep_map.as_ref()) {
                    TestAction::Run(files) => tests_to_run.extend(files),
                    TestAction::FullSuite => {
                        run_full = true;
                        break;
                    }
                }
            }

            let files: Vec<PathBuf> = if run_full {
                test_files.to_vec()
            } else {
                tests_to_run.sort();
                tests_to_run.dedup();
                tests_to_run
            };

            if files.is_empty() {
                let st = state.lock().unwrap();
                st.log
                    .push("scrutin", "run: no test files affected by uncommitted changes\n");
                return Ok(());
            }
            start_test_run(pkg, &files, state, event_tx, false)?;
        }
        i => {
            let g = match st.run_groups.get(i - 5) {
                Some(g) => g.clone(),
                None => return Ok(()),
            };
            drop(st);
            let mut files: Vec<PathBuf> = test_files.to_vec();
            scrutin_core::filter::apply_include_exclude(&mut files, &g.include, &g.exclude);
            if !files.is_empty() {
                start_test_run(pkg, &files, state, event_tx, false)?;
            }
        }
    }
    Ok(())
}

fn open_in_editor(path: &Path, line: Option<u32>) {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let path_str = path.to_string_lossy();
    let mut cmd = std::process::Command::new(&editor);
    if let Some(line) = line {
        match editor.as_str() {
            "code" | "code-insiders" | "positron" => {
                cmd.arg("--goto").arg(format!("{}:{}", path_str, line));
            }
            _ => {
                cmd.arg(format!("+{}", line)).arg(path);
            }
        }
    } else {
        cmd.arg(path);
    }
    let _ = cmd
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
}

fn suspend_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    poll_paused: Option<&Arc<AtomicBool>>,
    action: impl FnOnce(),
) -> Result<()> {
    if let Some(p) = poll_paused {
        p.store(true, Ordering::Relaxed);
    }
    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    action();
    crossterm::execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    enable_raw_mode()?;
    terminal.clear()?;
    if let Some(p) = poll_paused {
        p.store(false, Ordering::Relaxed);
    }
    Ok(())
}

fn run_plugin_action(
    action: &scrutin_core::project::plugin::PluginAction,
    file_paths: &[PathBuf],
    cwd: &Path,
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    _terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) -> Result<bool> {
    let mut cmd = std::process::Command::new(&action.command[0]);
    for arg in &action.command[1..] {
        cmd.arg(arg);
    }
    for p in file_paths {
        cmd.arg(p);
    }
    let label = action.label;
    let rerun = action.rerun;
    let rerun_paths = file_paths.to_vec();

    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Capture tool output into the shared log buffer so users can review
    // it via Mode::Log (`L`). No separate overlay: fix actions should feel
    // as invisible as possible when they succeed and be easy to inspect
    // when they don't.
    let tag = action.name;
    let output = cmd.output();
    {
        let st = state.lock().unwrap();
        st.log.push("scrutin", &format!("{label}: running\n"));
        match &output {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                // Tool output (ruff --fix, jarl --fix) typically carries
                // ANSI colors; strip them so the log pane stays readable.
                for line in stderr.lines() {
                    st.log.push(tag, &format!("{}\n", super::view::strip_ansi(line)));
                }
                for line in stdout.lines() {
                    st.log.push(tag, &format!("{}\n", super::view::strip_ansi(line)));
                }
                if out.status.success() {
                    st.log.push("scrutin", &format!("{label}: done\n"));
                } else {
                    st.log
                        .push("scrutin", &format!("{label}: exited {}\n", out.status));
                }
            }
            Err(e) => {
                st.log.push("scrutin", &format!("{label}: {e}\n"));
            }
        }
    }

    if rerun {
        {
            let st = state.lock().unwrap();
            st.log
                .push("scrutin", &format!("re-running after {label}\n"));
        }
        let _ = event_tx.send(TuiEvent::WatchEvent(rerun_paths));
    }

    Ok(false)
}

fn make_filter_pattern(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    if input.contains('*') {
        Some(input.to_string())
    } else {
        Some(format!("*{}*", input))
    }
}

