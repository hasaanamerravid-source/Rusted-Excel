mod core;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use fltk::{
    app,
    button::{Button, ToggleButton},
    dialog::{self, FileDialog, FileDialogAction, FileDialogType},
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

const ROWS: i32 = 100;
const COLS: i32 = 26;
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

#[derive(Clone, Copy, Default)]
struct CellStyle {
    bold: bool,
    italic: bool,
    align: u8, // 0 = automatic, 1 = left, 2 = center, 3 = right
}

struct Tab {
    sheet: Sheet,
    styles: HashMap<(i32, i32), CellStyle>,
}

type Snapshot = Vec<Vec<Vec<String>>>;

struct State {
    tabs: Vec<Tab>,
    active: usize,
    sel: (i32, i32, i32, i32), // top, left, bottom, right
    rect: (i32, i32, i32, i32), // selected cell rect, relative to the table
    editing: bool,
    path: Option<PathBuf>,
    dirty: bool,
    clipboard: Vec<Vec<String>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

type Shared = Rc<RefCell<State>>;

impl State {
    fn new() -> Self {
        Self {
            tabs: vec![Tab {
                sheet: Sheet::new(ROWS, COLS),
                styles: HashMap::new(),
            }],
            active: 0,
            sel: (0, 0, 0, 0),
            rect: (0, 0, 0, 0),
            editing: false,
            path: None,
            dirty: false,
            clipboard: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn cell(&self) -> (i32, i32) {
        (self.sel.0, self.sel.1)
    }

    fn snapshot(&self) -> Snapshot {
        self.tabs
            .iter()
            .map(|tab| {
                (0..tab.sheet.rows())
                    .map(|r| {
                        (0..tab.sheet.cols())
                            .map(|c| tab.sheet.raw(r, c))
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }

    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 40 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn restore(&mut self, snap: &Snapshot) {
        for (ti, tab) in self.tabs.iter_mut().enumerate() {
            if let Some(rows) = snap.get(ti) {
                for (r, row) in rows.iter().enumerate() {
                    for (c, text) in row.iter().enumerate() {
                        if r < tab.sheet.rows() as usize && c < tab.sheet.cols() as usize {
                            tab.sheet.set(r as i32, c as i32, text);
                        }
                    }
                }
            }
        }
        self.dirty = true;
    }
}

fn active_cell(state: &Shared) -> (i32, i32) {
    state.borrow().cell()
}

fn column_name(col: i32) -> String {
    Sheet::name(0, col)
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

fn refresh_header(state: &Shared, name: &mut Input, formula: &mut Input) {
    let s = state.borrow();
    let (r, c) = s.cell();
    let tab = &s.tabs[s.active];
    name.set_value(&Sheet::name(r, c));
    formula.set_value(&tab.sheet.raw(r, c));
}

fn refresh_status(state: &Shared, status: &mut Frame) {
    let s = state.borrow();
    let (r1, c1, r2, c2) = s.sel;
    let tab = &s.tabs[s.active];
    let mut sum = 0.0;
    let mut count = 0;
    for r in r1..=r2 {
        for c in c1..=c2 {
            let text = tab.sheet.display(r, c);
            if let Ok(n) = text.parse::<f64>() {
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
    status.set_label(&format!(
        "Sheet {} of {}    |    {}",
        s.active + 1,
        s.tabs.len(),
        summary
    ));
}

fn refresh_tabs(state: &Shared, buttons: &mut [Button]) {
    let s = state.borrow();
    for (i, button) in buttons.iter_mut().enumerate() {
        if i < s.tabs.len() {
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
}

fn select_range(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    tabs: &mut [Button],
    r1: i32,
    c1: i32,
    r2: i32,
    c2: i32,
) {
    let s = state.borrow();
    let rows = s.tabs[s.active].sheet.rows();
    let cols = s.tabs[s.active].sheet.cols();
    let (r1, c1) = (r1.clamp(0, rows - 1), c1.clamp(0, cols - 1));
    let (r2, c2) = (r2.clamp(0, rows - 1), c2.clamp(0, cols - 1));
    drop(s);
    state.borrow_mut().sel = (r1, c1, r2, c2);
    let (lo_r, hi_r) = (r1.min(r2), r1.max(r2));
    let (lo_c, hi_c) = (c1.min(c2), c1.max(c2));
    table.set_selection(lo_r, lo_c, hi_r, hi_c);
    refresh_header(state, name, formula);
    refresh_status(state, status);
    refresh_tabs(state, tabs);
    table.redraw();
}

fn select_cell(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    tabs: &mut [Button],
    row: i32,
    col: i32,
) {
    select_range(state, table, name, formula, status, tabs, row, col, row, col);
}

fn begin_edit(state: &Shared, editor: &mut Input, table: &mut Table, initial: char) {
    let (x, y, w, h) = state.borrow().rect;
    if w <= 0 || h <= 0 {
        return;
    }
    let text = if initial == '\0' {
        let s = state.borrow();
        let (r, c) = s.cell();
        s.tabs[s.active].sheet.raw(r, c)
    } else {
        initial.to_string()
    };
    editor.set_pos(table.x() + x + 1, table.y() + y + 1);
    editor.set_size(w - 2, h - 2);
    editor.set_value(&text);
    editor.show();
    let _ = editor.take_focus();
    state.borrow_mut().editing = true;
}

fn commit_edit(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    editor: &mut Input,
) {
    let (r, c) = active_cell(state);
    let text = editor.value();
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
    state.borrow_mut().editing = false;
    editor.hide();
    refresh_header(state, name, formula);
    refresh_status(state, status);
    table.redraw();
}

fn set_cell(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    r: i32,
    c: i32,
    text: &str,
) {
    let changed = {
        let s = state.borrow();
        s.tabs[s.active].sheet.raw(r, c) != text
    };
    if changed {
        state.borrow_mut().push_undo();
        let mut s = state.borrow_mut();
        let idx = s.active;
        s.tabs[idx].sheet.set(r, c, text);
    }
    refresh_header(state, name, formula);
    refresh_status(state, status);
    table.redraw();
}

fn undo(state: &Shared, table: &mut Table, name: &mut Input, formula: &mut Input, status: &mut Frame) {
    let current = state.borrow().snapshot();
    let snap = {
        let mut s = state.borrow_mut();
        match s.undo.pop() {
            Some(snap) => {
                s.redo.push(current);
                snap
            }
            None => return,
        }
    };
    state.borrow_mut().restore(&snap);
    refresh_header(state, name, formula);
    refresh_status(state, status);
    table.redraw();
}

fn redo(state: &Shared, table: &mut Table, name: &mut Input, formula: &mut Input, status: &mut Frame) {
    let current = state.borrow().snapshot();
    let snap = {
        let mut s = state.borrow_mut();
        match s.redo.pop() {
            Some(snap) => {
                s.undo.push(current);
                snap
            }
            None => return,
        }
    };
    state.borrow_mut().restore(&snap);
    refresh_header(state, name, formula);
    refresh_status(state, status);
    table.redraw();
}

fn copy_selection(state: &Shared, cut: bool) {
    let mut s = state.borrow_mut();
    let (r1, c1, r2, c2) = s.sel;
    let idx = s.active;
    let tab = &mut s.tabs[idx];
    let (lo_r, hi_r) = (r1.min(r2), r1.max(r2));
    let (lo_c, hi_c) = (c1.min(c2), c1.max(c2));
    let mut data = Vec::new();
    for r in lo_r..=hi_r {
        let mut row = Vec::new();
        for c in lo_c..=hi_c {
            row.push(tab.sheet.raw(r, c));
            if cut {
                tab.sheet.set(r, c, "");
            }
        }
        data.push(row);
    }
    s.clipboard = data;
    if cut {
        s.dirty = true;
    }
}

fn paste_selection(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
) {
    let data = state.borrow().clipboard.clone();
    if data.is_empty() {
        return;
    }
    state.borrow_mut().push_undo();
    let (r0, c0) = state.borrow().cell();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        for (dr, row) in data.iter().enumerate() {
            for (dc, text) in row.iter().enumerate() {
                s.tabs[idx].sheet.set(r0 + dr as i32, c0 + dc as i32, text);
            }
        }
    }
    refresh_header(state, name, formula);
    refresh_status(state, status);
    table.redraw();
}

fn insert_sum(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
) {
    let (r, c) = active_cell(state);
    let mut start = r;
    if r > 0 {
        let s = state.borrow();
        let tab = &s.tabs[s.active];
        let mut i = r - 1;
        loop {
            let text = tab.sheet.display(i, c);
            if text.is_empty() || text.parse::<f64>().is_ok() {
                start = i;
                if i == 0 {
                    break;
                }
                i -= 1;
            } else {
                break;
            }
        }
    }
    let formula_text = format!("=SUM({}:{})", Sheet::name(start, c), Sheet::name(r, c));
    formula.set_value(&formula_text);
    set_cell(state, table, name, formula, status, r, c, &formula_text);
}

fn save_csv(state: &Shared) {
    let s = state.borrow();
    let mut out = String::new();
    let tab = &s.tabs[s.active];
    for r in 0..tab.sheet.rows() {
        for c in 0..tab.sheet.cols() {
            if c > 0 {
                out.push(',');
            }
            out.push_str(&csv_escape(&tab.sheet.raw(r, c)));
        }
        out.push('\n');
    }
    let path = match &s.path {
        Some(p) => p.clone(),
        None => {
            let mut dialog = FileDialog::new(FileDialogType::BrowseSaveFile);
            dialog.set_title("Save sheet as CSV");
            dialog.set_filter("*.csv");
            match dialog.try_show() {
                Ok(FileDialogAction::Success) => dialog.filename(),
                _ => return,
            }
        }
    };
    if let Err(e) = std::fs::write(&path, out) {
        dialog::alert(400, 200, &format!("Could not save file:\n{e}"));
        return;
    }
    drop(s);
    state.borrow_mut().path = Some(path);
    state.borrow_mut().dirty = false;
}

fn open_csv(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
) {
    let mut dialog = FileDialog::new(FileDialogType::BrowseFile);
    dialog.set_title("Open CSV");
    dialog.set_filter("*.csv");
    let path = match dialog.try_show() {
        Ok(FileDialogAction::Success) => dialog.filename(),
        _ => return,
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            dialog::alert(400, 200, &format!("Could not open file:\n{e}"));
            return;
        }
    };
    let mut cells = Vec::new();
    let mut max_row = 0;
    let mut max_col = 0;
    if parse_csv(&contents, |row, col, text| {
        max_row = max_row.max(row);
        max_col = max_col.max(col);
        cells.push((row, col, text.to_string()));
    })
    .is_err()
    {
        dialog::alert(400, 200, "That CSV file looks malformed (unterminated quote).");
        return;
    }

    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        let idx = s.active;
        let tab = &mut s.tabs[idx];
        tab.sheet.clear();
        if max_row >= tab.sheet.rows() || max_col >= tab.sheet.cols() {
            tab.sheet.resize(max_row + 1, max_col + 1);
        }
        for (r, c, text) in cells {
            tab.sheet.set(r, c, &text);
        }
        s.path = Some(path);
        s.dirty = false;
    }
    table.set_rows(state.borrow().tabs[state.borrow().active].sheet.rows());
    table.set_cols(state.borrow().tabs[state.borrow().active].sheet.cols());
    select_cell(state, table, name, formula, status, &mut [], 0, 0);
}

fn new_document(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    tabs: &mut [Button],
) {
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        s.tabs = vec![Tab {
            sheet: Sheet::new(ROWS, COLS),
            styles: HashMap::new(),
        }];
        s.active = 0;
        s.path = None;
    }
    table.set_rows(ROWS);
    table.set_cols(COLS);
    select_cell(state, table, name, formula, status, tabs, 0, 0);
    refresh_tabs(state, tabs);
}

fn add_sheet(
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    tabs: &mut [Button],
) {
    state.borrow_mut().push_undo();
    {
        let mut s = state.borrow_mut();
        s.tabs.push(Tab {
            sheet: Sheet::new(ROWS, COLS),
            styles: HashMap::new(),
        });
        s.active = s.tabs.len() - 1;
    }
    table.set_rows(ROWS);
    table.set_cols(COLS);
    select_cell(state, table, name, formula, status, tabs, 0, 0);
    refresh_tabs(state, tabs);
}

fn switch_sheet(
    index: usize,
    state: &Shared,
    table: &mut Table,
    name: &mut Input,
    formula: &mut Input,
    status: &mut Frame,
    tabs: &mut [Button],
) {
    {
        let mut s = state.borrow_mut();
        if index >= s.tabs.len() {
            return;
        }
        s.active = index;
        s.sel = (0, 0, 0, 0);
        table.set_rows(s.tabs[index].sheet.rows());
        table.set_cols(s.tabs[index].sheet.cols());
    }
    select_cell(state, table, name, formula, status, tabs, 0, 0);
}

fn toggle_style(state: &Shared, table: &mut Table, which: u8) {
    let (r, c) = active_cell(state);
    let mut s = state.borrow_mut();
    let idx = s.active;
    let styles = &mut s.tabs[idx].styles;
    let entry = styles.entry((r, c)).or_default();
    match which {
        1 => entry.bold = !entry.bold,
        2 => entry.italic = !entry.italic,
        3 => entry.align = if entry.align == 3 { 0 } else { 3 },
        4 => entry.align = if entry.align == 2 { 0 } else { 2 },
        5 => entry.align = if entry.align == 1 { 0 } else { 1 },
        _ => {}
    }
    drop(s);
    table.redraw();
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Gtk);
    app::set_font(Font::Helvetica);

    let mut win = Window::default()
        .with_size(WIN_W, WIN_H)
        .center_screen()
        .with_label("Rusted Excel");
    win.size_range(800, 520, 0, 0);
    win.make_resizable(true);
    if let Ok(icon) = PngImage::from_data(include_bytes!("../assets/logo.png")) {
        win.set_icon(Some(icon));
    }

    let state: Shared = Rc::new(RefCell::new(State::new()));

    // Menu ------------------------------------------------------------------
    let mut menu = MenuBar::new(0, 0, WIN_W, MENU_H, None);

    // Toolbar ---------------------------------------------------------------
    let mut toolbar = Group::new(0, MENU_H, WIN_W, TOOL_H, None);
    toolbar.set_frame(FrameType::FlatBox);
    toolbar.set_color(Color::from_rgb(240, 242, 237));

    fn tool_button(x: i32, y: i32, w: i32, label: &str, tip: &str) -> Button {
        let mut b = Button::new(x, y, w, 26, label);
        b.set_tooltip(tip);
        b.set_color(Color::from_rgb(250, 250, 248));
        b.set_label_color(Color::from_rgb(40, 55, 42));
        b
    }

    let mut btn_new = tool_button(8, 32, 46, "New", "New sheet (Ctrl+N)");
    let mut btn_open = tool_button(58, 32, 46, "Open", "Open CSV (Ctrl+O)");
    let mut btn_save = tool_button(108, 32, 46, "Save", "Save CSV (Ctrl+S)");
    let _ = Frame::new(160, 34, 2, 22, "");
    let mut btn_cut = tool_button(168, 32, 40, "Cut", "Cut (Ctrl+X)");
    let mut btn_copy = tool_button(212, 32, 44, "Copy", "Copy (Ctrl+C)");
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

    // Formula bar -----------------------------------------------------------
    let mut fbar = Group::new(0, MENU_H + TOOL_H, WIN_W, BAR_H, None);
    fbar.set_frame(FrameType::FlatBox);
    fbar.set_color(Color::from_rgb(248, 249, 246));
    let mut name = Input::new(8, MENU_H + TOOL_H + 4, 96, 22, None);
    name.set_tooltip("Name box — type a cell like AB12 and press Enter");
    let mut fx = Frame::new(108, MENU_H + TOOL_H + 2, 26, 24, "fx");
    fx.set_label_font(Font::TimesBoldItalic);
    fx.set_label_color(GREEN);
    let mut formula = Input::new(136, MENU_H + TOOL_H + 4, WIN_W - 144, 22, None);
    formula.set_tooltip("Cell value or formula");
    fbar.end();

    // Grid ------------------------------------------------------------------
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

    // Cell editor (child of the window, positioned over the active cell).
    let mut editor = Input::new(0, 0, 100, 22, None);
    editor.hide();
    editor.set_frame(FrameType::FlatBox);
    editor.set_text_font(Font::Helvetica);

    // Sheet tabs ------------------------------------------------------------
    let mut tabs_group = Group::new(0, WIN_H - TABS_H - STATUS_H, WIN_W, TABS_H, None);
    tabs_group.set_frame(FrameType::FlatBox);
    tabs_group.set_color(Color::from_rgb(232, 234, 228));
    let mut tab_buttons: Vec<Button> = Vec::new();
    for i in 0..6 {
        let label = if i < 6 && i < 5 {
            match i {
                0 => "Sheet1",
                1 => "Sheet2",
                2 => "Sheet3",
                3 => "Sheet4",
                _ => "Sheet5",
            }
        } else {
            ""
        };
        let mut b = Button::new(4 + i as i32 * 92, WIN_H - TABS_H - STATUS_H + 3, 88, 22, label);
        b.set_color(Color::from_rgb(224, 226, 220));
        if i >= 1 {
            b.hide();
        }
        tab_buttons.push(b);
    }
    let mut btn_add_tab = Button::new(4 + 5 * 92, WIN_H - TABS_H - STATUS_H + 3, 30, 22, "+");
    btn_add_tab.set_tooltip("Add sheet");
    tabs_group.end();

    // Status bar ------------------------------------------------------------
    let mut status = Frame::new(0, WIN_H - STATUS_H, WIN_W, STATUS_H, "Ready");
    status.set_frame(FrameType::DownBox);
    status.set_align(Align::Left | Align::Inside);

    win.end();
    win.show();

    // Menu callbacks (widgets now exist) ------------------------------------
    menu.add("&File", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("File/&New", Shortcut::Ctrl | 'n', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |_| new_document(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone())
    });
    menu.add("File/&Open CSV...", Shortcut::Ctrl | 'o', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| open_csv(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    menu.add("File/&Save CSV", Shortcut::Ctrl | 's', MenuFlag::Normal, {
        let state = state.clone();
        move |_| save_csv(&state)
    });
    menu.add("File/&Quit", Shortcut::None, MenuFlag::Normal, |_| app::quit());

    menu.add("&Edit", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Edit/&Undo", Shortcut::Ctrl | 'z', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| undo(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    menu.add("Edit/&Redo", Shortcut::Ctrl | 'y', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| redo(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    menu.add("Edit/C&ut", Shortcut::Ctrl | 'x', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| {
            copy_selection(&state, true);
            refresh_header(&state, &mut name.clone(), &mut formula.clone());
            refresh_status(&state, &mut status.clone());
            table.redraw();
        }
    });
    menu.add("Edit/&Copy", Shortcut::Ctrl | 'c', MenuFlag::Normal, {
        let state = state.clone();
        move |_| copy_selection(&state, false)
    });
    menu.add("Edit/&Paste", Shortcut::Ctrl | 'v', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| paste_selection(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });

    menu.add("&Sheet", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Sheet/&Add Sheet", Shortcut::None, MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |_| add_sheet(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone())
    });

    menu.add("&Help", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Help/&About", Shortcut::None, MenuFlag::Normal, |_| {
        dialog::message(
            500,
            300,
            "Rusted Excel\n\
             A featherweight spreadsheet built with Rust, a C++ engine and a C CSV helper.\n\n\
             Formulas: =SUM(A1:A10), =AVG(B1:B5), =IF(A1>0,1,0)\n\
             Type in a cell or use the formula bar. F2/Enter edits, Delete clears.",
        );
    });

    // Toolbar callbacks -----------------------------------------------------
    btn_new.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |_| new_document(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone())
    });
    btn_open.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| open_csv(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    btn_save.set_callback({
        let state = state.clone();
        move |_| save_csv(&state)
    });
    btn_cut.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| {
            copy_selection(&state, true);
            refresh_header(&state, &mut name.clone(), &mut formula.clone());
            refresh_status(&state, &mut status.clone());
            table.redraw();
        }
    });
    btn_copy.set_callback({
        let state = state.clone();
        move |_| copy_selection(&state, false)
    });
    btn_paste.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| paste_selection(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    btn_undo.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| undo(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    btn_redo.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| redo(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    btn_sum.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |_| insert_sum(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone())
    });
    btn_bold.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        move |_| toggle_style(&state, &mut table, 1)
    });
    btn_italic.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        move |_| toggle_style(&state, &mut table, 2)
    });
    btn_left.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        move |_| toggle_style(&state, &mut table, 5)
    });
    btn_center.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        move |_| toggle_style(&state, &mut table, 4)
    });
    btn_right.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        move |_| toggle_style(&state, &mut table, 3)
    });
    btn_add_tab.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |_| add_sheet(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone())
    });

    for i in 0..tab_buttons.len().min(5) {
        let buttons_for_cb = tab_buttons.clone();
        tab_buttons[i].set_callback({
            let state = state.clone();
            let mut table = table.clone();
            let name = name.clone();
            let formula = formula.clone();
            let status = status.clone();
            let tab_buttons = buttons_for_cb.clone();
            move |_| switch_sheet(i, &state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone())
        });
    }

    // Grid rendering --------------------------------------------------------
    table.draw_cell({
        let state = state.clone();
        move |_t, ctx, row, col, x, y, w, h| {
            match ctx {
                TableContext::RowHeader => {
                    draw::set_draw_color(HEADER_BG);
                    draw::draw_rectf(x, y, w, h);
                    let active_row = state.borrow().cell().0;
                    if row == active_row {
                        draw::set_draw_color(GREEN);
                        draw::draw_rectf(x, y, w, 2);
                    }
                    draw::set_draw_color(Color::from_rgb(80, 80, 80));
                    draw::draw_text2(&(row + 1).to_string(), x, y, w, h, Align::Center);
                }
                TableContext::ColHeader => {
                    draw::set_draw_color(HEADER_BG);
                    draw::draw_rectf(x, y, w, h);
                    let active_col = state.borrow().cell().1;
                    if col == active_col {
                        draw::set_draw_color(GREEN);
                        draw::draw_rectf(x, y, w, 2);
                    }
                    draw::set_draw_color(Color::from_rgb(80, 80, 80));
                    draw::draw_text2(&column_name(col), x, y, w, h, Align::Center);
                }
                TableContext::Cell => {
                    draw::set_draw_color(Color::White);
                    draw::draw_rectf(x, y, w, h);
                    let s = state.borrow();
                    let tab = &s.tabs[s.active];
                    let text = tab.sheet.display(row, col);
                    let raw = tab.sheet.raw(row, col);
                    let style = tab.styles.get(&(row, col)).copied().unwrap_or_default();
                    let in_range = row >= s.sel.0.min(s.sel.2)
                        && row <= s.sel.0.max(s.sel.2)
                        && col >= s.sel.1.min(s.sel.3)
                        && col <= s.sel.1.max(s.sel.3);
                    let is_active = row == s.sel.0 && col == s.sel.1;
                    let numeric = text.parse::<f64>().is_ok() && !raw.is_empty();
                    drop(s);

                    if in_range {
                        draw::set_draw_color(GREEN_LIGHT);
                        draw::draw_rectf(x, y, w, h);
                    }
                    draw::set_draw_color(GRID_LINE);
                    draw::draw_rect(x, y, w, h);
                    if is_active {
                        state.borrow_mut().rect = (x, y, w, h);
                        draw::set_draw_color(Color::White);
                        draw::draw_rectf(x + 1, y + 1, w - 2, h - 2);
                        draw::set_draw_color(GREEN);
                        draw::draw_rect(x, y, w, h);
                        draw::draw_rect(x + 1, y + 1, w - 2, h - 2);
                        draw::set_draw_color(Color::White);
                        draw::draw_rectf(x + w - 6, y + h - 6, 5, 5);
                        draw::set_draw_color(GREEN);
                        draw::draw_rect(x + w - 6, y + h - 6, 5, 5);
                    }

                    if !text.is_empty() {
                        let mut font = Font::Helvetica;
                        if style.bold && style.italic {
                            font = Font::HelveticaBoldItalic;
                        } else if style.bold {
                            font = Font::HelveticaBold;
                        } else if style.italic {
                            font = Font::HelveticaItalic;
                        }
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
                            _ => if numeric { Align::Right } else { Align::Left },
                        };
                        draw::draw_text2(&text, x + 4, y, w - 8, h, align);
                        draw::set_font(Font::Helvetica, 13);
                    }
                }
                _ => {}
            }
        }
    });

    // Selection -------------------------------------------------------------
    table.set_callback({
        let state = state.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |t| {
            if let Some((r1, c1, r2, c2)) = t.try_get_selection() {
                if r1 < 0 || c1 < 0 {
                    return;
                }
                state.borrow_mut().sel = (r1, c1, r2, c2);
                refresh_header(&state, &mut name.clone(), &mut formula.clone());
                refresh_status(&state, &mut status.clone());
                refresh_tabs(&state, &mut tab_buttons.clone());
                t.redraw();
            }
        }
    });

    // Keyboard and mouse ----------------------------------------------------
    table.handle({
        let state = state.clone();
        let mut editor = editor.clone();
        let mut table_for_edit = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |t, ev| {
            if ev == Event::Push && app::event_clicks() {
                begin_edit(&state, &mut editor, &mut table_for_edit, '\0');
                return true;
            }
            if ev != Event::KeyDown || state.borrow().editing {
                return false;
            }
            let (r, c) = active_cell(&state);
            match app::event_key() {
                Key::Left => select_cell(&state, t, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone(), r, c - 1),
                Key::Right => select_cell(&state, t, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone(), r, c + 1),
                Key::Up => select_cell(&state, t, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone(), r - 1, c),
                Key::Down => select_cell(&state, t, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone(), r + 1, c),
                Key::Home => select_cell(&state, t, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone(), r, 0),
                Key::Enter | Key::F2 => begin_edit(&state, &mut editor, &mut table_for_edit, '\0'),
                Key::Delete | Key::BackSpace => {
                    set_cell(&state, t, &mut name.clone(), &mut formula.clone(), &mut status.clone(), r, c, "");
                }
                _ => {
                    let text = app::event_text();
                    if text.chars().count() == 1 && !text.chars().next().unwrap().is_control() {
                        begin_edit(&state, &mut editor, &mut table_for_edit, text.chars().next().unwrap());
                    }
                }
            };
            true
        }
    });

    editor.handle({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        move |e, ev| {
            if ev == Event::KeyDown {
                match app::event_key() {
                    Key::Escape => {
                        state.borrow_mut().editing = false;
                        e.hide();
                        let _ = table.take_focus();
                        return true;
                    }
                    Key::Enter | Key::Tab => {
                        commit_edit(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), e);
                        let _ = table.take_focus();
                        if app::event_key() == Key::Tab {
                            let (r, c) = active_cell(&state);
                            select_cell(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut [], r, c + 1);
                        }
                        return true;
                    }
                    _ => return false,
                }
            }
            if ev == Event::Unfocus && state.borrow().editing {
                commit_edit(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), e);
            }
            false
        }
    });

    // Name box --------------------------------------------------------------
    name.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        let status = status.clone();
        let tab_buttons = tab_buttons.clone();
        move |n| {
            if app::event_key() != Key::Enter {
                return;
            }
            if let Some((r, c)) = Sheet::parse(&n.value()) {
                select_cell(&state, &mut table, &mut name.clone(), &mut formula.clone(), &mut status.clone(), &mut tab_buttons.clone(), r, c);
            }
            let _ = table.take_focus();
        }
    });

    // Formula bar -----------------------------------------------------------
    formula.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let status = status.clone();
        move |f| {
            if state.borrow().editing {
                return;
            }
            let (r, c) = active_cell(&state);
            let text = f.value();
            set_cell(&state, &mut table, &mut name.clone(), f, &mut status.clone(), r, c, &text);
        }
    });

    // Window resize ---------------------------------------------------------
    win.handle({
        let mut table = table.clone();
        let mut fbar = fbar.clone();
        let mut tabs_group = tabs_group.clone();
        let mut status = status.clone();
        let mut formula = formula.clone();
        let mut sheet_info = sheet_info.clone();
        move |w, ev| {
            if ev == Event::Resize {
                let width = w.width();
                let height = w.height();
                table.set_size(width, height - GRID_Y - TABS_H - STATUS_H);
                fbar.set_size(width, BAR_H);
                tabs_group.resize(0, height - TABS_H - STATUS_H, width, TABS_H);
                status.resize(0, height - STATUS_H, width, STATUS_H);
                formula.set_size(width - 144, 22);
                sheet_info.set_size(width - 700 - 8, 22);
                table.redraw();
            }
            false
        }
    });

    select_cell(&state, &mut table, &mut name, &mut formula, &mut status, &mut tab_buttons, 0, 0);
    refresh_tabs(&state, &mut tab_buttons);
    app.run().unwrap();
}
