//! Usage command implementation

use clap::Args;
use serde::Serialize;

use crate::core::{FetchContext, ProviderFetchResult, ProviderId, SourceMode};

mod claude_swap;
mod fetch_helpers;
mod render;

use fetch_helpers::{fetch_provider_json_output, fetch_provider_text_output};
pub(super) use render::{
    append_status_line, format_percent, render_status_indicator, render_text_error,
};
use render::{is_terminal, print_usage_output};
pub use render::{render_brief_text, render_text, render_text_with_status};

pub(super) enum UsageOutput {
    Text(Vec<String>),
    Json {
        results: Vec<serde_json::Value>,
        pretty: bool,
    },
    Toon(Vec<serde_json::Value>),
}

pub const PROVIDER_ARG_HELP: &str = "Provider to query (for example: codex, claude, gemini, antigravity/agy, nanogpt, deepseek, codebuff, windsurf, all, both)";

/// Arguments for the usage command
#[derive(Args, Debug, Default)]
pub struct UsageArgs {
    #[arg(short, long, help = PROVIDER_ARG_HELP)]
    pub provider: Option<String>,

    /// Output format: text, json, or toon
    #[arg(short, long, default_value = "text")]
    pub format: UsageOutputFormat,

    /// Shorthand for --format json
    #[arg(long)]
    pub json: bool,

    /// Skip credits line in output
    #[arg(long = "no-credits")]
    pub no_credits: bool,

    /// Disable ANSI colors in text output
    #[arg(long = "no-color")]
    pub no_color: bool,

    /// Pretty-print JSON output
    #[arg(long)]
    pub pretty: bool,

    /// Fetch and include provider status pages
    #[arg(long)]
    pub status: bool,

    /// Fetch all token accounts where supported
    #[arg(long = "all-accounts")]
    pub all_accounts: bool,

    /// Token-account label or 1-based index (requires a single provider)
    #[arg(long = "account")]
    pub account: Option<String>,

    /// Data source: auto, oauth, web, cli
    #[arg(long, default_value = "auto", value_parser = ["auto", "web", "cli", "oauth"])]
    pub source: String,

    /// Web fetch timeout in seconds
    #[arg(long = "web-timeout", default_value = "60")]
    pub web_timeout: u64,

    /// Print one compact line per provider
    #[arg(long)]
    pub brief: bool,
}

/// Output format enum
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UsageOutputFormat {
    #[default]
    Text,
    Json,
    Toon,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!("Invalid format: {}. Use 'text' or 'json'", s)),
        }
    }
}

impl std::str::FromStr for UsageOutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(UsageOutputFormat::Text),
            "json" => Ok(UsageOutputFormat::Json),
            "toon" => Ok(UsageOutputFormat::Toon),
            _ => Err(format!(
                "Invalid format: {}. Use 'text', 'json', or 'toon'",
                s
            )),
        }
    }
}

/// Provider selection from CLI args
#[derive(Debug, Clone)]
pub enum ProviderSelection {
    Single(ProviderId),
    Both,
    All,
}

impl ProviderSelection {
    pub fn from_arg(arg: Option<&str>) -> anyhow::Result<Self> {
        match arg.map(|s| s.to_lowercase()).as_deref() {
            Some("all") => Ok(ProviderSelection::All),
            Some("both") => Ok(ProviderSelection::Both),
            Some(name) => {
                if let Some(id) = ProviderId::from_cli_name(name) {
                    Ok(ProviderSelection::Single(id))
                } else {
                    anyhow::bail!(
                        "Unknown provider: '{}'. Use --help to see available providers.",
                        name
                    )
                }
            }
            None => Ok(ProviderSelection::Single(ProviderId::Claude)), // Default to Claude
        }
    }

    pub fn as_list(&self) -> Vec<ProviderId> {
        match self {
            ProviderSelection::Single(id) => vec![*id],
            ProviderSelection::Both => vec![ProviderId::Codex, ProviderId::Claude],
            ProviderSelection::All => ProviderId::all().to_vec(),
        }
    }
}

/// JSON output payload
#[derive(Debug, Serialize)]
pub struct ProviderPayload {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub source: String,
    #[serde(flatten)]
    pub result: ProviderFetchResult,
}

/// Error payload for JSON output
#[derive(Debug, Serialize)]
struct ErrorPayload {
    provider: String,
    error: String,
}

/// Run the usage command
pub async fn run(args: UsageArgs) -> anyhow::Result<()> {
    let command = UsageCommand::from_args(args)?;
    command.log();
    let output = claude_swap::collect_usage_output(&command).await;
    print_usage_output(output)
}

struct UsageCommand {
    format: UsageOutputFormat,
    providers: Vec<ProviderId>,
    use_color: bool,
    brief: bool,
    fetch_status: bool,
    pretty: bool,
    /// Optional token-account label/index for a single-provider fetch.
    account: Option<String>,
    /// Read every external claude-swap account for Claude (read-only).
    all_accounts: bool,
    ctx: FetchContext,
}

impl UsageCommand {
    fn from_args(args: UsageArgs) -> anyhow::Result<Self> {
        let format = effective_format(&args);
        let source_mode = SourceMode::parse(&args.source).unwrap_or(SourceMode::Auto);
        let providers = ProviderSelection::from_arg(args.provider.as_deref())?.as_list();
        if args.account.is_some() && providers.len() != 1 {
            anyhow::bail!("--account requires a single --provider (not all/both)");
        }
        if args.all_accounts && args.account.is_some() {
            anyhow::bail!("--all-accounts cannot be combined with --account");
        }

        Ok(Self {
            format,
            providers,
            use_color: !args.no_color && is_terminal(),
            brief: args.brief,
            fetch_status: args.status,
            pretty: args.pretty,
            account: args.account.clone(),
            all_accounts: args.all_accounts,
            ctx: build_usage_fetch_context(&args, source_mode),
        })
    }

    fn log(&self) {
        tracing::debug!(
            "Running usage command: providers={:?}, format={:?}, source={:?}, status={}",
            self.providers,
            self.format,
            self.ctx.source_mode,
            self.fetch_status
        );
    }
}

fn effective_format(args: &UsageArgs) -> UsageOutputFormat {
    if args.json {
        UsageOutputFormat::Json
    } else {
        args.format
    }
}

fn build_usage_fetch_context(args: &UsageArgs, source_mode: SourceMode) -> FetchContext {
    FetchContext {
        source_mode,
        include_credits: !args.no_credits,
        web_timeout: args.web_timeout,
        verbose: false,
        manual_cookie_header: None,
        api_key: None,
        workspace_id: None,
        api_region: None,
        gateway_url: None,
        auto_prefer_web: false,
        // `codexbar usage` is a foreground read: optional enrichment (e.g. the
        // OpenCode Go Zen balance) is worth its full bounded wait (#2583).
        requires_optional_usage_completeness: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        ProviderAccountData, ProviderInventoryItem, RateWindow, TokenAccount, TokenAccountSupport,
        UsageSnapshot,
    };
    use crate::providers::claude::claude_swap::ClaudeSwapAccount;
    use crate::status::{ProviderStatus as StatusInfo, StatusLevel};
    use chrono::Utc;
    use fetch_helpers::find_token_account;
    use render::render_json_result;

    fn fetch_result(usage: UsageSnapshot) -> ProviderFetchResult {
        ProviderFetchResult::new(usage, "test")
    }

    fn sample_swap_account() -> ClaudeSwapAccount {
        use crate::providers::claude::claude_swap::{
            ClaudeSwapScopedWindowDto, ClaudeSwapUsageWindowDto,
        };
        ClaudeSwapAccount {
            id: "claude-swap:2".to_string(),
            slot: 2,
            label: "work@example.com".to_string(),
            email: Some("work@example.com".to_string()),
            organization: None,
            alias: None,
            is_active: false,
            status: "ok".to_string(),
            error: None,
            five_hour: Some(ClaudeSwapUsageWindowDto {
                used_percent: 81.0,
                resets_at: None,
            }),
            seven_day: Some(ClaudeSwapUsageWindowDto {
                used_percent: 18.0,
                resets_at: None,
            }),
            scoped: vec![ClaudeSwapScopedWindowDto {
                name: "Fable only".to_string(),
                used_percent: 4.0,
                resets_at: None,
            }],
            action: Some(crate::providers::claude::claude_swap::ClaudeSwapAccountAction::Switch),
            is_disabled: false,
            spend: None,
            historical_usage: None,
        }
    }

    #[test]
    fn claude_swap_json_payload_is_allow_listed() {
        let payload = super::claude_swap::claude_swap_json_payload(&sample_swap_account(), None);
        assert_eq!(payload["provider"], "claude");
        assert_eq!(payload["source"], "claude-swap");
        assert_eq!(payload["account"]["id"], "claude-swap:2");
        assert_eq!(payload["account"]["fiveHour"]["usedPercent"], 81.0);
        assert_eq!(payload["account"]["scoped"][0]["name"], "Fable only");
    }

    #[test]
    fn claude_swap_json_payload_keeps_provider_status_distinct() {
        let status = StatusInfo {
            level: StatusLevel::Degraded,
            description: "Degraded Performance".to_string(),
            ..Default::default()
        };
        let payload =
            super::claude_swap::claude_swap_json_payload(&sample_swap_account(), Some(&status));
        assert_eq!(payload["account"]["status"], "ok");
        assert_eq!(payload["status"]["level"], "degraded");
        assert_eq!(payload["status"]["description"], "Degraded Performance");
    }

    #[test]
    fn claude_swap_brief_renderer_keeps_one_line_per_provider() {
        let mut first = sample_swap_account();
        first.is_active = true;
        let mut second = sample_swap_account();
        second.id = "claude-swap:3".to_string();
        second.slot = 3;
        second.label = "personal@example.com".to_string();
        let status = StatusInfo {
            level: StatusLevel::Operational,
            description: "All Systems Operational".to_string(),
            ..Default::default()
        };

        let text =
            super::claude_swap::render_claude_swap_brief(&[first, second], Some(&status), false);
        assert!(!text.contains('\n'));
        assert!(text.contains("work@example.com (active)"));
        assert!(text.contains("personal@example.com"));
        assert!(text.contains("Status All Systems Operational"));
    }

    #[test]
    fn claude_swap_text_renderer_shows_windows_and_status() {
        let text = super::claude_swap::render_claude_swap_text(&sample_swap_account(), None, false);
        assert!(text.contains("claude-swap"));
        assert!(text.contains("work@example.com"));
        assert!(text.contains("Session 81%"));
        assert!(text.contains("Weekly 18%"));
        assert!(text.contains("Fable only 4%"));
    }

    #[test]
    fn claude_swap_detailed_text_shows_history_but_brief_does_not() {
        use crate::providers::claude::claude_swap::{
            ClaudeSwapHistoricalUsageDto, ClaudeSwapSpendWindowDto, ClaudeSwapUsageWindowDto,
        };
        let mut account = sample_swap_account();
        account.spend = Some(ClaudeSwapSpendWindowDto {
            used: 2.0,
            limit: 20.0,
            used_percent: 10.0,
            currency_code: Some("USD".to_string()),
            resets_at: None,
        });
        account.historical_usage = Some(ClaudeSwapHistoricalUsageDto {
            five_hour: Some(ClaudeSwapUsageWindowDto {
                used_percent: 44.0,
                resets_at: None,
            }),
            seven_day: None,
            scoped: vec![],
            spend: None,
            fetched_at: "2026-09-12T00:45:00Z".parse().unwrap(),
            provenance: "source_reported_last_good",
        });
        let detailed = super::claude_swap::render_claude_swap_text(&account, None, false);
        assert!(detailed.contains("Spend 2.00/20.00 USD (10%)"));
        assert!(detailed.contains("Last known usage (captured 2026-09-12T00:45:00+00:00)"));
        assert!(detailed.contains("Session 44%"));

        let brief = super::claude_swap::render_claude_swap_brief(&[account], None, false);
        assert!(!brief.contains("Last known usage"));
        assert!(!brief.contains("44%"));
    }

    #[test]
    fn all_accounts_conflicts_with_explicit_account() {
        let args = UsageArgs {
            all_accounts: true,
            account: Some("work".to_string()),
            ..Default::default()
        };
        assert!(UsageCommand::from_args(args).is_err());
    }

    #[test]
    fn usage_output_format_accepts_toon() {
        assert_eq!(
            "toon".parse::<UsageOutputFormat>(),
            Ok(UsageOutputFormat::Toon)
        );
        assert!("toon".parse::<OutputFormat>().is_err());
    }

    #[test]
    fn openrouter_account_ref_resolves_labeled_key() {
        let mut data = ProviderAccountData::new();
        data.add_account(TokenAccount::new("Personal", "sk-or-v1-personal"));
        data.add_account(TokenAccount::new("Work", "sk-or-v1-work"));
        data.set_active(0);

        let work = find_token_account(&data, "Work").unwrap();
        let env = TokenAccountSupport::env_override(ProviderId::OpenRouter, &work.token).unwrap();
        assert_eq!(
            env.get("OPENROUTER_API_KEY").map(String::as_str),
            Some("sk-or-v1-work")
        );

        let by_index = find_token_account(&data, "2").unwrap();
        assert_eq!(by_index.token, "sk-or-v1-work");
    }

    #[test]
    fn text_rendering_shows_sub_one_percent_usage() {
        let result = fetch_result(UsageSnapshot::new(RateWindow::new(0.4)));

        let output = render_text_with_status(ProviderId::Codex, &result, None, false);

        assert!(output.contains("<1% used"));
    }

    #[test]
    fn brief_rendering_keeps_one_line_per_provider() {
        let result = fetch_result(
            UsageSnapshot::new(RateWindow::new(0.4))
                .with_secondary(RateWindow::new(100.0))
                .with_login_method("Pro"),
        );

        let output = render_brief_text(ProviderId::Claude, &result);

        assert_eq!(
            output,
            "Claude: Session (5h) <1%, Weekly 100%, resets n/a, Pro"
        );
    }

    #[test]
    fn inventory_is_rendered_in_full_text_but_not_brief_text() {
        let result = fetch_result(UsageSnapshot::new(RateWindow::new(10.0))).with_inventory_item(
            ProviderInventoryItem {
                id: "reset-credits".to_string(),
                title: "Limit Reset Credits".to_string(),
                available_count: 2,
                next_expires_at: Some(Utc::now() + chrono::Duration::hours(3)),
            },
        );

        let full = render_text_with_status(ProviderId::Grok, &result, None, false);
        let brief = render_brief_text(ProviderId::Grok, &result);

        assert!(full.contains("Limit Reset Credits: 2 available"));
        assert!(full.contains("Next expires in"));
        assert!(!brief.contains("Limit Reset Credits"));
    }

    #[test]
    fn json_inventory_is_additive_and_contains_no_redemption_token() {
        let result = fetch_result(UsageSnapshot::new(RateWindow::new(10.0))).with_inventory_item(
            ProviderInventoryItem {
                id: "reset-credits".to_string(),
                title: "Limit Reset Credits".to_string(),
                available_count: 1,
                next_expires_at: None,
            },
        );

        let json = render_json_result(ProviderId::Grok, result, None);
        assert_eq!(json["inventory"][0]["availableCount"], 1);
        assert!(
            serde_json::to_string(&json)
                .unwrap()
                .contains("reset-credits")
        );
        assert!(
            !serde_json::to_string(&json)
                .unwrap()
                .contains("coupon-token-secret")
        );
    }

    #[test]
    fn secondary_label_override_is_shared_by_full_and_brief_renderers() {
        let result = fetch_result(
            UsageSnapshot::new(RateWindow::new(10.0))
                .with_secondary(RateWindow::new(20.0))
                .with_secondary_label("Weekly"),
        );
        let full = render_text_with_status(ProviderId::Antigravity, &result, None, false);
        let brief = render_brief_text(ProviderId::Antigravity, &result);
        assert!(full.contains("Weekly:"));
        assert!(brief.contains("Weekly 20%"));
    }
    #[test]
    fn primary_label_override_is_shared_by_full_and_brief_renderers() {
        let result =
            fetch_result(UsageSnapshot::new(RateWindow::new(42.0)).with_primary_label("Monthly"));

        let full = render_text_with_status(ProviderId::Grok, &result, None, false);
        let brief = render_brief_text(ProviderId::Grok, &result);

        assert!(full.contains("Monthly:"));
        assert!(brief.contains("Grok: Monthly 42%"));
        assert!(!brief.contains("Credits 42%"));
    }

    #[test]
    fn gemini_plan_preserves_acronym_casing() {
        let result = fetch_result(
            UsageSnapshot::new(RateWindow::new(0.0))
                .with_login_method("Gemini Code Assist in Google One AI Pro"),
        );

        let output = render_text(ProviderId::Gemini, &result, false);

        assert!(output.contains("Plan:    Gemini Code Assist in Google One AI Pro"));
        assert!(!output.contains("Google One Ai Pro"));
    }
}
