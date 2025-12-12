use crate::analysis::AnalysisReport;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub baseline_tokens: i32,
    pub current_tokens: i32,
    pub diff: i32,
    pub diff_percent: f64,
    pub tool_changes: Vec<ToolChange>,
    pub passed: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChange {
    pub name: String,
    pub change_type: ChangeType,
    pub baseline_tokens: Option<i32>,
    pub current_tokens: Option<i32>,
    pub diff: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct ThresholdConfig {
    pub max_percent_increase: Option<f64>,
    pub max_absolute_increase: Option<i32>,
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            max_percent_increase: Some(5.0),
            max_absolute_increase: None,
        }
    }
}

pub fn compare_reports(
    baseline: &AnalysisReport,
    current: &AnalysisReport,
    thresholds: &ThresholdConfig,
) -> ComparisonResult {
    let diff = current.total_tokens - baseline.total_tokens;
    let diff_percent = if baseline.total_tokens > 0 {
        (diff as f64 / baseline.total_tokens as f64) * 100.0
    } else {
        0.0
    };

    // Build maps for tool comparison
    let baseline_tools: HashMap<&str, i32> = baseline
        .tools
        .items
        .iter()
        .map(|t| (t.name.as_str(), t.tokens))
        .collect();

    let current_tools: HashMap<&str, i32> = current
        .tools
        .items
        .iter()
        .map(|t| (t.name.as_str(), t.tokens))
        .collect();

    let mut tool_changes = Vec::new();

    // Find added and modified tools
    for (name, &current_tokens) in &current_tools {
        let change = if let Some(&baseline_tokens) = baseline_tools.get(name) {
            let tool_diff = current_tokens - baseline_tokens;
            if tool_diff != 0 {
                ToolChange {
                    name: name.to_string(),
                    change_type: ChangeType::Modified,
                    baseline_tokens: Some(baseline_tokens),
                    current_tokens: Some(current_tokens),
                    diff: tool_diff,
                }
            } else {
                continue; // Unchanged, skip
            }
        } else {
            ToolChange {
                name: name.to_string(),
                change_type: ChangeType::Added,
                baseline_tokens: None,
                current_tokens: Some(current_tokens),
                diff: current_tokens,
            }
        };
        tool_changes.push(change);
    }

    // Find removed tools
    for (name, &baseline_tokens) in &baseline_tools {
        if !current_tools.contains_key(name) {
            tool_changes.push(ToolChange {
                name: name.to_string(),
                change_type: ChangeType::Removed,
                baseline_tokens: Some(baseline_tokens),
                current_tokens: None,
                diff: -baseline_tokens,
            });
        }
    }

    // Sort by absolute diff descending
    tool_changes.sort_by(|a, b| b.diff.abs().cmp(&a.diff.abs()));

    // Check thresholds
    let mut failure_reason = None;

    if let Some(max_percent) = thresholds.max_percent_increase
        && diff_percent > max_percent
    {
        failure_reason = Some(format!(
            "Token increase of {:.1}% exceeds threshold of {:.1}%",
            diff_percent, max_percent
        ));
    }

    if failure_reason.is_none()
        && let Some(max_absolute) = thresholds.max_absolute_increase
        && diff > max_absolute
    {
        failure_reason = Some(format!(
            "Token increase of {} exceeds threshold of {}",
            diff, max_absolute
        ));
    }

    ComparisonResult {
        baseline_tokens: baseline.total_tokens,
        current_tokens: current.total_tokens,
        diff,
        diff_percent,
        tool_changes,
        passed: failure_reason.is_none(),
        failure_reason,
    }
}

impl ComparisonResult {
    pub fn format_text(&self) -> String {
        let mut out = String::new();

        out.push_str("Baseline Comparison\n");
        out.push_str(&format!("{}\n\n", "=".repeat(60)));

        out.push_str(&format!("Baseline: {} tokens\n", self.baseline_tokens));
        out.push_str(&format!("Current:  {} tokens\n", self.current_tokens));

        let sign = if self.diff >= 0 { "+" } else { "" };
        out.push_str(&format!(
            "Change:   {}{} tokens ({}{:.1}%)\n\n",
            sign, self.diff, sign, self.diff_percent
        ));

        if !self.tool_changes.is_empty() {
            out.push_str("Tool Changes:\n");
            out.push_str(&format!("{}\n", "-".repeat(40)));

            for change in &self.tool_changes {
                let sign = if change.diff >= 0 { "+" } else { "" };
                let change_str = match change.change_type {
                    ChangeType::Added => {
                        format!("[+] {} (added, {} tokens)", change.name, change.diff)
                    }
                    ChangeType::Removed => format!(
                        "[-] {} (removed, {} tokens)",
                        change.name,
                        change.baseline_tokens.unwrap_or(0)
                    ),
                    ChangeType::Modified => {
                        format!("[~] {} ({}{} tokens)", change.name, sign, change.diff)
                    }
                    ChangeType::Unchanged => continue,
                };
                out.push_str(&format!("  {}\n", change_str));
            }
            out.push('\n');
        }

        if self.passed {
            out.push_str("Result: PASSED\n");
        } else {
            out.push_str(&format!(
                "Result: FAILED - {}\n",
                self.failure_reason
                    .as_deref()
                    .unwrap_or("threshold exceeded")
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{CategoryTokens, CounterInfo, ItemTokens, ServerInfoReport};

    fn make_report(total: i32, tools: Vec<(&str, i32)>) -> AnalysisReport {
        AnalysisReport {
            counter: CounterInfo {
                provider: "test".to_string(),
                model: "test".to_string(),
            },
            server_info: ServerInfoReport {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
            },
            total_tokens: total,
            tools: CategoryTokens {
                total,
                count: tools.len(),
                items: tools
                    .into_iter()
                    .map(|(name, tokens)| ItemTokens {
                        name: name.to_string(),
                        tokens,
                        description_tokens: None,
                        schema_tokens: None,
                    })
                    .collect(),
            },
            resources: None,
            prompts: None,
        }
    }

    #[test]
    fn test_compare_no_change() {
        let baseline = make_report(1000, vec![("tool1", 500), ("tool2", 500)]);
        let current = make_report(1000, vec![("tool1", 500), ("tool2", 500)]);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(result.passed);
        assert_eq!(result.diff, 0);
        assert_eq!(result.diff_percent, 0.0);
        assert!(result.tool_changes.is_empty());
    }

    #[test]
    fn test_compare_increase_within_threshold() {
        let baseline = make_report(1000, vec![("tool1", 500), ("tool2", 500)]);
        let current = make_report(1040, vec![("tool1", 520), ("tool2", 520)]);
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(5.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(result.passed);
        assert_eq!(result.diff, 40);
        assert_eq!(result.diff_percent, 4.0);
    }

    #[test]
    fn test_compare_increase_exceeds_percent_threshold() {
        let baseline = make_report(1000, vec![("tool1", 500), ("tool2", 500)]);
        let current = make_report(1100, vec![("tool1", 550), ("tool2", 550)]);
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(5.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(!result.passed);
        assert_eq!(result.diff, 100);
        assert!(result.failure_reason.unwrap().contains("10.0%"));
    }

    #[test]
    fn test_compare_increase_exceeds_absolute_threshold() {
        let baseline = make_report(1000, vec![("tool1", 1000)]);
        let current = make_report(1200, vec![("tool1", 1200)]);
        let thresholds = ThresholdConfig {
            max_percent_increase: None,
            max_absolute_increase: Some(100),
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(!result.passed);
        assert_eq!(result.diff, 200);
    }

    #[test]
    fn test_compare_tool_added() {
        let baseline = make_report(500, vec![("tool1", 500)]);
        let current = make_report(1000, vec![("tool1", 500), ("tool2", 500)]);
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(200.0), // High threshold so it passes
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.tool_changes.len(), 1);
        assert!(matches!(
            result.tool_changes[0].change_type,
            ChangeType::Added
        ));
        assert_eq!(result.tool_changes[0].name, "tool2");
    }

    #[test]
    fn test_compare_tool_removed() {
        let baseline = make_report(1000, vec![("tool1", 500), ("tool2", 500)]);
        let current = make_report(500, vec![("tool1", 500)]);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(result.passed); // Decrease is allowed
        assert_eq!(result.diff, -500);
        assert_eq!(result.tool_changes.len(), 1);
        assert!(matches!(
            result.tool_changes[0].change_type,
            ChangeType::Removed
        ));
    }

    #[test]
    fn test_compare_tool_modified() {
        let baseline = make_report(1000, vec![("tool1", 400), ("tool2", 600)]);
        let current = make_report(1100, vec![("tool1", 500), ("tool2", 600)]);
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(20.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.tool_changes.len(), 1);
        assert!(matches!(
            result.tool_changes[0].change_type,
            ChangeType::Modified
        ));
        assert_eq!(result.tool_changes[0].name, "tool1");
        assert_eq!(result.tool_changes[0].diff, 100);
    }

    #[test]
    fn test_format_text_passed() {
        let result = ComparisonResult {
            baseline_tokens: 1000,
            current_tokens: 1000,
            diff: 0,
            diff_percent: 0.0,
            tool_changes: vec![],
            passed: true,
            failure_reason: None,
        };

        let text = result.format_text();
        assert!(text.contains("PASSED"));
        assert!(text.contains("1000 tokens"));
    }

    #[test]
    fn test_format_text_failed() {
        let result = ComparisonResult {
            baseline_tokens: 1000,
            current_tokens: 1100,
            diff: 100,
            diff_percent: 10.0,
            tool_changes: vec![],
            passed: false,
            failure_reason: Some("exceeded threshold".to_string()),
        };

        let text = result.format_text();
        assert!(text.contains("FAILED"));
        assert!(text.contains("exceeded threshold"));
    }
}
