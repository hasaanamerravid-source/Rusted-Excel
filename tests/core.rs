#[path = "../src/core.rs"]
mod core;

use core::{parse_csv, Sheet};

#[test]
fn arithmetic_and_ranges() {
    let mut sheet = Sheet::new(10, 10);
    sheet.set(0, 0, "2"); // A1
    sheet.set(0, 1, "3"); // B1
    sheet.set(0, 2, "=SUM(A1:B1)");
    assert_eq!(sheet.display(0, 2), "5");

    sheet.set(1, 0, "10");
    sheet.set(2, 0, "20");
    sheet.set(0, 3, "=AVG(A1:A3)");
    assert_eq!(sheet.display(0, 3), "10.66666667");

    sheet.set(0, 4, "=POW(2,8)");
    assert_eq!(sheet.display(0, 4), "256");

    sheet.set(0, 5, "=IF(A1>B1,1,0)");
    assert_eq!(sheet.display(0, 5), "0");
}

#[test]
fn errors_are_cells_not_panics() {
    let mut sheet = Sheet::new(4, 4);
    sheet.set(0, 0, "=B1");
    sheet.set(0, 1, "=A1");
    assert_eq!(sheet.display(0, 0), "#CYCLE!");

    sheet.set(1, 0, "=1/0");
    assert_eq!(sheet.display(1, 0), "#DIV/0!");

    sheet.set(1, 1, "=NOPE(1)");
    assert_eq!(sheet.display(1, 1), "#NAME?");
}

#[test]
fn errors_propagate_through_operators_and_functions() {
    let mut sheet = Sheet::new(6, 6);

    sheet.set(0, 0, "=A1"); // self-cycle
    sheet.set(0, 1, "=A1+1");
    assert_eq!(sheet.display(0, 1), "#CYCLE!");

    sheet.set(1, 1, "=1/0");
    sheet.set(0, 2, "=1+B2");
    assert_eq!(sheet.display(0, 2), "#DIV/0!");

    sheet.set(0, 3, "=ABS(B2)");
    assert_eq!(sheet.display(0, 3), "#DIV/0!");

    sheet.set(0, 4, "=SUM(A2:B2)");
    assert_eq!(sheet.display(0, 4), "#DIV/0!");

    sheet.set(2, 2, "=B2"); // C3
    sheet.set(0, 5, "=1+C3");
    assert_eq!(sheet.display(0, 5), "#DIV/0!");
}

#[test]
fn arithmetic_precedence_and_literals() {
    let mut sheet = Sheet::new(4, 8);
    sheet.set(0, 0, "=-2^2");
    assert_eq!(sheet.display(0, 0), "-4");
    sheet.set(0, 1, "=2^3^2");
    assert_eq!(sheet.display(0, 1), "512");
    sheet.set(0, 2, "=TRUE");
    assert_eq!(sheet.display(0, 2), "1");
    sheet.set(0, 3, "=FALSE");
    assert_eq!(sheet.display(0, 3), "0");
    sheet.set(0, 4, "=IF(TRUE,1,0)");
    assert_eq!(sheet.display(0, 4), "1");
    sheet.set(0, 5, "=LEN(\"é\")");
    assert_eq!(sheet.display(0, 5), "1");
}

#[test]
fn cell_references_are_strict_and_bounded() {
    let mut sheet = Sheet::new(10, 10);
    sheet.set(0, 7, "42"); // H1
    sheet.set(0, 0, "=A.1");
    assert_eq!(sheet.display(0, 0), "#NAME?");
    sheet.set(0, 0, "=H1");
    assert_eq!(sheet.display(0, 0), "42");

    assert_eq!(Sheet::parse("A99999999999999999999"), None);
    assert_eq!(Sheet::parse("ABCDEFGHIJKLMNOPQRSTUVWXYZ1"), None);
    assert_eq!(Sheet::name(i32::MAX, 0), "");
    assert_eq!(Sheet::name(0, i32::MAX), "");
}

#[test]
fn cell_names_round_trip() {
    for (row, col) in [(0, 0), (0, 25), (9, 26), (99, 701)] {
        let name = Sheet::name(row, col);
        assert_eq!(Sheet::parse(&name), Some((row, col)));
    }
    assert_eq!(Sheet::name(0, 0), "A1");
    assert_eq!(Sheet::name(1, 26), "AA2");
}

#[test]
fn csv_quotes_and_escapes() {
    let mut cells = Vec::new();
    parse_csv("a,\"b,c\",\"say \"\"hi\"\"\"\r\n1,2,3", |r, c, text| {
        cells.push((r, c, text.to_string()));
    })
    .unwrap();
    assert_eq!(cells[0], (0, 0, "a".to_string()));
    assert_eq!(cells[1], (0, 1, "b,c".to_string()));
    assert_eq!(cells[2], (0, 2, "say \"hi\"".to_string()));
    assert_eq!(cells[3], (1, 0, "1".to_string()));

    assert_eq!(core::csv_escape("plain"), "plain");
    assert_eq!(core::csv_escape("a,b"), "\"a,b\"");
    assert_eq!(core::csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
}

#[test]
fn csv_trailing_newline_is_not_a_phantom_row() {
    let mut cells = Vec::new();
    parse_csv("a,b\n", |r, c, text| {
        cells.push((r, c, text.to_string()));
    })
    .unwrap();
    assert_eq!(cells, vec![(0, 0, "a".into()), (0, 1, "b".into())]);

    let mut cells = Vec::new();
    parse_csv("a,", |r, c, text| {
        cells.push((r, c, text.to_string()));
    })
    .unwrap();
    assert_eq!(cells, vec![(0, 0, "a".into()), (0, 1, String::new())]);
}

#[test]
fn malformed_csv_is_rejected() {
    let mut cells = Vec::new();
    assert!(parse_csv("\"a\"b,c", |r, c, text| {
        cells.push((r, c, text.to_string()));
    })
    .is_err());
    assert!(cells.is_empty());

    assert!(parse_csv("a\"b,c", |_, _, _| {}).is_err());
    assert!(parse_csv("\"unterminated", |_, _, _| {}).is_err());
    assert!(parse_csv("a\0b", |_, _, _| {}).is_err());
}

#[test]
fn panic_in_csv_sink_is_contained() {
    // Without the catch_unwind boundary this would abort the test process.
    assert!(parse_csv("a,b", |_, _, _| panic!("sink failed")).is_err());
}

#[test]
fn interior_nul_is_not_silently_dropped() {
    assert_eq!(core::csv_escape("a\0b"), "a\u{fffd}b");

    let mut sheet = Sheet::new(2, 2);
    sheet.set(0, 0, "a\0b");
    assert_eq!(sheet.raw(0, 0), "a\u{fffd}b");
}
