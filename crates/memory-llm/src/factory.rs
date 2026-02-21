//! Factory for creating the right LLM provider based on configuration.
//!
//! Reads `llm.provider` from the admin DB (via `db_config`), then fetches
//! the API key from `system_credentials` (encrypted) or falls back to env vars.

use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, warn};

use crate::anthropic::AnthropicProvider;
use crate::error::LlmError;
use crate::gemini::GeminiProvider;
use crate::guardrails::GuardrailConfig;
use crate::openai::OpenAiProvider;
use crate::provider::LlmProvider;
use crate::types::LlmProviderKind;

/// Configuration resolved from DB + env for building a provider.
#[derive(Clone)]
pub struct LlmConfig {
    pub provider: LlmProviderKind,
    pub api_key: String,
    pub model: String,
    pub api_url: Option<String>,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConfig")
            .field("provider", &self.provider)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("api_url", &self.api_url)
            .finish()
    }
}

/// Build an [`LlmProvider`] from explicit config.
pub fn build_provider(config: &LlmConfig) -> Arc<dyn LlmProvider> {
    match config.provider {
        LlmProviderKind::Anthropic => {
            info!("Using Anthropic provider (model: {})", config.model);
            Arc::new(AnthropicProvider::new(
                config.api_key.clone(),
                config.model.clone(),
            ))
        }
        LlmProviderKind::OpenAi => {
            let provider = if let Some(url) = &config.api_url {
                info!(
                    "Using OpenAI-compatible provider (model: {}, url: {})",
                    config.model, url
                );
                OpenAiProvider::with_url(config.api_key.clone(), config.model.clone(), url.clone())
            } else {
                info!("Using OpenAI provider (model: {})", config.model);
                OpenAiProvider::new(config.api_key.clone(), config.model.clone())
            };
            Arc::new(provider)
        }
        LlmProviderKind::Gemini => {
            let provider = if let Some(url) = &config.api_url {
                info!(
                    "Using Gemini provider (model: {}, url: {})",
                    config.model, url
                );
                GeminiProvider::with_url(config.api_key.clone(), config.model.clone(), url.clone())
            } else {
                info!("Using Gemini provider (model: {})", config.model);
                GeminiProvider::new(config.api_key.clone(), config.model.clone())
            };
            Arc::new(provider)
        }
    }
}

/// Load LLM configuration from the admin DB + env vars, then build a provider.
///
/// Resolution order for each setting:
///   1. `system_config` table (admin panel)
///   2. Environment variable
///   3. Compiled default
///
/// For the API key specifically:
///   1. `system_credentials` table (encrypted) — credential name `anthropic_api_key` / `openai_api_key`
///   2. Environment variable (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`)
pub async fn build_provider_from_db(pool: &PgPool) -> Result<Arc<dyn LlmProvider>, LlmError> {
    // 1. Which provider?
    let provider_str =
        memory_common::db_config::load_str(pool, "llm.provider", "LLM_PROVIDER", "anthropic").await;
    let kind = LlmProviderKind::from_str_loose(&provider_str);

    // 2. Which model?
    let (model_db_key, model_env, model_default) = match kind {
        LlmProviderKind::Anthropic => (
            "llm.extraction_model",
            "EXTRACTION_MODEL",
            "claude-haiku-4-5-20251001",
        ),
        LlmProviderKind::OpenAi => ("llm.openai_model", "OPENAI_MODEL", "gpt-4o-mini"),
        LlmProviderKind::Gemini => ("llm.gemini_model", "GEMINI_MODEL", "gemini-2.0-flash"),
    };
    let model =
        memory_common::db_config::load_str(pool, model_db_key, model_env, model_default).await;

    // 3. API key — try encrypted credentials table first, then env var.
    let (cred_name, env_key) = match kind {
        LlmProviderKind::Anthropic => ("anthropic_api_key", "ANTHROPIC_API_KEY"),
        LlmProviderKind::OpenAi => ("openai_api_key", "OPENAI_API_KEY"),
        LlmProviderKind::Gemini => ("gemini_api_key", "GEMINI_API_KEY"),
    };
    let api_key = load_credential(pool, cred_name).await.unwrap_or_else(|| {
        let val = std::env::var(env_key).unwrap_or_default();
        if val.is_empty() {
            warn!(
                "No API key found for {} (checked credentials table '{}' and env '{}')",
                provider_str, cred_name, env_key
            );
        }
        val
    });

    // 4. Optional custom API URL (for OpenAI-compatible or Vertex AI endpoints)
    let api_url = match kind {
        LlmProviderKind::OpenAi => {
            let url = memory_common::db_config::load_str(
                pool,
                "llm.openai_api_url",
                "OPENAI_API_URL",
                "",
            )
            .await;
            if url.is_empty() {
                None
            } else {
                Some(url)
            }
        }
        LlmProviderKind::Gemini => {
            let url = memory_common::db_config::load_str(
                pool,
                "llm.gemini_api_url",
                "GEMINI_API_URL",
                "",
            )
            .await;
            if url.is_empty() {
                None
            } else {
                Some(url)
            }
        }
        _ => None,
    };

    let config = LlmConfig {
        provider: kind,
        api_key,
        model,
        api_url,
    };

    Ok(build_provider(&config))
}

/// Load [`TriageConfig`] from the admin DB, falling back to defaults.
///
/// Keys: `llm.enable_triage`, `llm.triage_min_words`, `llm.triage_light_threshold`.
pub async fn build_triage_config_from_db(pool: &PgPool) -> crate::triage::TriageConfig {
    use memory_common::db_config::{load_bool, load_u64};

    let defaults = crate::triage::TriageConfig::default();

    let enabled = load_bool(
        pool,
        "llm.enable_triage",
        "LLM_ENABLE_TRIAGE",
        defaults.enabled,
    )
    .await;
    let min_words = load_u64(
        pool,
        "llm.triage_min_words",
        "LLM_TRIAGE_MIN_WORDS",
        defaults.min_words as u64,
    )
    .await as usize;
    let light_threshold_words = load_u64(
        pool,
        "llm.triage_light_threshold",
        "LLM_TRIAGE_LIGHT_THRESHOLD",
        defaults.light_threshold_words as u64,
    )
    .await as usize;

    info!(
        "Triage config loaded: enabled={}, min_words={}, light_threshold={}",
        enabled, min_words, light_threshold_words
    );

    crate::triage::TriageConfig {
        enabled,
        min_words,
        light_threshold_words,
    }
}

/// Load [`GuardrailConfig`] from the admin DB, falling back to defaults for any
/// missing keys.
///
/// Each field is read from `system_config` under the `guardrails` category:
///   `guardrails.redact_pii`, `guardrails.pii_mode`, etc.
pub async fn build_guardrail_config_from_db(pool: &PgPool) -> GuardrailConfig {
    use memory_common::db_config::{load_bool, load_f64, load_str, load_str_array};

    let defaults = GuardrailConfig::default();

    let redact_pii = load_bool(pool, "guardrails.redact_pii", "", defaults.redact_pii).await;
    let pii_mode = load_str(pool, "guardrails.pii_mode", "", &defaults.pii_mode).await;
    let strict_memory_types = load_bool(
        pool,
        "guardrails.strict_memory_types",
        "",
        defaults.strict_memory_types,
    )
    .await;
    let strict_entity_types = load_bool(
        pool,
        "guardrails.strict_entity_types",
        "",
        defaults.strict_entity_types,
    )
    .await;
    let max_memory_content_len = load_f64(
        pool,
        "guardrails.max_memory_content_len",
        "",
        defaults.max_memory_content_len as f64,
    )
    .await as usize;
    let max_entity_name_len = load_f64(
        pool,
        "guardrails.max_entity_name_len",
        "",
        defaults.max_entity_name_len as f64,
    )
    .await as usize;
    let max_memories_per_extraction = load_f64(
        pool,
        "guardrails.max_memories_per_extraction",
        "",
        defaults.max_memories_per_extraction as f64,
    )
    .await as usize;
    let max_entities_per_extraction = load_f64(
        pool,
        "guardrails.max_entities_per_extraction",
        "",
        defaults.max_entities_per_extraction as f64,
    )
    .await as usize;
    let min_confidence = load_f64(
        pool,
        "guardrails.min_confidence",
        "",
        defaults.min_confidence as f64,
    )
    .await as f32;
    let detect_prompt_injection = load_bool(
        pool,
        "guardrails.detect_prompt_injection",
        "",
        defaults.detect_prompt_injection,
    )
    .await;
    let max_input_content_len = load_f64(
        pool,
        "guardrails.max_input_content_len",
        "",
        defaults.max_input_content_len as f64,
    )
    .await as usize;
    let blocked_keywords = load_str_array(pool, "guardrails.blocked_keywords")
        .await
        .unwrap_or_default();

    info!(
        "Guardrails config loaded: pii={} (mode={}), injection_detect={}, min_conf={:.2}, blocked_keywords={}",
        redact_pii, pii_mode, detect_prompt_injection, min_confidence, blocked_keywords.len()
    );

    GuardrailConfig {
        redact_pii,
        pii_mode,
        strict_memory_types,
        strict_entity_types,
        max_memory_content_len,
        max_entity_name_len,
        max_memories_per_extraction,
        max_entities_per_extraction,
        min_confidence,
        detect_prompt_injection,
        max_input_content_len,
        blocked_keywords,
    }
}

/// Load [`GroundingConfig`] from the admin DB, falling back to defaults for any
/// missing keys.
///
/// Each field is read from `system_config` under the `grounding` category:
///   `grounding.enable_citations`, `grounding.enable_verification`, etc.
pub async fn build_grounding_config_from_db(pool: &PgPool) -> memory_common::GroundingConfig {
    use memory_common::db_config::{load_bool, load_f64, load_str};

    let defaults = memory_common::GroundingConfig::default();

    let enable_citations = load_bool(
        pool,
        "grounding.enable_citations",
        "",
        defaults.enable_citations,
    )
    .await;
    let enable_verification = load_bool(
        pool,
        "grounding.enable_verification",
        "",
        defaults.enable_verification,
    )
    .await;
    let verification_model = load_str(
        pool,
        "grounding.verification_model",
        "",
        &defaults.verification_model,
    )
    .await;
    let min_verification_score = load_f64(
        pool,
        "grounding.min_verification_score",
        "",
        defaults.min_verification_score as f64,
    )
    .await as f32;

    info!(
        "Grounding config loaded: citations={}, verification={}, model={}, min_score={:.2}",
        enable_citations, enable_verification, verification_model, min_verification_score
    );

    memory_common::GroundingConfig {
        enable_citations,
        enable_verification,
        verification_model,
        min_verification_score,
    }
}

/// Try to load a decrypted credential value from the `system_credentials` table.
///
/// Returns `None` if the credential doesn't exist or decryption fails.
/// This avoids a hard dependency on the encryption crate — the value is
/// stored as base64(nonce || ciphertext) in the DB.
async fn load_credential(pool: &PgPool, name: &str) -> Option<String> {
    #[derive(sqlx::FromRow)]
    struct CredRow {
        encrypted_value: Vec<u8>,
    }

    let row = sqlx::query_as::<_, CredRow>(
        "SELECT encrypted_value FROM system_credentials WHERE name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    // Decrypt using the same AES-256-GCM scheme as service-admin.
    // Fail closed if key is missing/invalid.
    let key = load_encryption_key()?;
    decrypt_credential(&row.encrypted_value, &key)
}

/// Minimal AES-256-GCM decryption (mirrors service-admin/crypto.rs).
fn decrypt_credential(data: &[u8], key: &[u8; 32]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }

    use aes_gcm::{aead::Aead, aead::KeyInit, Aes256Gcm, Nonce};

    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Nonce::from_slice(&data[..12]);
    let plaintext = cipher.decrypt(nonce, &data[12..]).ok()?;
    String::from_utf8(plaintext).ok()
}

/// Load the AES-256-GCM encryption key (same logic as service-admin/crypto.rs).
/// Returns None when missing/invalid to avoid insecure fallbacks.
fn load_encryption_key() -> Option<[u8; 32]> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    let b64 = std::env::var("ADMIN_ENCRYPTION_KEY").ok()?;
    let bytes = B64.decode(&b64).ok()?;
    if bytes.len() != 32 {
        warn!("ADMIN_ENCRYPTION_KEY invalid length; expected 32-byte decoded key");
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Some(key)
}
