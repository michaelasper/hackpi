# How to Connect to Different LLM Providers

hackpi uses an Anthropic-compatible API. This guide covers connecting to various providers.

## Connect to ds4-server (DeepSeek V4 Flash)

The default configuration targets a local ds4-server:

```bash
export HACKPI_ENDPOINT=http://localhost:11434/api/chat
export HACKPI_MODEL=llama3.2
```

Start ds4-server first, then launch hackpi.

## Connect to a remote API

Point `HACKPI_ENDPOINT` at any server that implements the Anthropic `/v1/messages` format:

```bash
export HACKPI_ENDPOINT=https://api.anthropic.com/v1/messages
export HACKPI_MODEL=claude-sonnet-4-20250514
export HACKPI_MAX_TOKENS=8192
```

If your provider requires an API key, set it via environment variable as supported by your server. hackpi sends requests with the model and max_tokens you configure.

## Adjust context length

For models with smaller context windows, reduce `HACKPI_MAX_TOKENS`:

```bash
export HACKPI_MAX_TOKENS=2048
```

The agent has a hard turn limit of 25 tool-use rounds per request, regardless of token limits.

## Troubleshooting connection issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| "connection refused" | Server not running | Start your LLM server first |
| "401 unauthorized" | Missing or invalid API key | Set the appropriate environment variable for your provider |
| "404 not found" | Wrong endpoint path | Verify the API path matches your server's route |
| Timeouts on responses | Model is too slow or max_tokens too high | Reduce `HACKPI_MAX_TOKENS` or use a faster model |