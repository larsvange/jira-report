# Architecture — Jira Time Report Web App

> Reference: [PRD.md](./PRD.md) | [TASKS.md](./TASKS.md)

---

## 1. Tech Stack & Version Pinning

| Crate | Version | Rationale |
|-------|---------|-----------|
| `axum` | `0.8` | Latest stable (0.8.8). Uses native async traits (no `#[async_trait]` macro). Includes `State` extractor for shared app state. |
| `tokio` | `1` | Async runtime. `features = ["full"]` enables `rt-multi-thread`, `macros`, `time` (needed for sleep in retry). |
| `reqwest` | `0.13` | Latest stable (0.13.2). HTTP client with native `json()` deserialization. |
| `tera` | `1` | Jinja2-style template engine. Stable at 1.x; simple API for the single page. |
| `rust_xlsxwriter` | `0.94` | Latest (0.94.0). Pure Rust, no C deps. `save_to_buffer()` returns `Vec<u8>` for in-memory workbook serialization. |
| `tower-http` | `0.6` | Middleware for Axum. `features = ["fs"]` for potential static file serving. Aligned with axum 0.8. |
| `dashmap` | `6` | Lock-free concurrent HashMap. Avoids `Mutex` contention on the job store for concurrent status polls. |
| `serde` / `serde_json` | `1` | De/serialization. Ubiquitous, stable. |
| `uuid` | `1` | `features = ["v4"]` for random job IDs. |
| `chrono` | `0.4` | Date parsing and arithmetic. `features = ["serde"]` for deserializing date strings. |
| `dotenvy` | `0.15` | Loads `.env` file in development. Maintained fork of `dotenv`. |
| `thiserror` | `2` | Derive macro for custom error types. |
| `tracing` / `tracing-subscriber` | `0.1` / `0.3` | Structured logging. `env-filter` feature for `RUST_LOG` control. |

**Rust edition:** 2021 (or 2024 if stable by implementation time).
**MSRV:** 1.77+ (required for axum 0.8 async trait support).

---

## 2. System Architecture

### Request Flow

```
Browser                          Axum Server                     Jira Cloud API
  │                                  │                                │
  │  GET /                           │                                │
  │─────────────────────────────────>│                                │
  │                                  │  GET /rest/api/3/project/search│
  │                                  │───────────────────────────────>│
  │                                  │<───────────────────────────────│
  │  HTML (project dropdown filled)  │                                │
  │<─────────────────────────────────│                                │
  │                                  │                                │
  │  POST /generate                  │                                │
  │  {project_key, start, end}       │                                │
  │─────────────────────────────────>│                                │
  │                                  │── create JobState (Pending)    │
  │                                  │── tokio::spawn(async task) ──> │
  │  { "job_id": "uuid" }           │                                │
  │<─────────────────────────────────│                                │
  │                                  │                                │
  │  GET /status/:id  (poll loop)   │     [async task running]       │
  │─────────────────────────────────>│                                │
  │  { "status":"running",           │  GET /rest/api/3/search        │
  │    "message":"Fetching..."}      │───────────────────────────────>│
  │<─────────────────────────────────│<───────────────────────────────│
  │         ...repeat...             │                                │
  │                                  │  GET /rest/api/3/issue/X/worklog
  │                                  │───────────────────────────────>│
  │                                  │<───────────────────────────────│
  │                                  │── generate Excel in memory     │
  │                                  │── store bytes in JobState      │
  │                                  │── set status = Done            │
  │  GET /status/:id                 │                                │
  │─────────────────────────────────>│                                │
  │  { "status": "done" }           │                                │
  │<─────────────────────────────────│                                │
  │                                  │                                │
  │  GET /download/:id               │                                │
  │─────────────────────────────────>│                                │
  │  .xlsx bytes                     │                                │
  │  (Content-Disposition: attach.)  │                                │
  │<─────────────────────────────────│                                │
```

### Component Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                        Axum Server                          │
│                                                             │
│  ┌───────────┐   ┌────────────┐   ┌──────────────────────┐ │
│  │  Handlers │   │  AppState  │   │   Background Tasks   │ │
│  │           │   │            │   │                      │ │
│  │ GET /     │──>│ JiraClient │   │ tokio::spawn'd jobs  │ │
│  │ POST /gen │   │ Tera       │   │ - fetch issues       │ │
│  │ GET /stat │──>│ JobStore   │<──│ - fetch worklogs     │ │
│  │ GET /dl   │   │ (DashMap)  │   │ - build Excel        │ │
│  └───────────┘   └────────────┘   └──────────────────────┘ │
│                                                             │
└─────────────────────────────────────────────────────────────┘
         │                                    │
         │ HTTP (port 3000)                   │ HTTPS (Basic Auth)
         ▼                                    ▼
    ┌─────────┐                      ┌──────────────┐
    │ Browser │                      │  Jira Cloud  │
    └─────────┘                      └──────────────┘
```

---

## 3. Data Models

### JobState (in-memory store)

```rust
use chrono::NaiveDate;
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone)]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Error,
}

#[derive(Clone)]
pub struct JobState {
    pub status: JobStatus,
    pub message: Option<String>,       // progress or error detail
    pub bytes: Option<Vec<u8>>,        // completed .xlsx
    pub project_key: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub created_at: Instant,           // for TTL cleanup (P1-4)
}

// The job store:
pub type JobStore = Arc<DashMap<Uuid, JobState>>;
```

### Jira API Response Types

```rust
// GET /rest/api/3/project/search
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSearchResponse {
    pub values: Vec<Project>,
    pub is_last: bool,
    pub start_at: u32,
    pub max_results: u32,
    pub total: u32,
}

#[derive(Deserialize, Clone)]
pub struct Project {
    pub key: String,
    pub name: String,
}

// GET /rest/api/3/search (JQL issue search)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSearchResponse {
    pub issues: Vec<Issue>,
    pub start_at: u32,
    pub max_results: u32,
    pub total: u32,
}

#[derive(Deserialize, Clone)]
pub struct Issue {
    pub key: String,
    pub fields: IssueFields,
}

#[derive(Deserialize, Clone)]
pub struct IssueFields {
    pub summary: String,
    pub issuetype: IssueType,
    pub parent: Option<ParentRef>,     // links sub-task -> parent
    // Epic link field (customfield_10014 on most Jira instances)
    #[serde(rename = "customfield_10014")]
    pub epic_link: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct IssueType {
    pub name: String,                  // "Epic", "Story", "Task", "Sub-task"
    pub subtask: bool,
}

#[derive(Deserialize, Clone)]
pub struct ParentRef {
    pub key: String,
}

// GET /rest/api/3/issue/{key}/worklog
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorklogResponse {
    pub worklogs: Vec<JiraWorklog>,
    pub start_at: u32,
    pub max_results: u32,
    pub total: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraWorklog {
    pub author: WorklogAuthor,
    pub started: String,               // ISO 8601 datetime
    pub time_spent_seconds: u64,
    pub comment: Option<WorklogComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorklogAuthor {
    pub display_name: String,
}

// Jira v3 uses ADF (Atlassian Document Format) for comments
#[derive(Deserialize)]
pub struct WorklogComment {
    pub content: Option<Vec<serde_json::Value>>,  // ADF nodes
}
```

### Internal Types (flattened for report generation)

```rust
pub struct Worklog {
    pub issue_key: String,
    pub issue_summary: String,
    pub author: String,
    pub date: NaiveDate,
    pub hours: f64,                    // time_spent_seconds / 3600.0
    pub comment: String,               // extracted plain text or ""
}

// For hierarchy tab
pub struct IssueNode {
    pub key: String,
    pub summary: String,
    pub issue_type: String,
    pub parent_key: Option<String>,    // from parent field
    pub epic_key: Option<String>,      // from epic_link or parent if type is Epic
    pub total_hours: f64,
}
```

---

## 4. Module Responsibilities

### `src/main.rs`

**Owns:** Application bootstrap, routing, request handlers, shared state.

| Responsibility | Detail |
|---------------|--------|
| Env validation | Read and validate `JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`, `PORT` on startup. Fail fast with named error. |
| State construction | Build `AppState { jira_client, tera, job_store }` wrapped in `Arc`. |
| Router definition | `GET /` -> `index_handler`, `POST /generate` -> `generate_handler`, `GET /status/:id` -> `status_handler`, `GET /download/:id` -> `download_handler`. |
| `index_handler` | Calls `jira_client.fetch_projects()`, renders `templates/index.html` with project list (+ date defaults in P1). |
| `generate_handler` | Parses form, validates dates, creates `JobState::Pending`, inserts into `DashMap`, spawns async task, returns JSON `{ job_id }`. |
| `status_handler` | Looks up job by UUID path param. Returns 404 or JSON `{ status, message }`. |
| `download_handler` | Looks up job. If `Done`, clones bytes and returns with xlsx content-type + Content-Disposition. Else 404. |
| Async task | Orchestrates: update to Running -> call `jira.search_issues()` -> for each issue call `jira.fetch_worklogs()` -> call `report::generate_workbook()` -> store bytes -> update to Done. Catch errors -> update to Error. |

### `src/jira.rs`

**Owns:** All communication with the Jira REST API.

| Responsibility | Detail |
|---------------|--------|
| `JiraClient` struct | Holds `reqwest::Client` (reused connection pool), `base_url: String`, `email: String`, `api_token: String`. |
| `fetch_projects()` | `GET {base}/rest/api/3/project/search?startAt=N&maxResults=50`. Paginates until `isLast == true`. Returns `Vec<Project>`. |
| `search_issues()` | `GET {base}/rest/api/3/search?jql=...&startAt=N&maxResults=100&fields=summary,issuetype,parent,customfield_10014`. Paginates until `startAt + issues.len() >= total`. Returns `Vec<Issue>`. |
| `fetch_worklogs()` | `GET {base}/rest/api/3/issue/{key}/worklog?startedAfter={ms}&startedBefore={ms}`. Paginates if `total > maxResults`. Filters entries to date range. Returns `Vec<JiraWorklog>`. |
| Auth | Every request sets `Authorization: Basic base64(email:token)`. |
| Error mapping | Maps reqwest errors and non-2xx to `JiraError` variants (Unauthorized, Forbidden, RateLimited, NotFound, Other). |

### `src/report.rs`

**Owns:** Transforming worklog/issue data into an Excel workbook.

| Responsibility | Detail |
|---------------|--------|
| `generate_workbook()` | Entry point. Accepts `&[Worklog]` and `&[IssueNode]`. Returns `Result<Vec<u8>>`. |
| Tab 1 — Worklogs | Writes raw rows. Applies date format, number format for hours. |
| Tab 2 — Summary by Person | Groups by author, sums hours. Sorted alphabetically. |
| Tab 3 — Summary by Issue | Groups by issue key, sums hours. Sorted by key. |
| Tab 4 — Hierarchy | Builds a tree: Epic -> Story/Task -> Sub-task. Writes grouped rows with indentation and rolled-up totals at each level. Handles orphan issues (no epic). |
| Formatting | Bold headers, auto-fit column widths, number format `#,##0.00` for hours, date format `yyyy-mm-dd`. |

---

## 5. API Contract Summary

| Endpoint | Method | Request | Success Response | Error Response |
|----------|--------|---------|-----------------|----------------|
| `/` | GET | — | 200 HTML (rendered template with project dropdown) | 500 if Jira unreachable |
| `/generate` | POST | Form: `project_key`, `start_date`, `end_date` | 200 `{ "job_id": "<uuid>" }` | 400 `{ "error": "..." }` |
| `/status/:id` | GET | Path: UUID | 200 `{ "status": "pending\|running\|done\|error", "message": "..." }` | 404 unknown ID |
| `/download/:id` | GET | Path: UUID | 200 `.xlsx` bytes + headers | 404 if not found or not done |

See [PRD.md](./PRD.md) sections "API Contract" and "Excel Workbook Schema" for full field-level detail.

---

## 6. Error Handling Strategy

### Error Type Hierarchy

```rust
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Jira API error: {0}")]
    Jira(#[from] JiraError),

    #[error("Report generation failed: {0}")]
    Report(String),

    #[error("Template rendering failed: {0}")]
    Template(#[from] tera::Error),

    #[error("Invalid input: {0}")]
    Validation(String),
}

#[derive(thiserror::Error, Debug)]
pub enum JiraError {
    #[error("HTTP request failed: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Jira returned {status}: {body}")]
    ApiError { status: u16, body: String },

    #[error("Rate limited (429); retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("Failed to parse Jira response: {0}")]
    Parse(String),
}
```

### Error Propagation

| Layer | Strategy |
|-------|----------|
| **Handlers** (sync path) | Return `axum::response::Result`. Implement `IntoResponse` for `AppError` to produce 400/500 JSON responses. |
| **Spawned task** (async) | Catch all errors at the top of the task closure. On any error, update `JobState` to `Error` with the error's `Display` string. Never panic inside a spawned task. |
| **Jira client** | Return `Result<T, JiraError>`. Caller decides whether to retry or propagate. |
| **Report generation** | Return `Result<Vec<u8>, AppError>`. Map `rust_xlsxwriter` errors to `AppError::Report`. |

### User-Facing Errors

- Validation errors (bad dates, missing fields) -> 400 JSON on the `POST /generate` response itself.
- Runtime errors (Jira down, auth failure) -> job reaches `Error` state; `/status/:id` returns `{ "status": "error", "message": "..." }`; UI displays the message in red.
- Unknown job ID -> 404.

---

## 7. Concurrency Model

### Tokio Runtime

The app uses `#[tokio::main]` with the multi-threaded runtime (`features = ["full"]`). This is appropriate because:
- Jira API calls are I/O-bound and benefit from concurrent execution.
- Multiple report jobs can run simultaneously.
- The DashMap job store is designed for concurrent reads/writes without a global lock.

### Shared State

```
Arc<AppState>
  ├── jira_client: JiraClient          // immutable after init; reqwest::Client is Clone + thread-safe
  ├── tera: Tera                       // immutable after init; read-only template rendering
  └── job_store: Arc<DashMap<Uuid, JobState>>  // concurrent read/write
```

**DashMap access patterns:**
- `POST /generate`: one `insert()` per request (write).
- `GET /status/:id`: one `get()` per poll (read). High frequency but read-only.
- `GET /download/:id`: one `get()` to read bytes (read). Bytes are `clone()`d out to avoid holding the lock during streaming.
- Spawned task: multiple `get_mut()` calls to update status/message/bytes (write). Only one task writes to a given key.

**Why DashMap over Mutex:** DashMap uses sharded internal locking. Each job ID hashes to a different shard, so concurrent jobs rarely contend.

### Task Lifecycle

```
generate_handler()
  │
  ├── Insert JobState { status: Pending } into DashMap
  │
  └── tokio::spawn(async move {
        // Owns: job_id, project_key, dates, Arc<AppState>
        // Step 1: Update status to Running
        // Step 2: Fetch issues (paginated, sequential)
        // Step 3: Fetch worklogs per issue (sequential to avoid rate limits)
        // Step 4: Generate Excel in memory
        // Step 5: Store bytes, set status to Done
        // On any error: set status to Error with message
      })
```

**Why sequential worklog fetching:** Jira Cloud enforces rate limits. Fetching worklogs for many issues concurrently risks 429s. Sequential fetching with progress updates is simpler and more reliable. In Phase 2 (P1-5), retry-with-backoff handles any 429s that still occur.

### Cleanup Task (P1-4)

A single `tokio::spawn` at startup runs an infinite loop:
```rust
loop {
    tokio::time::sleep(Duration::from_secs(300)).await;  // every 5 min
    job_store.retain(|_, v| v.created_at.elapsed() < Duration::from_secs(3600));
}
```

---

## 8. Security Considerations

### Credentials

- Jira credentials are **never** exposed to the browser. They exist only in server-side env vars and in the `JiraClient` struct's memory.
- `.env` is in `.gitignore`. `.env.example` contains placeholder values only.
- Basic Auth header is constructed at runtime: `base64("{email}:{token}")`.

### Network Posture

- This app is designed for **internal network use only** (per PRD non-goal #1). There is no user authentication or authorization layer.
- If exposed to the internet, add a reverse proxy with authentication (e.g., OAuth2 proxy, Cloudflare Access, or Tailscale).
- The app listens on `0.0.0.0:{PORT}` — bind to `127.0.0.1` if only local access is needed.

### Input Validation

- `project_key`: validated against the list of fetched projects (prevent JQL injection).
- `start_date` / `end_date`: parsed as `NaiveDate` (strict ISO 8601). Invalid formats return 400.
- `end_date >= start_date`: enforced.
- Job IDs: parsed as `Uuid`; malformed IDs return 404 (Axum path extractor handles this).

### Denial of Service

- No authentication means anyone on the network can submit jobs. Mitigations:
  - Job store is in-memory with TTL cleanup; old jobs are evicted.
  - Consider adding a max-concurrent-jobs limit (future).
  - Jira API rate limits act as a natural throttle.

### Dependencies

- All dependencies are from crates.io with active maintenance.
- `cargo audit` should be run in CI to check for known vulnerabilities.

---

## 9. Future Extensibility

| Area | Current Design | Extension Path |
|------|---------------|----------------|
| **Storage** | In-memory `DashMap` | Swap for a trait-based `JobStore`. Implement with SQLite or Redis for persistence (P2-1). |
| **Multi-project** | Single project per report | Accept `Vec<String>` for project keys. JQL supports `project in (A, B)` (P2-2). |
| **Auth** | None (env-var Jira creds) | Add `tower` middleware for OAuth2/OIDC. Per-user Jira tokens stored in session (P2-3). |
| **Custom columns** | Fixed 4-tab schema | Move column definitions to a config file or admin UI (P2-4). |
| **Notifications** | Browser polling | Add SSE via `axum::response::Sse` or WebSocket upgrade on `/ws/:id` (P2-5). |
| **Output formats** | .xlsx only | `report.rs` produces `Vec<u8>`. Add a `format` parameter and parallel CSV/PDF generators behind a trait. |
| **Testing** | Manual | Use `wiremock` for HTTP-level Jira mocking. `report.rs` is pure data-in/bytes-out — unit-testable with fixture data. |

### Recommended Testing Approach (when adding tests)

```
tests/
├── jira_mock.rs       # wiremock server returning fixture JSON
├── report_test.rs     # unit tests: known worklogs -> validate xlsx bytes
└── integration.rs     # spawn server, POST /generate, poll, download, validate
```

---

## 10. Project Layout (Final)

```
jira-report/
├── Cargo.toml
├── Cargo.lock
├── .env.example
├── .gitignore
├── PRD.md
├── TASKS.md
├── ARCHITECTURE.md
├── templates/
│   └── index.html         # Tera template: form + JS polling logic
└── src/
    ├── main.rs            # Router, handlers, AppState, spawned tasks
    ├── jira.rs            # JiraClient, API types, pagination, auth
    └── report.rs          # generate_workbook(), 4-tab Excel schema
```
