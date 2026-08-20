use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::Connection;
use serde_json::{Map, Value};

/// A single Claude Code provider profile stored in Codex++ settings.
/// The `auth_token` field is stored plaintext — same security model as
/// ~/.codex/auth.json.  `skip_serializing` is intentionally NOT applied
/// because we need the token to survive Codex++ restarts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub auth_token: String,
    /// Maps to ANTHROPIC_DEFAULT_HAIKU_MODEL env var
    #[serde(default)]
    pub model_haiku: String,
    /// Maps to ANTHROPIC_DEFAULT_SONNET_MODEL env var
    #[serde(default)]
    pub model_sonnet: String,
    /// Maps to ANTHROPIC_DEFAULT_OPUS_MODEL env var
    #[serde(default)]
    pub model_opus: String,
    /// The value for the top-level "model" key in ~/.claude/settings.json
    /// Expected values: "haiku" | "sonnet" | "opus"
    #[serde(default = "default_claude_model")]
    pub default_model: String,
}

fn default_claude_model() -> String {
    "haiku".to_string()
}

impl Default for ClaudeProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            auth_token: String::new(),
            model_haiku: String::new(),
            model_sonnet: String::new(),
            model_opus: String::new(),
            default_model: default_claude_model(),
        }
    }
}

/// Status of the current ~/.claude/settings.json as reported to the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStatus {
    /// Whether ~/.claude/settings.json exists
    pub settings_exists: bool,
    /// Current ANTHROPIC_AUTH_TOKEN (empty string = not set or hidden; we show
    /// only first 8 chars + "…" so the UI can confirm which key is active)
    pub active_token_preview: String,
    /// Current ANTHROPIC_BASE_URL
    pub active_base_url: String,
    /// Current ANTHROPIC_DEFAULT_HAIKU_MODEL
    pub active_model_haiku: String,
    /// Current ANTHROPIC_DEFAULT_SONNET_MODEL
    pub active_model_sonnet: String,
    /// Current ANTHROPIC_DEFAULT_OPUS_MODEL
    pub active_model_opus: String,
    /// Current top-level "model" value
    pub active_model: String,
}

/// Read ~/.claude/settings.json and return its status for display.
pub fn read_claude_status() -> ClaudeStatus {
    let path = claude_settings_path();
    let Ok(contents) = fs::read_to_string(&path) else {
        return ClaudeStatus {
            settings_exists: false,
            active_token_preview: String::new(),
            active_base_url: String::new(),
            active_model_haiku: String::new(),
            active_model_sonnet: String::new(),
            active_model_opus: String::new(),
            active_model: String::new(),
        };
    };
    let obj: Value = serde_json::from_str(&contents).unwrap_or(Value::Object(Map::new()));
    let env = obj.get("env").and_then(Value::as_object).cloned().unwrap_or_default();

    let token = env_str(&env, "ANTHROPIC_AUTH_TOKEN").unwrap_or_default();
    let token_preview = if token.is_empty() {
        String::new()
    } else if token.len() <= 8 {
        format!("{}…", &token)
    } else {
        format!("{}…", &token[..8])
    };

    ClaudeStatus {
        settings_exists: true,
        active_token_preview: token_preview,
        active_base_url: env_str(&env, "ANTHROPIC_BASE_URL").unwrap_or_default(),
        active_model_haiku: env_str(&env, "ANTHROPIC_DEFAULT_HAIKU_MODEL").unwrap_or_default(),
        active_model_sonnet: env_str(&env, "ANTHROPIC_DEFAULT_SONNET_MODEL").unwrap_or_default(),
        active_model_opus: env_str(&env, "ANTHROPIC_DEFAULT_OPUS_MODEL").unwrap_or_default(),
        active_model: obj.get("model").and_then(Value::as_str).unwrap_or("").to_string(),
    }
}

/// Write only the Claude-relevant fields into ~/.claude/settings.json,
/// preserving every other key (permissions, hooks, includeCoAuthoredBy, …).
/// Creates the file and parent directories if they don't exist.
/// Backs up the existing file to ~/.claude/settings.json.bak before writing.
pub fn apply_claude_profile(profile: &ClaudeProfile) -> anyhow::Result<()> {
    let path = claude_settings_path();

    // Load or initialise the existing settings object
    let mut obj: Map<String, Value> = if path.exists() {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        // Backup before overwriting
        let bak = path.with_extension("json.bak");
        fs::write(&bak, &contents)
            .with_context(|| format!("failed to write backup {}", bak.display()))?;
        serde_json::from_str::<Value>(&contents)
            .ok()
            .and_then(|v| if let Value::Object(m) = v { Some(m) } else { None })
            .unwrap_or_default()
    } else {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        Map::new()
    };

    // Build the env sub-object, merging with what's already there
    let mut env = obj
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    set_or_remove(&mut env, "ANTHROPIC_AUTH_TOKEN", &profile.auth_token);
    set_or_remove(&mut env, "ANTHROPIC_BASE_URL", &profile.base_url);
    set_or_remove(&mut env, "ANTHROPIC_DEFAULT_HAIKU_MODEL", &profile.model_haiku);
    set_or_remove(&mut env, "ANTHROPIC_DEFAULT_SONNET_MODEL", &profile.model_sonnet);
    set_or_remove(&mut env, "ANTHROPIC_DEFAULT_OPUS_MODEL", &profile.model_opus);

    obj.insert("env".to_string(), Value::Object(env));

    // Only set "model" if the profile provides a value
    if !profile.default_model.is_empty() {
        obj.insert("model".to_string(), Value::String(profile.default_model.clone()));
    }

    let bytes = serde_json::to_vec_pretty(&Value::Object(obj))
        .context("failed to serialise claude settings")?;
    atomic_write(&path, &bytes)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(())
}

/// Import a single Claude provider from a cc-switch-style `settings_config`
/// JSON blob.  Returns `None` if the blob doesn't contain enough information.
pub fn claude_profile_from_ccs_value(
    source_id: &str,
    name: &str,
    config: &Value,
    existing_ids: &[String],
) -> Option<ClaudeProfile> {
    // Accept both direct ANTHROPIC_* env keys and the nested env object
    let env = config.get("env").and_then(Value::as_object).cloned().unwrap_or_default();

    let auth_token = env_str(&env, "ANTHROPIC_AUTH_TOKEN")
        .or_else(|| env_str(&env, "OPENAI_API_KEY"))  // fallback for chat-compat providers
        .or_else(|| config.get("apiKey").and_then(Value::as_str).map(str::to_string))
        .or_else(|| config.get("api_key").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();

    let base_url = env_str(&env, "ANTHROPIC_BASE_URL")
        .or_else(|| config.get("base_url").and_then(Value::as_str).map(|s| s.to_string()))
        .or_else(|| config.get("baseURL").and_then(Value::as_str).map(|s| s.to_string()))
        .unwrap_or_default();

    // Need at least a base_url to be useful
    if base_url.is_empty() && auth_token.is_empty() {
        return None;
    }

    let base_id = format!("ccs-claude-{}", sanitize_id(source_id));
    let id = unique_profile_id(&base_id, existing_ids);

    Some(ClaudeProfile {
        id,
        name: format!("{}（ccswitch）", name.trim()),
        base_url: base_url.trim().trim_end_matches('/').to_string(),
        auth_token,
        model_haiku: env_str(&env, "ANTHROPIC_DEFAULT_HAIKU_MODEL").unwrap_or_default(),
        model_sonnet: env_str(&env, "ANTHROPIC_DEFAULT_SONNET_MODEL").unwrap_or_default(),
        model_opus: env_str(&env, "ANTHROPIC_DEFAULT_OPUS_MODEL").unwrap_or_default(),
        default_model: config
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("haiku")
            .to_string(),
    })
}

/// Deduplication key for a ClaudeProfile — same logic as the Codex relay import.
pub fn claude_profile_identity(profile: &ClaudeProfile) -> String {
    claude_import_key(
        profile.name.trim()
            .strip_suffix("（ccswitch）")
            .or_else(|| profile.name.trim().strip_suffix("(ccswitch)"))
            .unwrap_or_else(|| profile.name.trim()),
        &profile.base_url,
    )
}

/// Deduplication key for a raw CCS import row (name + base_url).
pub fn claude_import_key_from_raw(name: &str, base_url: &str) -> String {
    claude_import_key(name, base_url)
}

/// Query cc-switch database for Claude-type providers (app_type = 'claude').
/// Returns a list of (source_id, name, settings_config_value) tuples.
pub fn list_claude_providers_from_ccs_db(
    path: &Path,
) -> anyhow::Result<Vec<(String, String, Value)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open cc-switch db {}", path.display()))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, settings_config
         FROM providers
         WHERE app_type = 'claude'
         ORDER BY COALESCE(sort_index, 999999), created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (id, name, cfg_str) = row?;
        if let Ok(v) = serde_json::from_str::<Value>(&cfg_str) {
            result.push((id, name, v));
        }
    }
    Ok(result)
}

/// Test connectivity to a Claude provider by issuing a GET /v1/models.
/// Returns Ok(http_status_code) on a network-level success (even 401 means reachable).
pub async fn test_claude_provider_connection(
    base_url: &str,
    auth_token: &str,
) -> anyhow::Result<u16> {
    let base = if base_url.is_empty() {
        "https://api.anthropic.com"
    } else {
        base_url.trim_end_matches('/')
    };
    let url = format!("{base}/v1/models");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build HTTP client")?;
    let mut req = client.get(&url);
    if !auth_token.is_empty() {
        req = req
            .header("x-api-key", auth_token)
            .header("anthropic-version", "2023-06-01");
    }
    let resp = req.send().await.context("network request failed")?;
    Ok(resp.status().as_u16())
}

/// Fetch the model list from a Claude provider's upstream `/models` endpoint.
/// Mirrors the Codex-side `model_catalog::fetch_relay_profile_model_ids`, but
/// sends Anthropic-style auth headers alongside the bearer token so both the
/// official Anthropic API and OpenAI-compatible relays answer the same request.
/// Returns `(model_ids, endpoint)`.
pub async fn fetch_claude_profile_model_ids(
    base_url: &str,
    auth_token: &str,
) -> anyhow::Result<(Vec<String>, String)> {
    let base = if base_url.trim().is_empty() {
        "https://api.anthropic.com"
    } else {
        base_url.trim()
    };
    let endpoint = crate::model_catalog::models_endpoint_for(base);
    if endpoint.is_empty() {
        anyhow::bail!("Base URL 无效");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")?;
    let mut req = client
        .get(&endpoint)
        .header(reqwest::header::ACCEPT, "application/json");
    let token = auth_token.trim();
    if !token.is_empty() {
        req = req
            .header("x-api-key", token)
            .header("anthropic-version", "2023-06-01")
            .bearer_auth(token);
    }
    let resp = req.send().await.context("网络请求失败")?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {}", status.as_u16());
    }
    let payload: Value = resp.json().await.context("上游响应不是合法 JSON")?;
    let models = crate::model_catalog::parse_model_ids(&payload);
    if models.is_empty() {
        anyhow::bail!("上游没有返回可用模型");
    }
    Ok((models, endpoint))
}

fn claude_settings_path() -> PathBuf {
    home_dir().join(".claude").join("settings.json")
}

fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn env_str(env: &Map<String, Value>, key: &str) -> Option<String> {
    env.get(key).and_then(Value::as_str).map(str::to_string)
}

fn set_or_remove(env: &mut Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() {
        env.remove(key);
    } else {
        env.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn claude_import_key(name: &str, base_url: &str) -> String {
    format!(
        "{}\n{}",
        name.trim().to_ascii_lowercase(),
        base_url.trim().trim_end_matches('/').to_ascii_lowercase()
    )
}

fn sanitize_id(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-').to_string();
    if result.is_empty() { "provider".to_string() } else { result }
}

fn unique_profile_id(base: &str, existing_ids: &[String]) -> String {
    if !existing_ids.iter().any(|id| id == base) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base}-{index}");
        if !existing_ids.iter().any(|id| id == &candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// Atomic write: write to a temp file next to the target, then rename.
fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent dir for {}", path.display()))?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings")
    ));
    fs::write(&tmp, data)
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn with_home(dir: &TempDir) -> PathBuf {
        dir.path().to_path_buf()
    }

    /// Override home_dir for tests by writing a settings file at a known path.
    fn make_settings_at(dir: &TempDir, value: &Value) -> PathBuf {
        let p = dir.path().join(".claude").join("settings.json");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        p
    }

    #[test]
    fn apply_sets_env_keys_preserves_others() {
        let dir = tempfile::tempdir().unwrap();
        let existing = json!({
            "includeCoAuthoredBy": false,
            "theme": "dark",
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "old-token",
                "SOME_OTHER_VAR": "preserved"
            },
            "permissions": { "allow": [], "deny": [] }
        });
        let path = make_settings_at(&dir, &existing);

        let profile = ClaudeProfile {
            id: "p1".to_string(),
            name: "Test".to_string(),
            base_url: "https://new.example.com".to_string(),
            auth_token: "new-token-abc".to_string(),
            model_haiku: "claude-haiku-4".to_string(),
            model_sonnet: "claude-sonnet-5".to_string(),
            model_opus: "claude-opus-5".to_string(),
            default_model: "sonnet".to_string(),
        };

        // We can't call apply_claude_profile() directly here because it uses
        // the real home_dir().  Instead we test the underlying logic inline.
        let contents = fs::read_to_string(&path).unwrap();
        let mut obj: Map<String, Value> = serde_json::from_str::<Value>(&contents)
            .unwrap()
            .as_object()
            .cloned()
            .unwrap();
        let mut env = obj.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
        set_or_remove(&mut env, "ANTHROPIC_AUTH_TOKEN", &profile.auth_token);
        set_or_remove(&mut env, "ANTHROPIC_BASE_URL", &profile.base_url);
        set_or_remove(&mut env, "ANTHROPIC_DEFAULT_HAIKU_MODEL", &profile.model_haiku);
        set_or_remove(&mut env, "ANTHROPIC_DEFAULT_SONNET_MODEL", &profile.model_sonnet);
        set_or_remove(&mut env, "ANTHROPIC_DEFAULT_OPUS_MODEL", &profile.model_opus);
        obj.insert("env".to_string(), Value::Object(env.clone()));
        obj.insert("model".to_string(), Value::String(profile.default_model.clone()));

        // Verify Claude keys updated
        assert_eq!(env_str(&env, "ANTHROPIC_AUTH_TOKEN").unwrap(), "new-token-abc");
        assert_eq!(env_str(&env, "ANTHROPIC_BASE_URL").unwrap(), "https://new.example.com");
        // Verify other key preserved
        assert_eq!(env_str(&env, "SOME_OTHER_VAR").unwrap(), "preserved");
        // Verify non-env keys preserved
        assert_eq!(obj.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(
            obj.get("includeCoAuthoredBy").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(obj.get("model").and_then(Value::as_str), Some("sonnet"));
    }

    #[test]
    fn apply_removes_key_when_empty_token() {
        let mut env = Map::new();
        env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), Value::String("existing".to_string()));
        set_or_remove(&mut env, "ANTHROPIC_AUTH_TOKEN", "");
        assert!(env.get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn claude_profile_from_ccs_env_format() {
        let config = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "ANTHROPIC_BASE_URL": "https://ccs.example.com",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "haiku-3",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "sonnet-3",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "opus-3"
            },
            "model": "sonnet"
        });
        let profile = claude_profile_from_ccs_value("id1", "Provider A", &config, &[]).unwrap();
        assert_eq!(profile.auth_token, "sk-test");
        assert_eq!(profile.base_url, "https://ccs.example.com");
        assert_eq!(profile.model_haiku, "haiku-3");
        assert_eq!(profile.default_model, "sonnet");
    }

    #[test]
    fn claude_profile_from_ccs_direct_fields() {
        let config = json!({
            "apiKey": "direct-key",
            "base_url": "https://direct.example.com/"
        });
        let profile = claude_profile_from_ccs_value("id2", "Provider B", &config, &[]).unwrap();
        assert_eq!(profile.auth_token, "direct-key");
        assert_eq!(profile.base_url, "https://direct.example.com");  // trailing slash stripped
    }

    #[test]
    fn ccs_import_deduplication() {
        let config = json!({ "apiKey": "k", "base_url": "https://x.com" });
        let existing = vec!["ccs-claude-id3".to_string()];
        let profile = claude_profile_from_ccs_value("id3", "X", &config, &existing).unwrap();
        assert_eq!(profile.id, "ccs-claude-id3-2");
    }

    #[test]
    fn returns_none_for_empty_config() {
        let config = json!({});
        let result = claude_profile_from_ccs_value("id4", "Empty", &config, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn unique_profile_id_no_collision() {
        let id = unique_profile_id("test", &[]);
        assert_eq!(id, "test");
    }

    #[test]
    fn unique_profile_id_collision() {
        let existing = vec!["test".to_string(), "test-2".to_string()];
        let id = unique_profile_id("test", &existing);
        assert_eq!(id, "test-3");
    }
}
