# Noether Integration

[Noether](https://github.com/solv-noether/noether) is a verified composition platform. Caloron agents use it for reproducible, type-checked computations.

## What Noether Does

Noether manages **stages** — immutable, content-addressed units of computation. Each stage has typed inputs and outputs, verified before execution. When an agent needs to parse JSON, validate a schema, or call an API, it can use a Noether stage instead of writing ad-hoc code.

Benefits:

- **Reproducibility**: same input always produces same output
- **Reuse**: a computation done before is not redone
- **Verification**: output types are guaranteed before passing to the next stage

## Configuration

```toml
[noether]
enabled = true
binary = "noether"              # Path to noether binary
endpoint = ""                   # Optional remote registry URL
```

## How Agents Use It

From an agent's perspective, Noether is a tool they can invoke:

```bash
# Search for stages
noether stage search "parse json"

# Compose a solution from a problem description
noether compose "parse this CSV and extract email addresses"

# Execute a composition graph
noether run graph.json --input '{"data": "..."}'
```

The agent references Noether tools in its feedback comment:

```yaml
tools_used:
  - "noether:parse_json"
  - "noether:http_post_json"
  - "noether:json_validate"
```

## Retro Integration

The Retro Engine tracks Noether stage usage across sprints:

- **Most-used stages**: identifies high-value stages in the store
- **Tasks using Noether**: tracks adoption rate
- **Total invocations**: measures reuse efficiency

This data appears in the retro report:

```markdown
## Noether Usage

- Tasks using Noether: 3/5
- Total stage invocations: 12
- Unique stages used: 6

Most used stages:
- `parse_json` (4 uses)
- `http_post_json` (3 uses)
```

## Available Stages

Noether ships with 76+ stdlib stages covering:

| Category | Examples |
|----------|---------|
| Scalar | `parse_number`, `parse_json`, `to_string` |
| Collections | `list_map`, `list_filter`, `list_sort`, `group_by` |
| I/O | `http_get`, `http_post`, `read_file` |
| LLM | `llm_complete`, `llm_classify`, `llm_extract` |
| Text | `regex_match`, `text_split`, `text_join` |
| Data | `json_merge`, `json_path`, `csv_parse` |

Agents can also create custom stages via `noether stage add`.
