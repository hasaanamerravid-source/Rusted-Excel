mod core;

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use fltk::{
    app,
    button::{Button, ToggleButton},
    dialog::{self, FileDialog, FileDialogAction, FileDialogOptions, FileDialogType},
    draw,
    enums::{Align, Color, Event, Font, FrameType, Key, Shortcut},
    frame::Frame,
    group::Group,
    image::PngImage,
    input::Input,
    menu::{MenuBar, MenuFlag},
    prelude::*,
    table::{Table, TableContext},
    window::Window,
};

use core::{csv_escape, parse_csv, Sheet};

const APP_NAME: &str = "Rusted Excel";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const ROWS: i32 = 100;
const COLS: i32 = 26;
const MAX_TABS: usize = 5;
const MAX_LOAD_ROWS: i32 = 5000;
const MAX_LOAD_COLS: i32 = 200;
const MAX_UNDO: usize = 50;

const MENU_H: i32 = 26;
const TOOL_H: i32 = 36;
const BAR_H: i32 = 30;
const TABS_H: i32 = 28;
const STATUS_H: i32 = 24;
const GRID_Y: i32 = MENU_H + TOOL_H + BAR_H;
const WIN_W: i32 = 1280;
const WIN_H: i32 = 800;

const GREEN: Color = Color::from_rgb(16, 110, 45);
const GREEN_LIGHT: Color = Color::from_rgb(222, 244, 226);
const HEADER_BG: Color = Color::from_rgb(238, 240, 234);
const GRID_LINE: Color = Color::from_rgb(205, 208, 202);
const TEXT: Color = Color::from_rgb(30, 30, 30);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct CellStyle {
    bold: bool,
    italic: bool,
    align: u8, // 0 = automatic, 1 = left, 2 = center, 3 = right
}

#[derive(Clone)]
struct TabSnapshot {
    name: String,
    rows: i32,
    cols: i32,
    cells: Vec<String>,
    styles: HashMap<(i32, i32), CellStyle>,
    sel: (i32, i32, i32, i32),
}

struct Tab {
    name: String,
    sheet: Sheet,
    styles: HashMap<(i32, i32), CellStyle>,
    sel: (i32, i32, i32, i32),
}

impl Tab {
    fn new(name: String) -> Self {
        Self {
            name,
            sheet: Sheet::new(ROWS, COLS),
            styles: HashMap::new(),
            sel: (0, 0, 0, 0),
        }
    }
}

#[derive(Clone)]
struct Snapshot {
    tabs: Vec<TabSnapshot>,
    active: usize,
    sel: (i32, i32, i32, i32),
    path: Option<PathBuf>,
    dirty: bool,
}

struct State {
    tabs: Vec<Tab>,
    active: usize,
    sel: (i32, i32, i32, i32), // anchor row, anchor col, focus row, focus col
    editing: bool,
    edit_target: Option<(i32, i32)>,
    formula_target: Option<(i32, i32)>,
    path: Option<PathBuf>,
    dirty: bool,
    clipboard: Vec<Vec<String>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl State {
    fn new() -> Self {
        Self {
            tabs: vec![Tab::new("Sheet1".to_string())],
            active: 0,
            sel: (0, 0, 0, 0),
            editing: false,
            edit_target: None,
            formula_target: None,
            path: None,
            dirty: false,
            clipboard: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn cell(&self) -> (i32, i32) {
        (self.sel.2, self.sel.3)
    }

    fn bounds(&self) -> (i32, i32, i32, i32) {
        let (r1, c1, r2, c2) = self.sel;
        (r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2))
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tabs: self
                .tabs
                .iter()
                .map(|tab| {
                    let rows = tab.sheet.rows().max(1);
                    let cols = tab.sheet.cols().max(1);
                    let mut cells = Vec::with_capacity((rows * cols) as usize);
                    for r in 0..rows {
                        for c in 0..cols {
                            cells.push(tab.sheet.raw(r, c));
                        }
                    }
                    TabSnapshot {
                        name: tab.name.clone(),
                        rows,
                        cols,
                        cells,
                        styles: tab.styles.clone(),
                        sel: tab.sel,
                    }
                })
                .collect(),
            active: self.active,
            sel: self.sel,
            path: self.path.clone(),
            dirty: self.dirty,
        }
    }

    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn restore(&mut self, snap: &Snapshot) {
        self.tabs = snap
            .tabs
            .iter()
            .map(|t| {
                let rows = t.rows.max(1);
                let cols = t.cols.max(1);
                let mut sheet = Sheet::new(rows, cols);
                for (i, text) in t.cells.iter().enumerate() {
                    if i >= (rows * cols) as usize {
                        break;
                    }
                    sheet.set((i / cols as usize) as i32, (i % cols as usize) as i32, text);
                }
                Tab {
                    name: t.name.clone(),
                    sheet,
                    styles: t.styles.clone(),
                    sel: t.sel,
                }
            })
            .collect();
        if self.tabs.is_empty() {
            self.tabs.push(Tab::new("Sheet1".to_string()));
        }
        self.active = snap.active.min(self.tabs.len() - 1);
        let rows = self.tabs[self.active].sheet.rows();
        let cols = self.tabs[self.active].sheet.cols();
        self.sel = (
            snap.sel.0.clamp(0, rows - 1),
            snap.sel.1.clamp(0, cols - 1),
            snap.sel.2.clamp(0, rows - 1),
            snap.sel.3.clamp(0, cols - 1),
        );
        self.tabs[self.active].sel = self.sel;
        self.path = snap.path.clone();
        self.dirty = snap.dirty;
        self.editing = false;
        self.edit_target = None;
        self.formula_target = None;
    }
}

type Shared = Rc<RefCell<State>>;

#[derive(Clone)]
struct Ui {
    win: Window,
    menu: MenuBar,
    toolbar: Group,
    fbar: Group,
    name: Input,
    formula: Input,
    table: Table,
    editor: Input,
    tabs_group: Group,
    tab_buttons: Vec<Button>,
    add_tab: Button,
    sheet_info: Frame,
    status: Frame,
    bold: ToggleButton,
    italic: ToggleButton,
    left: ToggleButton,
    center: ToggleButton,
    right: ToggleButton,
}

fn file_label(state: &State) -> String {
    match &state.path {
        Some(path) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string()),
        None => "Untitled".to_string(),
    }
}

fn refresh_title(state: &Shared, ui: &Ui) {
    let s = state.borrow();
    let star = if s.dirty { " *" } else { "" };
    let mut win = ui.win.clone();
    win.set_label(&format!("{} — {}{}", APP_NAME, file_label(&s), star));
}

fn refresh_status(state: &Shared, ui: &Ui) {
    let s = state.borrow();
    let (r1, c1, r2, c2) = s.bounds();
    let tab = &s.tabs[s.active];
    let mut sum = 0.0;
    let mut count = 0;
    for r in r1..=r2 {
        for c in c1..=c2 {
            if let Ok(n) = tab.sheet.display(r, c).parse::<f64>() {
                sum += n;
                count += 1;
            }
        }
    }
    let summary = if count > 0 {
        format!(
            "Average: {:.2}    Sum: {:.2}    Count: {}",
            sum / count as f64,
            sum,
            count
        )
    } else {
        "Ready".to_string()
    };
    let save_state = if s.dirty {
        "Unsaved changes"
    } else {
        "All changes saved"
    };
    let mut status = ui.status.clone();
    status.set_label(&format!(
        "Sheet {} of {}    |    {}    |    {}",
        s.active + 1,
        s.tabs.len(),
        summary,
        save_state
    ));
}

fn refresh_tabs(state: &Shared, ui: &Ui) {
    let s = state.borrow();
    for (i, button) in ui.tab_buttons.iter().enumerate() {
        let mut button = button.clone();
        if i < s.tabs.len() {
            button.set_label(&s.tabs[i].name);
            button.show();
            if i == s.active {
                button.set_color(Color::White);
                button.set_label_color(GREEN);
            } else {
                button.set_color(Color::from_rgb(224, 226, 220));
                button.set_label_color(Color::from_rgb(70, 70, 70));
            }
        } else {
            button.hide();
        }
    }
    let mut add_tab = ui.add_tab.clone();
    if s.tabs.len() >= MAX_TABS {
        add_tab.hide();
    } else {
        add_tab.show();
    }
}

fn refresh_styles(state: &Shared, ui: &Ui) {
    let s = state.borrow();
    let (r, c) = s.cell();
    let style = s.tabs[s.active]
        .styles
        .get(&(r, c))
        .copied()
        .unwrap_or_default();
    ui.bold.clone().set(style.bold);
    ui.italic.clone().set(style.italic);
    ui.left.clone().set(style.align == 1);
    ui.center.clone().set(style.align == 2);
    ui.right.clone().set(style.align == 3);
}

fn refresh_all(state: &Shared, ui: &Ui) {
    if !state.borrow().editing {
        ui.editor.clone().hide();
    }
    let (rows, cols) = {
        let s = state.borrow();
        let tab = &s.tabs[s.active];
        (tab.sheet.rows(), tab.sheet.cols())
    };
    let (r1, c1, r2, c2) = {
        let s = state.borrow();
        s.bounds()
    };
    let mut table = ui.table.clone();
    table.set_rows(rows);
    table.set_cols(cols);
    table.set_selection(r1, c1, r2, c2);
    table.redraw();

    {
        let s = state.borrow();
        let (r, c) = s.cell();
        let raw = s.tabs[s.active].sheet.raw(r, c);
        ui.name.clone().set_value(&Sheet::name(r, c));
        // Do not overwrite text the user is currently editing in the formula bar.
        if s.formula_target.is_none() {
            ui.formula.clone().set_value(&raw);
        }
    }
    refresh_status(state, ui);
    refresh_tabs(state, ui);
    refresh_title(state, ui);
    refresh_styles(state, ui);
}

fn select_range(state: &Shared, ui: &Ui, r1: i32, c1: i32, r2: i32, c2: i32) {
    let (r1, c1, r2, c2) = {
        let s = state.borrow();
        let rows = s.tabs[s.active].sheet.rows();
        let cols = s.tabs[s.active].sheet.cols();
        (
            r1.clamp(0, rows - 1),
            c1.clamp(0, cols - 1),
            r2.clamp(0, rows - 1),
            c2.clamp(0, cols - 1),
        )
    };
    {
        let mut s = state.borrow_mut();
        s.sel = (r1, c1, r2, c2);
        let idx = s.active;
        s.tabs[idx].sel = s.sel;
    }
    refresh_all(state, ui);
}

fn select_cell(state: &Shared, ui: &Ui, row: i32, col: i32) {
    select_range(state, ui, row, col, row, col);
}

fn extend_selection(state: &Shared, ui: &Ui, row: i32, col: i32) {
    let (row, col) = {
        let s = state.borrow();
        let rows = s.tabs[s.active].sheet.rows();
        let cols = s.tabs[s.active].sheet.cols();
        (row.clamp(0, rows - 1), col.clamp(0, cols - 1))
    };
    {
        let mut s = state.borrow_mut();
        s.sel = (s.sel.0, s.sel.1, row, col);
        let idx = s.active;
        s.tabs[idx].sel = s.sel;
    }
    refresh_all(state, ui);
}

fn sync_selection_from_table(state: &Shared, ui: &Ui) -> bool {
    let selection = ui.table.clone().try_get_selection();
    if let Some((r1, c1, r2, c2)) = selection {
        if r1 < 0 || c1 < 0 {
            return false;
        }
        {
            let mut s = state.borrow_mut();
            s.sel = (r1, c1, r2, c2);
            let idx = s.active;
            s.tabs[idx].sel = s.sel;
        }
        refresh_all(state, ui);
        return true;
    }
    false
}

fn cell_rect(ui: &Ui, row: i32, col: i32) -> Option<(i32, i32, i32, i32)> {
    let mut table = ui.table.clone();
    let row_header = table.row_header_width();
    let col_header = table.col_header_height();
    let scrollbar = table.scrollbar_size();
    let view_w = (table.w() - row_header - scrollbar).max(1);
    let view_h = (table.h() - col_header - scrollbar).max(1);

    let col_start: i32 = (0..col.max(0)).map(|c| table.col_width(c)).sum();
    let col_end = col_start + table.col_width(col.max(0)).max(1);
    let row_start: i32 = (0..row.max(0)).map(|r| table.row_height(r)).sum();
    let row_end = row_start + table.row_height(row.max(0)).max(1);

    let mut hp = table.col_position().max(0);
    if col_start < hp {
        hp = col_start;
    } else if col_end > hp + view_w {
        hp = col_end - view_w;
    }
    hp = hp.max(0);

    let mut vp = table.row_position().max(0);
    if row_start < vp {
        vp = row_start;
    } else if row_end > vp + view_h {
        vp = row_end - view_h;
    }
    vp = vp.max(0);

    table.hscrollbar().set_value(hp as f64);
    table.scrollbar().set_value(vp as f64);
    table.redraw();

    let x = table.x() + row_header + col_start - hp;
    let y = table.y() + col_header + row_start - vp;
    let w = table.col_width(col.max(0));
    let h = table.row_height(row.max(0));
    if w <= 2 || h <= 2 {
        return None;
    }
    Some((x, y, w, h))
}

fn begin_edit(state: &Shared, ui: &Ui, initial: char) {
    let (r, c) = state.borrow().cell();
    let text = if initial == '\0' {
        let s = state.borrow();
        s.tabs[s.active].sheet.raw(r, c)
    } else {
        initial.to_string()
    };
    if let Some((x, y, w, h)) = cell_rect(ui, r, c) {
        let mut editor = ui.editor.clone();
        editor.set_pos(x + 1, y + 1);
        editor.set_size(w - 2, h - 2);
        editor.set_value(&text);
        editor.show();
        let _ = editor.take_focus();
        {
            let mut s = state.borrow_mut();
            s.editing = true;
            s.edit_target = Some((r, c));
        }
    }
}

fn cancel_edit(state: &Shared, ui: &Ui) {
    {
        let mut s = state.borrow_mut();
        s.editing = false;
        s.edit_target = None;
    }
    ui.editor.clone().hide();
    let _ = ui.table.clone().take_focus();
}

fn commit_edit(state: &Shared, ui: &Ui) {
    let target = state.borrow_mut().edit_target.take();
    let (r, c) = target.unwrap_or_else(|| state.borrow().cell());
    state.borrow_mut().editing = false;
    let text = ui.editor.clone().value();
    let changed = {
        let s = state.borrow();
        s.tabs[s.active].sheet.raw(r, c) != text
    };
    if changed {
        state.borrow_mut().push_undo();
        let mut s = state.borrow_mut();
        let idx = s.active;
        s.tabs[idx].sheet.set(r, c, &text);
    }
    ui.editor.clone().hide();
    let _ = ui.table.clone().take_focus();
    refresh_all(state, ui);
}

fn set_cell(state: &Shared, ui: &Ui, r: i32, c: i32, text: &str) {
    let changed = {
        let s = state.borrow();
        s.tabs[s.active].sheet.raw(r, c) != text
    };
    if changed {
        state.borrow_mut().push_undo();
        let mut s = state.borrow_mut();
        let idx = s.active;
        s.tabs[idx].sheet.set(r, c, text);
        refresh_all(state, ui);
    }
}

fn commit_formula(state: &Shared, ui: &Ui, input: &Input) {
    let target = state.borrow_mut().formula_target.take();
    let (r, c) = match target {
        Some(cell) => cell,
        None => return,
    };
    let text = input.value();
    let changed = {
        let s = state.borrow();
        s.tabs[s.active].sheet.raw(r, c) != text
    };
    if changed {
        state.borrow_mut().push_undo();
        let mut s = state.borrow_mut();
        let idx = s.active;
        s.tabs[idx].sheet.set(r, c, &text);
    }
    refresh_all(state, ui);
}

fn clear_selection(state: &Shared, ui: &Ui) {
    let (r1, c1, r2, c2) = state.borrow().bounds();
    let mut changed = false;
    {
        let s = state.borrow();
        let tab = &s.tabs[s.active];
        'outer: for r in r1..=r2 {
            for c in c1..=c2 {
                if !tab.sheet.raw(r, c).is_empty() {
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    if !changed {
        return;
    }
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        for r in r1..=r2 {
            for c in c1..=c2 {
                s.tabs[idx].sheet.set(r, c, "");
            }
        }
    }
    refresh_all(state, ui);
}

fn push_history(history: &mut Vec<Snapshot>, snap: Snapshot) {
    history.push(snap);
    if history.len() > MAX_UNDO {
        history.remove(0);
    }
}

fn undo(state: &Shared, ui: &Ui) {
    let (snap, current) = {
        let mut s = state.borrow_mut();
        match s.undo.pop() {
            Some(snap) => (snap, s.snapshot()),
            None => return,
        }
    };
    {
        let mut s = state.borrow_mut();
        push_history(&mut s.redo, current);
        s.restore(&snap);
    }
    refresh_all(state, ui);
}

fn redo(state: &Shared, ui: &Ui) {
    let (snap, current) = {
        let mut s = state.borrow_mut();
        match s.redo.pop() {
            Some(snap) => (snap, s.snapshot()),
            None => return,
        }
    };
    {
        let mut s = state.borrow_mut();
        push_history(&mut s.undo, current);
        s.restore(&snap);
    }
    refresh_all(state, ui);
}

fn grid_to_text(data: &[Vec<String>]) -> String {
    let mut out = String::new();
    for (i, row) in data.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let fields: Vec<String> = row.iter().map(|field| csv_escape(field)).collect();
        out.push_str(&fields.join(","));
    }
    out
}

fn text_to_grid(text: &str) -> Vec<Vec<String>> {
    let mut data: Vec<Vec<String>> = Vec::new();
    let parsed = parse_csv(text, |row, col, value| {
        let r = row.max(0) as usize;
        let c = col.max(0) as usize;
        while data.len() <= r {
            data.push(Vec::new());
        }
        while data[r].len() <= c {
            data[r].push(String::new());
        }
        data[r][c] = value.to_string();
    })
    .is_ok();
    if parsed && !data.is_empty() {
        data
    } else {
        vec![vec![text.to_string()]]
    }
}

fn copy_selection(state: &Shared, cut: bool) {
    let data = {
        let mut s = state.borrow_mut();
        let (r1, c1, r2, c2) = s.bounds();
        let idx = s.active;
        let mut data = Vec::new();
        for r in r1..=r2 {
            let mut row = Vec::new();
            for c in c1..=c2 {
                row.push(s.tabs[idx].sheet.raw(r, c));
            }
            data.push(row);
        }
        if cut {
            s.push_undo();
            for r in r1..=r2 {
                for c in c1..=c2 {
                    s.tabs[idx].sheet.set(r, c, "");
                }
            }
        }
        s.clipboard = data.clone();
        data
    };
    app::copy(&grid_to_text(&data));
}

fn paste_grid(state: &Shared, ui: &Ui, data: Vec<Vec<String>>) {
    if data.is_empty() {
        return;
    }
    let (r0, c0) = state.borrow().cell();
    let (rows, cols) = {
        let s = state.borrow();
        (s.tabs[s.active].sheet.rows(), s.tabs[s.active].sheet.cols())
    };
    let mut changes = Vec::new();
    let mut overflow = false;
    {
        let s = state.borrow();
        let tab = &s.tabs[s.active];
        for (dr, row) in data.iter().enumerate() {
            for (dc, text) in row.iter().enumerate() {
                let r = r0 + dr as i32;
                let c = c0 + dc as i32;
                if r >= rows || c >= cols {
                    overflow = true;
                    continue;
                }
                if tab.sheet.raw(r, c) != *text {
                    changes.push((r, c, text.clone()));
                }
            }
        }
    }
    if overflow {
        dialog::alert_default("Paste area exceeds the sheet bounds; extra cells were ignored.");
    }
    if changes.is_empty() {
        return;
    }
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        for (r, c, text) in changes {
            s.tabs[idx].sheet.set(r, c, &text);
        }
    }
    refresh_all(state, ui);
}

fn paste_selection(state: &Shared, ui: &Ui) {
    let data = state.borrow().clipboard.clone();
    paste_grid(state, ui, data);
}

fn paste_from_system(state: &Shared, ui: &Ui) {
    if state.borrow().editing || !app::clipboard_contains(app::ClipboardContent::Text) {
        return;
    }
    let mut editor = ui.editor.clone();
    app::paste_text(&editor);
    let text = editor.value();
    editor.hide();
    if text.is_empty() {
        return;
    }
    paste_grid(state, ui, text_to_grid(&text));
}

fn paste(state: &Shared, ui: &Ui) {
    if app::clipboard_contains(app::ClipboardContent::Text) {
        paste_from_system(state, ui);
    } else {
        paste_selection(state, ui);
    }
}

fn insert_sum(state: &Shared, ui: &Ui) {
    let (r, c) = state.borrow().cell();
    let start = {
        let s = state.borrow();
        let tab = &s.tabs[s.active];
        if r == 0 {
            r
        } else {
            let mut i = r - 1;
            let mut found = false;
            loop {
                if tab.sheet.display(i, c).parse::<f64>().is_ok() {
                    found = true;
                    if i == 0 {
                        break i;
                    }
                    i -= 1;
                } else {
                    break if found { i + 1 } else { r };
                }
            }
        }
    };
    if start == r {
        dialog::alert_default("No numeric cells are directly above this cell.");
        return;
    }
    let text = format!("=SUM({}:{})", Sheet::name(start, c), Sheet::name(r - 1, c));
    set_cell(state, ui, r, c, &text);
}

fn save_csv(state: &Shared, ui: &Ui, save_as: bool) -> bool {
    let (output, current_path) = {
        let s = state.borrow();
        let tab = &s.tabs[s.active];
        let rows = tab.sheet.rows();
        let cols = tab.sheet.cols();
        let mut last_row: Option<i32> = None;
        let mut last_col: Option<i32> = None;
        for r in 0..rows {
            for c in 0..cols {
                if !tab.sheet.raw(r, c).is_empty() {
                    last_row = Some(r);
                    last_col = Some(c.max(last_col.unwrap_or(0)));
                }
            }
        }
        let mut out = String::new();
        if let (Some(last_r), Some(last_c)) = (last_row, last_col) {
            for r in 0..=last_r {
                for c in 0..=last_c {
                    if c > 0 {
                        out.push(',');
                    }
                    out.push_str(&csv_escape(&tab.sheet.raw(r, c)));
                }
                out.push('\n');
            }
        }
        (out, s.path.clone())
    };

    let path = if save_as || current_path.is_none() {
        let mut dialog = FileDialog::new(FileDialogType::BrowseSaveFile);
        dialog.set_title("Save sheet as CSV");
        dialog.set_filter("*.csv");
        dialog.set_option(FileDialogOptions::SaveAsConfirm | FileDialogOptions::UseFilterExt);
        match dialog.try_show() {
            Ok(FileDialogAction::Success) => dialog.filename(),
            _ => return false,
        }
    } else {
        current_path.unwrap_or_default()
    };
    if path.as_os_str().is_empty() {
        return false;
    }
    let mut path = path;
    if path.extension().and_then(|e| e.to_str()) != Some("csv") {
        path.set_extension("csv");
    }
    // Write to a sibling temporary file first so a failed write never
    // truncates the user's existing document.
    let temp = path.with_extension("csv.tmp");
    if let Err(e) = std::fs::write(&temp, output) {
        let _ = std::fs::remove_file(&temp);
        dialog::alert_default(&format!("Could not save file:\n{e}"));
        return false;
    }
    if let Err(e) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        dialog::alert_default(&format!("Could not replace file:\n{e}"));
        return false;
    }
    {
        let mut s = state.borrow_mut();
        s.path = Some(path);
        s.dirty = false;
    }
    refresh_all(state, ui);
    true
}

fn load_csv(state: &Shared, ui: &Ui, path: &Path) -> bool {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            dialog::alert_default(&format!("Could not open file:\n{e}"));
            return false;
        }
    };
    let mut cells = Vec::new();
    let mut max_row = 0;
    let mut max_col = 0;
    if parse_csv(&contents, |row, col, text| {
        if !text.is_empty() {
            max_row = max_row.max(row);
            max_col = max_col.max(col);
        }
        cells.push((row, col, text.to_string()));
    })
    .is_err()
    {
        dialog::alert_default("That CSV file is malformed or contains a NUL byte.");
        return false;
    }
    if max_row >= MAX_LOAD_ROWS || max_col >= MAX_LOAD_COLS {
        dialog::alert_default(&format!(
            "That CSV is too large for this lightweight build.\nLimit: {} rows x {} columns.",
            MAX_LOAD_ROWS, MAX_LOAD_COLS
        ));
        return false;
    }

    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        s.sel = (0, 0, 0, 0);
        let tab = &mut s.tabs[idx];
        tab.sel = (0, 0, 0, 0);
        tab.sheet.clear();
        tab.styles.clear();
        if max_row >= tab.sheet.rows() || max_col >= tab.sheet.cols() {
            tab.sheet.resize(max_row + 1, max_col + 1);
        }
        for (r, c, text) in cells {
            tab.sheet.set(r, c, &text);
        }
        s.path = Some(path.to_path_buf());
        s.dirty = false;
    }
    refresh_all(state, ui);
    true
}

fn choose_open_path(state: &Shared) -> Option<PathBuf> {
    let mut dialog = FileDialog::new(FileDialogType::BrowseFile);
    dialog.set_title("Open CSV");
    dialog.set_filter("*.csv");
    if let Some(parent) = state.borrow().path.as_ref().and_then(|p| p.parent()) {
        let _ = dialog.set_directory(&parent);
    }
    match dialog.try_show() {
        Ok(FileDialogAction::Success) => {
            let path = dialog.filename();
            if path.as_os_str().is_empty() {
                None
            } else {
                Some(path)
            }
        }
        _ => None,
    }
}

fn confirm_discard(state: &Shared, ui: &Ui, action: &str) -> bool {
    if !state.borrow().dirty {
        return true;
    }
    let message = format!(
        "This document has unsaved changes.\n\n{} will discard them.",
        action
    );
    match dialog::choice2_default(&message, "&Save", "&Don't Save", "Cancel") {
        Some(0) => save_csv(state, ui, false),
        Some(1) => {
            state.borrow_mut().dirty = false;
            true
        }
        _ => false,
    }
}

fn new_document(state: &Shared, ui: &Ui) {
    if !confirm_discard(state, ui, "Creating a new document") {
        return;
    }
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        s.tabs = vec![Tab::new("Sheet1".to_string())];
        s.active = 0;
        s.sel = (0, 0, 0, 0);
        s.path = None;
        s.dirty = false;
    }
    refresh_all(state, ui);
}

fn open_document(state: &Shared, ui: &Ui) {
    let path = match choose_open_path(state) {
        Some(path) => path,
        None => return,
    };
    if !confirm_discard(state, ui, "Opening another document") {
        return;
    }
    let _ = load_csv(state, ui, &path);
}

fn add_sheet(state: &Shared, ui: &Ui) {
    let name = {
        let s = state.borrow();
        if s.tabs.len() >= MAX_TABS {
            dialog::alert_default(&format!(
                "A document can contain at most {} sheets.",
                MAX_TABS
            ));
            return;
        }
        format!("Sheet{}", s.tabs.len() + 1)
    };
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        s.tabs.push(Tab::new(name));
        s.active = s.tabs.len() - 1;
        s.sel = (0, 0, 0, 0);
    }
    refresh_all(state, ui);
}

fn remove_sheet(state: &Shared, ui: &Ui) {
    if state.borrow().tabs.len() <= 1 {
        dialog::alert_default("A document must contain at least one sheet.");
        return;
    }
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        s.tabs.remove(idx);
        s.active = s.active.min(s.tabs.len() - 1);
        s.sel = (0, 0, 0, 0);
    }
    refresh_all(state, ui);
}

fn rename_sheet(state: &Shared, ui: &Ui) {
    let (idx, old) = {
        let s = state.borrow();
        (s.active, s.tabs[s.active].name.clone())
    };
    let name = match dialog::input_default("Rename sheet:", &old) {
        Some(name) => name.trim().to_string(),
        None => return,
    };
    if name.is_empty() || name == old {
        return;
    }
    state.borrow_mut().push_undo();
    state.borrow_mut().tabs[idx].name = name;
    refresh_all(state, ui);
}

fn switch_sheet(index: usize, state: &Shared, ui: &Ui) {
    {
        let mut s = state.borrow_mut();
        if index >= s.tabs.len() || index == s.active {
            return;
        }
        let current = s.active;
        s.tabs[current].sel = s.sel;
        s.active = index;
        let rows = s.tabs[index].sheet.rows();
        let cols = s.tabs[index].sheet.cols();
        let sel = s.tabs[index].sel;
        s.sel = (
            sel.0.clamp(0, rows - 1),
            sel.1.clamp(0, cols - 1),
            sel.2.clamp(0, rows - 1),
            sel.3.clamp(0, cols - 1),
        );
    }
    refresh_all(state, ui);
}

fn toggle_style(state: &Shared, ui: &Ui, which: u8) {
    let (r, c, r1, c1, r2, c2) = {
        let s = state.borrow();
        let (r1, c1, r2, c2) = s.bounds();
        (s.sel.0, s.sel.1, r1, c1, r2, c2)
    };
    let (reference, updated) = {
        let s = state.borrow();
        let idx = s.active;
        let current = s.tabs[idx].styles.get(&(r, c)).copied().unwrap_or_default();
        let mut next = current;
        match which {
            1 => next.bold = !current.bold,
            2 => next.italic = !current.italic,
            3 => next.align = if current.align == 3 { 0 } else { 3 },
            4 => next.align = if current.align == 2 { 0 } else { 2 },
            5 => next.align = if current.align == 1 { 0 } else { 1 },
            _ => return,
        }
        (current, next)
    };
    if reference == updated {
        return;
    }
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        for r in r1..=r2 {
            for c in c1..=c2 {
                s.tabs[idx].styles.insert((r, c), updated);
            }
        }
    }
    refresh_all(state, ui);
}

fn request_quit(state: &Shared, ui: &Ui) {
    if confirm_discard(state, ui, "Quitting") {
        state.borrow_mut().dirty = false;
        app::quit();
    }
}

fn usage(program: &str) -> String {
    format!(
        "{} {}\n\n\
         Usage:\n  {} [OPTIONS] [FILE.csv]\n\n\
         Options:\n  -h, --help     Show this help and exit\n  -v, --version  Show version and exit\n\n\
         Open a CSV file on startup, or start with an empty document.",
        APP_NAME, VERSION, program
    )
}

fn parse_args() -> Option<PathBuf> {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "rusted-excel".to_string());
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage(&program));
                std::process::exit(0);
            }
            "-v" | "--version" => {
                println!("{} {}", APP_NAME, VERSION);
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option: {}\n\n{}", arg, usage(&program));
                std::process::exit(2);
            }
            _ => {
                if path.is_none() {
                    path = Some(PathBuf::from(arg));
                } else {
                    eprintln!("Only one CSV file can be opened.\n\n{}", usage(&program));
                    std::process::exit(2);
                }
            }
        }
    }
    path
}

fn main() {
    let initial_path = parse_args();

    let app = app::App::default().with_scheme(app::Scheme::Gtk);
    app::set_font(Font::Helvetica);

    let mut win = Window::default()
        .with_size(WIN_W, WIN_H)
        .center_screen()
        .with_label(APP_NAME);
    win.size_range(800, 520, 0, 0);
    win.make_resizable(true);
    if let Ok(icon) = PngImage::from_data(include_bytes!("../assets/logo.png")) {
        win.set_icon(Some(icon));
    }

    let state: Shared = Rc::new(RefCell::new(State::new()));

    let mut menu = MenuBar::new(0, 0, WIN_W, MENU_H, None);

    let mut toolbar = Group::new(0, MENU_H, WIN_W, TOOL_H, None);
    toolbar.set_frame(FrameType::FlatBox);
    toolbar.set_color(Color::from_rgb(240, 242, 237));

    fn tool_button(x: i32, y: i32, w: i32, label: &str, tip: &str) -> Button {
        let mut button = Button::new(x, y, w, 26, label);
        button.set_tooltip(tip);
        button.set_color(Color::from_rgb(250, 250, 248));
        button.set_label_color(Color::from_rgb(40, 55, 42));
        button
    }

    let mut btn_new = tool_button(8, 32, 46, "New", "New document (Ctrl+N)");
    let mut btn_open = tool_button(58, 32, 46, "Open", "Open CSV (Ctrl+O)");
    let mut btn_save = tool_button(108, 32, 46, "Save", "Save CSV (Ctrl+S)");
    let _ = Frame::new(160, 34, 2, 22, "");
    let mut btn_cut = tool_button(168, 32, 40, "Cut", "Cut selection (Ctrl+X)");
    let mut btn_copy = tool_button(212, 32, 44, "Copy", "Copy selection (Ctrl+C)");
    let mut btn_paste = tool_button(260, 32, 50, "Paste", "Paste (Ctrl+V)");
    let _ = Frame::new(316, 34, 2, 22, "");
    let mut btn_undo = tool_button(324, 32, 50, "Undo", "Undo (Ctrl+Z)");
    let mut btn_redo = tool_button(378, 32, 50, "Redo", "Redo (Ctrl+Y)");
    let _ = Frame::new(434, 34, 2, 22, "");
    let mut btn_sum = tool_button(442, 32, 44, "Σ", "Insert SUM of cells above");
    let mut btn_bold = ToggleButton::new(492, 32, 30, 26, "B");
    btn_bold.set_tooltip("Bold");
    btn_bold.set_label_font(Font::HelveticaBold);
    let mut btn_italic = ToggleButton::new(526, 32, 30, 26, "I");
    btn_italic.set_tooltip("Italic");
    btn_italic.set_label_font(Font::HelveticaItalic);
    let _ = Frame::new(562, 34, 2, 22, "");
    let mut btn_left = ToggleButton::new(570, 32, 32, 26, "≡");
    btn_left.set_tooltip("Align left");
    let mut btn_center = ToggleButton::new(606, 32, 32, 26, "◐");
    btn_center.set_tooltip("Align center");
    let mut btn_right = ToggleButton::new(642, 32, 32, 26, "▷");
    btn_right.set_tooltip("Align right");
    let mut sheet_info = Frame::new(700, 34, 500, 22, "  Rusted Excel — lightweight spreadsheet");
    sheet_info.set_label_color(Color::from_rgb(90, 100, 90));
    toolbar.end();

    let mut fbar = Group::new(0, MENU_H + TOOL_H, WIN_W, BAR_H, None);
    fbar.set_frame(FrameType::FlatBox);
    fbar.set_color(Color::from_rgb(248, 249, 246));
    let mut name = Input::new(8, MENU_H + TOOL_H + 4, 96, 22, None);
    name.set_tooltip("Name box — type a cell such as AB12 and press Enter");
    let mut fx = Frame::new(108, MENU_H + TOOL_H + 2, 26, 24, "fx");
    fx.set_label_font(Font::TimesBoldItalic);
    fx.set_label_color(GREEN);
    let mut formula = Input::new(136, MENU_H + TOOL_H + 4, WIN_W - 144, 22, None);
    formula.set_tooltip("Cell value or formula (Enter to apply, Escape to cancel)");
    fbar.end();

    let mut table = Table::new(0, GRID_Y, WIN_W, WIN_H - GRID_Y - TABS_H - STATUS_H, None);
    table.set_rows(ROWS);
    table.set_cols(COLS);
    table.set_row_header(true);
    table.set_col_header(true);
    table.set_row_resize(false);
    table.set_col_resize(false);
    table.set_row_height_all(22);
    table.set_col_width_all(92);
    table.set_scrollbar_size(14);
    table.set_selection_color(GREEN_LIGHT);
    table.set_color(Color::White);
    table.end();

    let mut editor = Input::new(0, 0, 100, 22, None);
    editor.hide();
    editor.set_frame(FrameType::FlatBox);
    editor.set_text_font(Font::Helvetica);

    let mut tabs_group = Group::new(0, WIN_H - TABS_H - STATUS_H, WIN_W, TABS_H, None);
    tabs_group.set_frame(FrameType::FlatBox);
    tabs_group.set_color(Color::from_rgb(232, 234, 228));
    let mut tab_buttons: Vec<Button> = Vec::new();
    for i in 0..MAX_TABS {
        let label = format!("Sheet{}", i + 1);
        let mut button = Button::new(
            4 + i as i32 * 92,
            WIN_H - TABS_H - STATUS_H + 3,
            88,
            22,
            Some(label.as_str()),
        );
        button.set_color(Color::from_rgb(224, 226, 220));
        if i > 0 {
            button.hide();
        }
        tab_buttons.push(button);
    }
    let mut add_tab = Button::new(
        4 + MAX_TABS as i32 * 92,
        WIN_H - TABS_H - STATUS_H + 3,
        30,
        22,
        "+",
    );
    add_tab.set_tooltip("Add sheet");
    tabs_group.end();

    let mut status = Frame::new(0, WIN_H - STATUS_H, WIN_W, STATUS_H, "Ready");
    status.set_frame(FrameType::DownBox);
    status.set_align(Align::Left | Align::Inside);

    win.end();
    win.show();

    let ui = Ui {
        win: win.clone(),
        menu: menu.clone(),
        toolbar: toolbar.clone(),
        fbar: fbar.clone(),
        name: name.clone(),
        formula: formula.clone(),
        table: table.clone(),
        editor: editor.clone(),
        tabs_group: tabs_group.clone(),
        tab_buttons: tab_buttons.clone(),
        add_tab: add_tab.clone(),
        sheet_info: sheet_info.clone(),
        status: status.clone(),
        bold: btn_bold.clone(),
        italic: btn_italic.clone(),
        left: btn_left.clone(),
        center: btn_center.clone(),
        right: btn_right.clone(),
    };

    // Menu ------------------------------------------------------------------
    menu.add("&File", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("File/&New", Shortcut::Ctrl | 'n', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| new_document(&state, &ui)
    });
    menu.add(
        "File/&Open CSV...",
        Shortcut::Ctrl | 'o',
        MenuFlag::Normal,
        {
            let state = state.clone();
            let ui = ui.clone();
            move |_| open_document(&state, &ui)
        },
    );
    menu.add("File/&Save CSV", Shortcut::Ctrl | 's', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| {
            save_csv(&state, &ui, false);
        }
    });
    menu.add(
        "File/Save &As...",
        Shortcut::Ctrl | Shortcut::Shift | 's',
        MenuFlag::Normal,
        {
            let state = state.clone();
            let ui = ui.clone();
            move |_| {
                save_csv(&state, &ui, true);
            }
        },
    );
    menu.add("File/&Quit", Shortcut::Ctrl | 'q', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| request_quit(&state, &ui)
    });

    menu.add("&Edit", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Edit/&Undo", Shortcut::Ctrl | 'z', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| undo(&state, &ui)
    });
    menu.add("Edit/&Redo", Shortcut::Ctrl | 'y', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| redo(&state, &ui)
    });
    menu.add("Edit/C&ut", Shortcut::Ctrl | 'x', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| {
            copy_selection(&state, true);
            refresh_all(&state, &ui);
        }
    });
    menu.add("Edit/&Copy", Shortcut::Ctrl | 'c', MenuFlag::Normal, {
        let state = state.clone();
        move |_| copy_selection(&state, false)
    });
    menu.add("Edit/&Paste", Shortcut::Ctrl | 'v', MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| paste(&state, &ui)
    });
    menu.add("Edit/&Clear", Shortcut::None, MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| clear_selection(&state, &ui)
    });

    menu.add("&Sheet", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Sheet/&Add Sheet", Shortcut::None, MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| add_sheet(&state, &ui)
    });
    menu.add("Sheet/&Rename Sheet", Shortcut::None, MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| rename_sheet(&state, &ui)
    });
    menu.add("Sheet/&Delete Sheet", Shortcut::None, MenuFlag::Normal, {
        let state = state.clone();
        let ui = ui.clone();
        move |_| remove_sheet(&state, &ui)
    });

    menu.add("&Help", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Help/&About", Shortcut::None, MenuFlag::Normal, |_| {
        dialog::message_default(
            "Rusted Excel\n\
             A lightweight spreadsheet built with Rust, a C++ engine and a C CSV helper.\n\n\
             Formulas: =SUM(A1:A10), =AVG(B1:B5), =IF(A1>0,1,0)\n\
             Type in a cell or use the formula bar. F2/Enter edits, Delete clears.",
        );
    });

    // Toolbar ---------------------------------------------------------------
    btn_new.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| new_document(&state, &ui)
    });
    btn_open.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| open_document(&state, &ui)
    });
    btn_save.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| {
            save_csv(&state, &ui, false);
        }
    });
    btn_cut.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| {
            copy_selection(&state, true);
            refresh_all(&state, &ui);
        }
    });
    btn_copy.set_callback({
        let state = state.clone();
        move |_| copy_selection(&state, false)
    });
    btn_paste.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| paste(&state, &ui)
    });
    btn_undo.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| undo(&state, &ui)
    });
    btn_redo.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| redo(&state, &ui)
    });
    btn_sum.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| insert_sum(&state, &ui)
    });
    btn_bold.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| toggle_style(&state, &ui, 1)
    });
    btn_italic.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| toggle_style(&state, &ui, 2)
    });
    btn_left.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| toggle_style(&state, &ui, 5)
    });
    btn_center.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| toggle_style(&state, &ui, 4)
    });
    btn_right.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| toggle_style(&state, &ui, 3)
    });
    add_tab.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| add_sheet(&state, &ui)
    });
    for (i, button) in tab_buttons.iter_mut().enumerate() {
        let state = state.clone();
        let ui = ui.clone();
        button.set_callback(move |_| switch_sheet(i, &state, &ui));
    }

    // Grid rendering --------------------------------------------------------
    table.draw_cell({
        let state = state.clone();
        move |_t, ctx, row, col, x, y, w, h| match ctx {
            TableContext::RowHeader => {
                draw::set_draw_color(HEADER_BG);
                draw::draw_rectf(x, y, w, h);
                if row == state.borrow().cell().0 {
                    draw::set_draw_color(GREEN);
                    draw::draw_rectf(x, y, w, 2);
                }
                draw::set_draw_color(Color::from_rgb(80, 80, 80));
                draw::draw_text2(&(row + 1).to_string(), x, y, w, h, Align::Center);
            }
            TableContext::ColHeader => {
                draw::set_draw_color(HEADER_BG);
                draw::draw_rectf(x, y, w, h);
                if col == state.borrow().cell().1 {
                    draw::set_draw_color(GREEN);
                    draw::draw_rectf(x, y, w, 2);
                }
                draw::set_draw_color(Color::from_rgb(80, 80, 80));
                let label = Sheet::name(row, col)
                    .trim_end_matches(|c: char| c.is_ascii_digit())
                    .to_string();
                draw::draw_text2(&label, x, y, w, h, Align::Center);
            }
            TableContext::Cell => {
                draw::set_draw_color(Color::White);
                draw::draw_rectf(x, y, w, h);
                let s = state.borrow();
                let tab = &s.tabs[s.active];
                let text = tab.sheet.display(row, col);
                let raw = tab.sheet.raw(row, col);
                let style = tab.styles.get(&(row, col)).copied().unwrap_or_default();
                let (r1, c1, r2, c2) = s.bounds();
                let in_range = row >= r1 && row <= r2 && col >= c1 && col <= c2;
                let is_active = row == s.sel.0 && col == s.sel.1;
                let numeric = !raw.is_empty() && text.parse::<f64>().is_ok();
                drop(s);

                if in_range {
                    draw::set_draw_color(GREEN_LIGHT);
                    draw::draw_rectf(x, y, w, h);
                }
                draw::set_draw_color(GRID_LINE);
                draw::draw_rect(x, y, w, h);
                if is_active {
                    draw::set_draw_color(Color::White);
                    draw::draw_rectf(x + 1, y + 1, w - 2, h - 2);
                    draw::set_draw_color(GREEN);
                    draw::draw_rect(x, y, w, h);
                    draw::draw_rect(x + 1, y + 1, w - 2, h - 2);
                    draw::set_draw_color(Color::White);
                    draw::draw_rectf(x + w - 7, y + h - 7, 6, 6);
                    draw::set_draw_color(GREEN);
                    draw::draw_rect(x + w - 7, y + h - 7, 6, 6);
                }
                if !text.is_empty() {
                    let font = match (style.bold, style.italic) {
                        (true, true) => Font::HelveticaBoldItalic,
                        (true, false) => Font::HelveticaBold,
                        (false, true) => Font::HelveticaItalic,
                        (false, false) => Font::Helvetica,
                    };
                    draw::set_font(font, 13);
                    draw::set_draw_color(if text.starts_with('#') {
                        Color::Red
                    } else {
                        TEXT
                    });
                    let align = match style.align {
                        1 => Align::Left,
                        2 => Align::Center,
                        3 => Align::Right,
                        _ if numeric => Align::Right,
                        _ => Align::Left,
                    };
                    draw::draw_text2(&text, x + 4, y, w - 8, h, align);
                    draw::set_font(Font::Helvetica, 13);
                }
            }
            _ => {}
        }
    });

    table.set_callback({
        let state = state.clone();
        let ui = ui.clone();
        move |_| {
            sync_selection_from_table(&state, &ui);
        }
    });

    table.handle({
        let state = state.clone();
        let ui = ui.clone();
        move |_t, ev| {
            if ev == Event::Push && app::event_clicks() {
                sync_selection_from_table(&state, &ui);
                begin_edit(&state, &ui, '\0');
                return true;
            }
            if ev != Event::KeyDown || state.borrow().editing {
                return false;
            }

            let key = app::event_key();
            let mods = app::event_state();
            let ctrl = mods.contains(Shortcut::Ctrl) || mods.contains(Shortcut::Meta);
            let shift = mods.contains(Shortcut::Shift);
            let (r, c) = state.borrow().cell();
            let (rows, cols) = {
                let s = state.borrow();
                (s.tabs[s.active].sheet.rows(), s.tabs[s.active].sheet.cols())
            };
            let mut handled = true;

            if ctrl {
                if key == Key::Home {
                    select_cell(&state, &ui, 0, 0);
                } else if key == Key::End {
                    select_cell(&state, &ui, rows - 1, cols - 1);
                } else if key == Key::Left {
                    select_cell(&state, &ui, r, 0);
                } else if key == Key::Right {
                    select_cell(&state, &ui, r, cols - 1);
                } else if key == Key::Up {
                    select_cell(&state, &ui, 0, c);
                } else if key == Key::Down {
                    select_cell(&state, &ui, rows - 1, c);
                } else if key == Key::from_char('a') {
                    select_range(&state, &ui, 0, 0, rows - 1, cols - 1);
                } else if key == Key::from_char('z') {
                    undo(&state, &ui);
                } else if key == Key::from_char('y') {
                    redo(&state, &ui);
                } else {
                    handled = false;
                }
            } else {
                let page = 20;
                match key {
                    Key::Left => {
                        if shift {
                            extend_selection(&state, &ui, r, c - 1)
                        } else {
                            select_cell(&state, &ui, r, c - 1)
                        }
                    }
                    Key::Right => {
                        if shift {
                            extend_selection(&state, &ui, r, c + 1)
                        } else {
                            select_cell(&state, &ui, r, c + 1)
                        }
                    }
                    Key::Up => {
                        if shift {
                            extend_selection(&state, &ui, r - 1, c)
                        } else {
                            select_cell(&state, &ui, r - 1, c)
                        }
                    }
                    Key::Down => {
                        if shift {
                            extend_selection(&state, &ui, r + 1, c)
                        } else {
                            select_cell(&state, &ui, r + 1, c)
                        }
                    }
                    Key::Home => {
                        if shift {
                            extend_selection(&state, &ui, r, 0)
                        } else {
                            select_cell(&state, &ui, r, 0)
                        }
                    }
                    Key::End => {
                        if shift {
                            extend_selection(&state, &ui, r, cols - 1)
                        } else {
                            select_cell(&state, &ui, r, cols - 1)
                        }
                    }
                    Key::PageUp => {
                        if shift {
                            extend_selection(&state, &ui, (r - page).max(0), c)
                        } else {
                            select_cell(&state, &ui, (r - page).max(0), c)
                        }
                    }
                    Key::PageDown => {
                        if shift {
                            extend_selection(&state, &ui, (r + page).min(rows - 1), c)
                        } else {
                            select_cell(&state, &ui, (r + page).min(rows - 1), c)
                        }
                    }
                    Key::Tab => select_cell(&state, &ui, r, if shift { c - 1 } else { c + 1 }),
                    Key::Escape => select_cell(&state, &ui, r, c),
                    Key::Enter | Key::F2 => {
                        begin_edit(&state, &ui, '\0');
                        return true;
                    }
                    Key::Delete | Key::BackSpace => clear_selection(&state, &ui),
                    _ => {
                        let text = app::event_text();
                        match text.chars().next() {
                            Some(ch) if !ch.is_control() => begin_edit(&state, &ui, ch),
                            _ => handled = false,
                        }
                    }
                }
            }
            handled
        }
    });

    editor.handle({
        let state = state.clone();
        let ui = ui.clone();
        move |_e, ev| {
            if ev == Event::KeyDown {
                match app::event_key() {
                    Key::Escape => {
                        cancel_edit(&state, &ui);
                        return true;
                    }
                    Key::Enter | Key::Tab => {
                        let (r, c) = state.borrow().cell();
                        let shift = app::event_state().contains(Shortcut::Shift);
                        commit_edit(&state, &ui);
                        if app::event_key() == Key::Tab {
                            select_cell(&state, &ui, r, if shift { c - 1 } else { c + 1 });
                        } else {
                            select_cell(&state, &ui, r + 1, c);
                        }
                        return true;
                    }
                    _ => return false,
                }
            }
            if ev == Event::Unfocus && state.borrow().editing {
                commit_edit(&state, &ui);
            }
            false
        }
    });

    name.handle({
        let state = state.clone();
        let ui = ui.clone();
        move |n, ev| {
            if ev != Event::KeyDown || app::event_key() != Key::Enter {
                return false;
            }
            let parsed = Sheet::parse(&n.value());
            let target = parsed.and_then(|(r, c)| {
                let s = state.borrow();
                let tab = &s.tabs[s.active];
                if r < tab.sheet.rows() && c < tab.sheet.cols() {
                    Some((r, c))
                } else {
                    None
                }
            });
            match target {
                Some((r, c)) => select_cell(&state, &ui, r, c),
                None => {
                    dialog::alert_default("Unknown cell. Try a name such as A1 or AB12.");
                    refresh_all(&state, &ui);
                }
            }
            let _ = ui.table.clone().take_focus();
            true
        }
    });

    formula.handle({
        let state = state.clone();
        let ui = ui.clone();
        move |f, ev| match ev {
            Event::Focus => {
                let target = state.borrow().cell();
                state.borrow_mut().formula_target = Some(target);
                false
            }
            Event::KeyDown => match app::event_key() {
                Key::Enter => {
                    commit_formula(&state, &ui, f);
                    let _ = ui.table.clone().take_focus();
                    true
                }
                Key::Escape => {
                    state.borrow_mut().formula_target = None;
                    refresh_all(&state, &ui);
                    let _ = ui.table.clone().take_focus();
                    true
                }
                _ => false,
            },
            Event::Unfocus => {
                commit_formula(&state, &ui, f);
                false
            }
            _ => false,
        }
    });

    win.handle({
        let state = state.clone();
        let ui = ui.clone();
        move |w, ev| match ev {
            Event::Close => {
                request_quit(&state, &ui);
                true
            }
            Event::Resize => {
                let width = w.width().max(800);
                let height = w.height().max(520);
                let mut menu = ui.menu.clone();
                let mut toolbar = ui.toolbar.clone();
                let mut fbar = ui.fbar.clone();
                let mut table = ui.table.clone();
                let mut tabs_group = ui.tabs_group.clone();
                let mut status = ui.status.clone();
                let mut formula = ui.formula.clone();
                let mut sheet_info = ui.sheet_info.clone();
                menu.set_size(width, MENU_H);
                toolbar.set_size(width, TOOL_H);
                fbar.set_size(width, BAR_H);
                table.set_size(width, height - GRID_Y - TABS_H - STATUS_H);
                tabs_group.resize(0, height - TABS_H - STATUS_H, width, TABS_H);
                status.resize(0, height - STATUS_H, width, STATUS_H);
                formula.set_size((width - 144).max(80), 22);
                sheet_info.set_size((width - 708).max(1), 22);
                table.redraw();
                true
            }
            _ => false,
        }
    });

    refresh_all(&state, &ui);
    if let Some(path) = initial_path {
        let _ = load_csv(&state, &ui, &path);
    }
    if let Err(e) = app.run() {
        eprintln!("{APP_NAME}: GUI error: {e}");
        std::process::exit(1);
    }
}
