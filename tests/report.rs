use calamine::{Data, Reader, Xlsx};
use chrono::NaiveDate;
use jira_report::jira::Worklog;
use jira_report::report::generate_workbook;
use std::io::Cursor;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn wl(issue_key: &str, issue_summary: &str, author: &str, hours: f64) -> Worklog {
    Worklog {
        issue_key: issue_key.to_string(),
        issue_summary: issue_summary.to_string(),
        author: author.to_string(),
        date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        hours,
        comment: String::new(),
    }
}

fn parse(bytes: Vec<u8>) -> Xlsx<Cursor<Vec<u8>>> {
    Xlsx::new(Cursor::new(bytes)).expect("failed to parse xlsx")
}

fn cell_str(wb: &mut Xlsx<Cursor<Vec<u8>>>, sheet: &str, row: u32, col: u32) -> String {
    let range = wb.worksheet_range(sheet).expect("sheet not found");
    match range.get_value((row, col)) {
        Some(Data::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn cell_float(wb: &mut Xlsx<Cursor<Vec<u8>>>, sheet: &str, row: u32, col: u32) -> f64 {
    let range = wb.worksheet_range(sheet).expect("sheet not found");
    match range.get_value((row, col)) {
        Some(Data::Float(f)) => *f,
        Some(Data::Int(i)) => *i as f64,
        other => panic!("expected float at ({row},{col}), got {other:?}"),
    }
}

fn row_count(wb: &mut Xlsx<Cursor<Vec<u8>>>, sheet: &str) -> usize {
    wb.worksheet_range(sheet)
        .expect("sheet not found")
        .rows()
        .count()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn workbook_has_three_sheets() {
    let wb = parse(generate_workbook(&[]).unwrap());
    let names: Vec<String> = wb.sheet_names().to_vec();
    assert_eq!(names, ["Worklogs", "Summary by Person", "Summary by Issue"]);
}

#[test]
fn worklogs_tab_row_count() {
    let worklogs = vec![
        wl("PROJ-1", "Fix bug", "Alice", 2.0),
        wl("PROJ-2", "Add feature", "Bob", 3.0),
    ];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    // 1 header row + 2 data rows + 1 total row
    assert_eq!(row_count(&mut wb, "Worklogs"), 4);
}

#[test]
fn worklogs_tab_cell_values() {
    let worklogs = vec![wl("PROJ-1", "Fix bug", "Alice", 2.5)];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    assert_eq!(cell_str(&mut wb, "Worklogs", 1, 0), "PROJ-1");
    assert_eq!(cell_str(&mut wb, "Worklogs", 1, 1), "Fix bug");
    assert_eq!(cell_str(&mut wb, "Worklogs", 1, 2), "Alice");
    assert_eq!(cell_float(&mut wb, "Worklogs", 1, 4), 2.5);
}

#[test]
fn worklogs_tab_has_total_row() {
    let worklogs = vec![
        wl("PROJ-1", "Fix bug", "Alice", 2.0),
        wl("PROJ-2", "Add feature", "Bob", 3.0),
    ];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    // Total label is in col 3 (Date column), formula result in col 4 (Hours)
    assert_eq!(cell_str(&mut wb, "Worklogs", 3, 3), "Total");
    assert_eq!(cell_float(&mut wb, "Worklogs", 3, 4), 5.0);
}

#[test]
fn summary_by_person_has_total_row() {
    let worklogs = vec![
        wl("PROJ-1", "Task", "Alice", 2.0),
        wl("PROJ-2", "Task", "Bob", 3.0),
    ];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    // 2 authors → total row is row 3
    assert_eq!(cell_str(&mut wb, "Summary by Person", 3, 0), "Total");
    assert_eq!(cell_float(&mut wb, "Summary by Person", 3, 1), 5.0);
}

#[test]
fn summary_by_issue_has_total_row() {
    let worklogs = vec![
        wl("PROJ-1", "Fix bug", "Alice", 2.0),
        wl("PROJ-2", "Add feature", "Bob", 3.0),
    ];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    // 2 issues → total row is row 3
    assert_eq!(cell_str(&mut wb, "Summary by Issue", 3, 0), "Total");
    assert_eq!(cell_float(&mut wb, "Summary by Issue", 3, 2), 5.0);
}

#[test]
fn summary_by_person_aggregates_hours() {
    let worklogs = vec![
        wl("PROJ-1", "Fix bug", "Alice", 2.0),
        wl("PROJ-2", "Add feature", "Alice", 1.5),
        wl("PROJ-3", "Review", "Bob", 3.0),
    ];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    // Rows are sorted alphabetically by author: Alice (row 1), Bob (row 2)
    assert_eq!(cell_str(&mut wb, "Summary by Person", 1, 0), "Alice");
    assert_eq!(cell_float(&mut wb, "Summary by Person", 1, 1), 3.5);
    assert_eq!(cell_str(&mut wb, "Summary by Person", 2, 0), "Bob");
    assert_eq!(cell_float(&mut wb, "Summary by Person", 2, 1), 3.0);
}

#[test]
fn summary_by_issue_aggregates_hours() {
    let worklogs = vec![
        wl("PROJ-1", "Fix bug", "Alice", 2.0),
        wl("PROJ-1", "Fix bug", "Bob", 1.0),
        wl("PROJ-2", "Add feature", "Alice", 4.0),
    ];
    let mut wb = parse(generate_workbook(&worklogs).unwrap());
    // Sorted by issue key: PROJ-1 (row 1), PROJ-2 (row 2)
    assert_eq!(cell_str(&mut wb, "Summary by Issue", 1, 0), "PROJ-1");
    assert_eq!(cell_float(&mut wb, "Summary by Issue", 1, 2), 3.0);
    assert_eq!(cell_str(&mut wb, "Summary by Issue", 2, 0), "PROJ-2");
    assert_eq!(cell_float(&mut wb, "Summary by Issue", 2, 2), 4.0);
}

