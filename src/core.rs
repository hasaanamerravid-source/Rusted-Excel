#![allow(dead_code)]
//! Thin safe wrappers around the C ABI exposed by `native/rx_core.cpp`
//! and `native/rx_csv.c`.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[repr(C)]
pub struct RxSheet {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn rx_sheet_new(rows: c_int, cols: c_int) -> *mut RxSheet;
    fn rx_sheet_free(sheet: *mut RxSheet);
    fn rx_sheet_rows(sheet: *const RxSheet) -> c_int;
    fn rx_sheet_cols(sheet: *const RxSheet) -> c_int;
    fn rx_sheet_resize(sheet: *mut RxSheet, rows: c_int, cols: c_int);
    fn rx_sheet_clear(sheet: *mut RxSheet);
    fn rx_sheet_set(sheet: *mut RxSheet, row: c_int, col: c_int, text: *const c_char);
    fn rx_sheet_raw(sheet: *const RxSheet, row: c_int, col: c_int) -> *const c_char;
    fn rx_sheet_display(sheet: *mut RxSheet, row: c_int, col: c_int) -> *const c_char;
    fn rx_sheet_number(sheet: *mut RxSheet, row: c_int, col: c_int) -> f64;
    fn rx_sheet_is_formula(sheet: *const RxSheet, row: c_int, col: c_int) -> c_int;
    fn rx_cell_name(row: c_int, col: c_int, buf: *mut c_char, len: usize) -> usize;
    fn rx_cell_parse(name: *const c_char, row: *mut c_int, col: *mut c_int) -> c_int;
    fn rx_csv_parse(
        input: *const c_char,
        cb: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char, usize)>,
        user: *mut c_void,
    ) -> c_int;
    fn rx_csv_escape(field: *const c_char, out: *mut c_char, out_len: usize) -> usize;
}

fn c_text(text: &str) -> CString {
    match CString::new(text) {
        Ok(value) => value,
        // Preserve the visible text instead of silently storing an empty cell.
        Err(_) => CString::new(text.replace('\0', "\u{fffd}"))
            .expect("replacement text cannot contain an interior NUL"),
    }
}

unsafe fn str_from(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

pub struct Sheet {
    ptr: *mut RxSheet,
}

impl Sheet {
    pub fn new(rows: i32, cols: i32) -> Self {
        let ptr = unsafe { rx_sheet_new(rows, cols) };
        assert!(!ptr.is_null(), "failed to allocate spreadsheet core");
        Self { ptr }
    }

    pub fn rows(&self) -> i32 {
        unsafe { rx_sheet_rows(self.ptr) }
    }

    pub fn cols(&self) -> i32 {
        unsafe { rx_sheet_cols(self.ptr) }
    }

    pub fn set(&mut self, row: i32, col: i32, text: &str) {
        let value = c_text(text);
        unsafe { rx_sheet_set(self.ptr, row, col, value.as_ptr()) }
    }

    pub fn raw(&self, row: i32, col: i32) -> String {
        unsafe { str_from(rx_sheet_raw(self.ptr, row, col)) }
    }

    pub fn display(&self, row: i32, col: i32) -> String {
        unsafe { str_from(rx_sheet_display(self.ptr, row, col)) }
    }

    pub fn number(&self, row: i32, col: i32) -> f64 {
        unsafe { rx_sheet_number(self.ptr, row, col) }
    }

    pub fn is_formula(&self, row: i32, col: i32) -> bool {
        unsafe { rx_sheet_is_formula(self.ptr, row, col) != 0 }
    }

    pub fn clear(&mut self) {
        unsafe { rx_sheet_clear(self.ptr) }
    }

    pub fn resize(&mut self, rows: i32, cols: i32) {
        unsafe { rx_sheet_resize(self.ptr, rows, cols) }
    }

    pub fn name(row: i32, col: i32) -> String {
        let mut buf = [0u8; 32];
        let len = unsafe { rx_cell_name(row, col, buf.as_mut_ptr().cast::<c_char>(), buf.len()) };
        String::from_utf8_lossy(&buf[..len.min(buf.len() - 1)]).into_owned()
    }

    pub fn parse(name: &str) -> Option<(i32, i32)> {
        let text = CString::new(name).ok()?;
        let mut row = 0;
        let mut col = 0;
        let ok = unsafe { rx_cell_parse(text.as_ptr(), &mut row, &mut col) };
        if ok == 0 {
            None
        } else {
            Some((row, col))
        }
    }
}

impl Drop for Sheet {
    fn drop(&mut self) {
        unsafe { rx_sheet_free(self.ptr) }
    }
}

struct CsvContext<'a> {
    sink: &'a mut dyn FnMut(i32, i32, &str),
    failed: bool,
}

unsafe extern "C" fn csv_callback(
    user: *mut c_void,
    row: c_int,
    col: c_int,
    text: *const c_char,
    len: usize,
) {
    if user.is_null() || text.is_null() {
        return;
    }
    let ctx = &mut *(user as *mut CsvContext<'_>);
    let bytes = std::slice::from_raw_parts(text.cast::<u8>(), len);
    let value = String::from_utf8_lossy(bytes);
    // A panic must never unwind through the C parser into an extern "C" frame.
    let result = catch_unwind(AssertUnwindSafe(|| (ctx.sink)(row, col, &value)));
    if result.is_err() {
        ctx.failed = true;
    }
}

/// Parse CSV, calling `sink` once for every field (including empty ones).
pub fn parse_csv(input: &str, mut sink: impl FnMut(i32, i32, &str)) -> Result<(), ()> {
    let text = CString::new(input).map_err(|_| ())?;
    let mut ctx = CsvContext {
        sink: &mut sink,
        failed: false,
    };
    let user = (&mut ctx as *mut CsvContext<'_>).cast::<c_void>();
    let ok = unsafe { rx_csv_parse(text.as_ptr(), Some(csv_callback), user) };
    if ok == 0 && !ctx.failed {
        Ok(())
    } else {
        Err(())
    }
}

/// Escape a single field for CSV output.
pub fn csv_escape(field: &str) -> String {
    let value = c_text(field);
    let needed = unsafe { rx_csv_escape(value.as_ptr(), std::ptr::null_mut(), 0) };
    let mut buf = vec![0u8; needed + 1];
    unsafe {
        rx_csv_escape(value.as_ptr(), buf.as_mut_ptr().cast::<c_char>(), buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}
