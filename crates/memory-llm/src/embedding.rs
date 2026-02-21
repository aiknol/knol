//! Embedding generation for vector search.
//!
//! Supports multiple embedding providers (OpenAI, Voyage, Gemini) with automatic
//! fallback and caching. Used by the retrieve service to convert queries into
//! vectors, and by the graph service to embed memories at write time.

use crate::error::LlmError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Configuration for the embedding provider.
#[derive(Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Provider: "openai", "voyage", "gemini", "local"
    pub provider: String,
    /// API key for the embedding provider.
    /// Excluded from Debug and serialization to prevent accidental exposure.
    #[serde(skip_serializing)]
    pub api_key: String,
    /// Model name (e.g., "text-embedding-3-small", "voyage-3", "text-embedding-004").
    pub model: String,
    /// Embedding dimension (must match pgvector column, typically 1024).
    pub dimensions: usize,
    /// Optional API base URL override.
    pub api_url: Option<String>,
    /// Enable in-memory embedding cache.
    pub cache_enabled: bool,
    /// Max cache entries (LRU eviction).
    pub cache_max_entries: usize,
}

impl std::fmt::Debug for EmbeddingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("provider", &self.provider)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_url", &self.api_url)
            .field("cache_enabled", &self.cache_enabled)
            .field("cache_max_entries", &self.cache_max_entries)
            .finish()
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            api_key: String::new(),
            model: "text-embedding-3-small".to_string(),
            dimensions: 1024,
            api_url: None,
            cache_enabled: true,
            cache_max_entries: 10_000,
        }
    }
}

/// Thread-safe embedding provider with optional caching.
pub struct EmbeddingProvider {
    config: EmbeddingConfig,
    http_client: reqwest::Client,
    cache: Arc<RwLock<LruCache>>,
}

/// Simple LRU cache for embeddings.
struct LruCache {
    map: HashMap<String, (Vec<f32>, u64)>, // hash -> (embedding, access_count)
    max_entries: usize,
    access_counter: u64,
}

impl LruCache {
    fn new(max_entries: usize) -> Self {
        Self {
            map: HashMap::new(),
            max_entries,
            access_counter: 0,
        }
    }

    fn get(&mut self, key: &str) -> Option<Vec<f32>> {
        if let Some((emb, count)) = self.map.get_mut(key) {
            self.access_counter += 1;
            *count = self.access_counter;
            Some(emb.clone())
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, embedding: Vec<f32>) {
        // Evict least recently used if at capacity
        if self.map.len() >= self.max_entries {
            if let Some(lru_key) = self
                .map
                .iter()
                .min_by_key(|(_, (_, count))| *count)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&lru_key);
            }
        }
        self.access_counter += 1;
        self.map.insert(key, (embedding, self.access_counter));
    }
}

impl EmbeddingProvider {
    /// Create a new embedding provider from config.
    pub fn new(config: EmbeddingConfig) -> Self {
        let cache_max = if config.cache_enabled {
            config.cache_max_entries
        } else {
            0
        };
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            http_client,
            cache: Arc::new(RwLock::new(LruCache::new(cache_max))),
        }
    }

    /// Build an EmbeddingProvider from admin DB configuration.
    pub async fn from_db(pool: &sqlx::PgPool) -> Result<Self, LlmError> {
        use memory_common::db_config;

        let provider =
            db_config::load_string(pool, "embedding.provider", "EMBEDDING_PROVIDER", "openai")
                .await;
        let api_key =
            db_config::load_secret(pool, "embedding.api_key", "EMBEDDING_API_KEY", "").await;
        let model = db_config::load_string(
            pool,
            "embedding.model",
            "EMBEDDING_MODEL",
            "text-embedding-3-small",
        )
        .await;
        let dimensions =
            db_config::load_u64(pool, "embedding.dimensions", "EMBEDDING_DIMENSIONS", 1024).await
                as usize;
        let cache_enabled = db_config::load_bool(
            pool,
            "embedding.cache_enabled",
            "EMBEDDING_CACHE_ENABLED",
            true,
        )
        .await;
        let cache_max = db_config::load_u64(
            pool,
            "embedding.cache_max_entries",
            "EMBEDDING_CACHE_MAX",
            10000,
        )
        .await as usize;

        // If no dedicated embedding key, try the main LLM key
        let effective_key = if api_key.is_empty() {
            let llm_provider =
                db_config::load_string(pool, "llm.provider", "LLM_PROVIDER", "gemini").await;
            match llm_provider.to_lowercase().as_str() {
                "openai" => db_config::load_secret(pool, "llm.api_key", "OPENAI_API_KEY", "").await,
                "gemini" | "google" => {
                    db_config::load_secret(pool, "llm.api_key", "GEMINI_API_KEY", "").await
                }
                _ => db_config::load_secret(pool, "llm.api_key", "ANTHROPIC_API_KEY", "").await,
            }
        } else {
            api_key
        };

        let config = EmbeddingConfig {
            provider,
            api_key: effective_key,
            model,
            dimensions,
            api_url: None,
            cache_enabled,
            cache_max_entries: cache_max,
        };

        Ok(Self::new(config))
    }

    /// Generate embedding for a single text.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let results = self.embed_batch(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Parse("Empty embedding response".into()))
    }

    /// Generate embeddings for multiple texts in a single API call.
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Check cache for each text
        let mut results: Vec<Option<Vec<f32>>> = Vec::with_capacity(texts.len());
        let mut uncached_indices: Vec<usize> = Vec::new();
        let mut uncached_texts: Vec<&str> = Vec::new();

        if self.config.cache_enabled {
            let mut cache = self.cache.write().await;
            for (i, text) in texts.iter().enumerate() {
                let key = cache_key_for_text(text, &self.config.model);
                if let Some(emb) = cache.get(&key) {
                    results.push(Some(emb));
                } else {
                    results.push(None);
                    uncached_indices.push(i);
                    uncached_texts.push(text);
                }
            }
        } else {
            for (i, text) in texts.iter().enumerate() {
                results.push(None);
                uncached_indices.push(i);
                uncached_texts.push(text);
            }
        }

        // If everything was cached, return early
        if uncached_texts.is_empty() {
            return Ok(results.into_iter().map(|r| r.unwrap()).collect());
        }

        // Call embedding API for uncached texts
        let embeddings = match self.config.provider.to_lowercase().as_str() {
            "openai" => self.embed_openai(&uncached_texts).await?,
            "voyage" => self.embed_voyage(&uncached_texts).await?,
            "gemini" | "google" => self.embed_gemini(&uncached_texts).await?,
            "local" => self.embed_local(&uncached_texts).await?,
            other => {
                warn!(
                    "Unknown embedding provider '{}', falling back to OpenAI",
                    other
                );
                self.embed_openai(&uncached_texts).await?
            }
        };

        // Populate cache and fill results
        if self.config.cache_enabled {
            let mut cache = self.cache.write().await;
            for (idx, embedding) in uncached_indices.iter().zip(embeddings.iter()) {
                let key = cache_key_for_text(texts[*idx], &self.config.model);
                cache.insert(key, embedding.clone());
                results[*idx] = Some(embedding.clone());
            }
        } else {
            for (idx, embedding) in uncached_indices.iter().zip(embeddings.into_iter()) {
                results[*idx] = Some(embedding);
            }
        }

        Ok(results.into_iter().map(|r| r.unwrap()).collect())
    }

    /// OpenAI embedding API (also compatible with Azure OpenAI, Together, etc.).
    async fn embed_openai(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let url = self
            .config
            .api_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/embeddings");

        let body = serde_json::json!({
            "model": self.config.model,
            "input": texts,
            "dimensions": self.config.dimensions,
        });

        let resp = self
            .http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Api(format!("OpenAI embedding request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!(
                "OpenAI embedding API error {}: {}",
                status, text
            )));
        }

        let json: OpenAiEmbeddingResponse = resp.json().await.map_err(|e| {
            LlmError::Parse(format!("Failed to parse OpenAI embedding response: {}", e))
        })?;

        let mut embeddings: Vec<Vec<f32>> = json.data.into_iter().map(|d| d.embedding).collect();

        // Ensure correct dimensions (truncate or pad if necessary)
        for emb in &mut embeddings {
            normalize_dimensions(emb, self.config.dimensions);
        }

        debug!(
            "OpenAI embedding: {} texts, {} tokens used",
            texts.len(),
            json.usage.total_tokens
        );
        Ok(embeddings)
    }

    /// Voyage AI embedding API.
    async fn embed_voyage(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let url = self
            .config
            .api_url
            .as_deref()
            .unwrap_or("https://api.voyageai.com/v1/embeddings");

        let body = serde_json::json!({
            "model": self.config.model,
            "input": texts,
            "output_dimension": self.config.dimensions,
        });

        let resp = self
            .http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Api(format!("Voyage embedding request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!(
                "Voyage embedding API error {}: {}",
                status, text
            )));
        }

        let json: VoyageEmbeddingResponse = resp.json().await.map_err(|e| {
            LlmError::Parse(format!("Failed to parse Voyage embedding response: {}", e))
        })?;

        let mut embeddings: Vec<Vec<f32>> = json.data.into_iter().map(|d| d.embedding).collect();

        for emb in &mut embeddings {
            normalize_dimensions(emb, self.config.dimensions);
        }

        debug!("Voyage embedding: {} texts", texts.len());
        Ok(embeddings)
    }

    /// Google Gemini embedding API.
    async fn embed_gemini(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let base_url = self
            .config
            .api_url
            .as_deref()
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta");

        // Gemini uses batchEmbedContents for multiple texts
        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|t| {
                serde_json::json!({
                    "model": format!("models/{}", self.config.model),
                    "content": {"parts": [{"text": t}]},
                    "outputDimensionality": self.config.dimensions,
                })
            })
            .collect();

        let url = format!(
            "{}/models/{}:batchEmbedContents",
            base_url, self.config.model
        );

        let body = serde_json::json!({ "requests": requests });

        let resp = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Api(format!("Gemini embedding request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!(
                "Gemini embedding API error {}: {}",
                status, text
            )));
        }

        let json: GeminiEmbeddingResponse = resp.json().await.map_err(|e| {
            LlmError::Parse(format!("Failed to parse Gemini embedding response: {}", e))
        })?;

        let mut embeddings: Vec<Vec<f32>> = json.embeddings.into_iter().map(|e| e.values).collect();

        for emb in &mut embeddings {
            normalize_dimensions(emb, self.config.dimensions);
        }

        debug!("Gemini embedding: {} texts", texts.len());
        Ok(embeddings)
    }

    /// Local/offline embedding using simple TF-IDF-like hashing.
    /// Useful for development/testing without API keys.
    async fn embed_local(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let embeddings: Vec<Vec<f32>> = texts
            .iter()
            .map(|text| hash_embedding(text, self.config.dimensions))
            .collect();

        debug!("Local embedding: {} texts (hash-based)", texts.len());
        Ok(embeddings)
    }

    /// Return the configured dimensions.
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    /// Return the provider name.
    pub fn provider_name(&self) -> &str {
        &self.config.provider
    }
}

/// Generate a deterministic hash-based embedding for development/testing.
/// Not suitable for production but allows vector search to function without API keys.
fn hash_embedding(text: &str, dimensions: usize) -> Vec<f32> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut embedding = vec![0.0f32; dimensions];

    for (i, word) in words.iter().enumerate() {
        let mut hasher = Sha256::new();
        hasher.update(word.to_lowercase().as_bytes());
        let hash = hasher.finalize();

        // Use hash bytes to set embedding dimensions
        for (j, byte) in hash.iter().enumerate() {
            let dim = (i * 32 + j) % dimensions;
            embedding[dim] += (*byte as f32 - 128.0) / 128.0;
        }
    }

    // L2 normalize
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut embedding {
            *v /= norm;
        }
    }

    embedding
}

/// Normalize embedding to target dimensions (truncate or zero-pad).
fn normalize_dimensions(embedding: &mut Vec<f32>, target: usize) {
    if embedding.len() > target {
        embedding.truncate(target);
    } else {
        embedding.resize(target, 0.0);
    }
}

/// Cache key for embedding text.
fn cache_key_for_text(text: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.update(b"|model:");
    hasher.update(model.as_bytes());
    format!("emb:{}", hex::encode(hasher.finalize()))
}

// ── API Response Types ──

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
    usage: OpenAiEmbeddingUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingUsage {
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct VoyageEmbeddingResponse {
    data: Vec<VoyageEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct VoyageEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbeddingResponse {
    embeddings: Vec<GeminiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbeddingData {
    values: Vec<f32>,
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_embedding_deterministic() {
        let e1 = hash_embedding("hello world", 1024);
        let e2 = hash_embedding("hello world", 1024);
        assert_eq!(e1, e2);
    }

    #[test]
    fn test_hash_embedding_correct_dimensions() {
        let e = hash_embedding("test text", 1024);
        assert_eq!(e.len(), 1024);

        let e_small = hash_embedding("test text", 256);
        assert_eq!(e_small.len(), 256);
    }

    #[test]
    fn test_hash_embedding_normalized() {
        let e = hash_embedding("some meaningful text to embed", 1024);
        let norm: f32 = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "Expected L2 norm ~1.0, got {}",
            norm
        );
    }

    #[test]
    fn test_hash_embedding_different_texts() {
        let e1 = hash_embedding("cats are great", 1024);
        let e2 = hash_embedding("dogs are great", 1024);
        assert_ne!(e1, e2);
    }

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = cache_key_for_text("hello", "model-a");
        let k2 = cache_key_for_text("hello", "model-a");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_cache_key_differs_by_model() {
        let k1 = cache_key_for_text("hello", "model-a");
        let k2 = cache_key_for_text("hello", "model-b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_normalize_dimensions_truncate() {
        let mut v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        normalize_dimensions(&mut v, 3);
        assert_eq!(v.len(), 3);
        assert_eq!(v, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_normalize_dimensions_pad() {
        let mut v = vec![1.0, 2.0];
        normalize_dimensions(&mut v, 5);
        assert_eq!(v.len(), 5);
        assert_eq!(v, vec![1.0, 2.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_lru_cache_basic() {
        let mut cache = LruCache::new(3);
        cache.insert("a".into(), vec![1.0]);
        cache.insert("b".into(), vec![2.0]);
        cache.insert("c".into(), vec![3.0]);

        assert_eq!(cache.get("a"), Some(vec![1.0]));
        assert_eq!(cache.get("d"), None);
    }

    #[test]
    fn test_lru_cache_eviction() {
        let mut cache = LruCache::new(2);
        cache.insert("a".into(), vec![1.0]);
        cache.insert("b".into(), vec![2.0]);
        // Access "a" to make it recently used
        cache.get("a");
        // Insert "c" — should evict "b" (least recently used)
        cache.insert("c".into(), vec![3.0]);

        assert_eq!(cache.get("a"), Some(vec![1.0]));
        assert_eq!(cache.get("b"), None); // evicted
        assert_eq!(cache.get("c"), Some(vec![3.0]));
    }

    #[test]
    fn test_embedding_config_default() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.dimensions, 1024);
        assert!(config.cache_enabled);
    }

    #[tokio::test]
    async fn test_local_embedding_provider() {
        let config = EmbeddingConfig {
            provider: "local".into(),
            api_key: String::new(),
            model: "local-hash".into(),
            dimensions: 1024,
            api_url: None,
            cache_enabled: true,
            cache_max_entries: 100,
        };
        let provider = EmbeddingProvider::new(config);
        let emb = provider
            .embed("test query about Rust programming")
            .await
            .unwrap();
        assert_eq!(emb.len(), 1024);
    }

    #[tokio::test]
    async fn test_batch_embedding_local() {
        let config = EmbeddingConfig {
            provider: "local".into(),
            api_key: String::new(),
            model: "local-hash".into(),
            dimensions: 512,
            api_url: None,
            cache_enabled: true,
            cache_max_entries: 100,
        };
        let provider = EmbeddingProvider::new(config);
        let embs = provider
            .embed_batch(&["hello world", "foo bar", "test query"])
            .await
            .unwrap();
        assert_eq!(embs.len(), 3);
        assert_eq!(embs[0].len(), 512);
    }

    #[tokio::test]
    async fn test_embedding_cache_hit() {
        let config = EmbeddingConfig {
            provider: "local".into(),
            api_key: String::new(),
            model: "local-hash".into(),
            dimensions: 256,
            api_url: None,
            cache_enabled: true,
            cache_max_entries: 100,
        };
        let provider = EmbeddingProvider::new(config);

        // First call — cache miss
        let emb1 = provider.embed("same text").await.unwrap();
        // Second call — cache hit
        let emb2 = provider.embed("same text").await.unwrap();
        assert_eq!(emb1, emb2);
    }
}
