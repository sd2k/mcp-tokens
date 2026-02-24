use crate::analysis::AnalysisReport;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub baseline_tokens: i32,
    pub current_tokens: i32,
    pub baseline_provider: String,
    pub current_provider: String,
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

    // When tools are counted individually, each count includes provider framing
    // overhead (e.g. Anthropic's tool-use system prompt) that only appears once
    // in the real context. We derive this overhead so we can strip it from
    // added/removed tool diffs (for modified tools the overhead cancels out).
    //   batch_total  = framing + Σ(content_i)
    //   individual_i = framing + content_i
    //   overhead     = (Σ(individual_i) - batch_total) / (N - 1)
    let compute_overhead = |report: &AnalysisReport| -> i32 {
        let n = report.tools.items.len() as i32;
        let raw_sum: i32 = report.tools.items.iter().map(|t| t.tokens).sum();
        if n > 1 && raw_sum > report.tools.total {
            (raw_sum - report.tools.total) / (n - 1)
        } else {
            0
        }
    };
    let baseline_overhead = compute_overhead(baseline);
    let current_overhead = compute_overhead(current);

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
            // For modified tools, the framing overhead is present on both sides
            // and cancels out in the diff, so we use the raw values directly.
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
            // For added tools, subtract the framing overhead so the diff
            // reflects the tool's actual content cost.
            let content_tokens = (current_tokens - current_overhead).max(0);
            ToolChange {
                name: name.to_string(),
                change_type: ChangeType::Added,
                baseline_tokens: None,
                current_tokens: Some(content_tokens),
                diff: content_tokens,
            }
        };
        tool_changes.push(change);
    }

    // Find removed tools
    for (name, &baseline_tokens) in &baseline_tools {
        if !current_tools.contains_key(name) {
            // For removed tools, subtract the framing overhead so the diff
            // reflects the tool's actual content cost.
            let content_tokens = (baseline_tokens - baseline_overhead).max(0);
            tool_changes.push(ToolChange {
                name: name.to_string(),
                change_type: ChangeType::Removed,
                baseline_tokens: Some(content_tokens),
                current_tokens: None,
                diff: -content_tokens,
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
        baseline_provider: baseline.counter.provider.clone(),
        current_provider: current.counter.provider.clone(),
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

        if self.baseline_provider != self.current_provider {
            out.push_str(&format!(
                "WARNING: Provider mismatch! Baseline used '{}', current uses '{}'\n",
                self.baseline_provider, self.current_provider
            ));
            out.push_str("Token counts may not be directly comparable.\n\n");
        }

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

    /// Create a report with no framing overhead (sum of items == total).
    /// This simulates tiktoken or any provider where individual counts
    /// don't include shared framing.
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

    /// Create a report with framing overhead, simulating the Anthropic case.
    /// `total` is the batch count (framing + Σ content), while each tool's
    /// token count is `content + overhead` (as returned by individual API calls).
    /// The overhead per tool is derived as (Σ items - total) / (N - 1).
    fn make_report_with_overhead(
        total: i32,
        tools: Vec<(&str, i32)>,
        overhead: i32,
    ) -> AnalysisReport {
        let items: Vec<ItemTokens> = tools
            .into_iter()
            .map(|(name, content_tokens)| ItemTokens {
                name: name.to_string(),
                tokens: content_tokens + overhead,
                description_tokens: None,
                schema_tokens: None,
            })
            .collect();
        AnalysisReport {
            counter: CounterInfo {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-5-20250929".to_string(),
            },
            server_info: ServerInfoReport {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
            },
            total_tokens: total,
            tools: CategoryTokens {
                total,
                count: items.len(),
                items,
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
            baseline_provider: "tiktoken".to_string(),
            current_provider: "tiktoken".to_string(),
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
            baseline_provider: "tiktoken".to_string(),
            current_provider: "tiktoken".to_string(),
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

    #[test]
    fn test_format_text_provider_mismatch() {
        let result = ComparisonResult {
            baseline_tokens: 1000,
            current_tokens: 1000,
            baseline_provider: "anthropic".to_string(),
            current_provider: "tiktoken".to_string(),
            diff: 0,
            diff_percent: 0.0,
            tool_changes: vec![],
            passed: true,
            failure_reason: None,
        };

        let text = result.format_text();
        assert!(text.contains("WARNING"));
        assert!(text.contains("Provider mismatch"));
        assert!(text.contains("anthropic"));
        assert!(text.contains("tiktoken"));
    }

    #[test]
    fn test_compare_preserves_providers() {
        let mut baseline = make_report(1000, vec![("tool1", 1000)]);
        baseline.counter.provider = "anthropic".to_string();

        let mut current = make_report(1000, vec![("tool1", 1000)]);
        current.counter.provider = "tiktoken".to_string();

        let thresholds = ThresholdConfig::default();
        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.baseline_provider, "anthropic");
        assert_eq!(result.current_provider, "tiktoken");
    }

    // ---------------------------------------------------------------
    // Framing overhead tests
    // ---------------------------------------------------------------

    #[test]
    fn test_overhead_no_changes_identical_reports() {
        // Two identical reports with 500 overhead per tool.
        // batch_total = overhead + content1 + content2 = 500 + 200 + 300 = 1000
        // item1 = 200 + 500 = 700, item2 = 300 + 500 = 800
        let baseline = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 500);
        let current = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 500);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(result.passed);
        assert_eq!(result.diff, 0);
        assert!(
            result.tool_changes.is_empty(),
            "Expected no tool changes, got: {:?}",
            result.tool_changes
        );
    }

    #[test]
    fn test_overhead_no_changes_different_overhead() {
        // Same tools, same content, but overhead differs between runs by ±1
        // (e.g. due to integer division). The raw item tokens will differ
        // but the tools haven't actually changed — the overhead cancels in
        // the diff since it shifts both tools equally.
        //
        // Baseline: overhead=500, items=[700, 800], total=1000
        // Current:  overhead=501, items=[701, 801], total=1000
        // tool1 diff = 701 - 700 = +1, tool2 diff = 801 - 800 = +1
        // These are real modified diffs (±1), which is fine — they reflect
        // that the measurement shifted. The important thing is the headline
        // diff is 0.
        let baseline = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 500);
        let current = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 501);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(result.passed);
        assert_eq!(result.diff, 0);
        // The per-tool diffs are ±1 due to overhead shift, but headline is correct
        assert_eq!(result.baseline_tokens, 1000);
        assert_eq!(result.current_tokens, 1000);
    }

    #[test]
    fn test_overhead_tool_added_strips_framing() {
        // Baseline: 2 tools, overhead=500
        //   batch_total = 500 + 200 + 300 = 1000
        //   items = [700, 800]
        // Current: 3 tools, overhead=500
        //   batch_total = 500 + 200 + 300 + 150 = 1150
        //   items = [700, 800, 650]
        let baseline = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 500);
        let current = make_report_with_overhead(
            1150,
            vec![("tool1", 200), ("tool2", 300), ("tool3", 150)],
            500,
        );
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(100.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, 150); // headline: 1150 - 1000
        assert_eq!(result.tool_changes.len(), 1);
        assert!(matches!(
            result.tool_changes[0].change_type,
            ChangeType::Added
        ));
        assert_eq!(result.tool_changes[0].name, "tool3");
        // The added tool should report content cost (150), not content+overhead (650)
        assert_eq!(result.tool_changes[0].diff, 150);
        assert_eq!(result.tool_changes[0].current_tokens, Some(150));
    }

    #[test]
    fn test_overhead_tool_removed_strips_framing() {
        // Baseline: 3 tools, overhead=500
        //   batch_total = 500 + 200 + 300 + 400 = 1400
        //   items = [700, 800, 900]
        // Current: 2 tools, overhead=500
        //   batch_total = 500 + 200 + 300 = 1000
        //   items = [700, 800]
        let baseline = make_report_with_overhead(
            1400,
            vec![("tool1", 200), ("tool2", 300), ("tool3", 400)],
            500,
        );
        let current = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 500);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -400); // headline: 1000 - 1400
        assert_eq!(result.tool_changes.len(), 1);
        assert!(matches!(
            result.tool_changes[0].change_type,
            ChangeType::Removed
        ));
        assert_eq!(result.tool_changes[0].name, "tool3");
        // The removed tool should report content cost (400), not content+overhead (900)
        assert_eq!(result.tool_changes[0].diff, -400);
        assert_eq!(result.tool_changes[0].baseline_tokens, Some(400));
    }

    #[test]
    fn test_overhead_tool_modified_unaffected() {
        // Modified tools should use raw diffs since overhead cancels.
        // Baseline: overhead=500, tool1 content=200 → item=700
        // Current:  overhead=500, tool1 content=350 → item=850
        // Diff should be 850-700 = 150 (same as content diff 350-200)
        let baseline = make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], 500);
        let current = make_report_with_overhead(1150, vec![("tool1", 350), ("tool2", 300)], 500);
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(100.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, 150); // headline
        assert_eq!(result.tool_changes.len(), 1);
        assert!(matches!(
            result.tool_changes[0].change_type,
            ChangeType::Modified
        ));
        assert_eq!(result.tool_changes[0].name, "tool1");
        assert_eq!(result.tool_changes[0].diff, 150);
    }

    #[test]
    fn test_overhead_mixed_add_remove_modify() {
        // Baseline: 4 tools, overhead=500
        //   content: tool1=200, tool2=300, tool3=400, tool4=100
        //   batch_total = 500 + 200 + 300 + 400 + 100 = 1500
        //   items = [700, 800, 900, 600]
        // Current: 4 tools (tool3 removed, tool5 added, tool1 modified), overhead=500
        //   content: tool1=250, tool2=300, tool4=100, tool5=180
        //   batch_total = 500 + 250 + 300 + 100 + 180 = 1330
        //   items = [750, 800, 600, 680]
        let baseline = make_report_with_overhead(
            1500,
            vec![
                ("tool1", 200),
                ("tool2", 300),
                ("tool3", 400),
                ("tool4", 100),
            ],
            500,
        );
        let current = make_report_with_overhead(
            1330,
            vec![
                ("tool1", 250),
                ("tool2", 300),
                ("tool4", 100),
                ("tool5", 180),
            ],
            500,
        );
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(100.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -170); // headline: 1330 - 1500

        // Should have 3 changes: tool1 modified, tool3 removed, tool5 added
        assert_eq!(result.tool_changes.len(), 3);

        let by_name: HashMap<&str, &ToolChange> = result
            .tool_changes
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        // tool1: modified, raw diff = 750 - 700 = 50 (overhead cancels)
        let tool1 = by_name["tool1"];
        assert!(matches!(tool1.change_type, ChangeType::Modified));
        assert_eq!(tool1.diff, 50);

        // tool3: removed, content cost = 400 (not 900)
        let tool3 = by_name["tool3"];
        assert!(matches!(tool3.change_type, ChangeType::Removed));
        assert_eq!(tool3.diff, -400);
        assert_eq!(tool3.baseline_tokens, Some(400));

        // tool5: added, content cost = 180 (not 680)
        let tool5 = by_name["tool5"];
        assert!(matches!(tool5.change_type, ChangeType::Added));
        assert_eq!(tool5.diff, 180);
        assert_eq!(tool5.current_tokens, Some(180));

        // Sum of tool diffs should match headline
        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(
            tool_diff_sum, result.diff,
            "Sum of tool diffs ({}) should equal headline diff ({})",
            tool_diff_sum, result.diff
        );
    }

    #[test]
    fn test_overhead_diffs_sum_to_headline_add_only() {
        // Adding a single tool. Verify sum of tool diffs == headline diff.
        // Baseline: 3 tools, overhead=496
        //   content: 200, 300, 400 → batch = 496 + 900 = 1396
        //   items = [696, 796, 896]
        // Current: 4 tools, overhead=496
        //   content: 200, 300, 400, 150 → batch = 496 + 1050 = 1546
        //   items = [696, 796, 896, 646]
        let baseline =
            make_report_with_overhead(1396, vec![("a", 200), ("b", 300), ("c", 400)], 496);
        let current = make_report_with_overhead(
            1546,
            vec![("a", 200), ("b", 300), ("c", 400), ("d", 150)],
            496,
        );
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(100.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, 150);
        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(tool_diff_sum, result.diff);
    }

    #[test]
    fn test_overhead_diffs_sum_to_headline_remove_only() {
        // Removing a single tool. Verify sum of tool diffs == headline diff.
        let baseline = make_report_with_overhead(
            1546,
            vec![("a", 200), ("b", 300), ("c", 400), ("d", 150)],
            496,
        );
        let current =
            make_report_with_overhead(1396, vec![("a", 200), ("b", 300), ("c", 400)], 496);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -150);
        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(tool_diff_sum, result.diff);
    }

    #[test]
    fn test_overhead_zero_when_no_framing() {
        // When sum(items) == total (tiktoken), overhead is 0 and added/removed
        // tools just use their raw token count.
        let baseline = make_report(1000, vec![("tool1", 400), ("tool2", 600)]);
        let current = make_report(700, vec![("tool1", 400), ("tool3", 300)]);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -300);

        let by_name: HashMap<&str, &ToolChange> = result
            .tool_changes
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        // tool2 removed: no overhead to strip, so diff = -600
        let tool2 = by_name["tool2"];
        assert_eq!(tool2.diff, -600);
        assert_eq!(tool2.baseline_tokens, Some(600));

        // tool3 added: no overhead to strip, so diff = +300
        let tool3 = by_name["tool3"];
        assert_eq!(tool3.diff, 300);
        assert_eq!(tool3.current_tokens, Some(300));
    }

    #[test]
    fn test_overhead_single_tool_no_division_by_zero() {
        // With only 1 tool, N-1 = 0, so overhead should be 0 (no crash).
        let baseline = make_report_with_overhead(500, vec![("tool1", 500)], 0);
        // Manually set items to have overhead but only 1 tool
        let mut current = make_report(800, vec![("tool2", 1000)]);
        // total=800 but item=1000 → raw_sum > total but N=1, so overhead=0
        current.tools.total = 800;
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(200.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        // Should not panic; tool2 added with raw tokens since overhead=0
        let added: Vec<_> = result
            .tool_changes
            .iter()
            .filter(|c| matches!(c.change_type, ChangeType::Added))
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].diff, 1000); // No overhead subtracted
    }

    #[test]
    fn test_overhead_large_realistic_scenario() {
        // Simulate a realistic Anthropic scenario: 57 tools → 54 tools
        // with ~496 overhead per tool.
        let overhead = 496;

        // Baseline: 57 tools
        let mut baseline_tools = Vec::new();
        for i in 0..57 {
            baseline_tools.push((format!("tool_{i}"), 100 + i * 10));
        }
        let baseline_content_sum: i32 = baseline_tools.iter().map(|(_, c)| c).sum();
        let baseline_total = overhead + baseline_content_sum; // framing + Σ content

        let baseline_tool_refs: Vec<(&str, i32)> = baseline_tools
            .iter()
            .map(|(n, c)| (n.as_str(), *c))
            .collect();
        let baseline = make_report_with_overhead(baseline_total, baseline_tool_refs, overhead);

        // Current: remove tools 54,55,56 and add a new tool
        let mut current_tools = Vec::new();
        for i in 0..54 {
            current_tools.push((format!("tool_{i}"), 100 + i * 10));
        }
        current_tools.push(("tool_new".to_string(), 250));
        let current_content_sum: i32 = current_tools.iter().map(|(_, c)| c).sum();
        let current_total = overhead + current_content_sum;

        let current_tool_refs: Vec<(&str, i32)> = current_tools
            .iter()
            .map(|(n, c)| (n.as_str(), *c))
            .collect();
        let current = make_report_with_overhead(current_total, current_tool_refs, overhead);

        let thresholds = ThresholdConfig {
            max_percent_increase: Some(100.0),
            max_absolute_increase: None,
        };
        let result = compare_reports(&baseline, &current, &thresholds);

        // Headline should match
        assert_eq!(result.diff, current_total - baseline_total);

        // Removed tools should show content cost, not content + overhead.
        // The removed tools are tool_54 (content=640), tool_55 (content=650),
        // tool_56 (content=660). Their raw individual counts would be
        // content+496, so we verify the overhead has been stripped.
        let removed: Vec<_> = result
            .tool_changes
            .iter()
            .filter(|c| matches!(c.change_type, ChangeType::Removed))
            .collect();
        assert_eq!(removed.len(), 3);

        let expected_removed: HashMap<&str, i32> =
            HashMap::from([("tool_54", 640), ("tool_55", 650), ("tool_56", 660)]);
        for r in &removed {
            let expected_content = expected_removed[r.name.as_str()];
            assert_eq!(
                r.baseline_tokens.unwrap(),
                expected_content,
                "Removed tool {} reported {} tokens, expected content cost {} (raw would be {})",
                r.name,
                r.baseline_tokens.unwrap(),
                expected_content,
                expected_content + overhead,
            );
            assert_eq!(r.diff, -expected_content);
        }

        // Added tool should show content cost
        let added: Vec<_> = result
            .tool_changes
            .iter()
            .filter(|c| matches!(c.change_type, ChangeType::Added))
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].name, "tool_new");
        assert_eq!(added[0].diff, 250);

        // No unchanged tools should appear
        let modified: Vec<_> = result
            .tool_changes
            .iter()
            .filter(|c| matches!(c.change_type, ChangeType::Modified))
            .collect();
        assert!(
            modified.is_empty(),
            "Unchanged tools should not appear as modified, got: {:?}",
            modified.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        // Sum of tool diffs should match headline
        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(
            tool_diff_sum, result.diff,
            "Sum of tool diffs ({}) should equal headline diff ({})",
            tool_diff_sum, result.diff
        );
    }

    #[test]
    fn test_overhead_replaced_tool_same_content() {
        // One tool removed, one tool added with exactly the same content size.
        // Headline diff should be 0 and tool diffs should sum to 0.
        let overhead = 500;
        let baseline = make_report_with_overhead(
            1500, // 500 + 400 + 600
            vec![("old_tool", 400), ("shared", 600)],
            overhead,
        );
        let current = make_report_with_overhead(
            1500, // 500 + 400 + 600
            vec![("new_tool", 400), ("shared", 600)],
            overhead,
        );
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, 0);
        assert_eq!(result.tool_changes.len(), 2);

        let by_name: HashMap<&str, &ToolChange> = result
            .tool_changes
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        assert_eq!(by_name["old_tool"].diff, -400);
        assert_eq!(by_name["new_tool"].diff, 400);

        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(tool_diff_sum, 0);
    }

    #[test]
    fn test_overhead_all_tools_replaced() {
        // Every tool is different between baseline and current.
        let overhead = 500;
        let baseline = make_report_with_overhead(
            1200, // 500 + 300 + 400
            vec![("old1", 300), ("old2", 400)],
            overhead,
        );
        let current = make_report_with_overhead(
            1100, // 500 + 250 + 350
            vec![("new1", 250), ("new2", 350)],
            overhead,
        );
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -100); // 1100 - 1200
        assert_eq!(result.tool_changes.len(), 4);

        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(
            tool_diff_sum, result.diff,
            "Sum of tool diffs ({}) should equal headline diff ({})",
            tool_diff_sum, result.diff
        );
    }

    #[test]
    fn test_overhead_with_modification_and_removal() {
        // One tool modified, one removed. Verify both diffs are correct.
        let overhead = 500;
        // Baseline: framing(500) + tool1(200) + tool2(300) = 1000
        let baseline =
            make_report_with_overhead(1000, vec![("tool1", 200), ("tool2", 300)], overhead);
        // Current: framing(500) + tool1(250) = 750 (tool2 removed)
        // N=1 so overhead=0 for current (can't compute with 1 tool)
        // But we need N>1 for overhead to be computed, so let's keep 2 tools:
        // Actually let's test the N=1 edge case here.
        let current = AnalysisReport {
            counter: CounterInfo {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-5-20250929".to_string(),
            },
            server_info: ServerInfoReport {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
            },
            total_tokens: 750,
            tools: CategoryTokens {
                total: 750,
                count: 1,
                items: vec![ItemTokens {
                    name: "tool1".to_string(),
                    tokens: 250 + overhead, // raw individual count still has overhead
                    description_tokens: None,
                    schema_tokens: None,
                }],
            },
            resources: None,
            prompts: None,
        };
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -250); // 750 - 1000

        let by_name: HashMap<&str, &ToolChange> = result
            .tool_changes
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        // tool1 modified: raw diff = 750 - 700 = 50 (overhead cancels)
        assert!(matches!(by_name["tool1"].change_type, ChangeType::Modified));
        assert_eq!(by_name["tool1"].diff, 50);

        // tool2 removed: content = 300 (baseline overhead stripped)
        assert!(matches!(by_name["tool2"].change_type, ChangeType::Removed));
        assert_eq!(by_name["tool2"].diff, -300);
    }

    #[test]
    fn test_overhead_compute_is_correct() {
        // Directly verify the overhead computation.
        // 3 tools with content 100, 200, 300 and overhead 496.
        // batch_total = 496 + 600 = 1096
        // items = [596, 696, 796], sum = 2088
        // overhead = (2088 - 1096) / (3 - 1) = 992 / 2 = 496 ✓
        let report = make_report_with_overhead(1096, vec![("a", 100), ("b", 200), ("c", 300)], 496);
        let raw_sum: i32 = report.tools.items.iter().map(|t| t.tokens).sum();
        let n = report.tools.items.len() as i32;
        let computed = (raw_sum - report.tools.total) / (n - 1);
        assert_eq!(computed, 496);
    }

    #[test]
    fn test_overhead_asymmetric_baseline_and_current() {
        // Baseline has different overhead than current (e.g. different API behavior
        // across runs). The overhead is computed independently per side.
        // Baseline: overhead=500, tools: a(200), b(300) → batch=1000
        // Current:  overhead=510, tools: a(200), c(150) → batch=860
        // (b removed, c added)
        let baseline = make_report_with_overhead(1000, vec![("a", 200), ("b", 300)], 500);
        let current = make_report_with_overhead(860, vec![("a", 200), ("c", 150)], 510);
        let thresholds = ThresholdConfig::default();

        let result = compare_reports(&baseline, &current, &thresholds);

        assert_eq!(result.diff, -140); // 860 - 1000

        let by_name: HashMap<&str, &ToolChange> = result
            .tool_changes
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        // b removed: content=300 (baseline overhead=500 stripped)
        assert_eq!(by_name["b"].diff, -300);

        // c added: content=150 (current overhead=510 stripped)
        assert_eq!(by_name["c"].diff, 150);

        // a unchanged: raw diff = (200+510) - (200+500) = 10
        // This is overhead drift, not a real change — but it's reported as modified
        assert_eq!(by_name["a"].diff, 10);

        // Tool diffs should sum to headline
        let tool_diff_sum: i32 = result.tool_changes.iter().map(|c| c.diff).sum();
        assert_eq!(tool_diff_sum, result.diff);
    }

    #[test]
    fn test_overhead_decrease_passes_threshold() {
        // A decrease should always pass (thresholds only check increases).
        let overhead = 496;
        let baseline =
            make_report_with_overhead(2000, vec![("a", 400), ("b", 500), ("c", 600)], overhead);
        let current = make_report_with_overhead(
            1400, // removed tool c
            vec![("a", 400), ("b", 500)],
            overhead,
        );
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(1.0), // Very strict
            max_absolute_increase: Some(10),
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        assert!(result.passed);
        assert_eq!(result.diff, -600);
    }

    #[test]
    fn test_overhead_sorted_by_absolute_diff() {
        // Verify tool changes are sorted by absolute diff descending.
        let overhead = 500;
        let baseline = make_report_with_overhead(
            2000,
            vec![("a", 100), ("b", 200), ("c", 300), ("d", 400)],
            overhead,
        );
        let current =
            make_report_with_overhead(1800, vec![("a", 100), ("b", 250), ("e", 350)], overhead);
        let thresholds = ThresholdConfig {
            max_percent_increase: Some(100.0),
            max_absolute_increase: None,
        };

        let result = compare_reports(&baseline, &current, &thresholds);

        // Verify sorted by |diff| descending
        for i in 1..result.tool_changes.len() {
            assert!(
                result.tool_changes[i - 1].diff.abs() >= result.tool_changes[i].diff.abs(),
                "Tool changes not sorted by absolute diff: {:?}",
                result
                    .tool_changes
                    .iter()
                    .map(|c| (&c.name, c.diff))
                    .collect::<Vec<_>>()
            );
        }
    }
}
