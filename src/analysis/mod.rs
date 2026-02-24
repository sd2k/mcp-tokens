use crate::counter::TokenCounter;
use crate::mcp::ServerData;
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{IsTerminal, Write};
use std::sync::{Arc, Mutex};

/// Maximum number of concurrent API requests for token counting.
const MAX_CONCURRENT_REQUESTS: usize = 8;

/// Token count for a single item (tool, resource, prompt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemTokens {
    pub name: String,
    pub tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_tokens: Option<i32>,
}

/// Token counts for a category (tools, resources, prompts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTokens {
    pub total: i32,
    pub count: usize,
    pub items: Vec<ItemTokens>,
}

/// Complete analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub counter: CounterInfo,
    pub server_info: ServerInfoReport,
    pub total_tokens: i32,
    pub tools: CategoryTokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<CategoryTokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<CategoryTokens>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterInfo {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfoReport {
    pub name: String,
    pub version: String,
}

/// Tracks in-flight items and updates the progress bar message.
struct InFlightTracker {
    in_flight: Mutex<HashSet<String>>,
    progress_bar: ProgressBar,
    prefix: String,
    total: u64,
    is_tty: bool,
    started_printed: Mutex<bool>,
}

impl InFlightTracker {
    fn new(progress_bar: ProgressBar, prefix: String, total: u64, is_tty: bool) -> Arc<Self> {
        Arc::new(Self {
            in_flight: Mutex::new(HashSet::new()),
            progress_bar,
            prefix,
            total,
            is_tty,
            started_printed: Mutex::new(false),
        })
    }

    fn print_started_if_needed(&self) {
        if self.is_tty {
            return;
        }
        let mut printed = self.started_printed.lock().unwrap();
        if !*printed {
            eprint!("{}... ", self.prefix);
            let _ = std::io::stderr().flush();
            *printed = true;
        }
    }

    fn start(&self, name: &str) {
        self.print_started_if_needed();
        {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight.insert(name.to_string());
        }
        self.update_message();
    }

    fn finish(&self, name: &str) {
        {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight.remove(name);
        }
        self.progress_bar.inc(1);
        self.update_message();
    }

    fn update_message(&self) {
        let in_flight = self.in_flight.lock().unwrap();
        if in_flight.is_empty() {
            self.progress_bar.set_message("");
        } else {
            let mut names: Vec<_> = in_flight.iter().cloned().collect();
            names.sort();
            let msg = names.join(", ");
            self.progress_bar.set_message(msg);
        }
    }

    fn finish_progress(&self) {
        self.progress_bar.finish_and_clear();
        if !self.is_tty {
            let printed = self.started_printed.lock().unwrap();
            if *printed {
                eprintln!("done ({} items)", self.total);
            }
        }
    }
}

/// Analyze MCP server data and count tokens.
pub struct Analyzer<'a> {
    counter: &'a dyn TokenCounter,
    show_progress: bool,
}

impl<'a> Analyzer<'a> {
    pub fn new(counter: &'a dyn TokenCounter) -> Self {
        Self {
            counter,
            show_progress: false,
        }
    }

    /// Enable or disable progress bar display.
    pub fn with_progress(mut self, show_progress: bool) -> Self {
        self.show_progress = show_progress;
        self
    }

    fn create_progress_bar(&self, len: u64, prefix: &str) -> ProgressBar {
        if !self.show_progress {
            return ProgressBar::hidden();
        }

        let pb = ProgressBar::new(len);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} {prefix}: [{bar:30.cyan/blue}] {pos}/{len} [{elapsed}<{eta}] {wide_msg}",
                )
                .expect("Invalid progress bar template")
                .progress_chars("━╸─")
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        pb.set_prefix(prefix.to_string());
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb
    }

    pub async fn analyze(&self, data: &ServerData) -> anyhow::Result<AnalysisReport> {
        // Count tokens for all tools combined (this is how they appear in context)
        let tools_progress =
            self.create_progress_bar((data.tools.len() + 1) as u64, "Counting tokens for tools");
        tools_progress.set_message("(all tools combined)");

        let is_tty = std::io::stderr().is_terminal();

        let tools_total = self.counter.count_tools(&data.tools).await?;
        tools_progress.inc(1);
        tools_progress.set_message("");

        // Count individual tools for breakdown, with concurrency
        let tracker = InFlightTracker::new(
            tools_progress,
            "Counting tokens for tools".to_string(),
            data.tools.len() as u64,
            is_tty,
        );

        let tool_results: Vec<Result<ItemTokens, anyhow::Error>> = stream::iter(data.tools.iter())
            .map(|tool| {
                let tracker = Arc::clone(&tracker);
                async move {
                    tracker.start(&tool.name);

                    let single_tool = vec![tool.clone()];
                    let tool_tokens = self.counter.count_tools(&single_tool).await?;

                    // Count description separately if available
                    let desc_tokens = if let Some(ref desc) = tool.description {
                        Some(self.counter.count_text(desc).await?)
                    } else {
                        None
                    };

                    // Schema tokens = total - description (approximate)
                    let schema_tokens = desc_tokens.map(|d| tool_tokens - d);

                    tracker.finish(&tool.name);

                    Ok(ItemTokens {
                        name: tool.name.clone(),
                        tokens: tool_tokens,
                        description_tokens: desc_tokens,
                        schema_tokens,
                    })
                }
            })
            .buffer_unordered(MAX_CONCURRENT_REQUESTS)
            .collect()
            .await;

        // Collect results, propagating any errors
        let mut tool_items: Vec<ItemTokens> = tool_results.into_iter().collect::<Result<_, _>>()?;

        tracker.finish_progress();

        // When counting tools individually, each call includes provider framing
        // overhead (e.g. Anthropic's tool-use system prompt) that only appears
        // once in the real context. We can derive the per-call overhead from:
        //   batch_total = framing + Σ(content_i)
        //   individual_i = framing + content_i
        //   Σ(individual_i) = N * framing + Σ(content_i)
        //   overhead = (Σ(individual_i) - batch_total) / (N - 1)
        // Subtracting this from each individual count gives the true content cost.
        let n = tool_items.len() as i32;
        let raw_sum: i32 = tool_items.iter().map(|t| t.tokens).sum();
        let overhead = if n > 1 && raw_sum > tools_total {
            (raw_sum - tools_total) / (n - 1)
        } else {
            0
        };

        if overhead > 0 {
            for item in tool_items.iter_mut() {
                item.tokens = (item.tokens - overhead).max(0);
                // Adjust schema estimate since it was derived from the inflated total
                item.schema_tokens = item.schema_tokens.map(|s| (s - overhead).max(0));
            }
        }

        // Sort by token count descending
        tool_items.sort_by(|a, b| b.tokens.cmp(&a.tokens));

        let tools = CategoryTokens {
            total: tools_total,
            count: data.tools.len(),
            items: tool_items,
        };

        // Count resources if present
        let resources = if !data.resources.is_empty() {
            let resources_progress = self
                .create_progress_bar(data.resources.len() as u64, "Counting tokens for resources");
            let tracker = InFlightTracker::new(
                resources_progress,
                "Counting tokens for resources".to_string(),
                data.resources.len() as u64,
                is_tty,
            );

            let resource_results: Vec<Result<ItemTokens, anyhow::Error>> =
                stream::iter(data.resources.iter())
                    .map(|r| {
                        let tracker = Arc::clone(&tracker);
                        async move {
                            tracker.start(&r.name);

                            let text = format!(
                                "{}{}{}",
                                r.name,
                                r.description.as_deref().unwrap_or(""),
                                r.uri
                            );
                            let tokens = self.counter.count_text(&text).await?;

                            tracker.finish(&r.name);

                            Ok(ItemTokens {
                                name: r.name.clone(),
                                tokens,
                                description_tokens: None,
                                schema_tokens: None,
                            })
                        }
                    })
                    .buffer_unordered(MAX_CONCURRENT_REQUESTS)
                    .collect()
                    .await;

            let resource_items: Vec<ItemTokens> =
                resource_results.into_iter().collect::<Result<_, _>>()?;

            tracker.finish_progress();

            let total: i32 = resource_items.iter().map(|i| i.tokens).sum();
            Some(CategoryTokens {
                total,
                count: data.resources.len(),
                items: resource_items,
            })
        } else {
            None
        };

        // Count prompts if present
        let prompts = if !data.prompts.is_empty() {
            let prompts_progress =
                self.create_progress_bar(data.prompts.len() as u64, "Counting tokens for prompts");
            let tracker = InFlightTracker::new(
                prompts_progress,
                "Counting tokens for prompts".to_string(),
                data.prompts.len() as u64,
                is_tty,
            );

            let prompt_results: Vec<Result<ItemTokens, anyhow::Error>> =
                stream::iter(data.prompts.iter())
                    .map(|p| {
                        let tracker = Arc::clone(&tracker);
                        async move {
                            tracker.start(&p.name);

                            let text =
                                format!("{}{}", p.name, p.description.as_deref().unwrap_or(""));
                            let tokens = self.counter.count_text(&text).await?;

                            tracker.finish(&p.name);

                            Ok(ItemTokens {
                                name: p.name.clone(),
                                tokens,
                                description_tokens: None,
                                schema_tokens: None,
                            })
                        }
                    })
                    .buffer_unordered(MAX_CONCURRENT_REQUESTS)
                    .collect()
                    .await;

            let prompt_items: Vec<ItemTokens> =
                prompt_results.into_iter().collect::<Result<_, _>>()?;

            tracker.finish_progress();

            let total: i32 = prompt_items.iter().map(|i| i.tokens).sum();
            Some(CategoryTokens {
                total,
                count: data.prompts.len(),
                items: prompt_items,
            })
        } else {
            None
        };

        let total_tokens = tools.total
            + resources.as_ref().map(|r| r.total).unwrap_or(0)
            + prompts.as_ref().map(|p| p.total).unwrap_or(0);

        Ok(AnalysisReport {
            counter: CounterInfo {
                provider: self.counter.name().to_string(),
                model: self.counter.model().to_string(),
            },
            server_info: ServerInfoReport {
                name: data.server_info.name.clone(),
                version: data.server_info.version.clone(),
            },
            total_tokens,
            tools,
            resources,
            prompts,
        })
    }
}
