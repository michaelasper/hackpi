# Environment Variables Reference

| Variable | Default | Description |
|----------|---------|-------------|
| `HACKPI_ENDPOINT` | `http://localhost:11434/api/chat` | LLM API endpoint URL |
| `HACKPI_MODEL` | `llama3.2` | Model name sent to the API |
| `HACKPI_MAX_TOKENS` | `4096` | Maximum tokens per response |

## Command-line flags

| Flag | Description |
|------|-------------|
| `--god` | Skip all guardrail checks (all tool calls allowed) |