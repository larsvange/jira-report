# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Run the app (requires .env — copy from .env.example and fill in credentials)
RUST_LOG=info cargo run

# Type-check without producing a binary (fast feedback loop)
cargo check

# Build release binary
cargo build --release

# Run clippy lints
cargo clippy -- -D warnings

# Format code
cargo fmt

# Check for known dependency vulnerabilities
cargo audit

# Run all tests
cargo test

# Run a single test
cargo test test_name_here
```

After modifying code, run all tests to ensure nothing broke:
```bash
cargo test
```

## Environment

The app reads `.env` from the working directory at startup (via `dotenvy`). Required variables:

| Variable | Description |
|---|---|
| `JIRA_BASE_URL` | e.g. `https://yourorg.atlassian.net` (no trailing slash) |
| `JIRA_EMAIL` | Jira account email |
| `JIRA_API_TOKEN` | API token from https://id.atlassian.com/manage-profile/security/api-tokens |

Optional: `PORT` (default `3000`). Missing required vars cause an immediate panic with the variable name.

## Architecture

Three source files and one template:

**`src/main.rs`** — Axum router, shared state, all HTTP handlers, and the `run_job` orchestrator.
- `AppState`: holds `JiraClient`, `Tera` (template engine), and `JobStore` (`Arc<DashMap<Uuid, JobState>>`).
- `JobState`: in-memory record with `status` (Pending → Running → Done/Error), an optional progress `message`, and the completed `.xlsx` `bytes`.
- `POST /generate` returns a UUID immediately, then `tokio::spawn`s `run_job` which drives the full Jira fetch → Excel pipeline and updates `JobState` in place.
- A background cleanup task (`tokio::spawn` at startup) evicts jobs older than 1 hour every 5 minutes.

**`src/jira.rs`** — `JiraClient` wrapping `reqwest`. Public methods paginate automatically and retry on HTTP 429 (up to 3 times, honouring `Retry-After`):
- `fetch_projects()` — `GET /rest/api/3/project/search`
- `search_issues(project, start, end)` — JQL search; requests `summary`, `issuetype`
- `fetch_worklogs(issue_key, start, end)` — `GET /rest/api/3/issue/{key}/worklog`; Jira v3 returns comments as ADF (Atlassian Document Format JSON), converted to plain text by `extract_adf_text`.

**`src/report.rs`** — Pure function `generate_workbook(&[Worklog]) -> Result<Vec<u8>>` using `rust_xlsxwriter`. Produces a 3-tab workbook in memory:
1. **Worklogs** — one row per raw worklog entry
2. **Summary by Person** — `BTreeMap` aggregation, alphabetical
3. **Summary by Issue** — `BTreeMap` aggregation, sorted by issue key

**`templates/index.html`** — Single Tera template. Receives `projects`, `default_start`, `default_end` from the index handler. Contains all polling JS inline: submits the form via `fetch()`, polls `/status/:id` every 2 seconds, and renders a download link on completion.

## Key design decisions

- **`DashMap` over `Mutex<HashMap>`**: `/status/:id` is polled frequently; sharded locking avoids contention between concurrent jobs.
- **Sequential worklog fetching** in `run_job`: issues are iterated one-by-one to avoid Jira rate limits. Progress messages (`"Fetching worklogs for issue N/total…"`) are written to `JobState.message` at each step.
- **Worklog `issue_key`/`issue_summary` are set by the caller** (`run_job`), not inside `fetch_worklogs`, since the worklog API response doesn't include issue metadata.
- **`Tera` is loaded once at startup** from `templates/**/*` and is read-only thereafter — safe to share across handlers without locking.

## Development Best Practices

- Use comments sparingly. Only comment complex code.
- Ensure .env is never added or tacked by git
