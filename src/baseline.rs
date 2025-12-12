//! Multi-provider baseline support.
//!
//! Baselines can contain token counts from multiple providers, allowing
//! like-for-like comparisons regardless of which provider is available.

use crate::analysis::AnalysisReport;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current baseline format version.
pub const BASELINE_VERSION: u32 = 1;

/// Multi-provider baseline containing reports from multiple token counters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiProviderBaseline {
    /// Format version for future compatibility.
    pub version: u32,
    /// Reports keyed by provider name (e.g., "anthropic", "tiktoken").
    pub providers: HashMap<String, AnalysisReport>,
}

impl MultiProviderBaseline {
    /// Create a new empty baseline.
    pub fn new() -> Self {
        Self {
            version: BASELINE_VERSION,
            providers: HashMap::new(),
        }
    }

    /// Add a report for a provider.
    pub fn add_report(&mut self, report: AnalysisReport) {
        let provider = report.counter.provider.clone();
        self.providers.insert(provider, report);
    }

    /// Get a report for a specific provider.
    pub fn get_report(&self, provider: &str) -> Option<&AnalysisReport> {
        self.providers.get(provider)
    }

    /// List available providers.
    pub fn available_providers(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for MultiProviderBaseline {
    fn default() -> Self {
        Self::new()
    }
}

/// Baseline format that can be either single-provider (legacy) or multi-provider.
#[derive(Debug, Clone)]
pub enum Baseline {
    /// Legacy single-provider format (just an AnalysisReport).
    Single(AnalysisReport),
    /// Multi-provider format with reports from multiple counters.
    Multi(MultiProviderBaseline),
}

impl Baseline {
    /// Parse a baseline from JSON, auto-detecting the format.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        // Try multi-provider format first (has "version" and "providers" fields)
        if let Ok(multi) = serde_json::from_str::<MultiProviderBaseline>(json) {
            return Ok(Baseline::Multi(multi));
        }

        // Fall back to legacy single-provider format
        let single: AnalysisReport = serde_json::from_str(json)?;
        Ok(Baseline::Single(single))
    }

    /// Get the report for a specific provider, if available.
    pub fn get_report(&self, provider: &str) -> Option<&AnalysisReport> {
        match self {
            Baseline::Single(report) => {
                if report.counter.provider == provider {
                    Some(report)
                } else {
                    None
                }
            }
            Baseline::Multi(multi) => multi.get_report(provider),
        }
    }

    /// Get any available report, preferring the specified provider.
    /// Returns the report and whether it's an exact provider match.
    pub fn get_best_report(&self, preferred_provider: &str) -> Option<(&AnalysisReport, bool)> {
        match self {
            Baseline::Single(report) => {
                let exact_match = report.counter.provider == preferred_provider;
                Some((report, exact_match))
            }
            Baseline::Multi(multi) => {
                // Try exact match first
                if let Some(report) = multi.get_report(preferred_provider) {
                    return Some((report, true));
                }
                // Fall back to any available provider
                multi
                    .providers
                    .values()
                    .next()
                    .map(|report| (report, false))
            }
        }
    }

    /// List available providers in this baseline.
    pub fn available_providers(&self) -> Vec<&str> {
        match self {
            Baseline::Single(report) => vec![report.counter.provider.as_str()],
            Baseline::Multi(multi) => multi.available_providers(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{CategoryTokens, CounterInfo, ServerInfoReport};

    fn make_report(provider: &str, total: i32) -> AnalysisReport {
        AnalysisReport {
            counter: CounterInfo {
                provider: provider.to_string(),
                model: "test".to_string(),
            },
            server_info: ServerInfoReport {
                name: "test-server".to_string(),
                version: "1.0.0".to_string(),
            },
            total_tokens: total,
            tools: CategoryTokens {
                total,
                count: 1,
                items: vec![],
            },
            resources: None,
            prompts: None,
        }
    }

    #[test]
    fn test_multi_provider_baseline() {
        let mut baseline = MultiProviderBaseline::new();
        baseline.add_report(make_report("anthropic", 1000));
        baseline.add_report(make_report("tiktoken", 950));

        assert_eq!(baseline.get_report("anthropic").unwrap().total_tokens, 1000);
        assert_eq!(baseline.get_report("tiktoken").unwrap().total_tokens, 950);
        assert!(baseline.get_report("unknown").is_none());
    }

    #[test]
    fn test_baseline_from_json_multi() {
        let json = r#"{
            "version": 1,
            "providers": {
                "tiktoken": {
                    "counter": { "provider": "tiktoken", "model": "cl100k_base" },
                    "server_info": { "name": "test", "version": "1.0" },
                    "total_tokens": 500,
                    "tools": { "total": 500, "count": 1, "items": [] }
                }
            }
        }"#;

        let baseline = Baseline::from_json(json).unwrap();
        assert!(matches!(baseline, Baseline::Multi(_)));
        assert!(baseline.get_report("tiktoken").is_some());
    }

    #[test]
    fn test_baseline_from_json_single() {
        let json = r#"{
            "counter": { "provider": "anthropic", "model": "claude-3" },
            "server_info": { "name": "test", "version": "1.0" },
            "total_tokens": 1000,
            "tools": { "total": 1000, "count": 1, "items": [] }
        }"#;

        let baseline = Baseline::from_json(json).unwrap();
        assert!(matches!(baseline, Baseline::Single(_)));
        assert!(baseline.get_report("anthropic").is_some());
    }

    #[test]
    fn test_get_best_report_exact_match() {
        let mut multi = MultiProviderBaseline::new();
        multi.add_report(make_report("anthropic", 1000));
        multi.add_report(make_report("tiktoken", 950));
        let baseline = Baseline::Multi(multi);

        let (report, exact) = baseline.get_best_report("tiktoken").unwrap();
        assert!(exact);
        assert_eq!(report.total_tokens, 950);
    }

    #[test]
    fn test_get_best_report_fallback() {
        let mut multi = MultiProviderBaseline::new();
        multi.add_report(make_report("tiktoken", 950));
        let baseline = Baseline::Multi(multi);

        let (report, exact) = baseline.get_best_report("anthropic").unwrap();
        assert!(!exact);
        assert_eq!(report.counter.provider, "tiktoken");
    }
}
