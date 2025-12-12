# mcp-tokens

Analyze token usage of MCP (Model Context Protocol) servers. Helps MCP server authors understand and track how much context window their server consumes.

## Why?

MCP servers expose tools, resources, and prompts to AI models. Each tool's name, description, and input schema consumes tokens in the model's context window. As servers grow in complexity, this adds up and affects both cost and available context for actual work.

`mcp-tokens` lets you:
- See exactly how many tokens each tool contributes
- Track token usage over time with baseline comparisons
- Fail CI builds when token usage increases beyond thresholds

## Installation

### GitHub Action

See [mcp-tokens-action](https://github.com/sd2k/mcp-tokens-action) for a GitHub Action that wraps this tool.

### From source

```bash
cargo install --path .
```

### Pre-built binaries

See [Releases](https://github.com/sd2k/mcp-tokens/releases) for pre-built binaries.

## Usage

### Basic analysis

```bash
# Analyze an MCP server (uses ANTHROPIC_API_KEY from environment)
mcp-tokens analyze -- npx @modelcontextprotocol/server-everything

# Analyze a local server
mcp-tokens analyze -- ./my-mcp-server
```

Output:
```
MCP Token Analysis: example-servers/everything v1.0.0
Counter: anthropic (claude-sonnet-4-5-20250929)
============================================================

Total tokens: 1934

Tools (11 tools, 1718 tokens):
----------------------------------------
     653 tokens  zip (desc: 43, schema: 610)
     638 tokens  annotatedMessage (desc: 19, schema: 619)
     629 tokens  sampleLLM (desc: 20, schema: 609)
     ...
```

### Token counting providers

**Anthropic (recommended)**: Uses the [Anthropic token counting API](https://docs.anthropic.com/en/api/messages-count-tokens) for accurate counts. Free to use, requires an API key.

```bash
export ANTHROPIC_API_KEY=sk-ant-...
mcp-tokens analyze -- ./my-server
```

**Tiktoken (offline)**: Uses tiktoken for approximate counts. No API key required, but counts are estimates based on GPT tokenization.

```bash
mcp-tokens analyze --provider tiktoken -- ./my-server
```

### CI integration

Generate a baseline on your main branch:

```bash
mcp-tokens analyze --format json --output tokens.json -- ./my-server
```

Compare against the baseline in PRs:

```bash
mcp-tokens analyze --baseline tokens.json --threshold-percent 5 -- ./my-server
```

The command exits with code 1 if the threshold is exceeded:

```
Baseline Comparison
============================================================

Baseline: 1000 tokens
Current:  1934 tokens
Change:   +934 tokens (+93.4%)

Result: FAILED - Token increase of 93.4% exceeds threshold of 5.0%
```

### JSON output

```bash
mcp-tokens analyze --format json -- ./my-server
```

```json
{
  "counter": {
    "provider": "anthropic",
    "model": "claude-sonnet-4-5-20250929"
  },
  "server_info": {
    "name": "my-server",
    "version": "1.0.0"
  },
  "total_tokens": 1934,
  "tools": {
    "total": 1718,
    "count": 11,
    "items": [
      {
        "name": "myTool",
        "tokens": 653,
        "description_tokens": 43,
        "schema_tokens": 610
      }
    ]
  }
}
```

## CLI Reference

```
mcp-tokens analyze [OPTIONS] -- <COMMAND>...

Arguments:
  <COMMAND>...  Command to start the MCP server

Options:
  -f, --format <FORMAT>          Output format: text or json [default: text]
  -p, --provider <PROVIDER>      Token counting provider: anthropic or tiktoken
  -m, --model <MODEL>            Model to use for token counting
      --anthropic-key <KEY>      Anthropic API key (or set ANTHROPIC_API_KEY)
  -b, --baseline <FILE>          Baseline JSON file to compare against
      --threshold-percent <N>    Max allowed percentage increase [default: 5.0]
      --threshold-absolute <N>   Max allowed absolute token increase
  -t, --timeout <SECONDS>        Timeout for server startup [default: 30]
  -o, --output <FILE>            Save report to file (for use as baseline)
  -h, --help                     Print help
```

## Environment variables

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Anthropic API key for accurate token counting |
| `MCP_TOKENS_PROVIDER` | Default provider (`anthropic` or `tiktoken`) |
| `MCP_TOKENS_MODEL` | Default model for token counting |

## Tips for reducing token usage

1. **Keep descriptions concise**: Tool descriptions are fully tokenized. Be clear but brief.

2. **Simplify schemas**: Complex nested objects with many optional fields add tokens. Consider splitting into multiple focused tools.

3. **Use enums wisely**: String enums with descriptions add tokens for each variant.

4. **Review schema defaults**: Default values and examples in JSON Schema add tokens.

## License

Licensed under the Apache License, Version 2.0 `<http://www.apache.org/licenses/LICENSE-2.0>` or the MIT license `<http://opensource.org/licenses/MIT>`, at your option.
