# Test Fixtures — Cassettes

Cassettes are TOML files that capture real LLM provider SSE responses for deterministic
replay in CI. Each cassette records one request/response exchange at the wire level.

## What is a cassette?

A cassette stores the exact HTTP response chunks that an LLM provider returned for a
specific request. On replay, the `MockUpstream` server feeds these chunks to the proxy
under test, giving the proxy real wire-format data without network access.

## Why are cassettes safe to commit?

Real API keys are **never** stored. The `secret_kind` field identifies which doppel
pattern set was used during recording. The body contains the **fake** (doppel-substituted)
key, not the original. On replay, tests embed the original synthetic key; the proxy swaps
it for the fake; the cassette player returns those frames; the proxy restores it.

## Naming convention

```
tests/fixtures/{provider}/{scenario}.toml
```

Examples:
- `tests/fixtures/anthropic/tool_use_input_json.toml`
- `tests/fixtures/openai/chat_tool_calls.toml`
- `tests/fixtures/openrouter/claude_colocated_finish.toml`
- `tests/fixtures/gemini/multi_part_thinking.toml`

## `secret_kind` values

| `secret_kind`    | doppel pattern function             | Example key prefix      |
|------------------|-------------------------------------|-------------------------|
| `anthropic`      | `doppel::patterns::anthropic()`     | `sk-ant-api03-`         |
| `openai_classic` | `doppel::patterns::openai_classic()`| `sk-`                   |
| `openai_project` | `doppel::patterns::openai_project()`| `sk-proj-`              |
| `gemini`         | `doppel::patterns::gcp()`           | `AIza`                  |

## How to record a new cassette

Recording is done with the `lcp-record` binary (Step 02). To add a scenario:

1. Add an entry to `tests/fixtures/scenarios.toml`.
2. Run the recorder:
   ```
   cargo test --test cassette_recorder --features record -- record \
     --scenario <id> \
     --out tests/fixtures/<provider>/<scenario>.toml
   ```
3. Verify the cassette loads: `cargo nextest run --test integration cassette_infrastructure`.
4. Commit the new `.toml` file.

Never edit body_chunks manually — they must match exact provider wire format.
