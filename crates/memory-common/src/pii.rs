//! PII (Personally Identifiable Information) detection and redaction module.
//!
//! This module provides configurable PII detection and redaction capabilities,
//! allowing you to scan text for sensitive information and apply various
//! redaction policies.

use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ops::Range;

/// Types of Personally Identifiable Information that can be detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PiiType {
    /// Email addresses
    Email,
    /// Phone numbers
    Phone,
    /// Social Security Numbers
    SSN,
    /// Credit card numbers
    CreditCard,
    /// IP addresses
    IpAddress,
    /// Dates of birth
    DateOfBirth,
    /// Names (requires custom detection)
    Name,
    /// Addresses (requires custom detection)
    Address,
    /// Custom PII type
    Custom(&'static str),
}

impl PiiType {
    /// Returns a human-readable name for this PII type.
    pub fn as_str(&self) -> &str {
        match self {
            PiiType::Email => "Email",
            PiiType::Phone => "Phone",
            PiiType::SSN => "SSN",
            PiiType::CreditCard => "CreditCard",
            PiiType::IpAddress => "IpAddress",
            PiiType::DateOfBirth => "DateOfBirth",
            PiiType::Name => "Name",
            PiiType::Address => "Address",
            PiiType::Custom(name) => name,
        }
    }
}

/// Redaction policies for handling detected PII.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiiPolicy {
    /// Replace with [REDACTED:type]
    Redact,
    /// Replace with masked characters (X's)
    Mask,
    /// Replace with hash of original value
    Hash,
    /// Allow the PII to pass through unchanged
    Allow,
}

/// Result of a single PII detection.
#[derive(Debug, Clone)]
pub struct PiiMatch {
    /// Type of PII detected
    pub pii_type: PiiType,
    /// Start position in text (inclusive)
    pub start: usize,
    /// End position in text (exclusive)
    pub end: usize,
    /// The matched text
    pub text: String,
}

impl PiiMatch {
    /// Returns the span as a Range
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// Result of redaction operation
#[derive(Debug, Clone)]
pub struct RedactionResult {
    /// The redacted/cleaned text
    pub text: String,
    /// List of PII that was redacted
    pub redactions: Vec<RedactionInfo>,
}

/// Information about a single redaction.
///
/// Custom Debug impl hides `original_text` to prevent PII leaking into logs.
#[derive(Clone)]
pub struct RedactionInfo {
    /// Type of PII that was redacted
    pub pii_type: PiiType,
    /// Original text before redaction
    pub original_text: String,
    /// Policy applied
    pub policy: PiiPolicy,
}

impl std::fmt::Debug for RedactionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedactionInfo")
            .field("pii_type", &self.pii_type)
            .field("original_text", &"[REDACTED]")
            .field("policy", &self.policy)
            .finish()
    }
}

/// PII Detector with configurable policies
pub struct PiiDetector {
    policies: HashMap<PiiType, PiiPolicy>,
    regexes: HashMap<PiiType, Regex>,
}

impl Default for PiiDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl PiiDetector {
    /// Creates a new PII detector with default policies (Redact for all types).
    pub fn new() -> Self {
        let mut policies = HashMap::new();
        policies.insert(PiiType::Email, PiiPolicy::Redact);
        policies.insert(PiiType::Phone, PiiPolicy::Redact);
        policies.insert(PiiType::SSN, PiiPolicy::Redact);
        policies.insert(PiiType::CreditCard, PiiPolicy::Redact);
        policies.insert(PiiType::IpAddress, PiiPolicy::Redact);
        policies.insert(PiiType::DateOfBirth, PiiPolicy::Redact);
        policies.insert(PiiType::Name, PiiPolicy::Allow);
        policies.insert(PiiType::Address, PiiPolicy::Allow);

        let mut regexes = HashMap::new();

        // Email pattern
        regexes.insert(
            PiiType::Email,
            Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap(),
        );

        // Phone pattern: handles various formats
        regexes.insert(
            PiiType::Phone,
            Regex::new(r"(\+?1[-.\s]?)?\(?[0-9]{3}\)?[-.\s]?[0-9]{3}[-.\s]?[0-9]{4}").unwrap(),
        );

        // SSN pattern
        regexes.insert(PiiType::SSN, Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap());

        // Credit Card pattern
        regexes.insert(
            PiiType::CreditCard,
            Regex::new(r"\b(?:\d{4}[-\s]?){3}\d{4}\b").unwrap(),
        );

        // IP Address pattern
        regexes.insert(
            PiiType::IpAddress,
            Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").unwrap(),
        );

        // Date of birth pattern: YYYY-MM-DD or MM/DD/YYYY or DD/MM/YYYY
        regexes.insert(
            PiiType::DateOfBirth,
            Regex::new(r"(?:\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{2,4})").unwrap(),
        );

        PiiDetector { policies, regexes }
    }

    /// Sets the policy for a specific PII type.
    pub fn set_policy(&mut self, pii_type: PiiType, policy: PiiPolicy) {
        self.policies.insert(pii_type, policy);
    }

    /// Gets the policy for a specific PII type.
    pub fn get_policy(&self, pii_type: PiiType) -> PiiPolicy {
        self.policies
            .get(&pii_type)
            .copied()
            .unwrap_or(PiiPolicy::Allow)
    }

    /// Detects all PII in the given text.
    ///
    /// Returns a vector of PiiMatch containing all detected PII with their types and positions.
    pub fn detect(&self, text: &str) -> Vec<PiiMatch> {
        let mut matches = Vec::new();

        // Check all standard patterns
        for (pii_type, regex) in &self.regexes {
            for mat in regex.find_iter(text) {
                matches.push(PiiMatch {
                    pii_type: *pii_type,
                    start: mat.start(),
                    end: mat.end(),
                    text: mat.as_str().to_string(),
                });
            }
        }

        // Sort by position
        matches.sort_by_key(|m| m.start);

        matches
    }

    /// Applies redaction policies to detected PII.
    ///
    /// Returns a RedactionResult containing the cleaned text and a list of what was redacted.
    pub fn redact(&self, text: &str) -> RedactionResult {
        let matches = self.detect(text);
        let mut result_text = text.to_string();
        let mut redactions = Vec::new();

        // Process matches in reverse order to maintain correct positions
        for pii_match in matches.iter().rev() {
            let policy = self.get_policy(pii_match.pii_type);

            // Skip if policy is Allow
            if policy == PiiPolicy::Allow {
                continue;
            }

            let replacement = match policy {
                PiiPolicy::Redact => format!("[REDACTED:{}]", pii_match.pii_type.as_str()),
                PiiPolicy::Mask => {
                    let len = pii_match.text.len();
                    let masked = if len <= 4 {
                        "X".repeat(len)
                    } else {
                        format!("{}***{}", &pii_match.text[..2], &pii_match.text[len - 2..])
                    };
                    masked
                }
                PiiPolicy::Hash => {
                    // Cryptographic SHA-256 hash — irreversible, safe for PII
                    let mut hasher = Sha256::new();
                    hasher.update(pii_match.text.as_bytes());
                    let digest = hasher.finalize();
                    // Use first 8 bytes (16 hex chars) for readability while staying secure
                    format!(
                        "[HASHED:{}:{}]",
                        pii_match.pii_type.as_str(),
                        hex::encode(&digest[..8])
                    )
                }
                PiiPolicy::Allow => pii_match.text.clone(),
            };

            result_text.replace_range(pii_match.start..pii_match.end, &replacement);

            redactions.push(RedactionInfo {
                pii_type: pii_match.pii_type,
                original_text: pii_match.text.clone(),
                policy,
            });
        }

        // Reverse redactions list since we processed in reverse order
        redactions.reverse();

        RedactionResult {
            text: result_text,
            redactions,
        }
    }

    /// Adds a custom regex pattern for a custom PII type.
    pub fn add_custom_pattern(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let regex = Regex::new(pattern)?;
        self.regexes.insert(PiiType::Custom("custom"), regex);
        self.policies
            .insert(PiiType::Custom("custom"), PiiPolicy::Redact);
        Ok(())
    }

    /// Checks if any PII is present in the text.
    pub fn has_pii(&self, text: &str) -> bool {
        !self.detect(text).is_empty()
    }

    /// Gets a summary of PII types found in the text.
    pub fn get_pii_summary(&self, text: &str) -> HashMap<String, usize> {
        let matches = self.detect(text);
        let mut summary = HashMap::new();

        for pii_match in matches {
            let type_name = pii_match.pii_type.as_str().to_string();
            *summary.entry(type_name).or_insert(0) += 1;
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_detection() {
        let detector = PiiDetector::new();
        let text = "Contact me at john.doe@example.com for more info";
        let matches = detector.detect(text);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pii_type, PiiType::Email);
        assert_eq!(matches[0].text, "john.doe@example.com");
        assert_eq!(matches[0].start, 14);
        assert_eq!(matches[0].end, 34);
    }

    #[test]
    fn test_multiple_emails() {
        let detector = PiiDetector::new();
        let text = "Send to alice@test.com or bob@example.org";
        let matches = detector.detect(text);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].text, "alice@test.com");
        assert_eq!(matches[1].text, "bob@example.org");
    }

    #[test]
    fn test_phone_detection() {
        let detector = PiiDetector::new();
        let text = "Call me at (555) 123-4567";
        let matches = detector.detect(text);

        let phone_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::Phone)
            .collect();

        assert_eq!(phone_matches.len(), 1);
        assert_eq!(phone_matches[0].text, "(555) 123-4567");
    }

    #[test]
    fn test_phone_variations() {
        let detector = PiiDetector::new();
        let texts = vec![
            "555-123-4567",
            "(555) 123-4567",
            "555.123.4567",
            "+1 (555) 123-4567",
            "1-555-123-4567",
        ];

        for text in texts {
            let matches = detector.detect(text);
            let phone_matches: Vec<_> = matches
                .iter()
                .filter(|m| m.pii_type == PiiType::Phone)
                .collect();
            assert_eq!(phone_matches.len(), 1, "Failed for: {}", text);
        }
    }

    #[test]
    fn test_ssn_detection() {
        let detector = PiiDetector::new();
        let text = "My SSN is 123-45-6789";
        let matches = detector.detect(text);

        let ssn_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::SSN)
            .collect();

        assert_eq!(ssn_matches.len(), 1);
        assert_eq!(ssn_matches[0].text, "123-45-6789");
    }

    #[test]
    fn test_credit_card_detection() {
        let detector = PiiDetector::new();
        let text = "Card number: 4532-1234-5678-9010";
        let matches = detector.detect(text);

        let cc_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::CreditCard)
            .collect();

        assert_eq!(cc_matches.len(), 1);
        assert_eq!(cc_matches[0].text, "4532-1234-5678-9010");
    }

    #[test]
    fn test_credit_card_no_dashes() {
        let detector = PiiDetector::new();
        let text = "Card: 4532123456789010";
        let matches = detector.detect(text);

        let cc_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::CreditCard)
            .collect();

        assert_eq!(cc_matches.len(), 1);
        assert_eq!(cc_matches[0].text, "4532123456789010");
    }

    #[test]
    fn test_ip_address_detection() {
        let detector = PiiDetector::new();
        let text = "Server IP: 192.168.1.1";
        let matches = detector.detect(text);

        let ip_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::IpAddress)
            .collect();

        assert_eq!(ip_matches.len(), 1);
        assert_eq!(ip_matches[0].text, "192.168.1.1");
    }

    #[test]
    fn test_date_of_birth_detection() {
        let detector = PiiDetector::new();
        let texts = vec![
            "Born on 1990-05-15",
            "DOB: 05/15/1990",
            "Date of birth 5/15/90",
        ];

        for text in texts {
            let matches = detector.detect(text);
            let dob_matches: Vec<_> = matches
                .iter()
                .filter(|m| m.pii_type == PiiType::DateOfBirth)
                .collect();
            assert!(!dob_matches.is_empty(), "Failed for: {}", text);
        }
    }

    #[test]
    fn test_redact_policy() {
        let detector = PiiDetector::new();
        let text = "Email: john@example.com Phone: 555-123-4567";

        let result = detector.redact(text);

        assert!(result.text.contains("[REDACTED:Email]"));
        assert!(result.text.contains("[REDACTED:Phone]"));
        assert!(!result.text.contains("john@example.com"));
        assert!(!result.text.contains("555-123-4567"));
        assert_eq!(result.redactions.len(), 2);
    }

    #[test]
    fn test_mask_policy() {
        let mut detector = PiiDetector::new();
        detector.set_policy(PiiType::Email, PiiPolicy::Mask);

        let text = "Contact: john@example.com";
        let result = detector.redact(text);

        // Email should be masked
        assert!(!result.text.contains("john@example.com"));
        assert!(result.text.contains("jo") || result.text.contains("***"));
    }

    #[test]
    fn test_hash_policy() {
        let mut detector = PiiDetector::new();
        detector.set_policy(PiiType::Email, PiiPolicy::Hash);

        let text = "Contact: john@example.com";
        let result = detector.redact(text);

        // Email should be hashed with SHA-256
        assert!(!result.text.contains("john@example.com"));
        assert!(result.text.contains("[HASHED:Email:"));
        // Verify determinism: same input produces same hash
        let result2 = detector.redact(text);
        assert_eq!(result.text, result2.text);
    }

    #[test]
    fn test_allow_policy() {
        let mut detector = PiiDetector::new();
        detector.set_policy(PiiType::Email, PiiPolicy::Allow);

        let text = "Contact: john@example.com";
        let result = detector.redact(text);

        // Email should remain
        assert!(result.text.contains("john@example.com"));
        assert_eq!(result.redactions.len(), 0);
    }

    #[test]
    fn test_mixed_policies() {
        let mut detector = PiiDetector::new();
        detector.set_policy(PiiType::Email, PiiPolicy::Redact);
        detector.set_policy(PiiType::Phone, PiiPolicy::Mask);

        let text = "Email: john@example.com Phone: 555-123-4567";
        let result = detector.redact(text);

        assert!(result.text.contains("[REDACTED:Email]"));
        assert!(!result.text.contains("555-123-4567"));
        assert_eq!(result.redactions.len(), 2);
    }

    #[test]
    fn test_has_pii() {
        let detector = PiiDetector::new();

        assert!(detector.has_pii("Email: john@example.com"));
        assert!(detector.has_pii("Call: 555-123-4567"));
        assert!(!detector.has_pii("Just regular text"));
    }

    #[test]
    fn test_pii_summary() {
        let detector = PiiDetector::new();
        let text = "Email john@test.com and alice@test.com. Phone: 555-123-4567 and 555-987-6543";

        let summary = detector.get_pii_summary(text);

        assert_eq!(summary.get("Email").copied(), Some(2));
        assert_eq!(summary.get("Phone").copied(), Some(2));
    }

    #[test]
    fn test_overlapping_matches() {
        let detector = PiiDetector::new();
        let text = "192.168.1.1 is my IP and 555-123-4567 is my phone";

        let matches = detector.detect(text);
        assert_eq!(matches.len(), 2);

        // Ensure no span overlaps
        for i in 0..matches.len() {
            for j in (i + 1)..matches.len() {
                assert!(matches[i].end <= matches[j].start || matches[j].end <= matches[i].start);
            }
        }
    }

    #[test]
    fn test_redaction_result_info() {
        let detector = PiiDetector::new();
        let text = "Email: john@example.com";
        let result = detector.redact(text);

        assert_eq!(result.redactions.len(), 1);
        assert_eq!(result.redactions[0].pii_type, PiiType::Email);
        assert_eq!(result.redactions[0].original_text, "john@example.com");
        assert_eq!(result.redactions[0].policy, PiiPolicy::Redact);
    }

    #[test]
    fn test_empty_text() {
        let detector = PiiDetector::new();
        let result = detector.redact("");

        assert_eq!(result.text, "");
        assert_eq!(result.redactions.len(), 0);
    }

    #[test]
    fn test_no_pii_text() {
        let detector = PiiDetector::new();
        let text = "This is just regular text without any personal information";
        let result = detector.redact(text);

        assert_eq!(result.text, text);
        assert_eq!(result.redactions.len(), 0);
    }

    #[test]
    fn test_complex_text() {
        let detector = PiiDetector::new();
        let text = "User john@example.com called 555-123-4567 to report issue with IP 192.168.1.100. SSN mentioned as 123-45-6789.";

        let result = detector.redact(text);

        assert!(result.text.contains("[REDACTED:Email]"));
        assert!(result.text.contains("[REDACTED:Phone]"));
        assert!(result.text.contains("[REDACTED:IpAddress]"));
        assert!(result.text.contains("[REDACTED:SSN]"));
        assert_eq!(result.redactions.len(), 4);
    }
}
