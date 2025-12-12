use anyhow::Result;
use clap::{Parser, Subcommand};
use mcp_tokens::{
    analysis::Analyzer,
    baseline::{Baseline, MultiProviderBaseline},
    counter::{AnthropicCounter, CounterConfig, TiktokenCounter, create_counter},
    mcp::Client,
    output::{ComparisonResult, OutputFormat, compare_reports, format_report},
};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(name = "mcp-tokens")]
#[command(about = "Analyze token usage of MCP servers")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze an MCP server's token usage
    Analyze {
        /// Command to start the MCP server
        #[arg(last = true, required = true)]
        command: Vec<String>,

        /// Output format (text or json)
        #[arg(short, long, default_value = "text")]
        format: String,

        /// Token counting provider (anthropic or tiktoken)
        #[arg(short, long, env = "MCP_TOKENS_PROVIDER")]
        provider: Option<String>,

        /// Model to use for token counting
        #[arg(short, long, env = "MCP_TOKENS_MODEL")]
        model: Option<String>,

        /// Anthropic API key
        #[arg(long, env = "ANTHROPIC_API_KEY")]
        anthropic_key: Option<String>,

        /// Baseline JSON file to compare against
        #[arg(short, long)]
        baseline: Option<PathBuf>,

        /// Maximum allowed percentage increase (for baseline comparison)
        #[arg(long, default_value = "5.0")]
        threshold_percent: f64,

        /// Maximum allowed absolute token increase (for baseline comparison)
        #[arg(long)]
        threshold_absolute: Option<i32>,

        /// Timeout in seconds for server startup
        #[arg(short, long, default_value = "30")]
        timeout: u64,

        /// Save report to file (for use as future baseline)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Generate baseline with all available providers (requires Anthropic key for full coverage)
        #[arg(long)]
        all_providers: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Analyze {
            command,
            format,
            provider,
            model,
            anthropic_key,
            baseline,
            threshold_percent,
            threshold_absolute,
            timeout,
            output,
            all_providers,
        } => {
            run_analyze(
                command,
                format,
                provider,
                model,
                anthropic_key,
                baseline,
                threshold_percent,
                threshold_absolute,
                timeout,
                output,
                all_providers,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_analyze(
    command: Vec<String>,
    format: String,
    provider: Option<String>,
    model: Option<String>,
    anthropic_key: Option<String>,
    baseline: Option<PathBuf>,
    threshold_percent: f64,
    threshold_absolute: Option<i32>,
    timeout: u64,
    output: Option<PathBuf>,
    all_providers: bool,
) -> Result<()> {
    if command.is_empty() {
        anyhow::bail!("No command provided. Usage: mcp-tokens analyze -- <command> [args...]");
    }

    let output_format: OutputFormat = format.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    // Connect to MCP server
    let mcp_client = Client::new(Duration::from_secs(timeout));
    let cmd = &command[0];
    let args: Vec<String> = command[1..].to_vec();

    if matches!(output_format, OutputFormat::Text) {
        eprintln!("Connecting to MCP server: {} {:?}", cmd, args);
    }

    let server_data = mcp_client.fetch_server_data(cmd, &args).await?;

    if matches!(output_format, OutputFormat::Text) {
        eprintln!(
            "Connected to {} {} ({} tools)\n",
            server_data.server_info.name,
            server_data.server_info.version,
            server_data.tools.len()
        );
    }

    // Handle --all-providers mode for baseline generation
    if all_providers {
        let mut multi_baseline = MultiProviderBaseline::new();

        // Always include tiktoken
        let tiktoken = TiktokenCounter::new(model.clone())?;
        if matches!(output_format, OutputFormat::Text) {
            eprintln!("Analyzing with tiktoken...");
        }
        let show_progress = matches!(output_format, OutputFormat::Text);
        let tiktoken_analyzer = Analyzer::new(&tiktoken).with_progress(show_progress);
        let tiktoken_report = tiktoken_analyzer.analyze(&server_data).await?;
        multi_baseline.add_report(tiktoken_report.clone());

        // Include Anthropic if key is available
        if let Some(ref key) = anthropic_key {
            let anthropic = AnthropicCounter::new(key.clone(), model.clone());
            if matches!(output_format, OutputFormat::Text) {
                eprintln!("Analyzing with Anthropic...");
            }
            let anthropic_analyzer = Analyzer::new(&anthropic).with_progress(show_progress);
            let anthropic_report = anthropic_analyzer.analyze(&server_data).await?;
            multi_baseline.add_report(anthropic_report);
        } else if matches!(output_format, OutputFormat::Text) {
            eprintln!(
                "Note: Anthropic API key not provided, baseline will only contain tiktoken counts."
            );
            eprintln!("      Provide --anthropic-key for full multi-provider baseline.");
        }

        // Save multi-provider baseline
        if let Some(output_path) = output {
            let json = serde_json::to_string_pretty(&multi_baseline)?;
            std::fs::write(&output_path, json)?;
            if matches!(output_format, OutputFormat::Text) {
                eprintln!(
                    "Multi-provider baseline saved to {} (providers: {:?})",
                    output_path.display(),
                    multi_baseline.available_providers()
                );
            }
        }

        // Output results
        match output_format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&multi_baseline)?);
            }
            OutputFormat::Text => {
                println!("Multi-Provider Baseline");
                println!("{}", "=".repeat(60));
                for (provider, models) in &multi_baseline.providers {
                    for (model, report) in models {
                        println!("\n[{}/{}] {} tokens", provider, model, report.total_tokens);
                    }
                }
            }
        }

        return Ok(());
    }

    // Standard single-provider mode
    let counter_config = CounterConfig {
        provider,
        model,
        anthropic_key,
    };
    let counter = create_counter(counter_config)?;

    // Show counter info
    if matches!(output_format, OutputFormat::Text) {
        eprintln!(
            "Using {} counter (model: {})",
            counter.name(),
            counter.model()
        );
        if counter.name() == "tiktoken" {
            eprintln!(
                "Warning: tiktoken counts are approximate. Use --anthropic-key for accurate counts."
            );
        }
    }

    // Analyze
    let show_progress = matches!(output_format, OutputFormat::Text);
    let analyzer = Analyzer::new(counter.as_ref()).with_progress(show_progress);
    let report = analyzer.analyze(&server_data).await?;

    // Save report if requested
    if let Some(ref output_path) = output {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(output_path, json)?;
        if matches!(output_format, OutputFormat::Text) {
            eprintln!("Report saved to {}", output_path.display());
        }
    }

    // Compare with baseline if provided
    let comparison: Option<ComparisonResult> = if let Some(baseline_path) = baseline {
        let baseline_json = std::fs::read_to_string(&baseline_path)?;
        let baseline = Baseline::from_json(&baseline_json)?;

        let current_provider = counter.name();
        let current_model = counter.model();

        // Get best matching baseline report
        let (baseline_report, provider_match, model_match) = baseline
            .get_best_report(current_provider, current_model)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No compatible baseline found for provider '{}' model '{}'",
                    current_provider,
                    current_model
                )
            })?;

        if matches!(output_format, OutputFormat::Text) {
            if !provider_match {
                eprintln!(
                    "Warning: No baseline for provider '{}', using '{}' instead.",
                    current_provider, baseline_report.counter.provider
                );
            } else if !model_match {
                eprintln!(
                    "Warning: No baseline for model '{}', using '{}' instead.",
                    current_model, baseline_report.counter.model
                );
            }
        }

        let thresholds = mcp_tokens::output::diff::ThresholdConfig {
            max_percent_increase: Some(threshold_percent),
            max_absolute_increase: threshold_absolute,
        };

        Some(compare_reports(baseline_report, &report, &thresholds))
    } else {
        None
    };

    // Output results
    match output_format {
        OutputFormat::Json => {
            if let Some(ref comp) = comparison {
                // Include both report and comparison in JSON output
                let combined = serde_json::json!({
                    "report": report,
                    "comparison": comp,
                });
                println!("{}", serde_json::to_string_pretty(&combined)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
        OutputFormat::Text => {
            println!("{}", format_report(&report, output_format)?);
            if let Some(ref comp) = comparison {
                println!("\n{}", comp.format_text());
            }
        }
    }

    // Exit with error if comparison failed
    if let Some(comp) = comparison
        && !comp.passed
    {
        std::process::exit(1);
    }

    Ok(())
}
