use std::collections::{BTreeMap, HashMap};

use rust_xlsxwriter::{Format, Workbook, XlsxError};

use crate::jira::Worklog;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct IssueNode {
    pub key: String,
    pub summary: String,
    pub issue_type: String,
    pub parent_key: Option<String>,
    pub epic_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn generate_workbook(
    worklogs: &[Worklog],
    issue_nodes: &[IssueNode],
) -> Result<Vec<u8>, XlsxError> {
    let mut wb = Workbook::new();

    // Formats
    let bold = Format::new().set_bold();
    let num  = Format::new().set_num_format("#,##0.00");
    let date = Format::new().set_num_format("yyyy-mm-dd");

    write_worklogs_tab(&mut wb, worklogs, &bold, &num, &date)?;
    write_summary_by_person(&mut wb, worklogs, &bold, &num)?;
    write_summary_by_issue(&mut wb, worklogs, &bold, &num)?;
    write_hierarchy(&mut wb, worklogs, issue_nodes, &bold, &num)?;

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

    Ok(())
}

// ---------------------------------------------------------------------------
// Tab 4 — Hierarchy
// ---------------------------------------------------------------------------

fn write_hierarchy(
    wb: &mut Workbook,
    worklogs: &[Worklog],
    issue_nodes: &[IssueNode],
    bold: &Format,
    num: &Format,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Hierarchy")?;

    ws.write_with_format(0, 0, "Issue Key", bold)?;
    ws.write_with_format(0, 1, "Summary", bold)?;
    ws.write_with_format(0, 2, "Type", bold)?;
    ws.write_with_format(0, 3, "Total Hours", bold)?;

    // Build hours map per issue
    let mut hours_map: HashMap<&str, f64> = HashMap::new();
    for wl in worklogs {
        *hours_map.entry(wl.issue_key.as_str()).or_default() += wl.hours;
    }

    // Separate epics, stories/tasks, sub-tasks
    let mut epics: Vec<&IssueNode> = Vec::new();
    let mut by_epic: HashMap<Option<&str>, Vec<&IssueNode>> = HashMap::new();
    let mut by_parent: HashMap<&str, Vec<&IssueNode>> = HashMap::new();

    for node in issue_nodes {
        if node.issue_type == "Epic" {
            epics.push(node);
        } else if node.issue_type == "Sub-task" || node.issue_type == "Subtask" {
            if let Some(p) = &node.parent_key {
                by_parent.entry(p.as_str()).or_default().push(node);
            }
        } else {
            let epic_key = node.epic_key.as_deref();
            by_epic.entry(epic_key).or_default().push(node);
        }
    }

    let mut row = 1u32;

    let write_epic_group = |ws: &mut rust_xlsxwriter::Worksheet,
                                 epic: Option<&IssueNode>,
                                 children: &[&IssueNode],
                                 row: &mut u32|
     -> Result<(), XlsxError> {
        // Epic row
        let (epic_key, epic_summary, epic_hours) = match epic {
            Some(e) => {
                let h: f64 = children.iter().map(|c| hours_map.get(c.key.as_str()).copied().unwrap_or(0.0)).sum::<f64>()
                    + hours_map.get(e.key.as_str()).copied().unwrap_or(0.0);
                (e.key.as_str(), e.summary.as_str(), h)
            }
            None => ("(No Epic)", "(No Epic)", children.iter().map(|c| hours_map.get(c.key.as_str()).copied().unwrap_or(0.0)).sum()),
        };

        ws.write_with_format(*row, 0, epic_key, bold)?;
        ws.write_with_format(*row, 1, epic_summary, bold)?;
        ws.write_with_format(*row, 2, "Epic", bold)?;
        ws.write_with_format(*row, 3, epic_hours, num)?;
        *row += 1;

        for child in children {
            let child_hours: f64 = by_parent
                .get(child.key.as_str())
                .map(|subs| subs.iter().map(|s| hours_map.get(s.key.as_str()).copied().unwrap_or(0.0)).sum())
                .unwrap_or(0.0)
                + hours_map.get(child.key.as_str()).copied().unwrap_or(0.0);

            ws.write(*row, 0, format!("  {}", child.key))?;
            ws.write(*row, 1, format!("  {}", child.summary))?;
            ws.write(*row, 2, &child.issue_type)?;
            ws.write_with_format(*row, 3, child_hours, num)?;
            *row += 1;

            if let Some(subs) = by_parent.get(child.key.as_str()) {
                for sub in subs.iter() {
                    let sub_hours = hours_map.get(sub.key.as_str()).copied().unwrap_or(0.0);
                    ws.write(*row, 0, format!("    {}", sub.key))?;
                    ws.write(*row, 1, format!("    {}", sub.summary))?;
                    ws.write(*row, 2, &sub.issue_type)?;
                    ws.write_with_format(*row, 3, sub_hours, num)?;
                    *row += 1;
                }
            }
        }
        Ok(())
    };

    // Write epics and their children
    let mut sorted_epics = epics;
    sorted_epics.sort_by_key(|e| e.key.as_str());

    for epic in &sorted_epics {
        let children = by_epic.remove(&Some(epic.key.as_str())).unwrap_or_default();
        write_epic_group(ws, Some(epic), &children, &mut row)?;
    }

    // Orphans (no matching epic node found)
    let orphans = by_epic.remove(&(None::<&str>)).unwrap_or_default();
    if !orphans.is_empty() {
        write_epic_group(ws, None, &orphans, &mut row)?;
    }

    Ok(())
}
