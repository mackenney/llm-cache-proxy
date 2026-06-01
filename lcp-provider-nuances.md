# LCP Provider Nuances
Observed during live e2e sessions — 2026-05-28.

---

## Anthropic

**Endpoint:** `POST /anthropic/v1/messages`
**Auth:** `x-api-key: <key>` + `anthropic-version: 2023-06-01`
**Token param:** `max_tokens` (required, hard limit)

| Model | Tier |
|---|---|
| claude-haiku-4-5 | haiku |
| claude-sonnet-4-6 | sonnet |
| claude-opus-4-6 | opus |

No quirks — all models respond consistently to `max_tokens: 10`.

---

## OpenAI

### Chat models (`v1/chat/completions`)

**Auth:** `Authorization: Bearer <key>`
**Token param:** `max_completion_tokens` — **`max_tokens` returns 400 on all GPT-5.x models**

| Model | Tier | Notes |
|---|---|---|
| gpt-4o-mini | haiku | accepts `max_tokens` |
| gpt-4o | sonnet | accepts `max_tokens` |
| gpt-5-nano | haiku | `max_completion_tokens` only; needs **2000+** (reasoning overhead) |
| gpt-5-mini | haiku | same |
| gpt-5 | sonnet | same |
| gpt-5.1 | sonnet | same |
| gpt-5.2 | sonnet | same |
| gpt-5.4-nano | haiku | same |
| gpt-5.4-mini | haiku | same |
| gpt-5.4 | sonnet | same |
| gpt-5.5 | sonnet | same |
| o1 | opus | `max_completion_tokens` only; needs **500+** |
| o3 | opus | same |
| o4-mini | haiku | same |

**Reasoning model token budget:** GPT-5.x base/nano/mini and all o-series consume
tokens on internal reasoning before emitting visible output. A budget of 50 returns
empty content. Use **2000+** for GPT-5.x, **500+** for o1/o3/o4-mini.

### Responses API (`v1/responses`) — `-pro` and `o3-pro` tier

**Models:** `gpt-5-pro`, `gpt-5.4-pro`, `gpt-5.5-pro`, `o3-pro`

These models are **not** available on `v1/chat/completions` (returns 404).
Use `v1/responses` instead:

```json
POST /openai/v1/responses
{
  "model": "gpt-5.5-pro",
  "input": "<prompt string>",
  "max_output_tokens": 2000
}
```

**Response shape differs from chat completions:**
```json
{
  "output": [
    { "type": "reasoning", "summary": [] },
    {
      "type": "message",
      "content": [{ "type": "output_text", "text": "pong" }]
    }
  ]
}
```
The `reasoning` block comes first and has no visible text. Extract from the
`type: "message"` block only. `choices[]` does not exist in this response.

lcp proxies `v1/responses` transparently — no special config needed.

---

## OpenRouter

**Endpoint:** `POST /openrouter/v1/chat/completions`
**Auth:** `Authorization: Bearer <key>`
**Token param:** `max_tokens` for most models; **`max_tokens: 2000+`** for any
model that has thinking/reasoning enabled (e.g. `google/gemini-2.5-pro`).

| Model | Tier | Notes |
|---|---|---|
| openai/gpt-4o-mini | haiku | `max_tokens: 10` fine |
| meta-llama/llama-3.3-70b-instruct | haiku | `max_tokens: 10` fine |
| openai/gpt-4o | sonnet | `max_tokens: 10` fine |
| google/gemini-2.5-pro | sonnet | thinking model — needs **`max_tokens: 2000+`** |
| deepseek/deepseek-r1 | opus | `max_tokens: 10` fine |
| openai/o1 | opus | `max_completion_tokens` only; needs **500+** |

OpenRouter wraps all providers in the OpenAI chat completions format, so
response parsing is uniform (`choices[0].message.content`). However the token
param name and budget must match what the underlying model requires.

---

## Gemini (direct)

**Endpoint:** `POST /gemini/v1beta/models/{model}:generateContent?key=<key>`
**Auth:** API key passed as **query param** (`?key=`), not a header.
**Token param:** `generationConfig.maxOutputTokens` in the request body.

```json
{
  "contents": [{"role": "user", "parts": [{"text": "..."}]}],
  "generationConfig": { "maxOutputTokens": 1000 }
}
```

**Response shape:**
```json
{
  "candidates": [{
    "content": { "parts": [{ "text": "pong" }] }
  }]
}
```

**Thinking models** (`gemini-2.5-pro`, `gemini-3.x` series): use thinking tokens
internally before emitting output. `maxOutputTokens: 10` hits `MAX_TOKENS`
and returns `candidates[0].content: {}` (no `parts`). Use **1000+**.

**Available model tiers (as of 2026-05-28):**

| Model | Tier | Notes |
|---|---|---|
| gemini-2.5-flash-lite | haiku | non-thinking, fast |
| gemini-2.5-flash | haiku/sonnet | non-thinking |
| gemini-3.5-flash | haiku | newest flash; thinking — needs 1000+ tokens |
| gemini-2.5-pro | sonnet | thinking — needs 1000+ tokens |
| gemini-3.1-pro-preview | opus | thinking — needs 1000+ tokens |

**Deprecated:** `gemini-2.0-flash` returns 404 for new users. Use `gemini-2.5-flash`
or `gemini-3.5-flash` instead.

---

## Common pitfalls

| Pitfall | Symptom | Fix |
|---|---|---|
| `max_tokens` on GPT-5.x | `400 Unsupported parameter` | Use `max_completion_tokens` |
| Low token budget on reasoning models | Empty `content` / `finish_reason: length` | Use 2000+ for GPT-5.x/Gemini thinking, 500+ for o-series |
| `-pro` models on `/chat/completions` | `404 not a chat model` | Use `/v1/responses` with `input` + `max_output_tokens` |
| Gemini API key in header | `400` / auth error | Key must be in query string `?key=` |
| `gemini-2.0-flash` | `404` | Deprecated for new users; use `gemini-2.5-flash` |
| Gemini `content: {}` (no parts) | MAX_TOKENS hit during thinking phase | Increase `maxOutputTokens` to 1000+ |
| OpenRouter thinking models (low tokens) | Empty `content` | Use `max_tokens: 2000+` regardless of param name |
