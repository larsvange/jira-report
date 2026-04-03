use chrono::NaiveDate;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum JiraError {
    #[error("HTTP request failed: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Jira returned {status}: {body}")]
    ApiError { status: u16, body: String },

    #[error("Rate limited; retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("Failed to parse Jira response: {0}")]
    Parse(String),
}

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Serialize, Clone)]
pub struct Project {
    pub key: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectSearchResponse {
    values: Vec<Project>,
    is_last: bool,
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
    pub parent: Option<ParentRef>,
    #[serde(rename = "customfield_10014")]
    pub epic_link: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct IssueType {
    pub name: String,
    pub subtask: bool,
}

#[derive(Deserialize, Clone)]
pub struct ParentRef {
    pub key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueSearchResponse {
    issues: Vec<Issue>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorklogResponse {
    worklogs: Vec<JiraWorklogRaw>,
    total: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JiraWorklogRaw {
    author: WorklogAuthor,
    started: String,
    time_spent_seconds: u64,
    comment: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorklogAuthor {
    display_name: String,
}

// ---------------------------------------------------------------------------
// Internal worklog type (populated during job execution)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Worklog {
    pub issue_key: String,
    pub issue_summary: String,
    pub author: String,
    pub date: NaiveDate,
    pub hours: f64,
    pub comment: String,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct JiraClient {
    client: Client,
    base_url: String,
    email: String,
    api_token: String,
}

impl JiraClient {
    pub fn new(base_url: String, email: String, api_token: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            email,
            api_token,
        }
    }

    /// Execute a GET request with Basic Auth and retry on 429.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
    ) -> Result<T, JiraError> {
        let max_retries = 3u32;
        for attempt in 0..=max_retries {
            let res = self
                .client
                .get(url)
                .basic_auth(&self.email, Some(&self.api_token))
                .send()
                .await?;

            if res.status() == 429 {
                let retry_after: u64 = res
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(5);
                if attempt < max_retries {
                    warn!("Rate limited by Jira; waiting {retry_after}s (attempt {attempt})");
                    tokio::time::sleep(tokio::time::Duration::from_secs(retry_after)).await;
                    continue;
                } else {
                    return Err(JiraError::RateLimited { retry_after_secs: retry_after });
                }
            }

            if !res.status().is_success() {
                let status = res.status().as_u16();
                let body = res.text().await.unwrap_or_default();
                return Err(JiraError::ApiError { status, body });
            }

            return res.json::<T>().await.map_err(|e| JiraError::Parse(e.to_string()));
        }
        unreachable!()
    }

    /// Fetch all visible Jira projects (paginated).
    pub async fn fetch_projects(&self) -> Result<Vec<Project>, JiraError> {
        let mut all = Vec::new();
        let mut start = 0u32;
        let page_size = 50u32;
        loop {
            let url = format!(
                "{}/rest/api/3/project/search?startAt={start}&maxResults={page_size}",
                self.base_url
            );
            let page: ProjectSearchResponse = self.get_json(&url).await?;
            all.extend(page.values);
            if page.is_last {
                break;
            }
            start += page_size;
        }
        Ok(all)
    }

    /// Search for issues in a project with worklogs in the given date range.
    pub async fn search_issues(
        &self,
        project_key: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<Issue>, JiraError> {
        let jql = format!(
            "project = \"{project_key}\" AND worklogDate >= \"{start}\" AND worklogDate <= \"{end}\" ORDER BY key ASC"
        );
        let fields = "summary,issuetype,parent,customfield_10014";
        let page_size = 100u32;
        let mut all = Vec::new();
        let mut next_page_token: Option<String> = None;
        loop {
            let mut url = format!(
                "{}/rest/api/3/search/jql?jql={}&maxResults={page_size}&fields={fields}",
                self.base_url,
                urlencoding::encode(&jql),
            );
            if let Some(token) = &next_page_token {
                url.push_str(&format!("&nextPageToken={token}"));
            }
            let page: IssueSearchResponse = self.get_json(&url).await?;
            let fetched = page.issues.len();
            all.extend(page.issues);
            next_page_token = page.next_page_token;
            if fetched == 0 || next_page_token.is_none() {
                break;
            }
        }
        Ok(all)
    }

    /// Fetch worklogs for a single issue, filtered to the date range.
    pub async fn fetch_worklogs(
        &self,
        issue_key: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<Worklog>, JiraError> {
        let started_after  = start.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis();
        let started_before = end.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp_millis();

        let mut all_raw: Vec<JiraWorklogRaw> = Vec::new();
        let mut start_at = 0u32;
        let page_size = 1000u32;
        loop {
            let url = format!(
                "{}/rest/api/3/issue/{issue_key}/worklog?startedAfter={started_after}&startedBefore={started_before}&startAt={start_at}&maxResults={page_size}",
                self.base_url,
            );
            let page: WorklogResponse = self.get_json(&url).await?;
            let fetched = page.worklogs.len() as u32;
            all_raw.extend(page.worklogs);
            if start_at + fetched >= page.total || fetched == 0 {
                break;
            }
            start_at += page_size;
        }

        let worklogs = all_raw
            .into_iter()
            .filter_map(|raw| {
                // started is ISO 8601: "2026-03-15T09:00:00.000+0000"
                let date = NaiveDate::parse_from_str(&raw.started[..10], "%Y-%m-%d").ok()?;
                if date < start || date > end {
                    return None;
                }
                let comment = extract_adf_text(raw.comment.as_ref());
                Some(Worklog {
                    issue_key: String::new(),   // filled by caller
                    issue_summary: String::new(),
                    author: raw.author.display_name,
                    date,
                    hours: raw.time_spent_seconds as f64 / 3600.0,
                    comment,
                })
            })
            .collect();

        Ok(worklogs)
    }
}

/// Extract plain text from an Atlassian Document Format (ADF) comment node.
fn extract_adf_text(value: Option<&serde_json::Value>) -> String {
    let Some(v) = value else { return String::new() };
    let mut out = String::new();
    collect_text(v, &mut out);
    out.trim().to_string()
}

fn collect_text(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::Object(map) => {
            if map.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = map.get("text").and_then(|t| t.as_str()) {
                    out.push_str(text);
                }
            }
            if let Some(content) = map.get("content") {
                collect_text(content, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_text(item, out);
            }
        }
        _ => {}
    }
}
