mod core;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use fltk::{
    app,
    dialog::{self, FileDialog, FileDialogAction, FileDialogType},
    draw,
    enums::{Align, Color, Event, Font, Key, Shortcut},
    image::PngImage,
    input::Input,
    menu::{MenuBar, MenuFlag},
    prelude::*,
    table::{Table, TableContext},
    window::Window,
};

use core::{csv_escape, parse_csv, Sheet};

const ROWS: i32 = 80;
const COLS: i32 = 20;

struct State {
    sheet: Sheet,
    row: i32,
    col: i32,
    rect: (i32, i32, i32, i32), // screen coords of the selected cell
    editing: bool,
    dirty: bool,
    path: Option<PathBuf>,
}

type Shared = Rc<RefCell<State>>;

fn refresh_header(state: &Shared, name: &mut Input, formula: &mut Input) {
    let s = state.borrow();
    name.set_value(&Sheet::name(s.row, s.col));
    formula.set_value(&s.sheet.raw(s.row, s.col));
}

fn select_cell(state: &Shared, table: &mut Table, name: &mut Input, formula: &mut Input,
               row: i32, col: i32) {
    let (row, col, rows, cols) = {
        let s = state.borrow();
        (row, col, s.sheet.rows(), s.sheet.cols())
    };
    let (row, col) = (row.clamp(0, rows - 1), col.clamp(0, cols - 1));
    {
        let mut s = state.borrow_mut();
        s.row = row;
        s.col = col;
    }
    table.set_selection(row, col, row, col);
    refresh_header(state, name, formula);
    table.redraw();
}

fn begin_edit(state: &Shared, editor: &mut Input, initial: char) {
    let (x, y, w, h) = state.borrow().rect;
    if w <= 0 || h <= 0 {
        return;
    }
    let text = if initial == '\0' {
        state.borrow().sheet.raw(state.borrow().row, state.borrow().col)
    } else {
        initial.to_string()
    };
    editor.set_pos(x + 1, y + 1);
    editor.set_size(w - 2, h - 2);
    editor.set_value(&text);
    editor.show();
    let _ = editor.take_focus();
    state.borrow_mut().editing = true;
}

fn commit_edit(state: &Shared, table: &mut Table, name: &mut Input, formula: &mut Input,
               editor: &mut Input) {
    let (row, col) = {
        let s = state.borrow();
        (s.row, s.col)
    };
    let text = editor.value();
    {
        let mut s = state.borrow_mut();
        if s.sheet.raw(row, col) != text {
            s.sheet.set(row, col, &text);
            s.dirty = true;
        }
        s.editing = false;
    }
    editor.hide();
    refresh_header(state, name, formula);
    table.redraw();
}

fn save_csv(state: &Shared) {
    let s = state.borrow();
    let mut out = String::new();
    for r in 0..s.sheet.rows() {
        for c in 0..s.sheet.cols() {
            if c > 0 {
                out.push(',');
            }
            out.push_str(&csv_escape(&s.sheet.raw(r, c)));
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
    state.borrow_mut().path = Some(path.clone());
    state.borrow_mut().dirty = false;
}

fn open_csv(state: &Shared, table: &mut Table, name: &mut Input, formula: &mut Input) {
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

    let mut cells: Vec<(i32, i32, String)> = Vec::new();
    let mut max_row = 0;
    let mut max_col = 0;
    let ok = parse_csv(&contents, |row, col, text| {
        max_row = max_row.max(row);
        max_col = max_col.max(col);
        cells.push((row, col, text.to_string()));
    });
    if ok.is_err() {
        dialog::alert(400, 200, "That CSV file looks malformed (unterminated quote).");
        return;
    }

    {
        let mut s = state.borrow_mut();
        s.sheet.clear();
        if max_row >= s.sheet.rows() || max_col >= s.sheet.cols() {
            s.sheet.resize(max_row + 1, max_col + 1);
        }
        for (r, c, text) in cells {
            s.sheet.set(r, c, &text);
        }
        s.row = 0;
        s.col = 0;
        s.path = Some(path);
        s.dirty = false;
    }
    table.set_rows(state.borrow().sheet.rows());
    table.set_cols(state.borrow().sheet.cols());
    select_cell(state, table, name, formula, 0, 0);
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Gtk);
    app::set_font(Font::Helvetica);

    let mut win = Window::default()
        .with_size(1120, 720)
        .center_screen()
        .with_label("Rusted Excel");

    if let Ok(icon) = PngImage::from_data(include_bytes!("../assets/logo.png")) {
        win.set_icon(Some(icon));
    }

    let state: Shared = Rc::new(RefCell::new(State {
        sheet: Sheet::new(ROWS, COLS),
        row: 0,
        col: 0,
        rect: (0, 0, 0, 0),
        editing: false,
        dirty: false,
        path: None,
    }));

    let mut menu = MenuBar::new(0, 0, 1120, 30, None);

    // Toolbar
    let mut toolbar = fltk::group::Group::new(0, 30, 1120, 38, None);
    toolbar.set_frame(fltk::enums::FrameType::FlatBox);
    let mut name = Input::new(10, 37, 90, 24, None);
    name.set_readonly(true);
    let mut fx_label = fltk::frame::Frame::new(108, 32, 28, 28, "fx");
    fx_label.set_label_font(Font::TimesBoldItalic);
    let mut formula = Input::new(138, 37, 960, 24, None);

    let mut table = Table::new(0, 68, 1120, 648, None);
    table.set_rows(ROWS);
    table.set_cols(COLS);
    table.set_row_header(true);
    table.set_col_header(true);
    table.set_row_resize(false);
    table.set_col_resize(false);
    table.set_selection_color(Color::from_rgb(220, 240, 220));
    table.set_color(Color::White);
    table.end();

    let mut editor = Input::new(0, 0, 100, 24, None);
    editor.hide();

    win.resizable(&table);
    win.end();
    win.show();

    // Menus that need the table/widgets (created above).
    menu.add("&File", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("File/&New", Shortcut::Ctrl | 'n', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let formula = formula.clone();
        let name = name.clone();
        move |_| {
            {
                let mut s = state.borrow_mut();
                s.sheet.resize(ROWS, COLS);
                s.sheet.clear();
                s.row = 0;
                s.col = 0;
                s.path = None;
                s.dirty = false;
            }
            table.set_rows(ROWS);
            table.set_cols(COLS);
            refresh_header(&state, &mut name.clone(), &mut formula.clone());
            table.redraw();
        }
    });
    menu.add("File/&Open CSV...", Shortcut::Ctrl | 'o', MenuFlag::Normal, {
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
        move |_| open_csv(&state, &mut table, &mut name.clone(), &mut formula.clone())
    });
    menu.add("File/&Save CSV", Shortcut::Ctrl | 's', MenuFlag::Normal, {
        let state = state.clone();
        move |_| save_csv(&state)
    });
    menu.add("File/&Quit", Shortcut::None, MenuFlag::Normal, |_| app::quit());
    menu.add("&Help", Shortcut::None, MenuFlag::Normal, |_| {});
    menu.add("Help/&About", Shortcut::None, MenuFlag::Normal, |_| {
        dialog::message(
            500,
            300,
            "Rusted Excel\n\
             A featherweight spreadsheet written in Rust with a C++ engine\n\
             and a C CSV helper.\n\n\
             Formulas: =SUM(A1:A10), =AVG(B1:B5), =POW(2,8), =IF(A1>0,1,0)\n\
             Keys: arrows to move, Enter/F2 to edit, Delete to clear.",
        );
    });

    // Drawing --------------------------------------------------------------
    table.draw_cell({
        let state = state.clone();
        move |_t, ctx, row, col, x, y, w, h| {
            match ctx {
                TableContext::RowHeader => {
                    draw::set_draw_color(Color::from_rgb(245, 245, 245));
                    draw::draw_rectf(x, y, w, h);
                    draw::set_draw_color(Color::from_rgb(80, 80, 80));
                    draw::draw_text2(&(row + 1).to_string(), x, y, w, h, Align::Center);
                }
                TableContext::ColHeader => {
                    draw::set_draw_color(Color::from_rgb(245, 245, 245));
                    draw::draw_rectf(x, y, w, h);
                    draw::set_draw_color(Color::from_rgb(80, 80, 80));
                    draw::draw_text2(&Sheet::name(row, col), x, y, w, h, Align::Center);
                }
                TableContext::Cell => {
                    let (selected, text, numeric) = {
                        let s = state.borrow();
                        let raw = s.sheet.raw(row, col);
                        let is_num = !raw.is_empty()
                            && !s.sheet.is_formula(row, col)
                            && raw.parse::<f64>().is_ok();
                        (row == s.row && col == s.col, s.sheet.display(row, col), is_num)
                    };
                    if selected {
                        state.borrow_mut().rect = (x, y, w, h);
                        draw::set_draw_color(Color::from_rgb(226, 244, 226));
                        draw::draw_rectf(x, y, w, h);
                    }
                    draw::set_draw_color(Color::from_rgb(210, 210, 210));
                    draw::draw_rect(x, y, w, h);
                    draw::set_draw_color(if selected { Color::from_rgb(0, 90, 45) } else { Color::Black });
                    let align = if numeric { Align::Right } else { Align::Left };
                    draw::draw_text2(&text, x + 4, y, w - 8, h, align);
                }
                _ => {}
            }
        }
    });

    // Selection ------------------------------------------------------------
    table.set_callback({
        let state = state.clone();
        let name = name.clone();
        let formula = formula.clone();
        move |t| {
            let (row, col, _, _) = match t.try_get_selection() {
                Some(s) => s,
                None => return,
            };
            {
                let mut s = state.borrow_mut();
                s.row = row;
                s.col = col;
            }
            refresh_header(&state, &mut name.clone(), &mut formula.clone());
        }
    });

    // Keyboard -------------------------------------------------------------
    table.handle({
        let state = state.clone();
        let mut editor = editor.clone();
        let name = name.clone();
        let formula = formula.clone();
        move |t, ev| {
            if ev != Event::KeyDown {
                return false;
            }
            let editing = state.borrow().editing;
            if editing {
                return false;
            }
            let (row, col) = (state.borrow().row, state.borrow().col);
            match app::event_key() {
                Key::Left => {
                    select_cell(&state, t, &mut name.clone(), &mut formula.clone(), row, col - 1);
                    true
                }
                Key::Right => {
                    select_cell(&state, t, &mut name.clone(), &mut formula.clone(), row, col + 1);
                    true
                }
                Key::Up => {
                    select_cell(&state, t, &mut name.clone(), &mut formula.clone(), row - 1, col);
                    true
                }
                Key::Down => {
                    select_cell(&state, t, &mut name.clone(), &mut formula.clone(), row + 1, col);
                    true
                }
                Key::Enter | Key::F2 => {
                    begin_edit(&state, &mut editor, '\0');
                    true
                }
                Key::Delete | Key::BackSpace => {
                    {
                        let mut s = state.borrow_mut();
                        if !s.sheet.raw(row, col).is_empty() {
                            s.sheet.set(row, col, "");
                            s.dirty = true;
                        }
                    }
                    refresh_header(&state, &mut name.clone(), &mut formula.clone());
                    t.redraw();
                    true
                }
                _ => {
                    let text = app::event_text();
                    if text.chars().count() == 1 && !text.chars().next().unwrap().is_control() {
                        begin_edit(&state, &mut editor, text.chars().next().unwrap());
                        true
                    } else {
                        false
                    }
                }
            }
        }
    });

    // Cell editor ----------------------------------------------------------
    editor.handle({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        let formula = formula.clone();
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
                        commit_edit(&state, &mut table, &mut name.clone(), &mut formula.clone(), e);
                        let _ = table.take_focus();
                        if app::event_key() == Key::Tab {
                            let (r, c) = (state.borrow().row, state.borrow().col);
                            select_cell(&state, &mut table, &mut name.clone(), &mut formula.clone(), r, c + 1);
                        }
                        return true;
                    }
                    _ => return false,
                }
            }
            if ev == Event::Unfocus && state.borrow().editing {
                commit_edit(&state, &mut table, &mut name.clone(), &mut formula.clone(), e);
            }
            false
        }
    });

    // Direct formula-bar editing -------------------------------------------
    formula.set_callback({
        let state = state.clone();
        let mut table = table.clone();
        let name = name.clone();
        move |f| {
            if state.borrow().editing {
                return;
            }
            let (row, col) = (state.borrow().row, state.borrow().col);
            let text = f.value();
            {
                let mut s = state.borrow_mut();
                if s.sheet.raw(row, col) != text {
                    s.sheet.set(row, col, &text);
                    s.dirty = true;
                }
            }
            refresh_header(&state, &mut name.clone(), &mut f.clone());
            table.redraw();
        }
    });

    select_cell(&state, &mut table, &mut name, &mut formula, 0, 0);
    app.run().unwrap();
}
