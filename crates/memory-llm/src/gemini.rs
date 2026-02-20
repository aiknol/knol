//! Google Gemini provider implementation.
//!
//! Supports the Gemini `generateContent` endpoint used by Google AI Studio
//! and Vertex AI.

use async_trait::async_trait;
use memory_common::{ExtractedMemory, ExtractionResult, MemoryVerification};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::error::LlmError;
use crate::guardrails::{sanitize_extraction, GuardrailConfig};
use crate::prompt::{
    build_system_prompt_ext, build_verification_prompt, parse_verification_response,
    validate_extraction_result,
};
use crate::provider::{ExtractionOptions, LlmProvider};
use crate::types::TokenUsage;
use secrecy::{ExposeSecret, Secret};

const DEFAULT_GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const MAX_RETRIES: usize = 3;
const INITIAL_RETRY_DELAY_MS: u64 = 100;
const MAX_RETRY_DELAY_MS: u64 = 5000;

/// Google Gemini provider.
pub struct GeminiProvider {
    client: Arc<Client>,
    api_key: Secret<String>,
    model: String,
    api_url: String,
    token_usage: Arc<Mutex<TokenUsage>>,
}

impl Clone for GeminiProvider {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            api_key: Secret::new(self.api_key.expose_secret().clone()),
            model: self.model.clone(),
            api_url: self.api_url.clone(),
            token_usage: self.token_usage.clone(),
        }
    }
}

impl GeminiProvider {
    /// Create with default Google AI Studio endpoint.
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_url(api_key, model, DEFAULT_GEMINI_URL.to_string())
    }

    /// Create with a custom endpoint (for Vertex AI, proxies, etc.).
    pub fn with_url(api_key: String, model: String, api_url: String) -> Self {
        Self {
            client: Arc::new(Client::new()),
            api_key: Secret::new(api_key),
            model,
            api_url,
            token_usage: Arc::new(Mutex::new(TokenUsage::default())),
        }
    }

    /// Build the full URL for the generateContent endpoint.
    fn endpoint_url(&self) -> String {
        format!(
            "{}/models/{}:generateContent",
            self.api_url.trim_end_matches('/'),
            self.model
        )
    }

    #[allow(dead_code)]
    async fn call_api(
        &self,
        content: &str,
        role: &str,
        existing_entities: &[String],
    ) -> Result<ExtractionResult, LlmError> {
        self.call_api_with_options(
            content,
            role,
            existing_entities,
            &ExtractionOptions::default(),
        )
        .await
    }

    async fn call_api_with_options(
        &self,
        content: &str,
        role: &str,
        existing_entities: &[String],
        options: &ExtractionOptions,
    ) -> Result<ExtractionResult, LlmError> {
        let system_prompt = build_system_prompt_ext(existing_entities, options.inline_verification);
        let user_message = format!("[{}]: {}", role, content);
        let max_tokens = options.max_output_tokens.unwrap_or(4096);

        let request = GeminiRequest {
            system_instruction: Some(SystemInstruction {
                parts: vec![GeminiPart {
                    text: system_prompt,
                }],
            }),
            contents: vec![GeminiContent {
                role: Some("user".to_string()),
                parts: vec![GeminiPart { text: user_message }],
            }],
            generation_config: Some(GenerationConfig {
                temperature: Some(0.0),
                max_output_tokens: Some(max_tokens),
                response_mime_type: Some("application/json".to_string()),
            }),
        };

        let url = self.endpoint_url();
        debug!(
            "Sending extraction request to Gemini API ({} chars, model: {})",
            content.len(),
            self.model
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .query(&[("key", self.api_key.expose_secret())])
            .json(&request)
            .send()
            .await
            .map_err(LlmError::Request)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!("Gemini HTTP {}: {}", status, body)));
        }

        let api_response: GeminiResponse = response.json().await.map_err(LlmError::Request)?;

        // Track token usage
        if let Some(usage) = &api_response.usage_metadata {
            let mut tracked = self.token_usage.lock().await;
            let input = usage.prompt_token_count.unwrap_or(0);
            let output = usage.candidates_token_count.unwrap_or(0);
            let total = usage.total_token_count.unwrap_or(input + output);
            tracked.input_tokens += input;
            tracked.output_tokens += output;
            tracked.total_tokens += total;
            debug!(
                "Gemini tokens — prompt: {}, candidates: {}, total: {}",
                input, output, total
            );
        }

        // Extract text from first candidate
        let text = api_response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.as_str())
            .unwrap_or("");

        if text.is_empty() {
            return Err(LlmError::Parse("Empty response from Gemini".to_string()));
        }

        let result: ExtractionResult = serde_json::from_str(text).map_err(|e| {
            let preview = if text.len() > 500 { &text[..500] } else { text };
            warn!("Gemini parse error: {}. Raw: {}", e, preview);
            LlmError::Parse(e.to_string())
        })?;

        validate_extraction_result(&result)?;
        let result = sanitize_extraction(result, &GuardrailConfig::default())?;
        Ok(result)
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn provider_name(&self) -> &str {
        "gemini"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn extract_memories(
        &self,
        content: &str,
        role: &str,
        existing_entities: &[String],
    ) -> Result<ExtractionResult, LlmError> {
        self.extract_memories_with_options(
            content,
            role,
            existing_entities,
            &ExtractionOptions::default(),
        )
        .await
    }

    async fn extract_memories_with_options(
        &self,
        content: &str,
        role: &str,
        existing_entities: &[String],
        options: &ExtractionOptions,
    ) -> Result<ExtractionResult, LlmError> {
        let mut last_error = None;
        for attempt in 0..=MAX_RETRIES {
            match self
                .call_api_with_options(content, role, existing_entities, options)
                .await
            {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        let delay_ms = std::cmp::min(
                            INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt as u32),
                            MAX_RETRY_DELAY_MS,
                        );
                        warn!(
                            "Gemini extraction failed (attempt {}/{}), retrying in {}ms: {}",
                            attempt + 1,
                            MAX_RETRIES,
                            delay_ms,
                            e
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    last_error = Some(e);
                }
            }
        }
        error!("Gemini extraction failed after {} retries", MAX_RETRIES);
        Err(last_error.unwrap())
    }

    async fn verify_memories(
        &self,
        memories: &[ExtractedMemory],
        source_content: &str,
    ) -> Result<Vec<MemoryVerification>, LlmError> {
        if memories.is_empty() {
            return Ok(Vec::new());
        }

        let prompt = build_verification_prompt(memories, source_content);

        let request = GeminiRequest {
            system_instruction: None,
            contents: vec![GeminiContent {
                role: Some("user".to_string()),
                parts: vec![GeminiPart { text: prompt }],
            }],
            generation_config: Some(GenerationConfig {
                temperature: Some(0.0),
                max_output_tokens: Some(4096),
                response_mime_type: Some("application/json".to_string()),
            }),
        };

        let url = self.endpoint_url();
        debug!(
            "Sending verification request to Gemini API ({} memories)",
            memories.len()
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .query(&[("key", self.api_key.expose_secret())])
            .json(&request)
            .send()
            .await
            .map_err(LlmError::Request)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!(
                "Gemini verification HTTP {}: {}",
                status, body
            )));
        }

        let api_response: GeminiResponse = response.json().await.map_err(LlmError::Request)?;

        if let Some(usage) = &api_response.usage_metadata {
            let mut tracked = self.token_usage.lock().await;
            let input = usage.prompt_token_count.unwrap_or(0);
            let output = usage.candidates_token_count.unwrap_or(0);
            let total = usage.total_token_count.unwrap_or(input + output);
            tracked.input_tokens += input;
            tracked.output_tokens += output;
            tracked.total_tokens += total;
        }

        let text = api_response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.as_str())
            .unwrap_or("");

        parse_verification_response(text)
    }

    async fn get_token_usage(&self) -> TokenUsage {
        self.token_usage.lock().await.clone()
    }

    async fn reset_token_usage(&self) {
        *self.token_usage.lock().await = TokenUsage::default();
    }
}

// ── Gemini API types ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction>,
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize)]
struct SystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    #[serde(default)]
    candidates: Option<Vec<Candidate>>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: GeminiContent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(default)]
    candidates_token_count: Option<u32>,
    #[serde(default)]
    total_token_count: Option<u32>,
}
