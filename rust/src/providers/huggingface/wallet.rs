use serde_json::Value;

use super::IdentitySnapshot;
use crate::core::ProviderError;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WalletCandidate {
    pub(super) user_id: String,
    pub(super) balance: f64,
}

pub(super) fn matching_wallet_balance(
    identity: Option<&IdentitySnapshot>,
    candidate: Option<WalletCandidate>,
) -> Option<f64> {
    let expected_user_id = identity?.user_id.as_deref()?;
    let candidate = candidate?;
    (candidate.user_id == expected_user_id).then_some(candidate.balance)
}

pub(super) fn parse_wallet_balance(html: &str) -> Result<f64, ProviderError> {
    let mut current = Vec::new();
    let mut legacy = Vec::new();
    let mut rest = html;
    while let Some(index) = rest.find("data-props") {
        rest = &rest[index + "data-props".len()..];
        let trimmed = rest.trim_start();
        let Some(after_equals) = trimmed.strip_prefix('=') else {
            continue;
        };
        let after_equals = after_equals.trim_start();
        let Some(quote) = after_equals
            .chars()
            .next()
            .filter(|value| matches!(value, '\'' | '"'))
        else {
            continue;
        };
        let payload = &after_equals[quote.len_utf8()..];
        let Some(end) = payload.find(quote) else {
            break;
        };
        let decoded = decode_html_entities(&payload[..end])?;
        rest = &payload[end + quote.len_utf8()..];
        let Ok(value) = serde_json::from_str::<Value>(&decoded) else {
            continue;
        };
        let Some(object) = value.as_object() else {
            continue;
        };
        if let Some(entity) = object.get("entity").and_then(Value::as_object)
            && entity.contains_key("currentBalanceUsd")
        {
            if entity.get("type").and_then(Value::as_str) != Some("user") {
                return Err(invalid_wallet("wallet entity type"));
            }
            current.push(wallet_number(
                entity.get("currentBalanceUsd"),
                "currentBalanceUsd",
            )?);
        }
        if object.contains_key("invoiceCreditsCents") {
            let cents = wallet_number(object.get("invoiceCreditsCents"), "invoiceCreditsCents")?;
            if cents.fract() != 0.0 {
                return Err(invalid_wallet("invoiceCreditsCents"));
            }
            legacy.push(cents / 100.0);
        }
    }
    match (current.as_slice(), legacy.as_slice()) {
        ([balance], _) => Ok(*balance),
        ([], [balance]) => Ok(*balance),
        ([], _) => Err(invalid_wallet("missing or ambiguous legacy wallet")),
        _ => Err(invalid_wallet("ambiguous current wallet")),
    }
}

fn wallet_number(value: Option<&Value>, field: &str) -> Result<f64, ProviderError> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| invalid_wallet(field))
}

fn invalid_wallet(field: &str) -> ProviderError {
    ProviderError::Parse(format!("Hugging Face wallet field '{field}' was invalid."))
}

fn decode_html_entities(raw: &str) -> Result<String, ProviderError> {
    let mut output = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index + 1..];
        let Some(end) = rest.find(';') else {
            return Err(invalid_wallet("HTML entity"));
        };
        let entity = &rest[..end];
        let decoded = match entity {
            "amp" => '&',
            "apos" => '\'',
            "gt" => '>',
            "lt" => '<',
            "nbsp" => '\u{00a0}',
            "quot" => '"',
            value if value.starts_with("#x") || value.starts_with("#X") => {
                u32::from_str_radix(&value[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| invalid_wallet("HTML entity"))?
            }
            value if value.starts_with('#') => value[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| invalid_wallet("HTML entity"))?,
            _ => return Err(invalid_wallet("HTML entity")),
        };
        output.push(decoded);
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_is_attached_only_to_the_matching_token_identity() {
        let identity = IdentitySnapshot {
            user_id: Some("user-a".to_string()),
            name: None,
            email: None,
            plan: None,
        };
        let candidate = WalletCandidate {
            user_id: "user-a".to_string(),
            balance: 12.5,
        };
        assert_eq!(
            matching_wallet_balance(Some(&identity), Some(candidate.clone())),
            Some(12.5)
        );

        let other_identity = IdentitySnapshot {
            user_id: Some("user-b".to_string()),
            ..identity.clone()
        };
        assert_eq!(
            matching_wallet_balance(Some(&other_identity), Some(candidate.clone())),
            None
        );
        assert_eq!(matching_wallet_balance(None, Some(candidate)), None);
    }

    #[test]
    fn parser_prefers_unique_current_balance_and_supports_legacy_cents() {
        let current = r#"<div data-props="{&quot;entity&quot;:{&quot;type&quot;:&quot;user&quot;,&quot;currentBalanceUsd&quot;:12.5}}">"#;
        assert_eq!(parse_wallet_balance(current).unwrap(), 12.5);

        let legacy = r#"<div data-props='{"invoiceCreditsCents":725}'">"#;
        assert_eq!(parse_wallet_balance(legacy).unwrap(), 7.25);
    }

    #[test]
    fn parser_rejects_ambiguous_or_non_user_balances() {
        let ambiguous = r#"<div data-props='{"entity":{"type":"user","currentBalanceUsd":1}}'><div data-props='{"entity":{"type":"user","currentBalanceUsd":2}}'>"#;
        assert!(parse_wallet_balance(ambiguous).is_err());
        let organization = r#"<div data-props='{"entity":{"type":"org","currentBalanceUsd":1}}'>"#;
        assert!(parse_wallet_balance(organization).is_err());
    }
}
