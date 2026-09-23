use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::PathBuf;
use std::time::Duration;

// --- OAuth credentials ---

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuthEntry>,
}

#[derive(Deserialize)]
struct OAuthEntry {
    #[serde(rename = "accessToken")]
    access_token: String,
}

fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .expect("home directory exists")
        .join(".claude/.credentials.json")
}

fn read_access_token() -> Result<String, String> {
    let path = credentials_path();
    let data = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read credentials: {e}"))?;
    let creds: Credentials =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse credentials: {e}"))?;
    creds
        .claude_ai_oauth
        .map(|o| o.access_token)
        .ok_or_else(|| "No OAuth token found in credentials".into())
}

// --- Usage API ---

#[derive(Debug, Clone, Deserialize)]
pub struct UsageResponse {
    /// Generic, self-describing limit list. Preferred over the named windows
    /// below: the server adds/removes limit kinds (Opus, Sonnet, Fable, ...)
    /// without us needing a matching field, and each entry carries its own
    /// label material in `kind` + `scope`.
    #[serde(default, deserialize_with = "lenient_vec")]
    pub limits: Vec<Limit>,
    // Legacy named windows, kept as a fallback for when `limits` is absent.
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub seven_day_opus: Option<UsageWindow>,
    pub seven_day_sonnet: Option<UsageWindow>,
    /// Newer form of `extra_usage`, with an explicit currency exponent.
    pub spend: Option<Spend>,
    pub extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageWindow {
    pub utilization: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Limit {
    /// e.g. "session", "weekly_all", "weekly_scoped".
    pub kind: String,
    /// Percent utilized, 0-100.
    #[serde(default)]
    pub percent: f64,
    /// e.g. "normal", "warning", "critical". Only ever escalates our
    /// threshold-derived bar color, never de-escalates it.
    pub severity: Option<String>,
    pub resets_at: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub scope: Option<LimitScope>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitScope {
    #[serde(default, deserialize_with = "lenient")]
    pub model: Option<ScopeEntity>,
    #[serde(default, deserialize_with = "lenient")]
    pub surface: Option<ScopeEntity>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScopeEntity {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Spend {
    #[serde(default)]
    pub enabled: bool,
    pub used: Option<Money>,
    pub limit: Option<Money>,
    pub percent: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Money {
    /// Amount in minor units (cents for USD), scaled by `exponent`.
    pub amount_minor: f64,
    pub currency: Option<String>,
    pub exponent: Option<i32>,
}

impl Money {
    pub fn amount(&self) -> f64 {
        self.amount_minor / 10f64.powi(self.exponent.unwrap_or(2))
    }

    pub fn currency(&self) -> &str {
        self.currency.as_deref().unwrap_or("USD")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
}

/// Deserialize a value, degrading a shape mismatch to `None` instead of
/// failing the whole response. The usage endpoint is undocumented and its
/// nested shapes change without notice; a new `scope` variant should cost us
/// one label, not every bar.
fn lenient<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(de)?;
    Ok(serde_json::from_value(value).ok())
}

/// Same idea for a list: entries we can't parse are dropped individually.
fn lenient_vec<'de, D, T>(de: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let values = Vec::<serde_json::Value>::deserialize(de)?;
    Ok(values
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect())
}

/// A failed fetch, with the server's `Retry-After` hint when it sent one.
#[derive(Debug)]
pub struct FetchError {
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_after: None,
        }
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(secs) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).to_std().ok()
}

/// Send `req` and return the body as raw JSON, after checking it parses as
/// `T` so a malformed response can't replace a good cached one.
async fn get_json<T: DeserializeOwned>(
    req: reqwest::RequestBuilder,
    what: &str,
) -> Result<serde_json::Value, FetchError> {
    let resp = req
        .send()
        .await
        .map_err(|e| format!("{what} request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError {
            message: format!("{what} API returned {status}"),
            retry_after: retry_after(resp.headers()),
        });
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to read {what} response: {e}"))?;
    T::deserialize(&body).map_err(|e| format!("Failed to parse {what} response: {e}"))?;
    Ok(body)
}

pub async fn fetch_usage() -> Result<serde_json::Value, FetchError> {
    let token = read_access_token()?;
    let req = reqwest::Client::new()
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(&token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Content-Type", "application/json");
    get_json::<UsageResponse>(req, "Usage").await
}

// --- Status API ---

#[derive(Debug, Clone, Deserialize)]
pub struct StatusSummary {
    pub status: OverallStatus,
    pub components: Vec<Component>,
    pub incidents: Vec<Incident>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverallStatus {
    pub indicator: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Incident {
    pub name: String,
    #[allow(dead_code)]
    pub status: String,
    pub impact: String,
    pub incident_updates: Vec<IncidentUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncidentUpdate {
    pub body: String,
    #[allow(dead_code)]
    pub status: String,
    #[allow(dead_code)]
    pub updated_at: String,
}

pub async fn fetch_status() -> Result<serde_json::Value, FetchError> {
    let req = reqwest::Client::new().get("https://status.claude.com/api/v2/summary.json");
    get_json::<StatusSummary>(req, "Status").await
}

/// Map a component/overall status string to a severity for icon coloring.
/// Returns: 0 = operational, 1 = degraded/minor, 2 = partial/major outage, 3 = critical
pub fn status_severity(indicator: &str) -> u8 {
    match indicator {
        "none" | "operational" => 0,
        "minor" | "degraded_performance" => 1,
        "major" | "partial_outage" => 2,
        "critical" | "major_outage" => 3,
        _ => 1,
    }
}

