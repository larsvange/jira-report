use std::collections::BTreeMap;

use rust_xlsxwriter::{Format, Formula, Workbook, XlsxError};

use crate::jira::Worklog;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn generate_workbook(worklogs: &[Worklog]) -> Result<Vec<u8>, XlsxError> {
    let mut wb = Workbook::new();

    // Formats
    let bold = Format::new().set_bold();
    let num  = Format::new().set_num_format("#,##0.00");
    let date = Format::new().set_num_format("yyyy-mm-dd");

    write_worklogs_tab(&mut wb, worklogs, &bold, &num, &date)?;
    write_summary_by_person(&mut wb, worklogs, &bold, &num)?;
    write_summary_by_issue(&mut wb, worklogs, &bold, &num)?;

    wb.save_to_buffer()
}

// ---------------------------------------------------------------------------
// Tab 1 — Worklogs
// ---------------------------------------------------------------------------

fn write_worklogs_tab(
    wb: &mut Workbook,
    worklogs: &[Worklog],
    bold: &Format,
    num: &Format,
    date_fmt: &Format,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Worklogs")?;

    let headers = ["Issue Key", "Issue Summary", "Author", "Date", "Hours", "Comment"];
    for (col, h) in headers.iter().enumerate() {
        ws.write_with_format(0, col as u16, *h, bold)?;
    }

    for (row, wl) in worklogs.iter().enumerate() {
        let r = (row + 1) as u32;
        ws.write(r, 0, &wl.issue_key)?;
        ws.write(r, 1, &wl.issue_summary)?;
        ws.write(r, 2, &wl.author)?;
        ws.write_with_format(r, 3, &wl.date, date_fmt)?;
        ws.write_with_format(r, 4, wl.hours, num)?;
        ws.write(r, 5, &wl.comment)?;
    }

    let total_hours: f64 = worklogs.iter().map(|wl| wl.hours).sum();
    let total_row = (worklogs.len() + 1) as u32;
    ws.write_with_format(total_row, 3, "Total", bold)?;
    ws.write_formula_with_format(
        total_row, 4,
        Formula::new(format!("=SUM(E2:E{})", worklogs.len() + 1)).set_result(total_hours.to_string()),
        num,
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tab 2 — Summary by Person
// ---------------------------------------------------------------------------

fn write_summary_by_person(
    wb: &mut Workbook,
    worklogs: &[Worklog],
    bold: &Format,
    num: &Format,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Summary by Person")?;

    ws.write_with_format(0, 0, "Author", bold)?;
    ws.write_with_format(0, 1, "Total Hours", bold)?;

    // Aggregate
    let mut totals: BTreeMap<&str, f64> = BTreeMap::new();
    for wl in worklogs {
        *totals.entry(wl.author.as_str()).or_default() += wl.hours;
    }

    for (row, (author, hours)) in totals.iter().enumerate() {
        let r = (row + 1) as u32;
        ws.write(r, 0, *author)?;
        ws.write_with_format(r, 1, *hours, num)?;
    }

    let total_hours: f64 = totals.values().sum();
    let total_row = (totals.len() + 1) as u32;
    ws.write_with_format(total_row, 0, "Total", bold)?;
    ws.write_formula_with_format(
        total_row, 1,
        Formula::new(format!("=SUM(B2:B{})", totals.len() + 1)).set_result(total_hours.to_string()),
        num,
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tab 3 — Summary by Issue
// ---------------------------------------------------------------------------

fn write_summary_by_issue(
    wb: &mut Workbook,
    worklogs: &[Worklog],
    bold: &Format,
    num: &Format,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Summary by Issue")?;

    ws.write_with_format(0, 0, "Issue Key", bold)?;
    ws.write_with_format(0, 1, "Issue Summary", bold)?;
    ws.write_with_format(0, 2, "Total Hours", bold)?;

    // Aggregate preserving summary
    let mut totals: BTreeMap<&str, (&str, f64)> = BTreeMap::new();
    for wl in worklogs {
        let entry = totals
            .entry(wl.issue_key.as_str())
            .or_insert((&wl.issue_summary, 0.0));
        entry.1 += wl.hours;
    }

    for (row, (key, (summary, hours))) in totals.iter().enumerate() {
        let r = (row + 1) as u32;
        ws.write(r, 0, *key)?;
        ws.write(r, 1, *summary)?;
        ws.write_with_format(r, 2, *hours, num)?;
    }

    let total_hours: f64 = totals.values().map(|(_, h)| h).sum();
    let total_row = (totals.len() + 1) as u32;
    ws.write_with_format(total_row, 0, "Total", bold)?;
    ws.write_formula_with_format(
        total_row, 2,
        Formula::new(format!("=SUM(C2:C{})", totals.len() + 1)).set_result(total_hours.to_string()),
        num,
    )?;

    Ok(())
}

