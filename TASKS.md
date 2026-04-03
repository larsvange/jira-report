# Jira Time Report — Task Breakdown

Derived from [PRD.md](./PRD.md) on 2026-04-03.

Legend: **P0** = must-have for v1, **P1** = nice-to-have for v1.1.

---

## Phase 0: Project Scaffolding

> Goal: A compilable, runnable Rust binary that starts an Axum server on the configured port and serves a placeholder page.

### Milestone 0.1 — Toolchain & Repo Init

- [ ] **0.1.1** Install Rust toolchain (rustup) if not present; verify `cargo --version` outputs >= 1.77.
- [ ] **0.1.2** Run `cargo init --name jira-report` inside the project directory to generate `Cargo.toml` and `src/main.rs`.
- [ ] **0.1.3** Initialize git repository (`git init`), create `.gitignore` with `/target`, `.env`, and OS files.
- [ ] **0.1.4** Create `.env.example` documenting all four env vars (`JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`, `PORT`).

### Milestone 0.2 — Dependency Wiring

- [ ] **0.2.1** Add all dependencies to `Cargo.toml`:
  - `axum = "0.8"`, `tokio = { version = "1", features = ["full"] }`, `tower-http = { version = "0.6", features = ["fs"] }`
  - `tera = "1"`, `reqwest = { version = "0.13", features = ["json"] }`, `rust_xlsxwriter = "0.94"`
  - `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `uuid = { version = "1", features = ["v4"] }`
  - `dashmap = "6"`, `dotenvy = "0.15"`, `chrono = { version = "0.4", features = ["serde"] }`, `thiserror = "2"`
  - `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`
  - **AC:** `cargo check` succeeds with zero errors.
- [ ] **0.2.2** Create `templates/` directory with a stub `index.html` (valid HTML, empty body).
- [ ] **0.2.3** Write minimal `src/main.rs` that reads `PORT` from env (default 3000), initializes Tera, binds an Axum router with `GET /` returning the rendered template.
  - **AC:** `cargo run` starts; `curl http://localhost:3000/` returns 200 with HTML.

### Milestone 0.3 — Module Skeleton

- [ ] **0.3.1** Create `src/jira.rs` with a public `JiraClient` struct (holds `reqwest::Client`, base URL, credentials) and a stub `pub async fn fetch_projects(&self) -> Result<Vec<Project>>` that returns an empty vec.
- [ ] **0.3.2** Create `src/report.rs` with a stub `pub fn generate_workbook(worklogs: &[Worklog]) -> Result<Vec<u8>>` that returns an empty xlsx byte vec.
- [ ] **0.3.3** Declare both modules in `main.rs` (`mod jira; mod report;`).
  - **AC:** `cargo check` succeeds; both modules compile.

---

## Phase 1: P0 — Core Functionality

> Goal: Complete end-to-end flow — user loads page, selects project, submits, polls status, downloads a valid 4-tab Excel file.

### Milestone 1.1 — Environment Validation & App State (P0-9)

- [ ] **1.1.1** On startup, read `JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN` from env (via `dotenvy`). Panic with a clear message naming the missing var if any is absent.
  - **AC:** Removing `JIRA_EMAIL` from `.env` causes startup to print `"JIRA_EMAIL environment variable is required"` and exit with code 1.
- [ ] **1.1.2** Define `AppState` struct containing `JiraClient`, `Tera`, and job store (`Arc<DashMap<Uuid, JobState>>`). Wrap in `Arc` and pass to Axum via `.with_state()`.
  - **AC:** Handler functions can extract `State<Arc<AppState>>`.

### Milestone 1.2 — Jira Project Listing (P0-1, P0-2)

- [ ] **1.2.1** Implement `JiraClient::fetch_projects()` calling `GET /rest/api/3/project/search` with Basic Auth, paginating with `startAt`/`maxResults` (page size 50) until all projects are collected.
  - **AC:** Returns a `Vec<Project>` with `key` and `name` fields; works against a real Jira instance.
- [ ] **1.2.2** Define `Project` serde struct: `{ key: String, name: String }`.
- [ ] **1.2.3** In the `GET /` handler, call `fetch_projects()`, pass the list into the Tera context, render `index.html`.
  - **AC:** Dropdown is populated dynamically with real Jira project names/keys.
- [ ] **1.2.4** Build out `templates/index.html`: project `<select>`, start-date `<input type="date">`, end-date `<input type="date">`, submit button. Minimal CSS for usability.
  - **AC:** Page renders correctly in a browser; form fields are functional.

### Milestone 1.3 — Job Submission & Async Dispatch (P0-3, P0-4)

- [ ] **1.3.1** Define `JobState` enum/struct:
  ```
  status: Pending | Running | Done | Error
  message: Option<String>
  bytes: Option<Vec<u8>>
  project_key: String
  start_date: NaiveDate
  end_date: NaiveDate
  ```
  - **AC:** Compiles; can be inserted into DashMap.
- [ ] **1.3.2** Implement `POST /generate` handler: parse form body (`project_key`, `start_date`, `end_date`), validate `end_date >= start_date`, generate UUID, insert `Pending` job into store, return `{ "job_id": "<uuid>" }`.
  - **AC:** Returns 200 + JSON within 500ms; returns 400 for invalid dates.
- [ ] **1.3.3** After inserting the job, `tokio::spawn` an async task that: updates state to `Running`, calls Jira APIs, generates Excel, stores bytes, updates state to `Done` (or `Error` on failure).
  - **AC:** Server does not block; second request can be submitted while first is running.

### Milestone 1.4 — Jira Worklog Fetching (supports P0-8)

- [ ] **1.4.1** Implement `JiraClient::search_issues(project_key, start_date, end_date)` using JQL: `project = "{key}" AND worklogDate >= "{start}" AND worklogDate <= "{end}"`. Paginate with `startAt`/`maxResults` (100 per page). Return `Vec<Issue>` with `key`, `summary`, `fields.parent` (for hierarchy), `fields.issuetype.name`.
  - **AC:** Returns all matching issues across multiple pages.
- [ ] **1.4.2** Implement `JiraClient::fetch_worklogs(issue_key)` calling `GET /rest/api/3/issue/{key}/worklog?startedAfter={epoch_ms}&startedBefore={epoch_ms}`, paginating if > 1000 results. Map each worklog to a `Worklog` struct: `{ issue_key, issue_summary, author_display_name, started: NaiveDate, time_spent_seconds: u64, comment: Option<String> }`.
  - **AC:** Returns filtered worklogs within the date range.
- [ ] **1.4.3** In the spawned task, iterate over issues and collect all worklogs into a single `Vec<Worklog>`. Also collect issue metadata for the hierarchy tab (epic link, parent key, issue type).
  - **AC:** All worklogs for the project + date range are aggregated.

### Milestone 1.5 — Excel Generation: 4 Tabs (P0-8)

- [ ] **1.5.1** **Tab 1 — Worklogs:** Create worksheet, write header row (`Issue Key`, `Issue Summary`, `Author`, `Date`, `Hours`, `Comment`), iterate worklogs writing one row each. Format `Hours` as `time_spent_seconds / 3600.0`.
  - **AC:** Sheet contains all worklogs; hours are decimal numbers.
- [ ] **1.5.2** **Tab 2 — Summary by Person:** Aggregate worklogs by `author_display_name`, sum hours. Write header (`Author`, `Total Hours`) + one row per author, sorted alphabetically.
  - **AC:** Totals match sum of Tab 1 rows per author.
- [ ] **1.5.3** **Tab 3 — Summary by Issue:** Aggregate worklogs by `issue_key`, sum hours. Write header (`Issue Key`, `Issue Summary`, `Total Hours`) + one row per issue, sorted by key.
  - **AC:** Totals match sum of Tab 1 rows per issue.
- [ ] **1.5.4** **Tab 4 — Hierarchy:** Group issues by Epic -> Story/Task -> Sub-task. For each epic, write epic row with rolled-up hours, then indented children, then indented sub-tasks. Issues without an epic go under an "(No Epic)" group.
  - **AC:** Three-level hierarchy is visible; rolled-up hours at each level are correct.
- [ ] **1.5.5** Serialize the workbook to `Vec<u8>` using `workbook.save_to_buffer()`. Store bytes in `JobState`.
  - **AC:** Bytes form a valid .xlsx file openable in Excel/LibreOffice.

### Milestone 1.6 — Status Polling & Download (P0-5, P0-6, P0-7)

- [ ] **1.6.1** Implement `GET /status/:id` handler: look up UUID in DashMap; return 404 if missing, else return `{ "status": "...", "message": "..." }`.
  - **AC:** Returns correct state for pending, running, done, and error jobs.
- [ ] **1.6.2** Implement `GET /download/:id` handler: look up UUID; return 404 if not found or not `Done`; return bytes with `Content-Type: application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` and `Content-Disposition: attachment; filename="report.xlsx"`.
  - **AC:** Browser downloads a valid Excel file.
- [ ] **1.6.3** Add JavaScript to `index.html`: on form submit, POST to `/generate` via `fetch()`, start polling `/status/:id` every 2 seconds, show a progress area with current status/message, on `done` show a download link to `/download/:id`, on `error` show the error message.
  - **AC:** Full round-trip works in browser without page reload.

### Milestone 1.7 — Error Handling & Edge Cases (P0-10)

- [ ] **1.7.1** Wrap all Jira API calls in proper error handling; map reqwest errors, non-2xx responses, and JSON parse failures to a unified `AppError` type (using `thiserror`).
  - **AC:** A Jira 403 results in job state `Error` with message "Jira API returned 403 Forbidden".
- [ ] **1.7.2** Handle edge case: project with zero worklogs in date range. Generate a valid Excel file with headers but no data rows; status goes to `Done`.
  - **AC:** Downloading an empty-range report yields a valid 4-tab file.
- [ ] **1.7.3** Handle edge case: invalid project key submitted. The JQL search returns zero issues; this is not an error.
  - **AC:** Status reaches `Done`; empty report is downloadable.
- [ ] **1.7.4** Add request tracing with `tracing` crate — log incoming requests, job lifecycle events, Jira API call durations.
  - **AC:** `RUST_LOG=info cargo run` shows structured log lines for each request.

---

## Phase 2: P1 — Polish & Reliability

> Goal: Improve UX, operational robustness, and Jira API resilience.

### Milestone 2.1 — Descriptive Filename (P1-1)

- [ ] **2.1.1** In the download handler, construct filename as `{PROJECT}_{start_date}_{end_date}.xlsx` (e.g., `PROJ_2026-03-01_2026-03-31.xlsx`). Set via `Content-Disposition` header.
  - **AC:** Downloaded file has the descriptive name in the user's Downloads folder.

### Milestone 2.2 — Date Defaults (P1-2)

- [ ] **2.2.1** In the `GET /` handler, compute first and last day of the current month. Pass as `default_start` and `default_end` to the Tera context. Set as `value` attributes on the date inputs.
  - **AC:** On page load, date fields are pre-filled with current month boundaries.

### Milestone 2.3 — Progress Messages (P1-3)

- [ ] **2.3.1** Update the spawned task to write progress messages to `JobState.message` at key points: "Fetching project issues...", "Fetching worklogs for issue 15/120...", "Generating Excel workbook...", etc.
  - **AC:** Polling `/status/:id` returns changing messages as the job progresses.
- [ ] **2.3.2** Update the JavaScript polling UI to display the `message` field in the progress area.
  - **AC:** User sees live progress text in the browser.

### Milestone 2.4 — Job Cleanup (P1-4)

- [ ] **2.4.1** Add a `created_at: Instant` field to `JobState`.
- [ ] **2.4.2** Spawn a background Tokio task on startup that runs every 5 minutes, iterates the DashMap, and removes entries older than 1 hour.
  - **AC:** After 1 hour, a completed job is no longer accessible via `/status/:id` or `/download/:id`.

### Milestone 2.5 — Rate-Limit Retry (P1-5)

- [ ] **2.5.1** In `JiraClient`, wrap each API call in a retry loop: on HTTP 429, read the `Retry-After` header (or default to 5s), sleep, retry up to 3 times.
  - **AC:** A simulated 429 response is retried and eventually succeeds.
- [ ] **2.5.2** Log each retry with `tracing::warn!` including the wait duration.
  - **AC:** Retry events appear in server logs.
