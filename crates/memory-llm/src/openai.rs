//! OpenAI (GPT) provider implementation.
//!
//! Supports the `/v1/chat/completions` endpoint used by OpenAI, Azure OpenAI,
//! and any OpenAI-compatible API (e.g., Ollama, LM Studio, vLLM).

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

const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const MAX_RETRIES: usize = 3;
const INITIAL_RETRY_DELAY_MS: u64 = 100;
const MAX_RETRY_DELAY_MS: u64 = 5000;

/// OpenAI-compatible provider.
pub struct OpenAiProvider {
    client: Arc<Client>,
    api_key: Secret<String>,
    model: String,
    api_url: String,
    token_usage: Arc<Mutex<TokenUsage>>,
}

impl Clone for OpenAiProvider {
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

impl OpenAiProvider {
    /// Create with default OpenAI endpoint.
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_url(api_key, model, DEFAULT_OPENAI_URL.to_string())
    }

    /// Create with a custom endpoint (for Azure, Ollama, etc.).
    pub fn with_url(api_key: String, model: String, api_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client: Arc::new(client),
            api_key: Secret::new(api_key),
            model,
            api_url,
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

        let request = OpenAiRequest {
            model: self.model.clone(),
            max_tokens: Some(max_tokens),
            temperature: Some(0.0),
            response_format: Some(ResponseFormat {
                format_type: "json_object".to_string(),
            }),
            messages: vec![
                OpenAiMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                OpenAiMessage {
                    role: "user".to_string(),
                    content: user_message,
                },
            ],
        };

        debug!(
            "Sending extraction request to OpenAI-compatible API ({} chars)",
            content.len()
        );

        let response = self
            .client
            .post(&self.api_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("Content-Type", "application/json")
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
                "OpenAI HTTP {}: {}",
                status, truncated
            )));
        }

        let api_response: OpenAiResponse = response.json().await.map_err(LlmError::Request)?;

        // Track token usage
        if let Some(usage) = &api_response.usage {
            let mut tracked = self.token_usage.lock().await;
            tracked.input_tokens += usage.prompt_tokens;
            tracked.output_tokens += usage.completion_tokens;
            tracked.total_tokens += usage.total_tokens;
            debug!(
                "OpenAI tokens — prompt: {}, completion: {}, total: {}",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            );
        }

        // Extract text from first choice
        let text = api_response
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");

        if text.is_empty() {
            return Err(LlmError::Parse("Empty response from OpenAI".to_string()));
        }

        let result: ExtractionResult = serde_json::from_str(text).map_err(|e| {
            let preview = if text.len() > 500 { &text[..500] } else { text };
            warn!("OpenAI parse error: {}. Raw: {}", e, preview);
            LlmError::Parse(e.to_string())
        })?;

        validate_extraction_result(&result)?;
        let result = sanitize_extraction(result, &GuardrailConfig::default())?;
        Ok(result)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn provider_name(&self) -> &str {
        "openai"
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
                            "OpenAI extraction failed (attempt {}/{}), retrying in {}ms: {}",
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
        error!("OpenAI extraction failed after {} retries", MAX_RETRIES);
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

        let request = OpenAiRequest {
            model: self.model.clone(),
            max_tokens: Some(4096),
            temperature: Some(0.0),
            response_format: Some(ResponseFormat {
                format_type: "json_object".to_string(),
            }),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        debug!(
            "Sending verification request to OpenAI API ({} memories)",
            memories.len()
        );

        let response = self
            .client
            .post(&self.api_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("Content-Type", "application/json")
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
                "OpenAI verification HTTP {}: {}",
                status, truncated
            )));
        }

        let api_response: OpenAiResponse = response.json().await.map_err(LlmError::Request)?;

        if let Some(usage) = &api_response.usage {
            let mut tracked = self.token_usage.lock().await;
            tracked.input_tokens += usage.prompt_tokens;
            tracked.output_tokens += usage.completion_tokens;
            tracked.total_tokens += usage.total_tokens;
        }

        let text = api_response
            .choices
            .first()
            .map(|c| c.message.content.as_str())
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

// ── OpenAI API types ──

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}
