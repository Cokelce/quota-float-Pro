use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    process::Stdio,
    time::{Duration, SystemTime},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use rusqlite::OpenFlags;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::timeout,
};

use crate::models::{ProviderSnapshot, UsageWindow};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_AUTH_BYTES: u64 = 256 * 1024;
const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(8);

struct Auth {
    access_token: String,
    account_id: Option<String>,
}

struct ApiConfig {
    provider_name: String,
    base_url: String,
    balance_url: Option<String>,
    api_key: String,
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

fn auth_path() -> Option<PathBuf> {
    codex_home().map(|home| home.join("auth.json"))
}

fn config_path() -> Option<PathBuf> {
    codex_home().map(|home| home.join("config.toml"))
}

fn cc_switch_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".cc-switch").join("cc-switch.db"))
}

fn codex_plus_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex-session-delete").join("settings.json"))
}

fn codex_plus_backup_dirs() -> Vec<PathBuf> {
    let Some(backups) = codex_home().map(|home| home.join("backups")) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(backups) else {
        return Vec::new();
    };
    let mut dirs = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_string_lossy();
            if !name.starts_with("codex-plus-live-") {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    dirs.sort_by(|left, right| right.0.cmp(&left.0));
    dirs.into_iter().map(|(_, path)| path).collect()
}

fn pick_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key)?.as_str())
}

fn account_id_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    pick_string(
        &value,
        &[
            "https://api.openai.com/auth.chatgpt_account_id",
            "chatgpt_account_id",
        ],
    )
    .map(str::to_owned)
}

fn load_auth() -> Result<Auth, &'static str> {
    let path = auth_path().ok_or("Codex login was not found.")?;
    let metadata = fs::metadata(&path).map_err(|_| "Please sign in to Codex Desktop first.")?;
    if !metadata.is_file() || metadata.len() > MAX_AUTH_BYTES {
        return Err("Codex login data is unavailable.");
    }
    let raw = fs::read_to_string(path).map_err(|_| "Please sign in to Codex Desktop first.")?;
    let value: Value = serde_json::from_str(&raw).map_err(|_| "Codex login format has changed.")?;
    let tokens = value.get("tokens").unwrap_or(&value);
    let access_token = pick_string(tokens, &["access_token", "accessToken"])
        .ok_or("Codex login expired. Please sign in again.")?
        .to_owned();
    let account_id = pick_string(tokens, &["account_id", "accountId"])
        .map(str::to_owned)
        .or_else(|| account_id_from_jwt(&access_token));
    Ok(Auth {
        access_token,
        account_id,
    })
}

fn parse_toml_string_line(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    let value = line
        .strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

fn top_level_toml_string(raw: &str, key: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if let Some(value) = parse_toml_string_line(line, key) {
            return Some(value);
        }
    }
    None
}

fn provider_toml_string(raw: &str, provider: &str, key: &str) -> Option<String> {
    let header = format!("[model_providers.{provider}]");
    let mut active = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            active = trimmed == header;
            continue;
        }
        if active {
            if let Some(value) = parse_toml_string_line(trimmed, key) {
                return Some(value);
            }
        }
    }
    None
}

fn usable_api_key(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("PROXY_MANAGED") {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn balance_source_key(parts: &[&str]) -> String {
    let mut hasher = DefaultHasher::new();
    parts.iter().for_each(|part| part.hash(&mut hasher));
    format!("{:016x}", hasher.finish())
}

fn load_api_key_from_auth_path(path: &PathBuf) -> Option<String> {
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_AUTH_BYTES {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    pick_string(
        &value,
        &[
            "OPENAI_API_KEY",
            "openai_api_key",
            "api_key",
            "apiKey",
            "experimental_bearer_token",
        ],
    )
    .map(str::to_owned)
    .and_then(usable_api_key)
}

fn load_api_key_from_auth_contents(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    json_string(
        &value,
        &[
            "OPENAI_API_KEY",
            "openai_api_key",
            "api_key",
            "apiKey",
            "experimental_bearer_token",
        ],
    )
}

fn json_string(value: &Value, keys: &[&str]) -> Option<String> {
    pick_string(value, keys)
        .map(str::to_owned)
        .and_then(usable_api_key)
}

fn js_string_property(code: &str, property: &str) -> Option<String> {
    let start = code.find(property)?;
    let rest = code[start + property.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

fn cc_switch_usage_url(meta: &Value, base_url: &str, api_key: &str) -> Option<String> {
    let script = meta
        .get("usage_script")
        .or_else(|| meta.get("usageScript"))?;
    if script.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let code = script.get("code")?.as_str()?;
    let template = js_string_property(code, "url")?;
    Some(
        template
            .replace("{{baseUrl}}", &trimmed_url(base_url))
            .replace("{{apiKey}}", api_key),
    )
}

fn cc_switch_api_config_from_values(
    provider_name: &str,
    settings: &Value,
    meta: &Value,
) -> Option<ApiConfig> {
    let config = settings.get("config")?.as_str()?;
    let provider =
        top_level_toml_string(config, "model_provider").unwrap_or_else(|| "custom".into());
    let base_url = provider_toml_string(config, &provider, "base_url")
        .or_else(|| top_level_toml_string(config, "base_url"))?;
    let api_key = settings
        .get("auth")
        .and_then(|auth| json_string(auth, &["OPENAI_API_KEY", "api_key", "apiKey"]))
        .or_else(|| json_string(settings, &["OPENAI_API_KEY", "api_key", "apiKey"]))?;
    let balance_url = cc_switch_usage_url(meta, &base_url, &api_key);
    Some(ApiConfig {
        provider_name: provider_name.to_owned(),
        base_url,
        balance_url,
        api_key,
    })
}

fn load_cc_switch_api_config() -> Option<ApiConfig> {
    let path = cc_switch_db_path()?;
    let connection = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let mut statement = connection
        .prepare(
            "select name, settings_config, meta from providers \
             where app_type = 'codex' and is_current = 1 limit 1",
        )
        .ok()?;
    let mut rows = statement.query([]).ok()?;
    let row = rows.next().ok()??;
    let provider_name: String = row.get(0).ok()?;
    let settings_raw: String = row.get(1).ok()?;
    let meta_raw: String = row.get(2).ok()?;
    let settings: Value = serde_json::from_str(&settings_raw).ok()?;
    let meta: Value = serde_json::from_str(&meta_raw).ok()?;
    cc_switch_api_config_from_values(&provider_name, &settings, &meta)
}

fn codex_plus_api_config_from_value(
    value: &Value,
    raw_config_hint: Option<&str>,
) -> Option<ApiConfig> {
    let config_contents = pick_string(value, &["configContents", "config"]).or(raw_config_hint);
    let provider = config_contents
        .and_then(|raw| top_level_toml_string(raw, "model_provider"))
        .unwrap_or_else(|| "custom".into());
    let provider_name = config_contents
        .and_then(|raw| provider_toml_string(raw, &provider, "name"))
        .unwrap_or_else(|| "Codex++".into());
    let base_url = json_string(
        value,
        &["relayBaseUrl", "upstreamBaseUrl", "base_url", "baseUrl"],
    )
    .or_else(|| config_contents.and_then(|raw| provider_toml_string(raw, &provider, "base_url")))?;
    let api_key =
        json_string(value, &["relayApiKey", "apiKey", "OPENAI_API_KEY"]).or_else(|| {
            pick_string(value, &["authContents", "auth"]).and_then(load_api_key_from_auth_contents)
        })?;
    let balance_url = config_contents.and_then(|raw| {
        provider_toml_string(raw, &provider, "balance_url")
            .or_else(|| provider_toml_string(raw, &provider, "usage_url"))
            .or_else(|| provider_toml_string(raw, &provider, "usage_endpoint"))
    });
    Some(ApiConfig {
        provider_name,
        base_url,
        balance_url,
        api_key,
    })
}

fn codex_plus_api_config_from_settings(
    settings: &Value,
    raw_config_hint: Option<&str>,
) -> Option<ApiConfig> {
    let profiles = settings
        .get("relayProfiles")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if let Some(active_id) = pick_string(settings, &["activeRelayId", "activeProfileId"]) {
        if let Some(profile) = profiles
            .iter()
            .find(|profile| pick_string(profile, &["id"]) == Some(active_id))
        {
            if let Some(config) = codex_plus_api_config_from_value(profile, raw_config_hint) {
                return Some(config);
            }
        }
    }
    codex_plus_api_config_from_value(settings, raw_config_hint).or_else(|| {
        profiles
            .iter()
            .find_map(|profile| codex_plus_api_config_from_value(profile, raw_config_hint))
    })
}

fn load_codex_plus_api_config(raw_config_hint: Option<&str>) -> Option<ApiConfig> {
    let path = codex_plus_settings_path()?;
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_AUTH_BYTES {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    let settings: Value = serde_json::from_str(&raw).ok()?;
    codex_plus_api_config_from_settings(&settings, raw_config_hint)
}

fn is_cc_switch_proxy_url(base_url: &str) -> bool {
    let lower = base_url.to_ascii_lowercase();
    lower.contains("127.0.0.1:15721") || lower.contains("localhost:15721")
}

fn is_codex_plus_proxy_url(base_url: &str) -> bool {
    let lower = base_url.to_ascii_lowercase();
    lower.contains("127.0.0.1:57321") || lower.contains("localhost:57321")
}

fn load_api_config_from_raw(
    raw_config: &str,
    auth_path: Option<&PathBuf>,
    source_hint: Option<&str>,
) -> Option<ApiConfig> {
    let provider =
        top_level_toml_string(raw_config, "model_provider").unwrap_or_else(|| "openai".into());
    let section_key = provider.as_str();
    let provider_name = provider_toml_string(raw_config, section_key, "name")
        .unwrap_or_else(|| provider.to_uppercase());
    let base_url = provider_toml_string(raw_config, section_key, "base_url")
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let proxy_managed = provider_toml_string(raw_config, section_key, "experimental_bearer_token")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("PROXY_MANAGED"));
    let provider_lower = provider_name.to_ascii_lowercase();
    if proxy_managed || provider_lower.contains("cc switch") || is_cc_switch_proxy_url(&base_url) {
        if let Some(config) = load_cc_switch_api_config() {
            return Some(config);
        }
    }
    let balance_url = provider_toml_string(raw_config, section_key, "balance_url")
        .or_else(|| provider_toml_string(raw_config, section_key, "usage_url"))
        .or_else(|| provider_toml_string(raw_config, section_key, "usage_endpoint"));
    let api_key = provider_toml_string(raw_config, section_key, "experimental_bearer_token")
        .and_then(usable_api_key)
        .or_else(|| {
            provider_toml_string(raw_config, section_key, "api_key_env_var")
                .and_then(|name| std::env::var(name).ok())
                .and_then(usable_api_key)
        })
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .and_then(usable_api_key)
        })
        .or_else(|| auth_path.and_then(load_api_key_from_auth_path))?;
    if api_key.trim().is_empty() {
        return None;
    }
    let provider_name = if (source_hint == Some("Codex++") || is_codex_plus_proxy_url(&base_url))
        && provider_name.eq_ignore_ascii_case("custom")
    {
        "Codex++".into()
    } else {
        provider_name
    };
    Some(ApiConfig {
        provider_name,
        base_url,
        balance_url,
        api_key,
    })
}

fn load_api_config() -> Option<ApiConfig> {
    if let Some(path) = config_path() {
        let raw_config = fs::read_to_string(path).unwrap_or_default();
        if let Some(config) = load_api_config_from_raw(&raw_config, auth_path().as_ref(), None) {
            return Some(config);
        }
        if let Some(config) = load_codex_plus_api_config(Some(&raw_config)) {
            return Some(config);
        }
    }

    if let Some(config) = load_codex_plus_api_config(None) {
        return Some(config);
    }

    for dir in codex_plus_backup_dirs() {
        let config_path = dir.join("config.toml");
        let auth_path = dir.join("auth.json");
        let Ok(raw_config) = fs::read_to_string(config_path) else {
            continue;
        };
        if let Some(config) =
            load_api_config_from_raw(&raw_config, Some(&auth_path), Some("Codex++"))
        {
            return Some(config);
        }
    }

    None
}

fn headers(auth: &Auth) -> Result<HeaderMap, &'static str> {
    let mut result = HeaderMap::new();
    let mut bearer = HeaderValue::from_str(&format!("Bearer {}", auth.access_token))
        .map_err(|_| "Codex login data is invalid.")?;
    bearer.set_sensitive(true);
    result.insert(AUTHORIZATION, bearer);
    result.insert(ACCEPT, HeaderValue::from_static("application/json"));
    result.insert("originator", HeaderValue::from_static("Codex Desktop"));
    result.insert("OAI-Product-Sku", HeaderValue::from_static("CODEX"));
    if let Some(account_id) = &auth.account_id {
        let mut value =
            HeaderValue::from_str(account_id).map_err(|_| "Account identifier is invalid.")?;
        value.set_sensitive(true);
        result.insert("ChatGPT-Account-Id", value);
    }
    Ok(result)
}

fn number_with_key<'a>(value: &'a Value, keys: &[&'a str]) -> Option<(&'a str, f64)> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_f64().map(|number| (*key, number)))
}

fn integer(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|item| u64::try_from(item).ok()))
    })
}

fn timestamp(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let item = value.get(*key)?;
        if let Some(text) = item.as_str() {
            return Some(text.to_owned());
        }
        item.as_i64()
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|time| time.to_rfc3339())
    })
}

fn collect_reset_credit_expirations(value: &Value) -> Vec<String> {
    fn visit(value: &Value, output: &mut Vec<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    visit(item, output);
                }
            }
            Value::Object(map) => {
                if let Some(time) = timestamp(
                    value,
                    &[
                        "expires_at",
                        "expiresAt",
                        "expiration_time",
                        "expirationTime",
                        "expires",
                    ],
                ) {
                    output.push(time);
                }
                for key in [
                    "credits",
                    "reset_credits",
                    "resetCredits",
                    "available",
                    "items",
                    "grants",
                ] {
                    if let Some(child) = map.get(key) {
                        visit(child, output);
                    }
                }
            }
            _ => {}
        }
    }

    let mut expirations = Vec::new();
    visit(value, &mut expirations);
    expirations.sort();
    expirations.dedup();
    expirations
}

fn scale_ratio_field(key: &str, value: f64) -> bool {
    matches!(
        key,
        "remaining_ratio" | "remainingRatio" | "used_ratio" | "usedRatio" | "utilization"
    ) || (!key.contains("percent") && !key.contains("pct") && value <= 1.0)
}

fn parse_window(value: Option<&Value>) -> Option<UsageWindow> {
    let value = value?;
    let remaining_percent = if let Some((key, remaining)) = number_with_key(
        value,
        &[
            "remaining_percent",
            "remainingPercent",
            "remaining_pct",
            "remainingPct",
            "remaining_ratio",
            "remainingRatio",
            "remaining",
        ],
    ) {
        if scale_ratio_field(key, remaining) {
            remaining * 100.0
        } else {
            remaining
        }
    } else {
        let (key, used) = number_with_key(
            value,
            &[
                "used_percent",
                "usedPercent",
                "used_pct",
                "usedPct",
                "used_ratio",
                "usedRatio",
                "utilization",
                "used",
            ],
        )?;
        let used_percent = if scale_ratio_field(key, used) {
            used * 100.0
        } else {
            used
        };
        100.0 - used_percent
    };
    Some(UsageWindow {
        remaining_percent: remaining_percent.clamp(0.0, 100.0),
        resets_at: timestamp(
            value,
            &[
                "reset_at",
                "resetAt",
                "resets_at",
                "resetsAt",
                "reset_time",
                "resetTime",
            ],
        ),
        window_seconds: integer(
            value,
            &[
                "limit_window_seconds",
                "limitWindowSeconds",
                "window_seconds",
                "windowSeconds",
                "duration_seconds",
                "durationSeconds",
                "period_seconds",
                "periodSeconds",
            ],
        )
        .unwrap_or(0),
    })
}

fn timestamp_from_seconds(seconds: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(seconds, 0).map(|time| time.to_rfc3339())
}

fn parse_app_server_window(value: Option<&Value>) -> Option<UsageWindow> {
    let value = value?;
    let used_percent = number_with_key(value, &["usedPercent", "used_percent", "used"])
        .map(|(_, number)| number)?;
    Some(UsageWindow {
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        resets_at: value
            .get("resetsAt")
            .or_else(|| value.get("resets_at"))
            .and_then(Value::as_i64)
            .and_then(timestamp_from_seconds),
        window_seconds: integer(value, &["windowDurationMins", "window_duration_mins"])
            .unwrap_or(0)
            * 60,
    })
}

fn app_server_rate_limit<'a>(value: &'a Value) -> Option<&'a Value> {
    if let Some(items) = value
        .get("rateLimitsByLimitId")
        .or_else(|| value.get("rate_limits_by_limit_id"))
        .and_then(Value::as_object)
    {
        if let Some(codex) = items.get("codex") {
            return Some(codex);
        }
        if let Some((_, codex)) = items.iter().find(|(_, item)| {
            pick_string(item, &["limitId", "limit_id"])
                .map(|limit_id| limit_id.eq_ignore_ascii_case("codex"))
                .unwrap_or(false)
        }) {
            return Some(codex);
        }
        if let Some(first) = items.values().next() {
            return Some(first);
        }
    }
    value.get("rateLimits").or_else(|| value.get("rate_limits"))
}

fn app_server_reset_credits(value: &Value) -> (Option<u64>, Vec<String>) {
    let Some(summary) = value
        .get("rateLimitResetCredits")
        .or_else(|| value.get("rate_limit_reset_credits"))
    else {
        return (None, Vec::new());
    };
    let count = integer(summary, &["availableCount", "available_count"]);
    let expirations = summary
        .get("credits")
        .and_then(Value::as_array)
        .map(|credits| {
            let mut values = credits
                .iter()
                .filter_map(|credit| {
                    credit
                        .get("expiresAt")
                        .or_else(|| credit.get("expires_at"))
                        .and_then(Value::as_i64)
                        .and_then(timestamp_from_seconds)
                })
                .collect::<Vec<_>>();
            values.sort();
            values.dedup();
            values
        })
        .unwrap_or_default();
    (count, expirations)
}

fn app_server_balance(rate_limit: &Value) -> Option<String> {
    let value = rate_limit.get("credits")?.get("balance")?;
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .or_else(|| value.as_f64().map(|number| number.to_string()))
}

fn app_server_snapshot(value: &Value) -> Option<ProviderSnapshot> {
    let rate_limit = app_server_rate_limit(value)?;
    let short_window = parse_app_server_window(rate_limit.get("primary"));
    let weekly_window = parse_app_server_window(rate_limit.get("secondary"));
    let balance = app_server_balance(rate_limit);
    if short_window.is_none() && weekly_window.is_none() && balance.is_none() {
        return None;
    }
    let (reset_credits, reset_credit_expires_at) = app_server_reset_credits(value);
    Some(ProviderSnapshot {
        provider: "codex".into(),
        display_name: "CODEX".into(),
        plan: pick_string(rate_limit, &["planType", "plan_type"])
            .map(|value| value.to_uppercase())
            .or_else(|| balance.as_ref().map(|_| "API".into())),
        short_window,
        weekly_window,
        balance,
        balance_label: None,
        balance_percent: None,
        balance_source_key: None,
        reset_credits,
        reset_credit_expires_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
        status: "ok".into(),
        message: None,
    })
}

fn find_window<'a>(
    rate_limit: &'a Value,
    names: &[&str],
    expected_seconds: u64,
) -> Option<&'a Value> {
    for name in names {
        if let Some(value) = rate_limit.get(*name) {
            let Some(window) = parse_window(Some(value)) else {
                continue;
            };
            if window.window_seconds == 0
                || (expected_seconds > 0 && window.window_seconds.abs_diff(expected_seconds) <= 60)
            {
                return Some(value);
            }
        }
    }

    for key in [
        "windows",
        "limit_windows",
        "limitWindows",
        "limits",
        "buckets",
    ] {
        let Some(items) = rate_limit.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(window) = parse_window(Some(item)) else {
                continue;
            };
            let matches_duration =
                expected_seconds > 0 && window.window_seconds.abs_diff(expected_seconds) <= 60;
            let matches_name = pick_string(item, &["name", "type", "id", "window", "label"])
                .map(|text| {
                    let lower = text.to_ascii_lowercase();
                    names.iter().any(|name| {
                        lower == name.to_ascii_lowercase()
                            || lower.contains(&name.to_ascii_lowercase())
                    })
                })
                .unwrap_or(false);
            if matches_duration || matches_name {
                return Some(item);
            }
        }
    }

    None
}

fn safe_http_failure(status: reqwest::StatusCode) -> (&'static str, &'static str) {
    match status.as_u16() {
        401 | 403 => ("signed_out", "Codex login expired. Please sign in again."),
        429 => (
            "unavailable",
            "Quota service is rate limited. It will retry automatically.",
        ),
        _ => ("unavailable", "Quota service is temporarily unavailable."),
    }
}

async fn limited_json(mut response: reqwest::Response) -> Result<Value, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_RESPONSE_BYTES {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

pub async fn fetch_snapshot(client: &reqwest::Client) -> ProviderSnapshot {
    if let Ok(snapshot) = fetch_snapshot_from_app_server().await {
        return snapshot;
    }
    if let Some(config) = load_api_config() {
        return fetch_snapshot_from_api_balance(client, &config).await;
    }
    fetch_snapshot_from_wham(client).await
}

fn trimmed_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

fn api_balance_urls(config: &ApiConfig) -> Vec<String> {
    let base_url = &config.base_url;
    let base = trimmed_url(base_url);
    if base.is_empty() {
        return Vec::new();
    }
    let root = base
        .strip_suffix("/v1")
        .or_else(|| base.strip_suffix("/V1"))
        .unwrap_or(&base)
        .to_owned();
    let mut urls = config
        .balance_url
        .as_ref()
        .map(|url| vec![url.clone()])
        .unwrap_or_default();
    let v1_base = format!("{root}/v1");
    for prefix in [base, v1_base, root] {
        for path in [
            "dashboard/billing/credit_grants",
            "billing/credit_grants",
            "usage",
            "api/user/self",
            "credit_grants",
            "credits",
            "user/balance",
            "balance",
        ] {
            let url = format!("{prefix}/{path}");
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

fn cc_switch_status_url(base_url: &str) -> Option<String> {
    let base = trimmed_url(base_url);
    if !(base.contains("127.0.0.1") || base.contains("localhost")) {
        return None;
    }
    let root = base
        .strip_suffix("/v1")
        .or_else(|| base.strip_suffix("/V1"))
        .unwrap_or(&base);
    Some(format!("{root}/status"))
}

fn numeric_balance(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{}", value as i64)
    } else {
        format!("{value:.2}")
    }
}

fn direct_balance_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        }
        Value::Number(number) => number.as_f64().map(numeric_balance),
        _ => None,
    }
}

fn direct_number_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => {
            let text = text.trim().replace([',', '$'], "");
            text.split_whitespace()
                .find_map(|part| part.parse::<f64>().ok())
                .or_else(|| text.parse::<f64>().ok())
        }
        _ => None,
    }
}

fn number_from_keys(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(direct_number_value))
}

fn balance_percent_from_json(value: &Value) -> Option<f64> {
    if let Value::Object(map) = value {
        let remaining = number_from_keys(
            value,
            &[
                "balance",
                "remaining",
                "remaining_balance",
                "remainingBalance",
                "quota_remaining",
                "quotaRemaining",
                "available_balance",
                "availableBalance",
                "total_available",
                "totalAvailable",
            ],
        );
        let total = number_from_keys(
            value,
            &[
                "total",
                "total_balance",
                "totalBalance",
                "credit_limit",
                "creditLimit",
                "limit",
            ],
        );
        let used = number_from_keys(
            value,
            &["used", "used_balance", "usedBalance", "usage", "spent"],
        );
        if let Some(remaining) = remaining {
            if let Some(total) = total.filter(|value| *value > 0.0) {
                return Some((remaining / total * 100.0).clamp(0.0, 100.0));
            }
            if let Some(used) = used.filter(|value| *value > 0.0) {
                return Some((remaining / (remaining + used) * 100.0).clamp(0.0, 100.0));
            }
        }
        for key in ["data", "result", "credit_grants", "creditGrants"] {
            if let Some(percent) = map.get(key).and_then(balance_percent_from_json) {
                return Some(percent);
            }
        }
    }
    None
}

fn balance_from_json(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &[
        "total_available",
        "totalAvailable",
        "available_balance",
        "availableBalance",
        "balance",
        "quota_remaining",
        "quotaRemaining",
        "remaining_quota",
        "remainingQuota",
        "remaining_balance",
        "remainingBalance",
        "remaining",
        "remain_quota",
        "remainQuota",
        "quota",
        "credits",
        "credit",
        "amount",
    ];

    if let Value::Object(map) = value {
        for key in KEYS {
            if let Some(balance) = map.get(*key).and_then(direct_balance_value) {
                return Some(balance);
            }
        }
        for key in ["data", "result", "credit_grants", "creditGrants"] {
            if let Some(balance) = map.get(key).and_then(balance_from_json) {
                return Some(balance);
            }
        }
        for child in map.values() {
            if let Some(balance) = balance_from_json(child) {
                return Some(balance);
            }
        }
    } else if let Value::Array(items) = value {
        for item in items {
            if let Some(balance) = balance_from_json(item) {
                return Some(balance);
            }
        }
    }
    None
}

fn api_failure(config: &ApiConfig, status: &str, message: &str) -> ProviderSnapshot {
    ProviderSnapshot {
        provider: "codex".into(),
        display_name: "CODEX".into(),
        plan: Some(format!("API · {}", config.provider_name)),
        short_window: None,
        weekly_window: None,
        balance: None,
        balance_label: None,
        balance_percent: None,
        balance_source_key: None,
        reset_credits: None,
        reset_credit_expires_at: Vec::new(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        status: status.into(),
        message: Some(message.into()),
    }
}

async fn fetch_snapshot_from_api_balance(
    client: &reqwest::Client,
    config: &ApiConfig,
) -> ProviderSnapshot {
    let mut headers = HeaderMap::new();
    let mut bearer = match HeaderValue::from_str(&format!("Bearer {}", config.api_key)) {
        Ok(value) => value,
        Err(_) => return api_failure(config, "signed_out", "API key format is invalid."),
    };
    bearer.set_sensitive(true);
    headers.insert(AUTHORIZATION, bearer);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("CC Switch"));
    if let Ok(mut value) = HeaderValue::from_str(&config.api_key) {
        value.set_sensitive(true);
        headers.insert("x-api-key", value);
    }

    let mut saw_success = false;
    for url in api_balance_urls(config) {
        let response = match client.get(url).headers(headers.clone()).send().await {
            Ok(response) => response,
            Err(_) => continue,
        };
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            return api_failure(
                config,
                "signed_out",
                "API key was rejected by the provider.",
            );
        }
        if !response.status().is_success() {
            continue;
        }
        saw_success = true;
        let Ok(value) = limited_json(response).await else {
            continue;
        };
        if let Some(balance) = balance_from_json(&value) {
            let balance_percent = balance_percent_from_json(&value);
            return ProviderSnapshot {
                provider: "codex".into(),
                display_name: "CODEX".into(),
                plan: Some(format!("API · {}", config.provider_name)),
                short_window: None,
                weekly_window: None,
                balance: Some(balance),
                balance_label: None,
                balance_percent,
                balance_source_key: Some(balance_source_key(&[
                    &config.provider_name,
                    &config.base_url,
                    &config.api_key,
                ])),
                reset_credits: None,
                reset_credit_expires_at: Vec::new(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                status: "ok".into(),
                message: None,
            };
        }
    }

    let mut saw_cc_switch = false;
    if let Some(url) = cc_switch_status_url(&config.base_url) {
        if let Ok(response) = client.get(url).headers(headers.clone()).send().await {
            saw_cc_switch = response.status().is_success();
        }
    }

    if saw_cc_switch {
        api_failure(
            config,
            "unavailable",
            "CC Switch is connected, but no USD balance endpoint was detected. Enable Usage Query in CC Switch or set balance_url in Codex config.",
        )
    } else if saw_success {
        api_failure(
            config,
            "unavailable",
            "API balance response did not contain a recognized balance field.",
        )
    } else {
        api_failure(
            config,
            "unavailable",
            "API key is connected, but this provider does not expose a supported balance endpoint.",
        )
    }
}

async fn write_app_server_message(
    stdin: &mut tokio::process::ChildStdin,
    value: Value,
) -> Result<(), ()> {
    stdin
        .write_all(value.to_string().as_bytes())
        .await
        .map_err(|_| ())?;
    stdin.write_all(b"\n").await.map_err(|_| ())?;
    stdin.flush().await.map_err(|_| ())
}

fn response_id_is(value: &Value, id: u64) -> bool {
    let expected = id.to_string();
    value.get("id").and_then(Value::as_u64) == Some(id)
        || value.get("id").and_then(Value::as_str) == Some(expected.as_str())
}

async fn read_app_server_response(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
) -> Result<Value, ()> {
    while let Some(line) = lines.next_line().await.map_err(|_| ())? {
        let value: Value = serde_json::from_str(&line).map_err(|_| ())?;
        if !response_id_is(&value, id) {
            continue;
        }
        if value.get("error").is_some() {
            return Err(());
        }
        return value.get("result").cloned().ok_or(());
    }
    Err(())
}

async fn read_app_server_rate_limits() -> Result<Value, ()> {
    let mut child = Command::new("codex")
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    let mut stdin = child.stdin.take().ok_or(())?;
    let stdout = child.stdout.take().ok_or(())?;
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        write_app_server_message(
            &mut stdin,
            serde_json::json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {
                        "name": "quota_float",
                        "title": "Quota Float",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": { "experimentalApi": true }
                }
            }),
        )
        .await?;
        read_app_server_response(&mut lines, 1).await?;
        write_app_server_message(&mut stdin, serde_json::json!({ "method": "initialized" }))
            .await?;
        write_app_server_message(
            &mut stdin,
            serde_json::json!({
                "method": "account/rateLimits/read",
                "id": 2,
                "params": null
            }),
        )
        .await?;
        read_app_server_response(&mut lines, 2).await
    }
    .await;

    let _ = child.kill().await;
    result
}

async fn fetch_snapshot_from_app_server() -> Result<ProviderSnapshot, ()> {
    let value = timeout(APP_SERVER_TIMEOUT, read_app_server_rate_limits())
        .await
        .map_err(|_| ())??;
    app_server_snapshot(&value).ok_or(())
}

async fn fetch_snapshot_from_wham(client: &reqwest::Client) -> ProviderSnapshot {
    let auth = match load_auth() {
        Ok(value) => value,
        Err(message) => return ProviderSnapshot::failure("signed_out", message),
    };
    let request_headers = match headers(&auth) {
        Ok(value) => value,
        Err(message) => return ProviderSnapshot::failure("signed_out", message),
    };

    let (usage_result, credits_result) = tokio::join!(
        client
            .get(USAGE_URL)
            .headers(request_headers.clone())
            .send(),
        client.get(CREDITS_URL).headers(request_headers).send(),
    );

    let usage_response = match usage_result {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            let (status, message) = safe_http_failure(response.status());
            return ProviderSnapshot::failure(status, message);
        }
        Err(_) => {
            return ProviderSnapshot::failure(
                "unavailable",
                "Network unavailable. It will retry automatically.",
            )
        }
    };
    let usage: Value = match limited_json(usage_response).await {
        Ok(value) => value,
        Err(_) => {
            return ProviderSnapshot::failure("unavailable", "Quota response format has changed.")
        }
    };
    let rate_limit = usage
        .get("rate_limit")
        .or_else(|| usage.get("rateLimit"))
        .unwrap_or(&usage);
    let short_window = parse_window(find_window(
        rate_limit,
        &[
            "primary_window",
            "primaryWindow",
            "short_window",
            "shortWindow",
            "five_hour_window",
            "fiveHourWindow",
            "5h",
            "primary",
        ],
        18_000,
    ));
    let weekly_window = parse_window(find_window(
        rate_limit,
        &[
            "secondary_window",
            "secondaryWindow",
            "weekly_window",
            "weeklyWindow",
            "week_window",
            "weekWindow",
            "weekly",
            "secondary",
            "primary_window",
            "primaryWindow",
            "primary",
        ],
        604_800,
    ));
    if short_window.is_none() && weekly_window.is_none() {
        return ProviderSnapshot::failure(
            "unavailable",
            "Quota response does not contain a recognized usage window.",
        );
    }

    let usage_credits = usage
        .get("rate_limit_reset_credits")
        .or_else(|| usage.get("rateLimitResetCredits"));
    let usage_reset_credits = usage_credits.and_then(|value| {
        integer(
            value,
            &[
                "available_count",
                "availableCount",
                "remaining",
                "count",
                "quantity",
            ],
        )
    });
    let usage_reset_credit_expires_at = usage_credits
        .map(collect_reset_credit_expirations)
        .unwrap_or_default();

    let (reset_credits, reset_credit_expires_at) = match credits_result {
        Ok(response) if response.status().is_success() => match limited_json(response).await.ok() {
            Some(value) => (
                integer(
                    &value,
                    &[
                        "available_count",
                        "availableCount",
                        "remaining",
                        "count",
                        "quantity",
                    ],
                )
                .or(usage_reset_credits),
                {
                    let expirations = collect_reset_credit_expirations(&value);
                    if expirations.is_empty() {
                        usage_reset_credit_expires_at
                    } else {
                        expirations
                    }
                },
            ),
            None => (usage_reset_credits, usage_reset_credit_expires_at),
        },
        _ => (usage_reset_credits, usage_reset_credit_expires_at),
    };

    ProviderSnapshot {
        provider: "codex".into(),
        display_name: "CODEX".into(),
        plan: pick_string(&usage, &["plan_type", "planType"]).map(|value| value.to_uppercase()),
        short_window,
        weekly_window,
        balance: None,
        balance_label: None,
        balance_percent: None,
        balance_source_key: None,
        reset_credits,
        reset_credit_expires_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
        status: "ok".into(),
        message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_window_shapes() {
        let snake = serde_json::json!({
            "used_percent": 26,
            "reset_at": 1738300000,
            "limit_window_seconds": 18000
        });
        let window = parse_window(Some(&snake)).unwrap();
        assert_eq!(window.remaining_percent, 74.0);
        assert_eq!(window.window_seconds, 18000);
        let camel = serde_json::json!({
            "utilization": 0.4,
            "resetsAt": "2026-07-07T00:00:00Z",
            "windowSeconds": 604800
        });
        assert_eq!(parse_window(Some(&camel)).unwrap().remaining_percent, 60.0);
    }

    #[test]
    fn prefers_explicit_remaining_percent() {
        let value = serde_json::json!({
            "remainingPercent": 73.4,
            "usedPercent": 99,
            "resetTime": "2026-07-07T00:00:00Z",
            "durationSeconds": 18000
        });
        let window = parse_window(Some(&value)).unwrap();
        assert_eq!(window.remaining_percent, 73.4);
        assert_eq!(window.window_seconds, 18000);
    }

    #[test]
    fn treats_fractional_percent_fields_as_ratios() {
        let explicit_remaining = serde_json::json!({"remaining": 0.25, "periodSeconds": 18000});
        assert_eq!(
            parse_window(Some(&explicit_remaining))
                .unwrap()
                .remaining_percent,
            25.0
        );

        let used_ratio = serde_json::json!({"used": 0.25, "periodSeconds": 18000});
        assert_eq!(
            parse_window(Some(&used_ratio)).unwrap().remaining_percent,
            75.0
        );
    }

    #[test]
    fn does_not_scale_explicit_percent_fields() {
        let explicit_remaining =
            serde_json::json!({"remaining_percent": 0.4, "windowSeconds": 18000});
        assert_eq!(
            parse_window(Some(&explicit_remaining))
                .unwrap()
                .remaining_percent,
            0.4
        );

        let explicit_used = serde_json::json!({"used_percent": 0.4, "windowSeconds": 18000});
        assert_eq!(
            parse_window(Some(&explicit_used))
                .unwrap()
                .remaining_percent,
            99.6
        );
    }

    #[test]
    fn finds_window_by_duration_or_name_in_arrays() {
        let rate_limit = serde_json::json!({
            "windows": [
                {"name": "weekly", "remainingPercent": 88, "windowSeconds": 604800},
                {"name": "primary", "remainingPercent": 51, "windowSeconds": 18000}
            ]
        });
        let short = parse_window(find_window(
            &rate_limit,
            &["primary_window", "primary"],
            18_000,
        ))
        .unwrap();
        let weekly = parse_window(find_window(
            &rate_limit,
            &["secondary_window", "weekly"],
            604_800,
        ))
        .unwrap();
        assert_eq!(short.remaining_percent, 51.0);
        assert_eq!(weekly.remaining_percent, 88.0);
    }

    #[test]
    fn does_not_treat_a_weekly_primary_field_as_a_short_window() {
        let value = serde_json::json!({
            "primary_window": {"remainingPercent": 98, "windowSeconds": 604800},
            "weekly_window": {"remainingPercent": 98, "windowSeconds": 604800}
        });
        assert!(find_window(&value, &["primary_window", "primary"], 18_000).is_none());
        assert!(find_window(&value, &["weekly_window", "weekly"], 604_800).is_some());
    }

    #[test]
    fn recognizes_a_weekly_primary_field_as_weekly_fallback() {
        let value = serde_json::json!({
            "primary": {"remainingPercent": 98, "windowSeconds": 604800}
        });
        let weekly = parse_window(find_window(
            &value,
            &["weekly_window", "weekly", "primary_window", "primary"],
            604_800,
        ))
        .unwrap();
        assert_eq!(weekly.remaining_percent, 98.0);
        assert_eq!(weekly.window_seconds, 604_800);
    }

    #[test]
    fn parses_app_server_rate_limit_response() {
        let value = serde_json::json!({
            "rateLimits": {
                "limitId": "other",
                "primary": {"usedPercent": 90, "windowDurationMins": 300}
            },
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "planType": "pro",
                    "primary": {
                        "usedPercent": 25,
                        "windowDurationMins": 300,
                        "resetsAt": 1738300000
                    },
                    "secondary": {
                        "usedPercent": 60,
                        "windowDurationMins": 10080,
                        "resetsAt": 1739000000
                    },
                    "credits": {
                        "hasCredits": true,
                        "unlimited": false,
                        "balance": "$12.34"
                    }
                }
            },
            "rateLimitResetCredits": {
                "availableCount": 2,
                "credits": [
                    {"expiresAt": 1739100000},
                    {"expiresAt": null}
                ]
            }
        });
        let snapshot = app_server_snapshot(&value).unwrap();
        assert_eq!(snapshot.plan.as_deref(), Some("PRO"));
        assert_eq!(snapshot.short_window.unwrap().remaining_percent, 75.0);
        assert_eq!(snapshot.weekly_window.unwrap().window_seconds, 604_800);
        assert_eq!(snapshot.balance.as_deref(), Some("$12.34"));
        assert_eq!(snapshot.reset_credits, Some(2));
        assert_eq!(
            snapshot.reset_credit_expires_at,
            vec![timestamp_from_seconds(1739100000).unwrap()]
        );
    }

    #[test]
    fn parses_app_server_balance_without_quota_windows() {
        let value = serde_json::json!({
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "credits": {
                        "hasCredits": true,
                        "unlimited": false,
                        "balance": "$4.56"
                    }
                }
            }
        });
        let snapshot = app_server_snapshot(&value).unwrap();
        assert!(snapshot.short_window.is_none());
        assert!(snapshot.weekly_window.is_none());
        assert_eq!(snapshot.plan.as_deref(), Some("API"));
        assert_eq!(snapshot.balance.as_deref(), Some("$4.56"));
    }

    #[test]
    fn ignores_proxy_managed_as_api_key() {
        assert!(usable_api_key("PROXY_MANAGED".into()).is_none());
        assert_eq!(
            usable_api_key(" sk-test ".into()).as_deref(),
            Some("sk-test")
        );
    }

    #[test]
    fn parses_cc_switch_usage_query_config() {
        let settings = serde_json::json!({
            "auth": {"OPENAI_API_KEY": "sk-test"},
            "config": "model_provider = \"custom\"\n[model_providers.custom]\nname = \"RuncodeAI\"\nbase_url = \"https://app.runcode.win\"\n"
        });
        let meta = serde_json::json!({
            "usage_script": {
                "enabled": true,
                "code": "({ request: { url: \"{{baseUrl}}/v1/usage\", method: \"GET\", headers: { \"Authorization\": \"Bearer {{apiKey}}\" } } })"
            }
        });
        let config = cc_switch_api_config_from_values("RuncodeAI", &settings, &meta).unwrap();
        assert_eq!(config.provider_name, "RuncodeAI");
        assert_eq!(config.base_url, "https://app.runcode.win");
        assert_eq!(
            config.balance_url.as_deref(),
            Some("https://app.runcode.win/v1/usage")
        );
        assert_eq!(config.api_key, "sk-test");
    }

    #[test]
    fn parses_codex_plus_top_level_relay_config() {
        let settings = serde_json::json!({
            "relayBaseUrl": "https://app.runcode.win",
            "relayApiKey": "sk-test"
        });
        let raw_config = "model_provider = \"custom\"\n[model_providers.custom]\nname = \"RuncodeAI\"\nbase_url = \"https://app.runcode.win\"\n";
        let config = codex_plus_api_config_from_settings(&settings, Some(raw_config)).unwrap();
        assert_eq!(config.provider_name, "RuncodeAI");
        assert_eq!(config.base_url, "https://app.runcode.win");
        assert_eq!(config.api_key, "sk-test");
    }

    #[test]
    fn parses_codex_plus_active_relay_profile_auth_contents() {
        let settings = serde_json::json!({
            "activeRelayId": "relay-a",
            "relayProfiles": [
                {"id": "default", "relayMode": "official"},
                {
                    "id": "relay-a",
                    "name": "30",
                    "upstreamBaseUrl": "https://app.runcode.win",
                    "authContents": "{\"OPENAI_API_KEY\":\"sk-test\"}",
                    "configContents": "model_provider = \"custom\"\n[model_providers.custom]\nname = \"RuncodeAI\"\nbase_url = \"https://app.runcode.win\"\n"
                }
            ]
        });
        let config = codex_plus_api_config_from_settings(&settings, None).unwrap();
        assert_eq!(config.provider_name, "RuncodeAI");
        assert_eq!(config.base_url, "https://app.runcode.win");
        assert_eq!(config.api_key, "sk-test");
    }

    #[test]
    fn ignores_cc_switch_request_stats_as_balance() {
        let status = serde_json::json!({
            "running": true,
            "current_provider": "RuncodeAI",
            "total_requests": 3144
        });
        assert!(balance_from_json(&status).is_none());
    }

    #[test]
    fn parses_common_api_balance_shapes() {
        let credit_grants = serde_json::json!({
            "object": "credit_summary",
            "total_available": 12.345
        });
        assert_eq!(balance_from_json(&credit_grants).as_deref(), Some("12.35"));

        let nested = serde_json::json!({
            "data": {
                "availableBalance": "$8.90"
            }
        });
        assert_eq!(balance_from_json(&nested).as_deref(), Some("$8.90"));

        let usage_query = serde_json::json!({
            "data": {
                "quotaRemaining": 7.5,
                "used": 2.5,
                "total": 10,
                "unit": "USD"
            }
        });
        assert_eq!(balance_from_json(&usage_query).as_deref(), Some("7.50"));
        assert_eq!(balance_percent_from_json(&usage_query), Some(75.0));

        let runcode_usage = serde_json::json!({
            "balance": 25.0,
            "daily_usage": [
                {"actual_cost": 25.0},
                {"cost": 50.0}
            ]
        });
        assert_eq!(balance_from_json(&runcode_usage).as_deref(), Some("25"));
        assert_eq!(balance_percent_from_json(&runcode_usage), None);
    }

    #[test]
    fn builds_api_balance_urls_from_v1_base() {
        let config = ApiConfig {
            provider_name: "RuncodeAI".into(),
            base_url: "http://127.0.0.1:15721/v1/".into(),
            balance_url: None,
            api_key: "sk-test".into(),
        };
        let urls = api_balance_urls(&config);
        assert!(urls.contains(&"http://127.0.0.1:15721/v1/dashboard/billing/credit_grants".into()));
        assert!(urls.contains(&"http://127.0.0.1:15721/v1/api/user/self".into()));
        assert!(urls.contains(&"http://127.0.0.1:15721/dashboard/billing/credit_grants".into()));
        assert!(urls.contains(&"http://127.0.0.1:15721/api/user/self".into()));
    }

    #[test]
    fn builds_api_balance_urls_from_root_base() {
        let config = ApiConfig {
            provider_name: "RuncodeAI".into(),
            base_url: "https://app.runcode.win".into(),
            balance_url: None,
            api_key: "sk-test".into(),
        };
        let urls = api_balance_urls(&config);
        assert!(urls.contains(&"https://app.runcode.win/v1/usage".into()));
    }

    #[test]
    fn custom_balance_url_is_tried_first() {
        let config = ApiConfig {
            provider_name: "Custom".into(),
            base_url: "https://example.com/v1".into(),
            balance_url: Some("https://example.com/custom-balance".into()),
            api_key: "sk-test".into(),
        };
        assert_eq!(
            api_balance_urls(&config).first().map(String::as_str),
            Some("https://example.com/custom-balance")
        );
    }
}
