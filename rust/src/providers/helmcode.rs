//! Helmcode Cloud and NaN Builders dashboard quota provider.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, TimeZone, Utc};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::time::Duration;

use crate::core::{
    CostSnapshot, FetchContext, Provider, ProviderError, ProviderFetchResult, ProviderId,
    ProviderMetadata, RateWindow, SourceMode, UsageSnapshot,
};
use crate::providers::{BoundedBodyError, read_bounded_response};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tenant {
    Helmcode,
    NanBuilders,
}

impl Tenant {
    fn domain(self) -> &'static str {
        match self {
            Self::Helmcode => "helmcode.com",
            Self::NanBuilders => "nan.builders",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Helmcode => "Helmcode Cloud",
            Self::NanBuilders => "NaN Builders",
        }
    }
    fn api(self) -> String {
        format!("https://cloud-api.{}", self.domain())
    }
    fn origin(self) -> String {
        format!("https://cloud.{}", self.domain())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ModelQuota {
    name: String,
    cap: f64,
    used: f64,
    credit: f64,
    window_hours: Option<u32>,
    resets_at: Option<DateTime<Utc>>,
}

pub struct HelmcodeProvider {
    metadata: ProviderMetadata,
    client: Client,
}

impl HelmcodeProvider {
    pub fn new() -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::Helmcode,
                display_name: "Helmcode",
                session_label: "Quota",
                weekly_label: "Quota",
                supports_opus: false,
                supports_credits: true,
                default_enabled: false,
                is_primary: false,
                dashboard_url: Some("https://cloud.helmcode.com/dashboard"),
                status_page_url: None,
                tertiary_label_key: None,
            },
            client: crate::core::credentialed_http_client_builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    async fn fetch_web(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        let manual_tenant = ctx
            .workspace_id
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("nanBuilders"));
        let candidates = if ctx.manual_cookie_header.is_some() {
            vec![if manual_tenant {
                Tenant::NanBuilders
            } else {
                Tenant::Helmcode
            }]
        } else {
            vec![Tenant::Helmcode, Tenant::NanBuilders]
        };
        let mut rejected = false;
        for tenant in candidates {
            let cookie = match ctx.manual_cookie_header.as_deref() {
                Some(raw) => crate::providers::normalize_cookie_header(raw)
                    .ok_or(ProviderError::NoCookies)?,
                None => match crate::providers::browser_cookie_header(&[tenant.domain()]) {
                    Ok(header) => header,
                    Err(_) => continue,
                },
            };
            match self.fetch_tenant(tenant, &cookie).await {
                Ok(result) => return Ok(result),
                Err(ProviderError::AuthRequired) => rejected = true,
                Err(error) => return Err(error),
            }
        }
        if rejected {
            Err(ProviderError::AuthRequired)
        } else {
            Err(ProviderError::NotInstalled(
                "Sign in to cloud.helmcode.com or cloud.nan.builders, or paste a Cookie header."
                    .into(),
            ))
        }
    }

    async fn fetch_tenant(
        &self,
        tenant: Tenant,
        cookie: &str,
    ) -> Result<ProviderFetchResult, ProviderError> {
        let quota_request = self.get(tenant, cookie, "/api/usage/quota", false);
        let billing_request = self.get(tenant, cookie, "/api/billing", true);
        let credits_request = async {
            if tenant == Tenant::Helmcode {
                self.get(tenant, cookie, "/api/billing/credits", true).await
            } else {
                Ok(None)
            }
        };
        let (quota_result, billing_result, credits_result) =
            tokio::join!(quota_request, billing_request, credits_request);

        let quota = quota_result?.ok_or_else(|| parse_failure("quota"))?;
        let billing = billing_result?;
        let premium = billing
            .as_ref()
            .and_then(|value| value.get("subscription"))
            .and_then(|value| value.get("premium"))
            .and_then(Value::as_bool)
            == Some(true);
        let mut models = parse_models(&quota, premium)?;
        models.sort_by(|a, b| {
            (b.used / b.cap)
                .partial_cmp(&(a.used / a.cap))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });
        let primary = models
            .first()
            .map(model_window)
            .unwrap_or_else(|| RateWindow::informational("No active model quota"));
        let mut usage = UsageSnapshot::new(primary)
            .with_organization(tenant.name())
            .with_login_method("Dashboard session");
        for model in models.iter().skip(1) {
            usage = usage.with_extra_rate_window(
                format!("helmcode-{}", model.name),
                model.name.clone(),
                model_window(model),
            );
        }
        let mut result = ProviderFetchResult::new(usage, "web");
        if let Some(credits) = credits_result?
            && let Some(balance_micros) = credits
                .get("balanceMicros")
                .and_then(nonnegative_balance_micros)
        {
            let currency = credits
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or("EUR")
                .to_ascii_uppercase();
            if currency.len() == 3 && currency.chars().all(|ch| ch.is_ascii_uppercase()) {
                result = result.with_cost(
                    CostSnapshot::new(0.0, currency, "Prepaid balance")
                        .with_balance((balance_micros as f64) / 1_000_000.0),
                );
            }
        }
        Ok(result)
    }

    async fn get(
        &self,
        tenant: Tenant,
        cookie: &str,
        path: &str,
        optional: bool,
    ) -> Result<Option<Value>, ProviderError> {
        let response = self
            .client
            .get(format!("{}{path}", tenant.api()))
            .header("Cookie", cookie)
            .header("Origin", tenant.origin())
            .header("Referer", format!("{}/dashboard", tenant.origin()))
            .send()
            .await?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || status.is_redirection()
        {
            return Err(ProviderError::AuthRequired);
        }
        if optional && !status.is_success() {
            return Ok(None);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderError::Other("Helmcode rate limit reached.".into()));
        }
        if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
            return Err(ProviderError::Other(
                "Helmcode dashboard is unavailable.".into(),
            ));
        }
        if !status.is_success() {
            return Err(ProviderError::Other(format!(
                "Helmcode dashboard returned HTTP {status}."
            )));
        }
        let body = read_bounded_response(response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                BoundedBodyError::TooLarge => parse_failure("response too large"),
                BoundedBodyError::Read(_) => parse_failure("invalid JSON"),
            })?;
        let value = serde_json::from_slice(&body).map_err(|_| parse_failure("invalid JSON"))?;
        Ok(Some(value))
    }
}

impl Default for HelmcodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for HelmcodeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Helmcode
    }
    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }
    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::Web => self.fetch_web(ctx).await,
            SourceMode::OAuth | SourceMode::Cli => {
                Err(ProviderError::UnsupportedSource(ctx.source_mode))
            }
        }
    }
    fn available_sources(&self) -> Vec<SourceMode> {
        vec![SourceMode::Auto, SourceMode::Web]
    }
    fn supports_web(&self) -> bool {
        true
    }
    fn owns_browser_cookie_resolution(&self) -> bool {
        true
    }
}

fn parse_models(quota: &Value, premium: bool) -> Result<Vec<ModelQuota>, ProviderError> {
    let object = quota
        .as_object()
        .ok_or_else(|| parse_failure("quota object"))?;
    let period_start = object
        .get("periodStart")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_failure("periodStart"))?;
    let fallback = DateTime::parse_from_rfc3339(period_start)
        .ok()
        .and_then(|date| {
            let date = date.with_timezone(&Utc);
            Utc.with_ymd_and_hms(date.year(), date.month(), 1, 0, 0, 0)
                .single()
        });
    let models = object
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_failure("models"))?;
    models
        .iter()
        .map(|value| {
            let row = value.as_object().ok_or_else(|| parse_failure("model"))?;
            let name = row
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| {
                    !name.is_empty()
                        && name.chars().count() <= 120
                        && !name.chars().any(char::is_control)
                })
                .ok_or_else(|| parse_failure("model name"))?
                .to_string();
            let cap = nonnegative(row.get("cap"), "cap")?;
            let used = nonnegative(row.get("tokensUsed"), "tokensUsed")?;
            let credit =
                optional_nonnegative(row.get("creditTokens"), "creditTokens")?.unwrap_or(0.0);
            let window_hours = optional_nonnegative(row.get("windowHours"), "windowHours")?
                .map(|value| format!("{value:.0}").parse::<u32>())
                .transpose()
                .map_err(|_| parse_failure("windowHours"))?;
            if window_hours.is_some_and(|hours| hours == 0 || hours > 8_760) {
                return Err(parse_failure("windowHours"));
            }
            let resets_at = row
                .get("periodEnd")
                .and_then(Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|date| date.with_timezone(&Utc))
                .or(fallback);
            Ok(ModelQuota {
                name,
                cap,
                used,
                credit,
                window_hours,
                resets_at,
            })
        })
        .filter_map(|result| match result {
            Ok(model) if model.cap > 0.0 && (model.window_hours.is_none() || premium) => {
                Some(Ok(model))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn model_window(model: &ModelQuota) -> RateWindow {
    let mut window = RateWindow::with_details(
        (model.used / model.cap * 100.0).clamp(0.0, 100.0),
        model.window_hours.map(|hours| hours * 60),
        model.resets_at,
        None,
    );
    window.reset_description = Some(format!(
        "{} · {:.0} / {:.0} tokens{}",
        model.name,
        model.used,
        model.cap,
        if model.credit > 0.0 {
            format!(" · {:.0} credit-funded", model.credit)
        } else {
            String::new()
        }
    ));
    window
}

fn nonnegative(value: Option<&Value>, field: &str) -> Result<f64, ProviderError> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .ok_or_else(|| parse_failure(field))
}
fn optional_nonnegative(value: Option<&Value>, field: &str) -> Result<Option<f64>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => nonnegative(Some(value), field).map(Some),
    }
}
fn nonnegative_balance_micros(value: &Value) -> Option<i64> {
    value.as_i64().filter(|amount| *amount >= 0)
}
fn parse_failure(field: impl AsRef<str>) -> ProviderError {
    ProviderError::Parse(format!(
        "Helmcode quota response format changed: {}",
        field.as_ref()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_models_filters_nonpremium_rolling_windows_and_orders_are_external() {
        let value = json!({"periodStart":"2030-02-12T00:00:00Z","models":[
            {"model":"monthly","cap":1000,"tokensUsed":250,"creditTokens":50,"periodEnd":null},
            {"model":"rolling","cap":100,"tokensUsed":90,"windowHours":5}
        ]});
        let free = parse_models(&value, false).unwrap();
        assert_eq!(free.len(), 1);
        assert_eq!(free[0].name, "monthly");
        assert_eq!(free[0].resets_at.unwrap().day(), 1);
        assert_eq!(parse_models(&value, true).unwrap().len(), 2);
    }

    #[test]
    fn rejects_fractional_or_negative_quota_counts() {
        assert!(parse_models(&json!({"periodStart":"2030-01-01T00:00:00Z","models":[{"model":"x","cap":1.5,"tokensUsed":0}]}), true).is_err());
        assert!(parse_models(&json!({"periodStart":"2030-01-01T00:00:00Z","models":[{"model":"x","cap":1,"tokensUsed":-1}]}), true).is_err());
        assert!(
            parse_models(
                &json!({"periodStart":"2030-01-01T00:00:00Z","models":[{"model":"x","cap":1,"tokensUsed":0,"windowHours":4_294_967_296_u64}]}),
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_nonnegative_balance_micros_and_rejects_negative_values() {
        assert_eq!(nonnegative_balance_micros(&json!(0)), Some(0));
        assert_eq!(
            nonnegative_balance_micros(&json!(1_250_000)),
            Some(1_250_000)
        );
        assert_eq!(nonnegative_balance_micros(&json!(-1)), None);
        assert_eq!(nonnegative_balance_micros(&json!(1.5)), None);
    }
}
