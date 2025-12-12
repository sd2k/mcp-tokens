//! Multi-provider, multi-model baseline support.
//!
//! Baselines can contain token counts from multiple providers and models,
//! allowing like-for-like comparisons regardless of which provider/model
//! combination is available.

use crate::analysis::AnalysisReport;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current baseline format version.
pub const BASELINE_VERSION: u32 = 2;

/// Provider-specific baseline containing reports for multiple models.
pub type ProviderModels = HashMap<String, AnalysisReport>;

/// Multi-provider, multi-model baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiProviderBaseline {
    /// Format version for future compatibility.
    pub version: u32,
    /// Reports keyed by provider name, then model name.
    /// e.g., providers["anthropic"]["claude-sonnet-4-20250514"] = report
    pub providers: HashMap<String, ProviderModels>,
}

impl MultiProviderBaseline {
    /// Create a new empty baseline.
    pub fn new() -> Self {
        Self {
            version: BASELINE_VERSION,
            providers: HashMap::new(),
        }
    }

    /// Add a report for a provider/model combination.
    pub fn add_report(&mut self, report: AnalysisReport) {
        let provider = report.counter.provider.clone();
        let model = report.counter.model.clone();
        self.providers
            .entry(provider)
            .or_default()
            .insert(model, report);
    }

    /// Get a report for a specific provider and model.
    pub fn get_report(&self, provider: &str, model: &str) -> Option<&AnalysisReport> {
        self.providers.get(provider)?.get(model)
    }

    /// Get any report for a provider (first available model).
    pub fn get_any_report_for_provider(&self, provider: &str) -> Option<&AnalysisReport> {
        self.providers.get(provider)?.values().next()
    }

    /// List available providers.
    pub fn available_providers(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }

    /// List available models for a provider.
    pub fn available_models(&self, provider: &str) -> Vec<&str> {
        self.providers
            .get(provider)
            .map(|models| models.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }
}

impl Default for MultiProviderBaseline {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy v1 format (provider -> report, no model nesting).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyMultiProviderBaseline {
    version: u32,
    providers: HashMap<String, AnalysisReport>,
}

/// Baseline format that can be single-provider (legacy), v1 multi-provider, or v2 multi-model.
#[derive(Debug, Clone)]
pub enum Baseline {
    /// Legacy single-provider format (just an AnalysisReport).
    Single(AnalysisReport),
    /// Multi-provider, multi-model format.
    Multi(MultiProviderBaseline),
}

impl Baseline {
    /// Parse a baseline from JSON, auto-detecting the format.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        // Try v2 multi-provider/multi-model format first
        if let Ok(multi) = serde_json::from_str::<MultiProviderBaseline>(json)
            && multi.version >= 2
        {
            return Ok(Baseline::Multi(multi));
        }

        // Try v1 legacy multi-provider format (provider -> report directly)
        if let Ok(legacy) = serde_json::from_str::<LegacyMultiProviderBaseline>(json)
            && legacy.version == 1
        {
            // Convert v1 to v2 format
            let mut multi = MultiProviderBaseline::new();
            for (_, report) in legacy.providers {
                multi.add_report(report);
            }
            return Ok(Baseline::Multi(multi));
        }

        // Fall back to legacy single-provider format
        let single: AnalysisReport = serde_json::from_str(json)?;
        Ok(Baseline::Single(single))
    }

    /// Get the report for a specific provider and model, if available.
    pub fn get_report(&self, provider: &str, model: &str) -> Option<&AnalysisReport> {
        match self {
            Baseline::Single(report) => {
                if report.counter.provider == provider && report.counter.model == model {
                    Some(report)
                } else {
                    None
                }
            }
            Baseline::Multi(multi) => multi.get_report(provider, model),
        }
    }

    /// Get the best matching report for the given provider and model.
    /// Returns (report, exact_provider_match, exact_model_match).
    pub fn get_best_report(
        &self,
        preferred_provider: &str,
        preferred_model: &str,
    ) -> Option<(&AnalysisReport, bool, bool)> {
        match self {
            Baseline::Single(report) => {
                let provider_match = report.counter.provider == preferred_provider;
                let model_match = report.counter.model == preferred_model;
                Some((report, provider_match, model_match))
            }
            Baseline::Multi(multi) => {
                // Try exact provider + model match
                if let Some(report) = multi.get_report(preferred_provider, preferred_model) {
                    return Some((report, true, true));
                }

                // Try same provider, any model
                if let Some(report) = multi.get_any_report_for_provider(preferred_provider) {
                    return Some((report, true, false));
                }

                // Fall back to any available provider/model
                for models in multi.providers.values() {
                    if let Some(report) = models.values().next() {
                        return Some((report, false, false));
                    }
                }

                None
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

    fn make_report(provider: &str, model: &str, total: i32) -> AnalysisReport {
        AnalysisReport {
            counter: CounterInfo {
                provider: provider.to_string(),
                model: model.to_string(),
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
    fn test_multi_provider_multi_model_baseline() {
        let mut baseline = MultiProviderBaseline::new();
        baseline.add_report(make_report("anthropic", "claude-sonnet-4-20250514", 1000));
        baseline.add_report(make_report("anthropic", "claude-3-5-haiku-20241022", 980));
        baseline.add_report(make_report("tiktoken", "cl100k_base", 950));

        assert_eq!(
            baseline
                .get_report("anthropic", "claude-sonnet-4-20250514")
                .unwrap()
                .total_tokens,
            1000
        );
        assert_eq!(
            baseline
                .get_report("anthropic", "claude-3-5-haiku-20241022")
                .unwrap()
                .total_tokens,
            980
        );
        assert_eq!(
            baseline
                .get_report("tiktoken", "cl100k_base")
                .unwrap()
                .total_tokens,
            950
        );
        assert!(baseline.get_report("tiktoken", "o200k_base").is_none());
    }

    #[test]
    fn test_available_models() {
        let mut baseline = MultiProviderBaseline::new();
        baseline.add_report(make_report("anthropic", "claude-sonnet-4-20250514", 1000));
        baseline.add_report(make_report("anthropic", "claude-3-5-haiku-20241022", 980));

        let models = baseline.available_models("anthropic");
        assert_eq!(models.len(), 2);
        assert!(models.contains(&"claude-sonnet-4-20250514"));
        assert!(models.contains(&"claude-3-5-haiku-20241022"));
    }

    #[test]
    fn test_baseline_from_json_v2() {
        let json = r#"{
            "version": 2,
            "providers": {
                "tiktoken": {
                    "cl100k_base": {
                        "counter": { "provider": "tiktoken", "model": "cl100k_base" },
                        "server_info": { "name": "test", "version": "1.0" },
                        "total_tokens": 500,
                        "tools": { "total": 500, "count": 1, "items": [] }
                    }
                }
            }
        }"#;

        let baseline = Baseline::from_json(json).unwrap();
        assert!(matches!(baseline, Baseline::Multi(_)));
        assert!(baseline.get_report("tiktoken", "cl100k_base").is_some());
    }

    #[test]
    fn test_baseline_from_json_v1_upgrade() {
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
        // v1 gets upgraded, so we can query by provider+model
        assert!(baseline.get_report("tiktoken", "cl100k_base").is_some());
    }

    #[test]
    fn test_baseline_from_json_single() {
        let json = r#"{
            "counter": { "provider": "anthropic", "model": "claude-sonnet-4-20250514" },
            "server_info": { "name": "test", "version": "1.0" },
            "total_tokens": 1000,
            "tools": { "total": 1000, "count": 1, "items": [] }
        }"#;

        let baseline = Baseline::from_json(json).unwrap();
        assert!(matches!(baseline, Baseline::Single(_)));
        assert!(
            baseline
                .get_report("anthropic", "claude-sonnet-4-20250514")
                .is_some()
        );
    }

    #[test]
    fn test_get_best_report_exact_match() {
        let mut multi = MultiProviderBaseline::new();
        multi.add_report(make_report("anthropic", "claude-sonnet-4-20250514", 1000));
        multi.add_report(make_report("tiktoken", "cl100k_base", 950));
        let baseline = Baseline::Multi(multi);

        let (report, provider_match, model_match) =
            baseline.get_best_report("tiktoken", "cl100k_base").unwrap();
        assert!(provider_match);
        assert!(model_match);
        assert_eq!(report.total_tokens, 950);
    }

    #[test]
    fn test_get_best_report_provider_match_model_fallback() {
        let mut multi = MultiProviderBaseline::new();
        multi.add_report(make_report("anthropic", "claude-sonnet-4-20250514", 1000));
        let baseline = Baseline::Multi(multi);

        let (report, provider_match, model_match) = baseline
            .get_best_report("anthropic", "claude-3-5-haiku-20241022")
            .unwrap();
        assert!(provider_match);
        assert!(!model_match);
        assert_eq!(report.counter.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_get_best_report_full_fallback() {
        let mut multi = MultiProviderBaseline::new();
        multi.add_report(make_report("tiktoken", "cl100k_base", 950));
        let baseline = Baseline::Multi(multi);

        let (report, provider_match, model_match) = baseline
            .get_best_report("anthropic", "claude-sonnet-4-20250514")
            .unwrap();
        assert!(!provider_match);
        assert!(!model_match);
        assert_eq!(report.counter.provider, "tiktoken");
    }
}
