#[path = "../src/core.rs"]
mod core;

use core::{parse_csv, Sheet};

#[test]
fn arithmetic_and_ranges() {
    let mut sheet = Sheet::new(10, 10);
    sheet.set(0, 0, "2");       // A1
    sheet.set(0, 1, "3");       // B1
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
    }).unwrap();
    assert_eq!(cells[0], (0, 0, "a".to_string()));
    assert_eq!(cells[1], (0, 1, "b,c".to_string()));
    assert_eq!(cells[2], (0, 2, "say \"hi\"".to_string()));
    assert_eq!(cells[3], (1, 0, "1".to_string()));

    assert_eq!(core::csv_escape("plain"), "plain");
    assert_eq!(core::csv_escape("a,b"), "\"a,b\"");
    assert_eq!(core::csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
}
