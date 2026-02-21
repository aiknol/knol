//! Feature flags module for open-core business model.
//!
//! This module provides a flexible feature flag system to separate open-source (OSS),
//! Starter, Pro, and Enterprise features. Each tier includes specific capabilities,
//! and per-tenant overrides allow fine-grained control over feature availability.
//!
//! # Tiers
//!
//! - **OpenSource**: Free tier with basic memory management capabilities
//! - **Starter**: Enhanced features for small teams with multi-tenancy and basic analytics
//! - **Pro**: Advanced features for growing organizations with adaptive retrieval and more
//! - **Enterprise**: Complete feature set with SSO, audit logs, and custom integrations
//!
//! # Example
//!
//! ```ignore
//! let mut flags = FeatureFlags::new(Tier::Pro);
//! if flags.is_enabled(Feature::AuditLog) {
//!     // Audit functionality available
//! }
//!
//! flags.require(Feature::SSOAuth)?; // Returns error if not available
//! flags.with_override(Feature::DataResidency, true); // Enable for this tenant
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Feature variants for the memory platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Feature {
    // OpenSource Tier Features
    BasicMemoryStore,
    VectorSearch,
    BasicGraphExtraction,
    RESTApi,
    SingleTenant,
    BasicRetrieval,

    // Starter Tier Features
    MultiTenant,
    BM25Search,
    PiiRedaction,
    WebhookConnectors,
    BasicAnalytics,
    EmailSupport,

    // Pro Tier Features
    AdaptiveRetrieval,
    MemoryConsolidation,
    ConflictDetection,
    ScopeCascade,
    CustomConnectors,
    PolicyEngine,
    PrometheusMetrics,
    PrioritySupport,

    // Enterprise Tier Features
    SSOAuth,
    AuditLog,
    DataResidency,
    CustomLLM,
    DedicatedInfra,
    SLAGuarantee,
    BillingApi,
    WhiteLabel,
}

impl Feature {
    /// Get a human-readable name for this feature.
    pub fn name(&self) -> &'static str {
        match self {
            // OSS
            Feature::BasicMemoryStore => "Basic Memory Store",
            Feature::VectorSearch => "Vector Search",
            Feature::BasicGraphExtraction => "Basic Graph Extraction",
            Feature::RESTApi => "REST API",
            Feature::SingleTenant => "Single Tenant",
            Feature::BasicRetrieval => "Basic Retrieval",

            // Starter
            Feature::MultiTenant => "Multi-Tenant",
            Feature::BM25Search => "BM25 Search",
            Feature::PiiRedaction => "PII Redaction",
            Feature::WebhookConnectors => "Webhook Connectors",
            Feature::BasicAnalytics => "Basic Analytics",
            Feature::EmailSupport => "Email Support",

            // Pro
            Feature::AdaptiveRetrieval => "Adaptive Retrieval",
            Feature::MemoryConsolidation => "Memory Consolidation",
            Feature::ConflictDetection => "Conflict Detection",
            Feature::ScopeCascade => "Scope Cascade",
            Feature::CustomConnectors => "Custom Connectors",
            Feature::PolicyEngine => "Policy Engine",
            Feature::PrometheusMetrics => "Prometheus Metrics",
            Feature::PrioritySupport => "Priority Support",

            // Enterprise
            Feature::SSOAuth => "SSO Authentication",
            Feature::AuditLog => "Audit Logging",
            Feature::DataResidency => "Data Residency",
            Feature::CustomLLM => "Custom LLM",
            Feature::DedicatedInfra => "Dedicated Infrastructure",
            Feature::SLAGuarantee => "SLA Guarantee",
            Feature::BillingApi => "Billing API",
            Feature::WhiteLabel => "White Label",
        }
    }

    /// Get the minimum tier required for this feature.
    pub fn min_tier(&self) -> Tier {
        match self {
            Feature::BasicMemoryStore
            | Feature::VectorSearch
            | Feature::BasicGraphExtraction
            | Feature::RESTApi
            | Feature::SingleTenant
            | Feature::BasicRetrieval => Tier::OpenSource,

            Feature::MultiTenant
            | Feature::BM25Search
            | Feature::PiiRedaction
            | Feature::WebhookConnectors
            | Feature::BasicAnalytics
            | Feature::EmailSupport => Tier::Starter,

            Feature::AdaptiveRetrieval
            | Feature::MemoryConsolidation
            | Feature::ConflictDetection
            | Feature::ScopeCascade
            | Feature::CustomConnectors
            | Feature::PolicyEngine
            | Feature::PrometheusMetrics
            | Feature::PrioritySupport => Tier::Pro,

            Feature::SSOAuth
            | Feature::AuditLog
            | Feature::DataResidency
            | Feature::CustomLLM
            | Feature::DedicatedInfra
            | Feature::SLAGuarantee
            | Feature::BillingApi
            | Feature::WhiteLabel => Tier::Enterprise,
        }
    }
}

/// Platform tier representing pricing/feature level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    OpenSource,
    Starter,
    Pro,
    Enterprise,
}

impl Tier {
    /// Parse a tier from a string name (case-insensitive).
    pub fn from_name(name: &str) -> Result<Self, FeatureError> {
        match name.to_lowercase().as_str() {
            "opensource" | "open-source" | "oss" | "free" => Ok(Tier::OpenSource),
            "starter" | "basic" => Ok(Tier::Starter),
            "pro" | "professional" => Ok(Tier::Pro),
            "enterprise" | "custom" => Ok(Tier::Enterprise),
            _ => Err(FeatureError::InvalidTier(name.to_string())),
        }
    }

    /// Get a human-readable name for this tier.
    pub fn name(&self) -> &'static str {
        match self {
            Tier::OpenSource => "Open Source",
            Tier::Starter => "Starter",
            Tier::Pro => "Pro",
            Tier::Enterprise => "Enterprise",
        }
    }
}

/// Error type for feature flag operations.
#[derive(Error, Debug)]
pub enum FeatureError {
    #[error("Feature '{0}' is not available in your current tier")]
    FeatureNotAvailable(String),

    #[error("Invalid tier: {0}")]
    InvalidTier(String),

    #[error("Feature flag configuration error: {0}")]
    ConfigError(String),
}

/// Feature flags management for a tenant or deployment.
///
/// This struct tracks the current tier and maintains per-tenant feature overrides
/// for fine-grained control over feature availability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlags {
    /// Current tier for this tenant/deployment
    pub tier: Tier,

    /// Per-tenant feature overrides (overrides tier defaults)
    pub overrides: HashMap<Feature, bool>,
}

impl FeatureFlags {
    /// Create a new FeatureFlags instance for the given tier.
    pub fn new(tier: Tier) -> Self {
        Self {
            tier,
            overrides: HashMap::new(),
        }
    }

    /// Create FeatureFlags from a plan name string.
    pub fn from_plan_name(name: &str) -> Result<Self, FeatureError> {
        let tier = Tier::from_name(name)?;
        Ok(Self::new(tier))
    }

    /// Check if a feature is enabled for this tier and tenant.
    ///
    /// Returns true if:
    /// 1. There's an override set to true, OR
    /// 2. The feature is available in the current tier (and no override says false)
    pub fn is_enabled(&self, feature: Feature) -> bool {
        if let Some(&override_value) = self.overrides.get(&feature) {
            return override_value;
        }

        // Check if feature is in tier's default features
        Self::tier_features(self.tier).contains(&feature)
    }

    /// Require a feature to be enabled, returning an error if it's not.
    pub fn require(&self, feature: Feature) -> Result<(), FeatureError> {
        if self.is_enabled(feature) {
            Ok(())
        } else {
            Err(FeatureError::FeatureNotAvailable(
                feature.name().to_string(),
            ))
        }
    }

    /// Get all features available in a given tier.
    pub fn tier_features(tier: Tier) -> Vec<Feature> {
        match tier {
            Tier::OpenSource => vec![
                Feature::BasicMemoryStore,
                Feature::VectorSearch,
                Feature::BasicGraphExtraction,
                Feature::RESTApi,
                Feature::SingleTenant,
                Feature::BasicRetrieval,
            ],
            Tier::Starter => {
                let mut features = Self::tier_features(Tier::OpenSource);
                features.extend(vec![
                    Feature::MultiTenant,
                    Feature::BM25Search,
                    Feature::PiiRedaction,
                    Feature::WebhookConnectors,
                    Feature::BasicAnalytics,
                    Feature::EmailSupport,
                ]);
                features
            }
            Tier::Pro => {
                let mut features = Self::tier_features(Tier::Starter);
                features.extend(vec![
                    Feature::AdaptiveRetrieval,
                    Feature::MemoryConsolidation,
                    Feature::ConflictDetection,
                    Feature::ScopeCascade,
                    Feature::CustomConnectors,
                    Feature::PolicyEngine,
                    Feature::PrometheusMetrics,
                    Feature::PrioritySupport,
                ]);
                features
            }
            Tier::Enterprise => {
                let mut features = Self::tier_features(Tier::Pro);
                features.extend(vec![
                    Feature::SSOAuth,
                    Feature::AuditLog,
                    Feature::DataResidency,
                    Feature::CustomLLM,
                    Feature::DedicatedInfra,
                    Feature::SLAGuarantee,
                    Feature::BillingApi,
                    Feature::WhiteLabel,
                ]);
                features
            }
        }
    }

    /// Set a per-tenant feature override.
    pub fn with_override(&mut self, feature: Feature, enabled: bool) -> &mut Self {
        self.overrides.insert(feature, enabled);
        self
    }

    /// Set a per-tenant feature override (builder pattern).
    pub fn with_override_owned(mut self, feature: Feature, enabled: bool) -> Self {
        self.overrides.insert(feature, enabled);
        self
    }

    /// Remove an override for a feature, reverting to tier default.
    pub fn clear_override(&mut self, feature: Feature) {
        self.overrides.remove(&feature);
    }

    /// Get the number of features available in the current tier.
    pub fn feature_count(&self) -> usize {
        Self::tier_features(self.tier).len()
    }

    /// Get all currently active overrides.
    pub fn get_overrides(&self) -> &HashMap<Feature, bool> {
        &self.overrides
    }

    /// Check if a feature has an active override.
    pub fn has_override(&self, feature: Feature) -> bool {
        self.overrides.contains_key(&feature)
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self::new(Tier::OpenSource)
    }
}

/// Axum middleware layer for enforcing feature requirements on endpoints.
///
/// # Example
///
/// ```ignore
/// use axum::{
///     routing::get,
///     Router,
/// };
///
/// async fn audit_handler() -> String {
///     "Audit log data".to_string()
/// }
///
/// // In your router setup:
/// let router = Router::new()
///     .route(
///         "/v1/admin/audit",
///         get(audit_handler)
///             .layer(require_feature(Feature::AuditLog))
///     );
/// ```
pub mod middleware {
    use super::*;
    use axum::{
        extract::Request,
        http::StatusCode,
        middleware::Next,
        response::{IntoResponse, Response},
    };

    /// Axum middleware function that checks feature availability.
    ///
    /// Usage with `axum::middleware::from_fn`:
    /// ```rust,ignore
    /// use axum::middleware;
    /// use memory_common::features::{Feature, FeatureFlags, middleware::feature_gate};
    ///
    /// let app = Router::new()
    ///     .route("/v1/admin/audit", get(handler))
    ///     .layer(middleware::from_fn(move |req, next| {
    ///         feature_gate(Feature::AuditLog, req, next)
    ///     }));
    /// ```
    pub async fn feature_gate(required: Feature, req: Request, next: Next) -> Response {
        // Extract FeatureFlags from request extensions if available
        match req.extensions().get::<FeatureFlags>() {
            Some(flags) => {
                if !flags.is_enabled(required) {
                    return (
                        StatusCode::FORBIDDEN,
                        format!(
                            "Feature '{}' requires {} tier or higher",
                            required.name(),
                            required.min_tier().name()
                        ),
                    )
                        .into_response();
                }
            }
            None => {
                // If no flags in extensions, deny by default (fail-closed).
                // This prevents feature bypass when middleware ordering issues
                // or auth errors cause FeatureFlags to not be populated.
                tracing::warn!(
                    "FeatureFlags missing from request extensions — denying access to '{}'",
                    required.name()
                );
                return (
                    StatusCode::FORBIDDEN,
                    format!("Feature '{}' is not available", required.name()),
                )
                    .into_response();
            }
        }
        next.run(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_from_name() {
        assert_eq!(Tier::from_name("opensource").unwrap(), Tier::OpenSource);
        assert_eq!(Tier::from_name("open-source").unwrap(), Tier::OpenSource);
        assert_eq!(Tier::from_name("oss").unwrap(), Tier::OpenSource);
        assert_eq!(Tier::from_name("free").unwrap(), Tier::OpenSource);
        assert_eq!(Tier::from_name("starter").unwrap(), Tier::Starter);
        assert_eq!(Tier::from_name("basic").unwrap(), Tier::Starter);
        assert_eq!(Tier::from_name("pro").unwrap(), Tier::Pro);
        assert_eq!(Tier::from_name("professional").unwrap(), Tier::Pro);
        assert_eq!(Tier::from_name("enterprise").unwrap(), Tier::Enterprise);
        assert_eq!(Tier::from_name("custom").unwrap(), Tier::Enterprise);
        assert!(Tier::from_name("invalid").is_err());
    }

    #[test]
    fn test_tier_name() {
        assert_eq!(Tier::OpenSource.name(), "Open Source");
        assert_eq!(Tier::Starter.name(), "Starter");
        assert_eq!(Tier::Pro.name(), "Pro");
        assert_eq!(Tier::Enterprise.name(), "Enterprise");
    }

    #[test]
    fn test_feature_name() {
        assert_eq!(Feature::BasicMemoryStore.name(), "Basic Memory Store");
        assert_eq!(Feature::AuditLog.name(), "Audit Logging");
        assert_eq!(Feature::SSOAuth.name(), "SSO Authentication");
    }

    #[test]
    fn test_feature_flags_new() {
        let flags = FeatureFlags::new(Tier::Pro);
        assert_eq!(flags.tier, Tier::Pro);
        assert!(flags.overrides.is_empty());
    }

    #[test]
    fn test_feature_flags_default() {
        let flags = FeatureFlags::default();
        assert_eq!(flags.tier, Tier::OpenSource);
        assert!(flags.overrides.is_empty());
    }

    #[test]
    fn test_feature_flags_from_plan_name() {
        let flags = FeatureFlags::from_plan_name("pro").unwrap();
        assert_eq!(flags.tier, Tier::Pro);

        let err = FeatureFlags::from_plan_name("invalid_tier").unwrap_err();
        assert!(matches!(err, FeatureError::InvalidTier(_)));
    }

    #[test]
    fn test_opensource_features() {
        let flags = FeatureFlags::new(Tier::OpenSource);

        // Should have OSS features
        assert!(flags.is_enabled(Feature::BasicMemoryStore));
        assert!(flags.is_enabled(Feature::VectorSearch));
        assert!(flags.is_enabled(Feature::BasicGraphExtraction));
        assert!(flags.is_enabled(Feature::RESTApi));
        assert!(flags.is_enabled(Feature::SingleTenant));
        assert!(flags.is_enabled(Feature::BasicRetrieval));

        // Should NOT have Starter+ features
        assert!(!flags.is_enabled(Feature::MultiTenant));
        assert!(!flags.is_enabled(Feature::BM25Search));
        assert!(!flags.is_enabled(Feature::AuditLog));
        assert!(!flags.is_enabled(Feature::SSOAuth));
    }

    #[test]
    fn test_starter_features() {
        let flags = FeatureFlags::new(Tier::Starter);

        // Should have OSS features
        assert!(flags.is_enabled(Feature::BasicMemoryStore));
        assert!(flags.is_enabled(Feature::VectorSearch));

        // Should have Starter features
        assert!(flags.is_enabled(Feature::MultiTenant));
        assert!(flags.is_enabled(Feature::BM25Search));
        assert!(flags.is_enabled(Feature::PiiRedaction));
        assert!(flags.is_enabled(Feature::BasicAnalytics));

        // Should NOT have Pro+ features
        assert!(!flags.is_enabled(Feature::AdaptiveRetrieval));
        assert!(!flags.is_enabled(Feature::AuditLog));
        assert!(!flags.is_enabled(Feature::SSOAuth));
    }

    #[test]
    fn test_pro_features() {
        let flags = FeatureFlags::new(Tier::Pro);

        // Should have everything up to Pro
        assert!(flags.is_enabled(Feature::BasicMemoryStore));
        assert!(flags.is_enabled(Feature::MultiTenant));
        assert!(flags.is_enabled(Feature::AdaptiveRetrieval));
        assert!(flags.is_enabled(Feature::MemoryConsolidation));
        assert!(flags.is_enabled(Feature::PolicyEngine));
        assert!(flags.is_enabled(Feature::PrometheusMetrics));

        // Should NOT have Enterprise features
        assert!(!flags.is_enabled(Feature::SSOAuth));
        assert!(!flags.is_enabled(Feature::AuditLog));
        assert!(!flags.is_enabled(Feature::DataResidency));
    }

    #[test]
    fn test_enterprise_features() {
        let flags = FeatureFlags::new(Tier::Enterprise);

        // Should have everything
        assert!(flags.is_enabled(Feature::BasicMemoryStore));
        assert!(flags.is_enabled(Feature::MultiTenant));
        assert!(flags.is_enabled(Feature::AdaptiveRetrieval));
        assert!(flags.is_enabled(Feature::SSOAuth));
        assert!(flags.is_enabled(Feature::AuditLog));
        assert!(flags.is_enabled(Feature::DataResidency));
        assert!(flags.is_enabled(Feature::CustomLLM));
        assert!(flags.is_enabled(Feature::DedicatedInfra));
        assert!(flags.is_enabled(Feature::SLAGuarantee));
        assert!(flags.is_enabled(Feature::BillingApi));
        assert!(flags.is_enabled(Feature::WhiteLabel));
    }

    #[test]
    fn test_require_feature_success() {
        let flags = FeatureFlags::new(Tier::Pro);
        assert!(flags.require(Feature::AdaptiveRetrieval).is_ok());
    }

    #[test]
    fn test_require_feature_failure() {
        let flags = FeatureFlags::new(Tier::OpenSource);
        let result = flags.require(Feature::AuditLog);
        assert!(result.is_err());
        assert!(matches!(result, Err(FeatureError::FeatureNotAvailable(_))));
    }

    #[test]
    fn test_with_override() {
        let mut flags = FeatureFlags::new(Tier::OpenSource);

        // AuditLog normally not available in OSS
        assert!(!flags.is_enabled(Feature::AuditLog));

        // Override to enable it
        flags.with_override(Feature::AuditLog, true);
        assert!(flags.is_enabled(Feature::AuditLog));

        // Override to disable it again
        flags.with_override(Feature::AuditLog, false);
        assert!(!flags.is_enabled(Feature::AuditLog));
    }

    #[test]
    fn test_with_override_owned() {
        let flags = FeatureFlags::new(Tier::OpenSource)
            .with_override_owned(Feature::AuditLog, true)
            .clone();

        assert!(flags.is_enabled(Feature::AuditLog));
    }

    #[test]
    fn test_clear_override() {
        let mut flags = FeatureFlags::new(Tier::OpenSource);
        flags.with_override(Feature::AuditLog, true);
        assert!(flags.is_enabled(Feature::AuditLog));

        flags.clear_override(Feature::AuditLog);
        assert!(!flags.is_enabled(Feature::AuditLog));
    }

    #[test]
    fn test_has_override() {
        let mut flags = FeatureFlags::new(Tier::OpenSource);
        assert!(!flags.has_override(Feature::AuditLog));

        flags.with_override(Feature::AuditLog, true);
        assert!(flags.has_override(Feature::AuditLog));

        flags.clear_override(Feature::AuditLog);
        assert!(!flags.has_override(Feature::AuditLog));
    }

    #[test]
    fn test_get_overrides() {
        let mut flags = FeatureFlags::new(Tier::OpenSource);
        flags.with_override(Feature::AuditLog, true);
        flags.with_override(Feature::SSOAuth, false);

        let overrides = flags.get_overrides();
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides.get(&Feature::AuditLog), Some(&true));
        assert_eq!(overrides.get(&Feature::SSOAuth), Some(&false));
    }

    #[test]
    fn test_feature_count() {
        assert_eq!(FeatureFlags::new(Tier::OpenSource).feature_count(), 6);
        assert_eq!(FeatureFlags::new(Tier::Starter).feature_count(), 12);
        assert_eq!(FeatureFlags::new(Tier::Pro).feature_count(), 20);
        assert_eq!(FeatureFlags::new(Tier::Enterprise).feature_count(), 28);
    }

    #[test]
    fn test_tier_features_cumulative() {
        let oss_features = FeatureFlags::tier_features(Tier::OpenSource);
        let starter_features = FeatureFlags::tier_features(Tier::Starter);
        let pro_features = FeatureFlags::tier_features(Tier::Pro);
        let enterprise_features = FeatureFlags::tier_features(Tier::Enterprise);

        // Each tier should include all previous tier features
        for feature in &oss_features {
            assert!(starter_features.contains(feature));
            assert!(pro_features.contains(feature));
            assert!(enterprise_features.contains(feature));
        }

        for feature in &starter_features {
            assert!(pro_features.contains(feature));
            assert!(enterprise_features.contains(feature));
        }

        for feature in &pro_features {
            assert!(enterprise_features.contains(feature));
        }

        // Tier feature counts
        assert!(starter_features.len() > oss_features.len());
        assert!(pro_features.len() > starter_features.len());
        assert!(enterprise_features.len() > pro_features.len());
    }

    #[test]
    fn test_multiple_overrides() {
        let mut flags = FeatureFlags::new(Tier::Starter);

        // Starter has BasicAnalytics, add some overrides
        flags.with_override(Feature::AuditLog, true);
        flags.with_override(Feature::DataResidency, false);

        // Check overrides take precedence
        assert!(flags.is_enabled(Feature::AuditLog));
        assert!(!flags.is_enabled(Feature::DataResidency));

        // Tier defaults still work
        assert!(flags.is_enabled(Feature::BasicAnalytics));

        // Clear one override
        flags.clear_override(Feature::AuditLog);
        assert!(!flags.is_enabled(Feature::AuditLog)); // Back to Starter default
    }

    #[test]
    fn test_serialization() {
        let mut flags = FeatureFlags::new(Tier::Pro);
        flags.with_override(Feature::AuditLog, true);

        let json = serde_json::to_string(&flags).unwrap();
        let deserialized: FeatureFlags = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tier, Tier::Pro);
        assert!(deserialized.is_enabled(Feature::AuditLog));
        assert!(deserialized.is_enabled(Feature::AdaptiveRetrieval));
    }

    #[test]
    fn test_override_takes_precedence_over_tier() {
        let mut flags = FeatureFlags::new(Tier::Enterprise);

        // Enterprise has AuditLog by default
        assert!(flags.is_enabled(Feature::AuditLog));

        // Override to disable it
        flags.with_override(Feature::AuditLog, false);
        assert!(!flags.is_enabled(Feature::AuditLog));

        // Clear override, should be back to enabled
        flags.clear_override(Feature::AuditLog);
        assert!(flags.is_enabled(Feature::AuditLog));
    }

    #[test]
    fn test_feature_error_display() {
        let err = FeatureError::FeatureNotAvailable("Audit Log".to_string());
        assert_eq!(
            err.to_string(),
            "Feature 'Audit Log' is not available in your current tier"
        );

        let err = FeatureError::InvalidTier("invalid".to_string());
        assert_eq!(err.to_string(), "Invalid tier: invalid");
    }

    #[test]
    fn test_all_features_assigned_to_tiers() {
        // Make sure every feature is covered by at least one tier
        let all_features = vec![
            Feature::BasicMemoryStore,
            Feature::VectorSearch,
            Feature::BasicGraphExtraction,
            Feature::RESTApi,
            Feature::SingleTenant,
            Feature::BasicRetrieval,
            Feature::MultiTenant,
            Feature::BM25Search,
            Feature::PiiRedaction,
            Feature::WebhookConnectors,
            Feature::BasicAnalytics,
            Feature::EmailSupport,
            Feature::AdaptiveRetrieval,
            Feature::MemoryConsolidation,
            Feature::ConflictDetection,
            Feature::ScopeCascade,
            Feature::CustomConnectors,
            Feature::PolicyEngine,
            Feature::PrometheusMetrics,
            Feature::PrioritySupport,
            Feature::SSOAuth,
            Feature::AuditLog,
            Feature::DataResidency,
            Feature::CustomLLM,
            Feature::DedicatedInfra,
            Feature::SLAGuarantee,
            Feature::BillingApi,
            Feature::WhiteLabel,
        ];

        let enterprise_features = FeatureFlags::tier_features(Tier::Enterprise);

        for feature in all_features {
            assert!(
                enterprise_features.contains(&feature),
                "Feature {:?} not assigned to any tier",
                feature
            );
        }
    }

    #[test]
    fn test_case_insensitive_tier_parsing() {
        assert_eq!(Tier::from_name("OPENSOURCE").unwrap(), Tier::OpenSource);
        assert_eq!(Tier::from_name("Pro").unwrap(), Tier::Pro);
        assert_eq!(Tier::from_name("ENTERPRISE").unwrap(), Tier::Enterprise);
    }
}
