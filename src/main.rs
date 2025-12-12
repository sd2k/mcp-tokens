use anyhow::Result;
use clap::{Parser, Subcommand};
use mcp_tokens::{
    analysis::Analyzer,
    counter::{CounterConfig, create_counter},
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
) -> Result<()> {
    if command.is_empty() {
        anyhow::bail!("No command provided. Usage: mcp-tokens analyze -- <command> [args...]");
    }

    let output_format: OutputFormat = format.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    // Create token counter
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
            "Connected to {} v{} ({} tools)\n",
            server_data.server_info.name,
            server_data.server_info.version,
            server_data.tools.len()
        );
    }

    // Analyze
    let analyzer = Analyzer::new(counter.as_ref());
    let report = analyzer.analyze(&server_data).await?;

    // Save report if requested
    if let Some(output_path) = output {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&output_path, json)?;
        if matches!(output_format, OutputFormat::Text) {
            eprintln!("Report saved to {}", output_path.display());
        }
    }

    // Compare with baseline if provided
    let comparison: Option<ComparisonResult> = if let Some(baseline_path) = baseline {
        let baseline_json = std::fs::read_to_string(&baseline_path)?;
        let baseline_report: mcp_tokens::analysis::AnalysisReport =
            serde_json::from_str(&baseline_json)?;

        let thresholds = mcp_tokens::output::diff::ThresholdConfig {
            max_percent_increase: Some(threshold_percent),
            max_absolute_increase: threshold_absolute,
        };

        Some(compare_reports(&baseline_report, &report, &thresholds))
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
