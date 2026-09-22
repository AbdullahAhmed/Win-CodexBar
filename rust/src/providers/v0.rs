//! v0 billing and API rate-limit provider.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reqwest::{Client, StatusCode, Url};
use serde_json::Value;
use std::time::Duration;

use crate::core::{
    FetchContext, Provider, ProviderDisplayDetail, ProviderError, ProviderFetchResult, ProviderId,
    ProviderMetadata, RateWindow, SourceMode, UsageSnapshot,
};

const API_BASE: &str = "https://api.v0.dev/v1";
const CREDENTIAL_TARGET: &str = "codexbar-v0";
const ENV_KEYS: &[&str] = &["V0_API_KEY"];

#[derive(Debug, Clone, PartialEq)]
struct Quota {
    used_percent: Option<f64>,
    resets_at: Option<chrono::DateTime<Utc>>,
    remaining: Option<f64>,
    limit: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Billing {
    quota: Quota,
    billing_type: String,
    on_demand_balance: Option<f64>,
}

pub struct V0Provider {
    metadata: ProviderMetadata,
    client: Client,
}

impl V0Provider {
    pub fn new() -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::V0,
                display_name: "v0",
                session_label: "Billing",
                weekly_label: "Rate limit",
                supports_opus: false,
                supports_credits: true,
                default_enabled: false,
                is_primary: false,
                dashboard_url: Some("https://v0.app/chat/settings/billing"),
                status_page_url: Some("https://www.vercel-status.com/"),
                tertiary_label_key: None,
            },
            client: crate::core::credentialed_http_client_builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    async fn fetch_api(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        let api_key =
            crate::providers::resolve_api_key(ctx.api_key.as_deref(), CREDENTIAL_TARGET, ENV_KEYS)?;
        let scope = ctx
            .workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                std::env::var("V0_SCOPE")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });
        let billing = parse_billing(
            &self
                .get_json("/user/billing", scope.as_deref(), &api_key)
                .await?,
        )?;
        let rate_limit = parse_quota(
            &self
                .get_json("/rate-limits", scope.as_deref(), &api_key)
                .await?,
            "rate limit response",
        )?;
        Ok(build_result(billing, rate_limit, scope.as_deref()))
    }

    async fn get_json(
        &self,
        path: &str,
        scope: Option<&str>,
        api_key: &str,
    ) -> Result<Value, ProviderError> {
        let mut url = Url::parse(&format!("{API_BASE}{path}"))
            .map_err(|_| ProviderError::Other("Invalid v0 API URL.".into()))?;
        if let Some(scope) = scope {
            url.query_pairs_mut().append_pair("scope", scope);
        }
        let response = self.client.get(url).bearer_auth(api_key).send().await?;
        classify_status(response.status())?;
        response.json().await.map_err(|_| {
            ProviderError::Parse(format!(
                "Could not parse v0 usage: {path} returned invalid JSON"
            ))
        })
    }
}

impl Default for V0Provider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for V0Provider {
    fn id(&self) -> ProviderId {
        ProviderId::V0
    }
    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::OAuth => self.fetch_api(ctx).await,
            SourceMode::Web | SourceMode::Cli => {
                Err(ProviderError::UnsupportedSource(ctx.source_mode))
            }
        }
    }

    fn available_sources(&self) -> Vec<SourceMode> {
        vec![SourceMode::Auto, SourceMode::OAuth]
    }
}

fn classify_status(status: StatusCode) -> Result<(), ProviderError> {
    match status {
        status if status.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED => Err(ProviderError::AuthRequired),
        StatusCode::FORBIDDEN => Err(ProviderError::Other(
            "v0 denied access to this scope.".into(),
        )),
        StatusCode::TOO_MANY_REQUESTS => {
            Err(ProviderError::Other("v0 API rate limit reached.".into()))
        }
        status if status.is_server_error() => Err(ProviderError::Other(format!(
            "v0 API returned HTTP {status}."
        ))),
        status => Err(ProviderError::Other(format!(
            "v0 API returned HTTP {status}."
        ))),
    }
}

fn parse_billing(value: &Value) -> Result<Billing, ProviderError> {
    let object = value
        .as_object()
        .ok_or_else(|| parse_failure("billing response"))?;
    let billing_type = text(object.get("billingType"), "billingType")?
        .ok_or_else(|| parse_failure("billingType"))?;
    let data = object
        .get("data")
        .ok_or_else(|| parse_failure("billing.data"))?;
    match billing_type.as_str() {
        "legacy" => Ok(Billing {
            quota: parse_quota(data, "billing.data")?,
            billing_type,
            on_demand_balance: None,
        }),
        "token" => {
            let data = data
                .as_object()
                .ok_or_else(|| parse_failure("billing.data"))?;
            let balance = data
                .get("balance")
                .and_then(Value::as_object)
                .ok_or_else(|| parse_failure("billing.data.balance"))?;
            let total = finite_number(balance.get("total"), "billing.data.balance.total")?;
            let remaining =
                finite_number(balance.get("remaining"), "billing.data.balance.remaining")?;
            if total < 0.0 {
                return Err(parse_failure(
                    "billing.data.balance.total must not be negative",
                ));
            }
            let resets_at = match data.get("billingCycle").and_then(Value::as_object) {
                Some(cycle) => parse_reset(cycle.get("end"))?,
                None => None,
            };
            let on_demand_balance = data
                .get("onDemand")
                .filter(|value| !value.is_null())
                .and_then(Value::as_object)
                .map(|on_demand| {
                    finite_number(on_demand.get("balance"), "billing.data.onDemand.balance")
                })
                .transpose()?;
            Ok(Billing {
                quota: Quota {
                    used_percent: percent(total - remaining, total),
                    resets_at,
                    remaining: Some(remaining),
                    limit: total,
                },
                billing_type,
                on_demand_balance,
            })
        }
        _ => Err(parse_failure("billingType")),
    }
}

fn parse_quota(value: &Value, field: &str) -> Result<Quota, ProviderError> {
    let object = value.as_object().ok_or_else(|| parse_failure(field))?;
    let limit = finite_number(object.get("limit"), &format!("{field}.limit"))?;
    if limit < 0.0 {
        return Err(parse_failure(format!("{field}.limit must not be negative")));
    }
    let remaining = optional_number(object.get("remaining"), &format!("{field}.remaining"))?;
    Ok(Quota {
        used_percent: remaining.and_then(|remaining| percent(limit - remaining, limit)),
        resets_at: parse_reset(object.get("reset"))?,
        remaining,
        limit,
    })
}

fn build_result(billing: Billing, rate_limit: Quota, scope: Option<&str>) -> ProviderFetchResult {
    let primary = billing.quota.used_percent.map_or_else(
        || RateWindow::informational("Billing usage unavailable"),
        |used| RateWindow::with_details(used, None, billing.quota.resets_at, None),
    );
    let mut usage = UsageSnapshot::new(primary).with_login_method("API key");
    if let Some(used) = rate_limit.used_percent {
        usage = usage.with_secondary(RateWindow::with_details(
            used,
            None,
            rate_limit.resets_at,
            None,
        ));
    }
    let mut result = ProviderFetchResult::new(usage, "api");
    let remaining = billing
        .quota
        .remaining
        .map_or_else(|| "Unavailable".into(), format_number);
    result = result.with_display_detail(ProviderDisplayDetail::new(
        "billing-remaining",
        "Billing remaining",
        format!("{remaining} of {}", format_number(billing.quota.limit)),
    ));
    if let Some(balance) = billing.on_demand_balance {
        result = result.with_display_detail(ProviderDisplayDetail::new(
            "on-demand",
            "On-demand balance",
            format_number(balance),
        ));
    }
    let rate_remaining = rate_limit
        .remaining
        .map_or_else(|| "Unavailable".into(), format_number);
    result = result.with_display_detail(ProviderDisplayDetail::new(
        "rate-limit-remaining",
        "Rate-limit remaining",
        format!("{rate_remaining} of {}", format_number(rate_limit.limit)),
    ));
    result = result.with_display_detail(ProviderDisplayDetail::new(
        "billing-type",
        "Billing type",
        billing.billing_type,
    ));
    if let Some(scope) = scope {
        result = result.with_display_detail(ProviderDisplayDetail::new(
            "scope",
            "Scope",
            scope.chars().take(120).collect::<String>(),
        ));
    }
    result
}

fn finite_number(value: Option<&Value>, field: &str) -> Result<f64, ProviderError> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| parse_failure(field))
}

fn optional_number(value: Option<&Value>, field: &str) -> Result<Option<f64>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => finite_number(Some(value), field).map(Some),
    }
}

fn text(value: Option<&Value>, field: &str) -> Result<Option<String>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            Ok((!value.trim().is_empty()).then(|| value.trim().to_string()))
        }
        Some(_) => Err(parse_failure(field)),
    }
}

fn parse_reset(value: Option<&Value>) -> Result<Option<chrono::DateTime<Utc>>, ProviderError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let raw = finite_number(Some(value), "reset")?;
    if raw <= 0.0 {
        return Ok(None);
    }
    let seconds = if raw >= 1_000_000_000_000.0 {
        raw / 1000.0
    } else {
        raw
    };
    let seconds = format!("{:.0}", seconds.trunc())
        .parse::<i64>()
        .map_err(|_| parse_failure("reset"))?;
    Ok(Utc.timestamp_opt(seconds, 0).single())
}

fn percent(used: f64, limit: f64) -> Option<f64> {
    (limit > 0.0 && used.is_finite()).then(|| (used.max(0.0) / limit * 100.0).clamp(0.0, 100.0))
}

fn format_number(value: f64) -> String {
    format!("{value:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
fn parse_failure(field: impl AsRef<str>) -> ProviderError {
    ProviderError::Parse(format!("Could not parse v0 usage: {}", field.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_token_billing_and_rate_limit_without_fabricating_windows() {
        let billing = parse_billing(&json!({
            "billingType": "token",
            "data": {"balance": {"total": 100, "remaining": 75}, "billingCycle": {"end": 1_800_000_000}, "onDemand": {"balance": 12.5}}
        })).unwrap();
        assert_eq!(billing.quota.used_percent, Some(25.0));
        assert_eq!(billing.on_demand_balance, Some(12.5));
        let unknown = parse_quota(
            &json!({"limit": 50, "remaining": null, "reset": null}),
            "rate",
        )
        .unwrap();
        assert_eq!(unknown.used_percent, None);
        assert_eq!(unknown.remaining, None);
    }

    #[test]
    fn rejects_unknown_billing_type_and_negative_limits() {
        assert!(parse_billing(&json!({"billingType": "future", "data": {}})).is_err());
        assert!(parse_quota(&json!({"limit": -1}), "rate").is_err());
        assert!(parse_reset(Some(&json!(1e30))).is_err());
    }
}
