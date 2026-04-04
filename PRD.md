# PRD: Jira Time Report Web App

**Status:** Shipped (v1.0)
**Date:** 2026-04-04
**Stack:** Rust / Axum / Tera / reqwest / rust_xlsxwriter

---

## Problem Statement

Teams using Jira need periodic time reports to track logged hours per person and per issue across a project and date range. Today this requires running a CLI skill manually, which creates friction for non-technical users and doesn't fit into browser-based workflows. By exposing the same report logic as a small web application, any team member can self-serve a time report without terminal access, and the result lands directly in their Downloads folder as an Excel file.

---

## Goals

1. Any team member can generate a time report for any Jira project without needing CLI access or local tooling.
2. Report generation is non-blocking — the browser remains usable while the job runs.
3. The Excel output is structurally identical to the existing `jira-timereport` skill (4 tabs, same schema), so existing consumers of that report don't need to change.
4. The app starts with a single `cargo run` and zero manual configuration beyond environment variables.
5. The full round-trip from page load to download completes in under 60 seconds for projects with up to 500 issues.

---

## Non-Goals

1. **Multi-user auth / login UI** — credentials come from the server's environment variables; the app is intended for single-operator or internal-network use only. Building a login flow is out of scope.
2. **Persistent job history / report archive** — completed jobs are held in memory for the server's lifetime only. Durable storage is a future concern.
3. **Report scheduling / recurring exports** — no cron or automated triggers in v1.
4. **Non-Jira sources** — only Jira Cloud REST API v3 is targeted. Linear, GitHub Issues, etc. are out of scope.
5. **CSV / PDF output formats** — Excel (`.xlsx`) is the only output format for v1.

---

## User Stories

### Project Manager / Team Lead

- As a project manager, I want to select a Jira project from a dropdown so that I don't have to remember or type project keys manually.
- As a project manager, I want to pick a start and end date so that I can scope the report to a specific sprint, month, or billing period.
- As a project manager, I want to click a single button to start the report so that I don't need to install anything or run commands.
- As a project manager, I want to see a progress indicator while the report is generating so that I know the app is working and haven't lost my request.
- As a project manager, I want a download link to appear automatically when the report is ready so that I can save it without refreshing the page.

### Any Browser User

- As a user, I want the page to load the project list automatically so that the dropdown is ready when I open the app.
- As a user, I want clear error messages if report generation fails so that I know whether to retry or contact the app operator.
- As a user, I want the downloaded file to be named descriptively (e.g., `PROJECT_2026-03-01_2026-03-31.xlsx`) so that I can identify it in my Downloads folder.

---

## Requirements

### Must-Have (P0)

| # | Requirement | Acceptance Criteria |
|---|-------------|---------------------|
| P0-1 | `GET /` renders an HTML page with a project dropdown, start date, end date, and submit button | Page loads without error; dropdown lists all Jira projects visible to the configured credentials |
| P0-2 | Project list is fetched from Jira API on page load | Dropdown is populated dynamically; hardcoded project lists are not acceptable |
| P0-3 | `POST /generate` accepts `project_key`, `start_date`, `end_date`; returns `{ "job_id": "<uuid>" }` | Form submission returns JSON with a valid UUID job ID within 500ms |
| P0-4 | Report generation runs asynchronously (Tokio task); the HTTP response is returned immediately | Server does not block on Jira API calls; browser is not stalled waiting for the full report |
| P0-5 | `GET /status/:id` returns job state (`pending`, `running`, `done`, `error`) and optional `progress` message | Browser can poll this endpoint; returns 404 for unknown IDs, 200 with JSON for known IDs |
| P0-6 | Browser polls `/status/:id` and shows a progress indicator until `done` or `error` | UI reflects live status without a full page reload |
| P0-7 | `GET /download/:id` streams the completed `.xlsx` file with `Content-Disposition: attachment` | Browser triggers a file download; file is a valid Excel workbook |
| P0-8 | Excel output has 4 sheets: Worklogs, Summary by Person, Summary by Issue, Hierarchy | All 4 sheets present; schema matches the existing `jira-timereport` skill output |
| P0-9 | Jira credentials (`JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`) are read from environment variables | App fails to start (with a clear error) if any required env var is missing |
| P0-10 | Error states are surfaced to the user | If generation fails, `/status/:id` returns `"error"` with a message; UI displays it clearly |

### Nice-to-Have (P1)

| # | Requirement | Notes |
|---|-------------|-------|
| P1-1 | Descriptive download filename: `<PROJECT>_<start>_<end>.xlsx` | Set via `Content-Disposition` header |
| P1-2 | Date pickers default to the first and last day of the current month | Reduces clicks for the common monthly-report case |
| P1-3 | Progress messages show which Jira page is being fetched (e.g., "Fetching issues 100/340…") | Requires passing a progress channel from `jira.rs` to the job state |
| P1-4 | In-memory job cleanup: remove jobs older than 1 hour | Prevents unbounded memory growth on long-running servers |
| P1-5 | Basic rate-limit handling on Jira API calls (retry with backoff on 429) | Jira Cloud throttles heavy queries; silent retries improve reliability |

### Future Considerations (P2)

| # | Idea | Why deferred |
|---|------|--------------|
| P2-1 | Persist completed reports to disk or object storage | Needs a storage strategy; unnecessary for single-user/local use |
| P2-2 | Multi-project selection (generate one report across several projects) | Requires UX and schema changes; low demand in v1 |
| P2-3 | OAuth / per-user Jira credentials | Needed for multi-tenant deployment; out of scope for internal tool |
| P2-4 | Configurable Excel template / custom columns | High complexity; the fixed schema covers all current consumers |
| P2-5 | Webhook / callback on completion instead of polling | Nice alternative to polling; can be layered on the existing job model later |

---

## API Contract

### `POST /generate`

**Request** (form-encoded or JSON):
```
project_key = "PROJ"
start_date  = "2026-03-01"
end_date    = "2026-03-31"
```

**Response `200 OK`:**
```json
{ "job_id": "550e8400-e29b-41d4-a716-446655440000" }
```

**Response `400 Bad Request`:**
```json
{ "error": "end_date must be after start_date" }
```

---

### `GET /status/:id`

**Response `200 OK`** (while running):
```json
{ "status": "running", "message": "Fetching issues 60/200…" }
```

**Response `200 OK`** (complete):
```json
{ "status": "done" }
```

**Response `200 OK`** (failed):
```json
{ "status": "error", "message": "Jira API returned 403 Forbidden" }
```

**Response `404 Not Found`:** unknown job ID.

---

### `GET /download/:id`

- Returns `404` if job not found or not yet `done`.
- Returns `200` with headers:
  - `Content-Type: application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`
  - `Content-Disposition: attachment; filename="<PROJECT>_<start>_<end>.xlsx"`
- Body: raw `.xlsx` bytes.

---

## Excel Workbook Schema

Matches the existing `jira-timereport` skill output exactly.

### Tab 1 — Worklogs
Raw worklog rows, one per logged entry.

| Column | Type | Notes |
|--------|------|-------|
| Issue Key | String | e.g. `PROJ-123` |
| Issue Summary | String | |
| Author | String | Display name |
| Date | Date | Worklog start date |
| Hours | Number | `timeSpentSeconds / 3600` |
| Comment | String | Worklog comment if present |

### Tab 2 — Summary by Person
Pivot: total hours per author.

| Column | Type |
|--------|------|
| Author | String |
| Total Hours | Number |

### Tab 3 — Summary by Issue
Pivot: total hours per issue.

| Column | Type |
|--------|------|
| Issue Key | String |
| Issue Summary | String |
| Total Hours | Number |

### Tab 4 — Hierarchy
Issues grouped by Epic → Story/Task → Sub-task, with rolled-up hours at each level.

---

## Architecture Notes

### Project Layout
```
jira-report/
├── Cargo.toml
├── .env.example          # Documents required env vars
├── templates/
│   └── index.html        # Tera template
└── src/
    ├── main.rs           # Axum router, job store (DashMap or Mutex<HashMap>), handlers
    ├── jira.rs           # Jira REST API client (reqwest), pagination, worklog fetch
    └── report.rs         # Excel workbook generation (rust_xlsxwriter), 4-tab schema
```

### Job State Store
- `Arc<Mutex<HashMap<Uuid, JobState>>>` or `Arc<DashMap<Uuid, JobState>>`
- `JobState`: `{ status: Pending|Running|Done|Error, message: Option<String>, bytes: Option<Vec<u8>> }`
- Jobs spawned via `tokio::spawn`; state updated in-place as the task progresses.

### Key Dependencies (`Cargo.toml`)
```toml
axum = "0.7"
tokio = { version = "1", features = ["full"] }
tera = "1"
reqwest = { version = "0.12", features = ["json"] }
rust_xlsxwriter = "0.7"
tower-http = { version = "0.5", features = ["fs"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
```

---

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `JIRA_BASE_URL` | Yes | e.g. `https://myorg.atlassian.net` |
| `JIRA_EMAIL` | Yes | Jira account email for Basic Auth |
| `JIRA_API_TOKEN` | Yes | Jira API token (not account password) |
| `PORT` | No | HTTP listen port, default `3000` |

---

## Success Metrics

### Leading Indicators (days–weeks post-launch)
- **Task completion rate**: ≥ 90% of report generation attempts result in a successful download (no error state).
- **Time to download**: P90 round-trip from form submit to file download < 60 seconds for projects with ≤ 500 issues.
- **Page load time**: Project dropdown populated within 2 seconds on a normal network connection.

### Lagging Indicators (weeks–months)
- **CLI skill usage reduction**: Decrease in direct invocations of the `jira-timereport` skill as users migrate to the web app.
- **Support requests**: Zero "how do I run the time report" questions from non-technical team members after rollout.

---

## Open Questions

| # | Question | Owner | Blocking? |
|---|----------|-------|-----------|
| OQ-1 | Should the app filter projects to only those the Jira user has worklog-read permission on, or show all visible projects? | Engineering | No — default to all visible; filter can be added if needed |
| OQ-2 | What is the maximum expected issue count per project/date range? This affects whether Jira's 100-item pagination needs a progress bar or is negligible. | Product | No |
| OQ-3 | Should the Hierarchy tab match the exact column layout from the CLI skill, or is a simplified version acceptable for v1? | Product | No. Simplified version is fine for Phase 1|
| OQ-4 | Is this app intended to run on a shared internal server, or only locally? This affects whether P1-4 (job cleanup) is critical for launch. | Product | No |
| OQ-5 | Are there date format preferences for the Excel cells (ISO 8601 vs locale-formatted)? | Product | No |

---

## Timeline Considerations

- No hard external deadline identified.
- Suggested phasing:
  - **Phase 1 (v1.0)**: All P0 requirements. Functional end-to-end flow with fixed 4-tab Excel output.
  - **Phase 2 (v1.1)**: P1 items — descriptive filename, date defaults, progress messages, job cleanup, rate-limit retry.
  - **Phase 3 (v2.0)**: P2 items as demand emerges.
