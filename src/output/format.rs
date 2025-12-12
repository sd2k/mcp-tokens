use crate::analysis::AnalysisReport;

#[derive(Debug, Clone, Copy, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!("Unknown format: {}. Use 'text' or 'json'", s)),
        }
    }
}

pub fn format_report(report: &AnalysisReport, format: OutputFormat) -> anyhow::Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(report)?),
        OutputFormat::Text => Ok(format_text(report)),
    }
}

fn format_text(report: &AnalysisReport) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "MCP Token Analysis: {} {}\n",
        report.server_info.name, report.server_info.version
    ));
    out.push_str(&format!(
        "Counter: {} ({})\n",
        report.counter.provider, report.counter.model
    ));
    out.push_str(&format!("{}\n\n", "=".repeat(60)));

    // Summary
    out.push_str(&format!("Total tokens: {}\n\n", report.total_tokens));

    // Tools
    out.push_str(&format!(
        "Tools ({} tools, {} tokens):\n",
        report.tools.count, report.tools.total
    ));
    out.push_str(&format!("{}\n", "-".repeat(40)));

    for item in &report.tools.items {
        let desc_info = match (item.description_tokens, item.schema_tokens) {
            (Some(d), Some(s)) => format!(" (desc: {}, schema: {})", d, s),
            _ => String::new(),
        };
        out.push_str(&format!(
            "  {:6} tokens  {}{}\n",
            item.tokens, item.name, desc_info
        ));
    }

    // Resources
    if let Some(ref resources) = report.resources {
        out.push_str(&format!(
            "\nResources ({} resources, {} tokens):\n",
            resources.count, resources.total
        ));
        out.push_str(&format!("{}\n", "-".repeat(40)));
        for item in &resources.items {
            out.push_str(&format!("  {:6} tokens  {}\n", item.tokens, item.name));
        }
    }

    // Prompts
    if let Some(ref prompts) = report.prompts {
        out.push_str(&format!(
            "\nPrompts ({} prompts, {} tokens):\n",
            prompts.count, prompts.total
        ));
        out.push_str(&format!("{}\n", "-".repeat(40)));
        for item in &prompts.items {
            out.push_str(&format!("  {:6} tokens  {}\n", item.tokens, item.name));
        }
    }

    out
}
