//! WorkBuddy monthly credits from the signed-in website billing API.
//!
//! Ported from upstream CodexBar v0.73.0 (`Resources/Plugins/workbuddy.ts`,
//! `Providers/WorkBuddy/*`). WorkBuddy is Tencent's desktop AI work
//! assistant; the website's Plans & usage page reads
//! `/billing/meter/get-user-resource-summary` with the session cookies.
//! WorkBuddy binds the session to the browser User-Agent, so requests carry
//! the installed Chrome's reduced User-Agent (and the previous major as a
//! retry). Neither cookies nor response bodies are logged or echoed in
//! errors.

mod model;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{
    Client,
    header::{ACCEPT, CONTENT_TYPE, COOKIE, ORIGIN, REFERER, USER_AGENT},
};
use serde_json::{Map, Value, json};

use crate::browser::cookies::Cookie;
use crate::core::{
    FetchContext, ManualEmptyCookiePolicy, Provider, ProviderError, ProviderFetchResult,
    ProviderId, ProviderMetadata, ProviderStateKind, SourceMode,
};
use crate::providers::{BoundedBodyError, read_bounded_response};

const BASE_URL: &str = "https://www.workbuddy.cn";
const ORIGIN_VALUE: &str = "https://www.workbuddy.cn";
const DASHBOARD_URL: &str = "https://www.workbuddy.cn/profile/plans-usage";
/// Browser cookies are read for the registrable domain, then narrowed to
/// the ones a request to `www.workbuddy.cn` would carry.
const COOKIE_DOMAIN: &str = "workbuddy.cn";
const REQUEST_HOST: &str = "www.workbuddy.cn";
const SUMMARY_PATH: &str = "/billing/meter/get-user-resource-summary";
const PAID_PATH: &str = "/billing/meter/get-user-resource-paid-packages";
const FREE_PATH: &str = "/billing/meter/get-user-resource-free-packages";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Upstream clamps the web timeout to 1-30 seconds.
const MAX_TIMEOUT_SECONDS: u64 = 30;
/// The reset lookup shares five seconds across both package listings and
/// stops when less than one second is left.
const RESET_BUDGET: Duration = Duration::from_secs(5);
const MIN_LISTING_BUDGET: Duration = Duration::from_secs(1);

/// Package codes the WorkBuddy client sends for its paid and free listings
/// (WorkBuddy 5.6.2); the listings reject requests without them.
const PAID_PACKAGE_CODES: [&str; 9] = [
    "TCACA_code_002_AkiJS3ZHF5",
    "TCACA_code_005_maRGyrHhw1",
    "TCACA_code_003_FAnt7lcmRT",
    "TCACA_code_023_4xbGhMrE6q",
    "TCACA_code_026_BaESVICNoi",
    "TCACA_code_027_0FCGVA6vSa",
    "TCACA_code_009_0XmEQc2xOf",
    "TCACA_code_038_OhvqZtiPKr",
    "TCACA_code_036_lupO5WgNdG",
];
const FREE_PACKAGE_CODES: [&str; 11] = [
    "TCACA_code_001_PqouKr6QWV",
    "TCACA_code_008_cfWoLwvjU4",
    "TCACA_code_035_ArVxJcGDsm",
    "TCACA_code_006_DbXS0lrypC",
    "TCACA_code_039_KRcQj7wUat",
    "TCACA_code_040_mi9rCYg46x",
    "TCACA_code_007_nzdH5h4Nl0",
    "TCACA_code_028_NtpWi0jzXs",
    "TCACA_code_037_WxOD3MpI2o",
    "TCACA_code_029_6wCGEWquYy",
    "TCACA_code_030_BjSt89qTvr",
];

const SESSION_EXPIRED: &str =
    "WorkBuddy session expired. Sign in at www.workbuddy.cn or paste a fresh Cookie header.";
const MISSING_SESSION: &str = "No WorkBuddy session cookies found. Sign in at www.workbuddy.cn (Chrome) or paste a Cookie header.";
const COOKIES_OFF: &str = "WorkBuddy cookies are disabled.";

/// Browser sessions to try, in order: one Cookie header per browser.
type SessionSource = fn() -> Result<Vec<String>, ProviderError>;

pub struct WorkBuddyProvider {
    metadata: ProviderMetadata,
    client: Option<Client>,
    base_url: String,
    chrome_major: fn() -> Option<u32>,
    now: fn() -> DateTime<Utc>,
    browser_sessions: SessionSource,
}

impl WorkBuddyProvider {
    pub fn new() -> Self {
        let client = crate::core::credentialed_http_client_builder()
            .cookie_store(false)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(MAX_TIMEOUT_SECONDS))
            .build()
            .ok();
        Self {
            metadata: metadata(),
            client,
            base_url: BASE_URL.to_owned(),
            chrome_major: installed_chrome_major,
            now: Utc::now,
            browser_sessions,
        }
    }

    #[cfg(test)]
    fn with_parts(
        base_url: &str,
        client: Client,
        chrome_major: fn() -> Option<u32>,
        now: fn() -> DateTime<Utc>,
        browser_sessions: SessionSource,
    ) -> Self {
        Self {
            metadata: metadata(),
            client: Some(client),
            base_url: base_url.trim_end_matches('/').to_owned(),
            chrome_major,
            now,
            browser_sessions,
        }
    }

    async fn fetch_web(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        let client = self.client.as_ref().ok_or_else(|| {
            ProviderError::Other("Could not create a secure WorkBuddy HTTP client.".into())
        })?;
        let (sessions, manual) = self.sessions(ctx)?;
        let site = Site {
            client,
            base_url: &self.base_url,
            timeout: request_timeout(ctx.web_timeout),
        };
        let agents = model::user_agents((self.chrome_major)());
        let mut rejected = false;
        for cookie in &sessions {
            let Some((data, agent)) = summary(&site, cookie, &agents).await? else {
                rejected = true;
                // A pasted header stays pinned to that account.
                if manual {
                    break;
                }
                continue;
            };
            let summary = model::parse_summary(&data)?;
            let resets_at = if summary.total > 0.0 {
                next_reset(&site, cookie, agent.as_deref(), (self.now)()).await
            } else {
                None
            };
            return Ok(model::result(&summary, resets_at));
        }
        if rejected {
            return Err(ProviderError::Other(SESSION_EXPIRED.into()));
        }
        Err(missing_session())
    }

    /// The pasted Cookie header alone, else each browser's session.
    fn sessions(&self, ctx: &FetchContext) -> Result<(Vec<String>, bool), ProviderError> {
        if let Some(header) = ctx.manual_cookie_header.as_deref() {
            let header = crate::providers::normalize_cookie_header(header)
                .filter(|header| !header.trim().is_empty())
                .ok_or_else(missing_session)?;
            return Ok((vec![header], true));
        }
        if ctx.manual_cookie_missing {
            return Err(missing_session());
        }
        Ok(((self.browser_sessions)()?, false))
    }
}

impl Default for WorkBuddyProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn metadata() -> ProviderMetadata {
    ProviderMetadata {
        id: ProviderId::WorkBuddy,
        display_name: "WorkBuddy",
        session_label: "Credits",
        weekly_label: "Plan",
        supports_opus: false,
        supports_credits: false,
        default_enabled: false,
        is_primary: false,
        dashboard_url: Some(DASHBOARD_URL),
        status_page_url: None,
        tertiary_label_key: None,
    }
}

#[async_trait]
impl Provider for WorkBuddyProvider {
    fn id(&self) -> ProviderId {
        ProviderId::WorkBuddy
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::Web => self.fetch_web(ctx).await,
            // The shell maps an Off cookie source to Cli for web-only
            // providers; upstream reports that cookies are disabled.
            SourceMode::Cli => Err(ProviderError::Other(COOKIES_OFF.into())),
            SourceMode::OAuth => Err(ProviderError::UnsupportedSource(SourceMode::OAuth)),
        }
    }

    fn available_sources(&self) -> Vec<SourceMode> {
        vec![SourceMode::Auto, SourceMode::Web]
    }

    fn supports_web(&self) -> bool {
        true
    }

    /// Automatic mode tries each browser's session in turn, so the shell
    /// must not pre-resolve a single merged header.
    fn owns_browser_cookie_resolution(&self) -> bool {
        true
    }

    /// An empty Manual source fails closed with the sign-in message instead
    /// of reporting that cookies are disabled.
    fn manual_empty_cookie_policy(&self) -> ManualEmptyCookiePolicy {
        ManualEmptyCookiePolicy::FailClosedWeb
    }

    fn error_state_kind(&self, error: &ProviderError) -> ProviderStateKind {
        match error {
            ProviderError::Other(message) if message == SESSION_EXPIRED => {
                ProviderStateKind::ExpiredSession
            }
            _ => error.state_kind(),
        }
    }
}

fn missing_session() -> ProviderError {
    ProviderError::BrowserSignInRequired {
        message: MISSING_SESSION.into(),
        sign_in_url: DASHBOARD_URL.into(),
    }
}

/// Upstream clamps `webTimeoutSeconds` to 1-30.
fn request_timeout(web_timeout: u64) -> Duration {
    Duration::from_secs(web_timeout.clamp(1, MAX_TIMEOUT_SECONDS))
}

/// The timeout for one package listing, or `None` when less than a second
/// of the shared budget remains.
fn listing_timeout(timeout: Duration, remaining: Duration) -> Option<Duration> {
    (remaining >= MIN_LISTING_BUDGET).then(|| timeout.min(remaining))
}

/// Upstream session loop body: each User-Agent in turn; only HTTP 401 moves
/// on to the next one. `None` means every attempt was rejected.
async fn summary(
    site: &Site<'_>,
    cookie: &str,
    agents: &[Option<String>],
) -> Result<Option<(Map<String, Value>, Option<String>)>, ProviderError> {
    for agent in agents {
        match site
            .post(
                cookie,
                agent.as_deref(),
                SUMMARY_PATH,
                &json!({}),
                site.timeout,
            )
            .await
        {
            Ok(data) => return Ok(Some((data, agent.clone()))),
            Err(Failure::Rejected) => {}
            Err(Failure::Error(error)) => return Err(error),
        }
    }
    Ok(None)
}

/// Upstream `nextReset`: the earliest future cycle end across the paid and
/// free listings. A failed listing never discards the balance.
async fn next_reset(
    site: &Site<'_>,
    cookie: &str,
    agent: Option<&str>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let deadline = Instant::now() + RESET_BUDGET;
    let listings: [(&str, &[&str]); 2] = [
        (PAID_PATH, &PAID_PACKAGE_CODES),
        (FREE_PATH, &FREE_PACKAGE_CODES),
    ];
    let mut earliest: Option<DateTime<Utc>> = None;
    for (path, codes) in listings {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(timeout) = listing_timeout(site.timeout, remaining) else {
            break;
        };
        // Status 0 is valid, 3 used up.
        let body = json!({
            "PageNumber": 1,
            "PageSize": 100,
            "PackageCodes": codes,
            "Status": [0, 3],
        });
        match site.post(cookie, agent, path, &body, timeout).await {
            Ok(page) => {
                if let Some(end) = model::earliest_reset(&page, now) {
                    earliest = Some(earliest.map_or(end, |current| current.min(end)));
                }
            }
            Err(_) => {
                tracing::debug!(path, "WorkBuddy package listing failed; reset time omitted");
            }
        }
    }
    earliest
}

enum Failure {
    /// HTTP 401: this session and User-Agent pair was rejected.
    Rejected,
    Error(ProviderError),
}

impl From<ProviderError> for Failure {
    fn from(error: ProviderError) -> Self {
        Self::Error(error)
    }
}

struct Site<'a> {
    client: &'a Client,
    base_url: &'a str,
    timeout: Duration,
}

impl Site<'_> {
    async fn post(
        &self,
        cookie: &str,
        agent: Option<&str>,
        path: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<Map<String, Value>, Failure> {
        let body = serde_json::to_vec(body)
            .map_err(|_| ProviderError::Other("Could not encode the WorkBuddy request.".into()))?;
        let mut request = self
            .client
            .post(format!("{}{path}", self.base_url))
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .header(ORIGIN, ORIGIN_VALUE)
            .header(REFERER, DASHBOARD_URL)
            .header(COOKIE, cookie)
            .timeout(timeout)
            .body(body);
        if let Some(agent) = agent {
            request = request.header(USER_AGENT, agent);
        }
        let response = request.send().await.map_err(ProviderError::from)?;
        let status = response.status().as_u16();
        match status {
            200 => {}
            401 => return Err(Failure::Rejected),
            403 => return Err(other("WorkBuddy denied access to billing data.")),
            429 => return Err(other("WorkBuddy billing requests are rate limited.")),
            _ => {
                return Err(other(&format!(
                    "WorkBuddy billing API returned HTTP {status}."
                )));
            }
        }
        let body = match read_bounded_response(response, MAX_RESPONSE_BYTES).await {
            Ok(body) => body,
            Err(BoundedBodyError::TooLarge) => {
                return Err(model::invalid("expected a JSON object").into());
            }
            Err(BoundedBodyError::Read(error)) => return Err(ProviderError::Network(error).into()),
        };
        Ok(model::response_data(&body)?)
    }
}

fn other(message: &str) -> Failure {
    Failure::Error(ProviderError::Other(message.to_owned()))
}

/// Each browser's WorkBuddy session as a Cookie header. No cookies at all is
/// an empty list, so the caller reports the missing session.
fn browser_sessions() -> Result<Vec<String>, ProviderError> {
    match crate::providers::browser_cookie_candidates_for_domain(COOKIE_DOMAIN) {
        Ok(candidates) => Ok(candidates
            .into_iter()
            .filter_map(|(_, cookies)| request_cookie_header(&cookies, SUMMARY_PATH))
            .collect()),
        Err(ProviderError::NoCookies) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// The cookies a browser would send to `https://www.workbuddy.cn<path>`
/// (upstream `request-url` selection). The first value of a name wins, so
/// two profiles' sessions are never merged into one header.
fn request_cookie_header(cookies: &[Cookie], path: &str) -> Option<String> {
    let mut names: Vec<&str> = Vec::new();
    let mut pairs: Vec<String> = Vec::new();
    for cookie in cookies {
        if !host_matches(&cookie.domain) || !path_matches(&cookie.path, path) {
            continue;
        }
        if cookie.name.is_empty() || names.contains(&cookie.name.as_str()) {
            continue;
        }
        names.push(&cookie.name);
        pairs.push(cookie.to_header_value());
    }
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

/// A host-only `www.workbuddy.cn` cookie or a domain cookie for
/// `www.workbuddy.cn` or `workbuddy.cn`.
fn host_matches(host_key: &str) -> bool {
    let host = host_key.trim().to_ascii_lowercase();
    host == REQUEST_HOST || host == ".www.workbuddy.cn" || host == ".workbuddy.cn"
}

/// RFC 6265 path matching.
fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    let cookie_path = if cookie_path.is_empty() {
        "/"
    } else {
        cookie_path
    };
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/') || request_path[cookie_path.len()..].starts_with('/')))
}

/// The installed Chrome's major version: the newest version folder of a
/// Chrome install (an update Chrome has not relaunched into counts, the
/// previous major is the retry), else the version Chrome last reported in
/// the registry.
fn installed_chrome_major() -> Option<u32> {
    #[cfg(windows)]
    {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(|root| {
                std::path::PathBuf::from(root)
                    .join("Google")
                    .join("Chrome")
                    .join("Application")
            });
        roots
            .filter_map(|dir| newest_version_major(&dir))
            .max()
            .or_else(registry_chrome_major)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The highest major among `a.b.c.d` version folders in a Chrome
/// `Application` directory.
fn newest_version_major(dir: &std::path::Path) -> Option<u32> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.split('.').count() == 4)
                .then(|| model::chrome_major(name))
                .flatten()
        })
        .max()
}

#[cfg(windows)]
fn registry_chrome_major() -> Option<u32> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Google\\Chrome\\BLBeacon")
        .ok()?;
    let version: String = key.get_value("version").ok()?;
    model::chrome_major(&version)
}
