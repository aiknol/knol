//! Anthropic Claude provider implementation.

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

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_RETRIES: usize = 3;
const INITIAL_RETRY_DELAY_MS: u64 = 100;
const MAX_RETRY_DELAY_MS: u64 = 5000;

/// Anthropic Claude provider.
pub struct AnthropicProvider {
    client: Arc<Client>,
    api_key: Secret<String>,
    model: String,
    token_usage: Arc<Mutex<TokenUsage>>,
}

impl Clone for AnthropicProvider {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            api_key: Secret::new(self.api_key.expose_secret().clone()),
            model: self.model.clone(),
            token_usage: self.token_usage.clone(),
        }
    }
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client: Arc::new(client),
            api_key: Secret::new(api_key),
            model,
            token_usage: Arc::new(Mutex::new(TokenUsage::default())),
        }
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

        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens,
            system: Some(system_prompt),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_message,
            }],
        };

        debug!(
            "Sending extraction request to Anthropic API ({} chars)",
            content.len()
        );

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(LlmError::Request)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let truncated = if body.len() > 200 {
                &body[..200]
            } else {
                &body
            };
            return Err(LlmError::Api(format!(
                "Anthropic HTTP {}: {}",
                status, truncated
            )));
        }

        let api_response: AnthropicResponse = response.json().await.map_err(LlmError::Request)?;

        // Track token usage
        if let Some(usage) = &api_response.usage {
            let mut tracked = self.token_usage.lock().await;
            tracked.input_tokens += usage.input_tokens;
            tracked.output_tokens += usage.output_tokens;
            tracked.total_tokens += usage.input_tokens + usage.output_tokens;
            debug!(
                "Anthropic tokens — input: {}, output: {}",
                usage.input_tokens, usage.output_tokens
            );
        }

        // Extract text content
        let text: String = api_response
            .content
            .iter()
            .filter_map(|b| {
                if b.block_type == "text" {
                    Some(b.text.as_str())
                } else {
                    None
                }
            })
            .collect();

        if text.is_empty() {
            return Err(LlmError::Parse("Empty response from Anthropic".to_string()));
        }

        let result: ExtractionResult = serde_json::from_str(&text).map_err(|e| {
            let preview = if text.len() > 500 {
                &text[..500]
            } else {
                &text
            };
            warn!("Anthropic parse error: {}. Raw: {}", e, preview);
            LlmError::Parse(e.to_string())
        })?;

        validate_extraction_result(&result)?;
        let result = sanitize_extraction(result, &GuardrailConfig::default())?;
        Ok(result)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn provider_name(&self) -> &str {
        "anthropic"
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
                            "Anthropic extraction failed (attempt {}/{}), retrying in {}ms: {}",
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
        error!("Anthropic extraction failed after {} retries", MAX_RETRIES);
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

        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            system: None,
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        debug!(
            "Sending verification request to Anthropic API ({} memories)",
            memories.len()
        );

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(LlmError::Request)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let truncated = if body.len() > 200 {
                &body[..200]
            } else {
                &body
            };
            return Err(LlmError::Api(format!(
                "Anthropic verification HTTP {}: {}",
                status, truncated
            )));
        }

        let api_response: AnthropicResponse = response.json().await.map_err(LlmError::Request)?;

        // Track token usage
        if let Some(usage) = &api_response.usage {
            let mut tracked = self.token_usage.lock().await;
            tracked.input_tokens += usage.input_tokens;
            tracked.output_tokens += usage.output_tokens;
            tracked.total_tokens += usage.input_tokens + usage.output_tokens;
        }

        let text: String = api_response
            .content
            .iter()
            .filter_map(|b| {
                if b.block_type == "text" {
                    Some(b.text.as_str())
                } else {
                    None
                }
            })
            .collect();

        parse_verification_response(&text)
    }

    async fn get_token_usage(&self) -> TokenUsage {
        self.token_usage.lock().await.clone()
    }

    async fn reset_token_usage(&self) {
        *self.token_usage.lock().await = TokenUsage::default();
    }
}

// ── Anthropic API types ──

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}
