mod jira;
mod report;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use chrono::NaiveDate;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tera::Tera;
use tokio::net::TcpListener;
use tracing::{error, info};
use uuid::Uuid;

use jira::JiraClient;

// ---------------------------------------------------------------------------
// Job state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Error,
}

#[derive(Clone)]
pub struct JobState {
    pub status: JobStatus,
    pub message: Option<String>,
    pub bytes: Option<Vec<u8>>,
    pub project_key: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub created_at: std::time::Instant,
}

pub type JobStore = Arc<DashMap<Uuid, JobState>>;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub jira: JiraClient,
    pub tera: Tera,
    pub jobs: JobStore,
}

// ---------------------------------------------------------------------------
// Request / response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GenerateForm {
    project_key: String,
    start_date: String,
    end_date: String,
}

#[derive(Serialize)]
struct JobIdResponse {
    job_id: Uuid,
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn index_handler(State(state): State<Arc<AppState>>) -> Response {
    let projects = match state.jira.fetch_projects().await {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to fetch projects: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e.to_string() }),
            )
                .into_response();
        }
    };

    let today = chrono::Local::now().date_naive();
    let default_start = today
        .with_day(1)
        .unwrap_or(today)
        .format("%Y-%m-%d")
        .to_string();
    let last_day = last_day_of_month(today);
    let default_end = last_day.format("%Y-%m-%d").to_string();

    let mut ctx = tera::Context::new();
    ctx.insert("projects", &projects);
    ctx.insert("default_start", &default_start);
    ctx.insert("default_end", &default_end);

    match state.tera.render("index.html", &ctx) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            error!("Template error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

async fn generate_handler(
    State(state): State<Arc<AppState>>,
    Form(form): Form<GenerateForm>,
) -> Response {
    let start = match NaiveDate::parse_from_str(&form.start_date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: "Invalid start_date format (expected YYYY-MM-DD)".into() }),
            )
                .into_response()
        }
    };
    let end = match NaiveDate::parse_from_str(&form.end_date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: "Invalid end_date format (expected YYYY-MM-DD)".into() }),
            )
                .into_response()
        }
    };
    if end < start {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "end_date must be after start_date".into() }),
        )
            .into_response();
    }

    let job_id = Uuid::new_v4();
    let job = JobState {
        status: JobStatus::Pending,
        message: None,
        bytes: None,
        project_key: form.project_key.clone(),
        start_date: start,
        end_date: end,
        created_at: std::time::Instant::now(),
    };
    state.jobs.insert(job_id, job);

    let state2 = Arc::clone(&state);
    let project_key = form.project_key.clone();
    tokio::spawn(async move {
        run_job(state2, job_id, project_key, start, end).await;
    });

    info!("Job {job_id} queued for project {}", form.project_key);
    (StatusCode::OK, Json(JobIdResponse { job_id })).into_response()
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.jobs.get(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(job) => {
            let (status, message) = match job.status {
                JobStatus::Pending => ("pending", job.message.clone()),
                JobStatus::Running => ("running", job.message.clone()),
                JobStatus::Done    => ("done", None),
                JobStatus::Error   => ("error", job.message.clone()),
            };
            Json(StatusResponse { status, message }).into_response()
        }
    }
}

async fn download_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Response {
    let entry = match state.jobs.get(&id) {
        None => return StatusCode::NOT_FOUND.into_response(),
        Some(e) => e,
    };
    if entry.status != JobStatus::Done {
        return StatusCode::NOT_FOUND.into_response();
    }
    let bytes = match &entry.bytes {
        Some(b) => b.clone(),
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let filename = format!(
        "{}_{}_{}",
        entry.project_key,
        entry.start_date.format("%Y-%m-%d"),
        entry.end_date.format("%Y-%m-%d")
    );
    drop(entry);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{filename}.xlsx\"")),
        ],
        bytes,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Background job
// ---------------------------------------------------------------------------

async fn run_job(
    state: Arc<AppState>,
    job_id: Uuid,
    project_key: String,
    start: NaiveDate,
    end: NaiveDate,
) {
    set_status(&state.jobs, job_id, JobStatus::Running, Some("Fetching issues…".into()));

    let issues = match state.jira.search_issues(&project_key, start, end).await {
        Ok(v) => v,
        Err(e) => {
            set_status(&state.jobs, job_id, JobStatus::Error, Some(e.to_string()));
            return;
        }
    };

    let total = issues.len();
    let mut all_worklogs = Vec::new();
    let mut issue_nodes = Vec::new();

    for (i, issue) in issues.iter().enumerate() {
        set_status(
            &state.jobs,
            job_id,
            JobStatus::Running,
            Some(format!("Fetching worklogs for issue {}/{total}…", i + 1)),
        );

        match state.jira.fetch_worklogs(&issue.key, start, end).await {
            Ok(mut wl) => {
                for w in &mut wl {
                    w.issue_key = issue.key.clone();
                    w.issue_summary = issue.fields.summary.clone();
                }
                all_worklogs.extend(wl);
            }
            Err(e) => {
                error!("Worklog fetch failed for {}: {e}", issue.key);
            }
        }

        let epic_key = issue
            .fields
            .epic_link
            .clone()
            .or_else(|| {
                if issue.fields.issuetype.name == "Epic" {
                    Some(issue.key.clone())
                } else {
                    None
                }
            });

        issue_nodes.push(report::IssueNode {
            key: issue.key.clone(),
            summary: issue.fields.summary.clone(),
            issue_type: issue.fields.issuetype.name.clone(),
            parent_key: issue.fields.parent.as_ref().map(|p| p.key.clone()),
            epic_key,
            total_hours: 0.0,
        });
    }

    set_status(&state.jobs, job_id, JobStatus::Running, Some("Generating Excel workbook…".into()));

    match report::generate_workbook(&all_worklogs, &issue_nodes) {
        Ok(bytes) => {
            if let Some(mut job) = state.jobs.get_mut(&job_id) {
                job.status = JobStatus::Done;
                job.bytes = Some(bytes);
                job.message = None;
            }
            info!("Job {job_id} complete");
        }
        Err(e) => {
            set_status(&state.jobs, job_id, JobStatus::Error, Some(e.to_string()));
        }
    }
}

fn set_status(jobs: &JobStore, id: Uuid, status: JobStatus, message: Option<String>) {
    if let Some(mut job) = jobs.get_mut(&id) {
        job.status = status;
        job.message = message;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn last_day_of_month(date: NaiveDate) -> NaiveDate {
    let next_month = if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
    };
    next_month.unwrap().pred_opt().unwrap()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jira_report=info".parse().unwrap()),
        )
        .init();

    dotenvy::dotenv().ok();

    let base_url = std::env::var("JIRA_BASE_URL")
        .expect("JIRA_BASE_URL environment variable is required");
    let email = std::env::var("JIRA_EMAIL")
        .expect("JIRA_EMAIL environment variable is required");
    let api_token = std::env::var("JIRA_API_TOKEN")
        .expect("JIRA_API_TOKEN environment variable is required");
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse()
        .expect("PORT must be a valid port number");

    let tera = Tera::new("templates/**/*").expect("Failed to load templates");
    let jira = JiraClient::new(base_url, email, api_token);
    let jobs: JobStore = Arc::new(DashMap::new());

    // Background cleanup: remove jobs older than 1 hour
    let jobs_cleanup = Arc::clone(&jobs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
            jobs_cleanup.retain(|_, v| {
                v.created_at.elapsed() < std::time::Duration::from_secs(3600)
            });
        }
    });

    let state = Arc::new(AppState { jira, tera, jobs });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/generate", post(generate_handler))
        .route("/status/:id", get(status_handler))
        .route("/download/:id", get(download_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Listening on http://{addr}");
    let listener = TcpListener::bind(addr).await.expect("Failed to bind port");
    axum::serve(listener, app).await.expect("Server error");
}
