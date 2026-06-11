//! SSE-aware restore helpers for `DoppelExt`.
//!
//! These functions are the building blocks for semantic-level secret
//! restoration in `text/event-stream` responses, where raw-byte Aho-Corasick
//! fails because the fake key never appears contiguously in the byte stream.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use doppel::restore as doppel_restore;
use doppel::restore_stream;
use doppel::{Entry, SessionKey};
use futures_util::future::BoxFuture;
use futures_util::{FutureExt, Stream, StreamExt};
use lcp_core::Provider;
use serde_json::Value;

use crate::extensions::ResponseStream;

/// Identifies a logically independent content stream within an SSE response.
/// Fields with the same key accumulate into the same buffer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum FieldKey {
    // Anthropic — keyed by (delta_type, content_block_index)
    AnthropicDelta { delta_type: String, index: u64 },
    // OpenAI / OpenRouter chat completions
    OpenAiContent,
    OpenAiToolCall { index: u64 },
    OpenAiReasoning,
    OpenAiFunctionCallArgs,
    OpenAiRefusal,
    // OpenAI Responses API
    ResponsesApiDelta { event_type: String },
    ResponsesApiDone { event_type: String },
    // Gemini
    GeminiText { thought: bool },
    GeminiCodeExecOutput { part_index: usize },
    GeminiFuncCallArg { part_index: usize, arg_key: String },
}

/// A content-bearing field extracted from one SSE event.
struct ExtractedField {
    key: FieldKey,
    text: String,
}

/// Information needed to write a restored value back into its JSON location.
#[cfg(test)]
#[derive(Debug, Clone)]
enum WriteBackInfo {
    /// Anthropic: `json["delta"][field_name]` where field_name is "text", "thinking", or "partial_json"
    AnthropicDelta { field_name: String },
    /// OpenAI: `json["choices"][0]["delta"]["content"]`
    OpenAiContent,
    /// OpenAI: `json["choices"][0]["delta"]["tool_calls"][array_pos]["function"]["arguments"]`
    /// `array_pos` is the position within the `tool_calls` array in *this* event's JSON.
    OpenAiToolCall { array_pos: usize },
    /// OpenAI: `json["choices"][0]["delta"]["reasoning_content"]`
    OpenAiReasoning,
    /// OpenAI: `json["choices"][0]["delta"]["refusal"]`
    OpenAiRefusal,
    /// Responses API: `json["delta"]`
    ResponsesApiDelta,
    /// Responses API: `json["text"]`
    ResponsesApiDone,
    /// Gemini: `json["candidates"][0]["content"]["parts"][part_index]["text"]`
    GeminiPartText { part_index: usize },
    /// Gemini: `json["candidates"][0]["content"]["parts"][part_index]["codeExecutionResult"]["output"]`
    GeminiPartCodeExecOutput { part_index: usize },
    /// Gemini: `json["candidates"][0]["content"]["parts"][part_index]["functionCall"]["args"][arg_key]`
    GeminiPartFuncCallArg { part_index: usize, arg_key: String },
}

/// Returns `true` if the first bytes of a response chunk look like an SSE stream.
///
/// Detects `data: `, `data:` (spaceless), `event: `, `event:` (spaceless), `": "` (SSE
/// comment with space), and `":"` followed by newline (empty SSE comment, no space).
/// Anthropic's real API prefixes each data line with a named `event:` line.
/// OpenRouter prefixes the stream with a `": OPENROUTER PROCESSING` comment before
/// any data lines. Non-SSE JSON responses begin with `{` or `[`.
pub fn is_sse_first_chunk(bytes: &[u8]) -> bool {
    bytes.starts_with(b"data:")
        || bytes.starts_with(b"event:")
        || bytes.starts_with(b": ")  // SSE comment line (e.g., OpenRouter)
        || bytes.starts_with(b":\n") // empty SSE comment
}

/// Returns the provider-specific content fields from a parsed SSE event JSON.
///
/// Returns an empty vec for events that do not carry text content (non-text
/// events or when the relevant fields are absent).
fn extract_fields(
    json: &Value,
    provider: Provider,
    event_type: Option<&str>,
) -> Vec<ExtractedField> {
    match provider {
        Provider::Anthropic => {
            if json["type"].as_str() != Some("content_block_delta") {
                return vec![];
            }
            let index = json["index"].as_u64().unwrap_or(0);
            let delta = &json["delta"];
            let delta_type = match delta["type"].as_str() {
                Some(dt) => dt,
                None => return vec![],
            };
            match delta_type {
                "text_delta" => {
                    if let Some(text) = delta["text"].as_str() {
                        vec![ExtractedField {
                            key: FieldKey::AnthropicDelta {
                                delta_type: "text_delta".into(),
                                index,
                            },
                            text: text.to_owned(),
                        }]
                    } else {
                        vec![]
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = delta["thinking"].as_str() {
                        vec![ExtractedField {
                            key: FieldKey::AnthropicDelta {
                                delta_type: "thinking_delta".into(),
                                index,
                            },
                            text: text.to_owned(),
                        }]
                    } else {
                        vec![]
                    }
                }
                "input_json_delta" => {
                    if let Some(text) = delta["partial_json"].as_str() {
                        vec![ExtractedField {
                            key: FieldKey::AnthropicDelta {
                                delta_type: "input_json_delta".into(),
                                index,
                            },
                            text: text.to_owned(),
                        }]
                    } else {
                        vec![]
                    }
                }
                "signature_delta" => {
                    // MUST NOT modify — return empty to skip accumulation entirely.
                    vec![]
                }
                _ => vec![],
            }
        }
        Provider::OpenAi | Provider::OpenRouter => {
            if let Some(et) = event_type {
                if et.starts_with("response.") {
                    return match et {
                        "response.output_text.delta" => {
                            if let Some(text) = json.get("delta").and_then(|v| v.as_str()) {
                                vec![ExtractedField {
                                    key: FieldKey::ResponsesApiDelta {
                                        event_type: "response.output_text.delta".into(),
                                    },
                                    text: text.to_owned(),
                                }]
                            } else {
                                vec![]
                            }
                        }
                        "response.output_text.done" => {
                            if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                                vec![ExtractedField {
                                    key: FieldKey::ResponsesApiDone {
                                        event_type: "response.output_text.done".into(),
                                    },
                                    text: text.to_owned(),
                                }]
                            } else {
                                vec![]
                            }
                        }
                        "response.reasoning_summary_text.delta" => {
                            if let Some(text) = json.get("delta").and_then(|v| v.as_str()) {
                                vec![ExtractedField {
                                    key: FieldKey::ResponsesApiDelta {
                                        event_type: "response.reasoning_summary_text.delta".into(),
                                    },
                                    text: text.to_owned(),
                                }]
                            } else {
                                vec![]
                            }
                        }
                        _ => vec![],
                    };
                }
            }

            let delta = match json.pointer("/choices/0/delta") {
                Some(d) => d,
                None => return vec![],
            };

            let mut fields = Vec::new();

            if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                // Skip empty-string content: it carries no secret and must not prevent
                // classify_terminal from running for co-located finish_reason frames.
                if !text.is_empty() {
                    fields.push(ExtractedField {
                        key: FieldKey::OpenAiContent,
                        text: text.to_owned(),
                    });
                }
            }

            if let Some(text) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                fields.push(ExtractedField {
                    key: FieldKey::OpenAiReasoning,
                    text: text.to_owned(),
                });
            }

            if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tool_calls.iter() {
                    let tc_index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                    {
                        if !args.is_empty() {
                            fields.push(ExtractedField {
                                key: FieldKey::OpenAiToolCall { index: tc_index },
                                text: args.to_owned(),
                            });
                        }
                    }
                }
            }

            if let Some(args) = delta
                .get("function_call")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
            {
                if !args.is_empty() {
                    fields.push(ExtractedField {
                        key: FieldKey::OpenAiFunctionCallArgs,
                        text: args.to_owned(),
                    });
                }
            }

            if let Some(text) = delta.get("refusal").and_then(|v| v.as_str()) {
                fields.push(ExtractedField {
                    key: FieldKey::OpenAiRefusal,
                    text: text.to_owned(),
                });
            }

            fields
        }
        Provider::Gemini => {
            let parts = match json
                .pointer("/candidates/0/content/parts")
                .and_then(|v| v.as_array())
            {
                Some(p) => p,
                None => return vec![],
            };

            let mut fields = Vec::new();

            for (i, part) in parts.iter().enumerate() {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    let thought = part
                        .get("thought")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    fields.push(ExtractedField {
                        key: FieldKey::GeminiText { thought },
                        text: text.to_owned(),
                    });
                }

                if let Some(output) = part
                    .pointer("/codeExecutionResult/output")
                    .and_then(|v| v.as_str())
                {
                    fields.push(ExtractedField {
                        key: FieldKey::GeminiCodeExecOutput { part_index: i },
                        text: output.to_owned(),
                    });
                }

                if let Some(args) = part
                    .pointer("/functionCall/args")
                    .and_then(|v| v.as_object())
                {
                    for (arg_key, arg_val) in args {
                        if let Some(s) = arg_val.as_str() {
                            fields.push(ExtractedField {
                                key: FieldKey::GeminiFuncCallArg {
                                    part_index: i,
                                    arg_key: arg_key.clone(),
                                },
                                text: s.to_owned(),
                            });
                        }
                        // Non-string values (numbers, booleans, objects) are skipped.
                    }
                }
            }

            fields
        }
    }
}

/// Writes restored values back into their JSON locations.
///
/// Returns `Err` if the target path doesn't exist in the JSON (signals
/// extract/apply mismatch).
#[cfg(test)]
fn apply_restored_fields(
    json: &mut Value,
    restorations: &[(WriteBackInfo, String)],
) -> Result<(), String> {
    for (write_back, text) in restorations {
        match write_back {
            WriteBackInfo::AnthropicDelta { field_name } => {
                let target = json
                    .get_mut("delta")
                    .and_then(|d| d.get_mut(field_name.as_str()));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(format!(
                            "apply_restored_fields: delta.{field_name} not found in JSON"
                        ));
                    }
                }
            }
            WriteBackInfo::OpenAiContent => {
                let target = json
                    .get_mut("choices")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("delta"))
                    .and_then(|d| d.get_mut("content"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(
                            "apply_restored_fields: choices[0].delta.content not found".to_owned()
                        );
                    }
                }
            }
            WriteBackInfo::OpenAiToolCall { array_pos } => {
                let target = json
                    .get_mut("choices")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("delta"))
                    .and_then(|d| d.get_mut("tool_calls"))
                    .and_then(|tc| tc.get_mut(*array_pos))
                    .and_then(|tc| tc.get_mut("function"))
                    .and_then(|f| f.get_mut("arguments"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(format!(
                            "apply_restored_fields: tool_calls[{array_pos}].function.arguments not found"
                        ));
                    }
                }
            }
            WriteBackInfo::OpenAiReasoning => {
                let target = json
                    .get_mut("choices")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("delta"))
                    .and_then(|d| d.get_mut("reasoning_content"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(
                            "apply_restored_fields: choices[0].delta.reasoning_content not found"
                                .to_owned(),
                        );
                    }
                }
            }
            WriteBackInfo::OpenAiRefusal => {
                let target = json
                    .get_mut("choices")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("delta"))
                    .and_then(|d| d.get_mut("refusal"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(
                            "apply_restored_fields: choices[0].delta.refusal not found".to_owned()
                        );
                    }
                }
            }
            WriteBackInfo::ResponsesApiDelta => {
                let target = json.get_mut("delta");
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => return Err("apply_restored_fields: delta not found".to_owned()),
                }
            }
            WriteBackInfo::ResponsesApiDone => {
                let target = json.get_mut("text");
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => return Err("apply_restored_fields: text not found".to_owned()),
                }
            }
            WriteBackInfo::GeminiPartText { part_index } => {
                let target = json
                    .get_mut("candidates")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("content"))
                    .and_then(|c| c.get_mut("parts"))
                    .and_then(|p| p.get_mut(*part_index))
                    .and_then(|p| p.get_mut("text"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(format!(
                            "apply_restored_fields: candidates[0].content.parts[{part_index}].text not found"
                        ));
                    }
                }
            }
            WriteBackInfo::GeminiPartCodeExecOutput { part_index } => {
                let target = json
                    .get_mut("candidates")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("content"))
                    .and_then(|c| c.get_mut("parts"))
                    .and_then(|p| p.get_mut(*part_index))
                    .and_then(|p| p.get_mut("codeExecutionResult"))
                    .and_then(|r| r.get_mut("output"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(format!(
                            "apply_restored_fields: candidates[0].content.parts[{part_index}].codeExecutionResult.output not found"
                        ));
                    }
                }
            }
            WriteBackInfo::GeminiPartFuncCallArg {
                part_index,
                arg_key,
            } => {
                let target = json
                    .get_mut("candidates")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("content"))
                    .and_then(|c| c.get_mut("parts"))
                    .and_then(|p| p.get_mut(*part_index))
                    .and_then(|p| p.get_mut("functionCall"))
                    .and_then(|fc| fc.get_mut("args"))
                    .and_then(|a| a.get_mut(arg_key.as_str()));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => {
                        return Err(format!(
                            "apply_restored_fields: candidates[0].content.parts[{part_index}].functionCall.args.{arg_key} not found"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Response stream wrapper that applies the correct restore strategy.
///
/// For SSE (`text/event-stream`): applies a per-FieldKey sliding-window
/// algorithm (SPEC.md §SSE-Aware Restore: Sliding Window). Holds at most
/// `max_fake_len` bytes per field before emitting restored synthetic frames,
/// so output flows in real time rather than waiting for EOF.
///
/// For non-SSE: buffers all bytes, runs `restore_stream` on the full buffer,
/// then drains. (Raw-byte restore handles non-SSE correctly because the fake
/// appears contiguously in the byte stream.)
pub struct SseRestoreStream {
    state: SseState,
}

enum SseState {
    /// Peek at the first non-empty chunk to decide SSE vs non-SSE.
    Detecting {
        inner: ResponseStream,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
        max_fake_len: usize,
    },
    /// Non-SSE: collect all bytes then run restore_stream.
    CollectingNonSse {
        inner: ResponseStream,
        raw_buf: Vec<u8>,
        entries: Vec<Entry>,
        session_key: SessionKey,
    },
    /// Processing the fully-collected non-SSE buffer (async).
    Processing(BoxFuture<'static, Result<VecDeque<Bytes>, io::Error>>),
    /// SSE sliding-window: emit restored synthetic frames as text accumulates.
    StreamingSse {
        inner: ResponseStream,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
        /// Hold window: max(fake.len()) across all entries.
        max_fake_len: usize,
        /// Per-FieldKey accumulation buffers (BTreeMap for deterministic flush order).
        accumulators: BTreeMap<FieldKey, String>,
        /// Incomplete frame bytes — no complete `\n\n` yet.
        raw_fragment: Vec<u8>,
        /// Frames ready to emit downstream.
        output_queue: VecDeque<Bytes>,
        /// Non-text Gemini content (thoughtSignature, groundingMetadata, non-string
        /// functionCall args) queued from mixed frames; emitted after the text flush.
        deferred_passthrough: VecDeque<Bytes>,
    },
    /// Draining the processed output queue.
    Emitting(VecDeque<Bytes>),
    /// Terminal: stream exhausted.
    Done,
}

impl SseRestoreStream {
    /// Wrap `stream` in SSE-aware restoring. `provider` identifies the
    /// provider-specific SSE frame schema.
    pub fn new(
        stream: ResponseStream,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
    ) -> Self {
        let max_fake_len = entries.iter().map(|e| e.fake.len()).max().unwrap_or(0);
        Self {
            state: SseState::Detecting {
                inner: stream,
                entries,
                session_key,
                provider,
                max_fake_len,
            },
        }
    }
}

impl Stream for SseRestoreStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match std::mem::replace(&mut self.state, SseState::Done) {
                SseState::Detecting {
                    mut inner,
                    entries,
                    session_key,
                    provider,
                    max_fake_len,
                } => match inner.as_mut().poll_next(cx) {
                    Poll::Ready(Some(Ok(chunk))) if chunk.is_empty() => {
                        // Empty leading chunk: wait for a non-empty one before latching is_sse.
                        self.state = SseState::Detecting {
                            inner,
                            entries,
                            session_key,
                            provider,
                            max_fake_len,
                        };
                        continue;
                    }
                    Poll::Ready(Some(Ok(chunk))) => {
                        if is_sse_first_chunk(&chunk) {
                            let mut raw_fragment = Vec::new();
                            raw_fragment.extend_from_slice(&chunk);
                            self.state = SseState::StreamingSse {
                                inner,
                                entries,
                                session_key,
                                provider,
                                max_fake_len,
                                accumulators: BTreeMap::new(),
                                raw_fragment,
                                output_queue: VecDeque::new(),
                                deferred_passthrough: VecDeque::new(),
                            };
                        } else {
                            let mut raw_buf = Vec::new();
                            raw_buf.extend_from_slice(&chunk);
                            self.state = SseState::CollectingNonSse {
                                inner,
                                raw_buf,
                                entries,
                                session_key,
                            };
                        }
                        continue;
                    }
                    Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                    Poll::Ready(None) => return Poll::Ready(None),
                    Poll::Pending => {
                        self.state = SseState::Detecting {
                            inner,
                            entries,
                            session_key,
                            provider,
                            max_fake_len,
                        };
                        return Poll::Pending;
                    }
                },

                SseState::CollectingNonSse {
                    mut inner,
                    mut raw_buf,
                    entries,
                    session_key,
                } => match inner.as_mut().poll_next(cx) {
                    Poll::Ready(Some(Ok(chunk))) => {
                        raw_buf.extend_from_slice(&chunk);
                        self.state = SseState::CollectingNonSse {
                            inner,
                            raw_buf,
                            entries,
                            session_key,
                        };
                        continue;
                    }
                    Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                    Poll::Ready(None) => {
                        let fut = restore_non_sse(raw_buf, entries, session_key).boxed();
                        self.state = SseState::Processing(fut);
                        continue;
                    }
                    Poll::Pending => {
                        self.state = SseState::CollectingNonSse {
                            inner,
                            raw_buf,
                            entries,
                            session_key,
                        };
                        return Poll::Pending;
                    }
                },

                SseState::Processing(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(queue)) => {
                        self.state = SseState::Emitting(queue);
                        continue;
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e))),
                    Poll::Pending => {
                        self.state = SseState::Processing(fut);
                        return Poll::Pending;
                    }
                },

                SseState::StreamingSse {
                    mut inner,
                    entries,
                    session_key,
                    provider,
                    max_fake_len,
                    mut accumulators,
                    mut raw_fragment,
                    mut output_queue,
                    mut deferred_passthrough,
                } => {
                    // Drain the output queue before polling for more input.
                    if let Some(bytes) = output_queue.pop_front() {
                        self.state = SseState::StreamingSse {
                            inner,
                            entries,
                            session_key,
                            provider,
                            max_fake_len,
                            accumulators,
                            raw_fragment,
                            output_queue,
                            deferred_passthrough,
                        };
                        return Poll::Ready(Some(Ok(bytes)));
                    }

                    // Process any data already in raw_fragment before polling inner.
                    // This handles the case where Detecting stored the first chunk
                    // in raw_fragment but we haven't parsed it yet.
                    if !raw_fragment.is_empty() {
                        if let Err(e) = process_sse_chunk(
                            &mut raw_fragment,
                            &[], // no new bytes — just parse what's there
                            &mut accumulators,
                            &SseCtx {
                                entries: &entries,
                                session_key: &session_key,
                                provider,
                                max_fake_len,
                            },
                            &mut output_queue,
                            &mut deferred_passthrough,
                        ) {
                            return Poll::Ready(Some(Err(e)));
                        }
                        if !output_queue.is_empty() {
                            self.state = SseState::StreamingSse {
                                inner,
                                entries,
                                session_key,
                                provider,
                                max_fake_len,
                                accumulators,
                                raw_fragment,
                                output_queue,
                                deferred_passthrough,
                            };
                            continue;
                        }
                    }

                    match inner.as_mut().poll_next(cx) {
                        Poll::Ready(Some(Ok(chunk))) => {
                            if let Err(e) = process_sse_chunk(
                                &mut raw_fragment,
                                &chunk,
                                &mut accumulators,
                                &SseCtx {
                                    entries: &entries,
                                    session_key: &session_key,
                                    provider,
                                    max_fake_len,
                                },
                                &mut output_queue,
                                &mut deferred_passthrough,
                            ) {
                                return Poll::Ready(Some(Err(e)));
                            }
                            self.state = SseState::StreamingSse {
                                inner,
                                entries,
                                session_key,
                                provider,
                                max_fake_len,
                                accumulators,
                                raw_fragment,
                                output_queue,
                                deferred_passthrough,
                            };
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                        Poll::Ready(None) => {
                            // EOF: flush all remaining accumulation buffers.
                            if let Err(e) = flush_all_accumulators(
                                &mut accumulators,
                                &entries,
                                &session_key,
                                &mut output_queue,
                            ) {
                                return Poll::Ready(Some(Err(e)));
                            }
                            // Append non-text Gemini pass-through extras AFTER text frames.
                            output_queue.extend(deferred_passthrough.drain(..));
                            // Emit any trailing bytes that never completed a frame.
                            if !raw_fragment.is_empty() {
                                output_queue.push_back(Bytes::from(raw_fragment));
                            }
                            self.state = SseState::Emitting(output_queue);
                            continue;
                        }
                        Poll::Pending => {
                            self.state = SseState::StreamingSse {
                                inner,
                                entries,
                                session_key,
                                provider,
                                max_fake_len,
                                accumulators,
                                raw_fragment,
                                output_queue,
                                deferred_passthrough,
                            };
                            return Poll::Pending;
                        }
                    }
                }

                SseState::Emitting(mut queue) => match queue.pop_front() {
                    Some(bytes) => {
                        if !queue.is_empty() {
                            self.state = SseState::Emitting(queue);
                        }
                        return Poll::Ready(Some(Ok(bytes)));
                    }
                    None => return Poll::Ready(None),
                },

                SseState::Done => return Poll::Ready(None),
            }
        }
    }
}

/// Shared context passed through the SSE processing helpers.
struct SseCtx<'a> {
    entries: &'a [Entry],
    session_key: &'a SessionKey,
    provider: Provider,
    max_fake_len: usize,
}

/// Appends `new_chunk` to `raw_fragment`, parses complete `\n\n`-terminated
/// SSE frames, and flushes safe prefixes of accumulation buffers.
fn process_sse_chunk(
    raw_fragment: &mut Vec<u8>,
    new_chunk: &[u8],
    accumulators: &mut BTreeMap<FieldKey, String>,
    ctx: &SseCtx<'_>,
    output_queue: &mut VecDeque<Bytes>,
    deferred_passthrough: &mut VecDeque<Bytes>,
) -> io::Result<()> {
    raw_fragment.extend_from_slice(new_chunk);
    let s = match std::str::from_utf8(raw_fragment) {
        Ok(s) => s.to_owned(),
        // Incomplete multibyte sequence at chunk boundary — wait for more bytes.
        Err(_) => return Ok(()),
    };
    let normalized = s.replace("\r\n", "\n");
    let mut pos = 0usize;
    let mut consumed = 0usize;
    while let Some(rel_end) = normalized[pos..].find("\n\n") {
        let frame_end = pos + rel_end + 2;
        process_one_frame(
            &normalized[pos..frame_end],
            accumulators,
            ctx,
            output_queue,
            deferred_passthrough,
        )?;
        consumed = frame_end;
        pos = frame_end;
    }
    // Store the tail (normalized, so \r\n is now \n) for the next chunk.
    *raw_fragment = normalized.as_bytes()[consumed..].to_vec();
    Ok(())
}

/// Scope of a terminal SSE event per SPEC.md §Terminal Event Ordering.
enum TerminalScope {
    /// Terminates one Anthropic content block; flush only
    /// `AnthropicDelta { index == N }` buffers (VC-SSE-16 isolation).
    Block(u64),
    /// Terminates the whole response; flush all buffers.
    Stream,
}

/// Returns the flush scope for a terminal SSE event, or `None` for non-terminal events.
///
/// Classification is provider-gated per SPEC.md §Terminal Event Ordering.
///
/// MUST only be called for frames where `extract_fields` returned empty. Co-located
/// content+terminal frames (VC-SSE-19 for Gemini) are handled by path B and MUST NOT
/// reach this function.
fn classify_terminal(
    json: &serde_json::Value,
    event_type: Option<&str>,
    provider: Provider,
) -> Option<TerminalScope> {
    match provider {
        Provider::Anthropic => match json["type"].as_str() {
            Some("content_block_stop") => Some(
                json["index"]
                    .as_u64()
                    .map(TerminalScope::Block)
                    .unwrap_or(TerminalScope::Stream),
            ),
            Some("message_delta") | Some("message_stop") => Some(TerminalScope::Stream),
            _ => None,
        },
        Provider::OpenAi | Provider::OpenRouter => {
            if let Some(
                "response.content_part.done"
                | "response.output_item.done"
                | "response.completed"
                | "response.failed"
                | "response.cancelled"
                | "response.incomplete",
            ) = event_type
            {
                return Some(TerminalScope::Stream);
            }
            if json
                .pointer("/choices/0/finish_reason")
                .is_some_and(|v| !v.is_null())
            {
                return Some(TerminalScope::Stream);
            }
            None
        }
        Provider::Gemini => {
            if json
                .pointer("/candidates/0/finishReason")
                .is_some_and(|v| !v.is_null())
            {
                Some(TerminalScope::Stream)
            } else {
                None
            }
        }
    }
}

/// Processes one complete SSE frame (ends with `\n\n`).
fn process_one_frame(
    frame: &str,
    accumulators: &mut BTreeMap<FieldKey, String>,
    ctx: &SseCtx<'_>,
    output_queue: &mut VecDeque<Bytes>,
    deferred_passthrough: &mut VecDeque<Bytes>,
) -> io::Result<()> {
    let event_type = frame.lines().find_map(|l| {
        l.strip_prefix("event: ")
            .or_else(|| l.strip_prefix("event:"))
            .filter(|s| !s.is_empty())
    });
    let data_content = frame
        .lines()
        .find_map(|l| l.strip_prefix("data: ").or_else(|| l.strip_prefix("data:")));

    let Some(data_str) = data_content else {
        // No data line (comment / keep-alive): pass through immediately.
        output_queue.push_back(Bytes::from(frame.as_bytes().to_vec()));
        return Ok(());
    };

    let Ok(json) = serde_json::from_str::<serde_json::Value>(data_str) else {
        // "[DONE]" is the Chat Completions stream terminator (OpenAI/OpenRouter
        // only): complete-flush all buffers before forwarding (SPEC VC-SSE-17).
        // Anthropic and Gemini never emit [DONE]; they use JSON terminals
        // (message_stop, finishReason). The provider guard prevents spurious
        // flushes from stray non-JSON frames.
        if data_str.trim() == "[DONE]"
            && matches!(ctx.provider, Provider::OpenAi | Provider::OpenRouter)
        {
            flush_all_accumulators(accumulators, ctx.entries, ctx.session_key, output_queue)?;
        }
        output_queue.push_back(Bytes::from(frame.as_bytes().to_vec()));
        return Ok(());
    };

    let extracted = extract_fields(&json, ctx.provider, event_type);
    if extracted.is_empty() {
        match classify_terminal(&json, event_type, ctx.provider) {
            Some(TerminalScope::Block(n)) => flush_accumulators_where(
                accumulators,
                |k| matches!(k, FieldKey::AnthropicDelta { index, .. } if *index == n),
                ctx.entries,
                ctx.session_key,
                output_queue,
            )?,
            Some(TerminalScope::Stream) => {
                flush_all_accumulators(accumulators, ctx.entries, ctx.session_key, output_queue)?;
                // Drain Gemini deferred pass-through (non-text metadata) before the
                // terminal frame so clients that finalize on finishReason see tool
                // args first (SPEC VC-SSE-12, VC-SSE-11).
                if ctx.provider == Provider::Gemini {
                    output_queue.extend(deferred_passthrough.drain(..));
                }
            }
            None => {}
        }
        output_queue.push_back(Bytes::from(frame.as_bytes().to_vec()));
        return Ok(());
    }

    for field in &extracted {
        accumulators
            .entry(field.key.clone())
            .or_default()
            .push_str(&field.text);
    }

    // For Gemini frames: collect non-text content that must pass through unchanged.
    // These are queued as deferred pass-through frames emitted after the text flush.
    if ctx.provider == Provider::Gemini {
        let mut extra = serde_json::json!({});
        // Top-level metadata fields that MUST NOT be modified (SPEC VC-SSE-12).
        if let Some(v) = json.get("thoughtSignature") {
            extra["thoughtSignature"] = v.clone();
        }
        if let Some(v) = json.get("groundingMetadata") {
            extra["groundingMetadata"] = v.clone();
        }
        // Non-string functionCall args and function name (SPEC VC-SSE-11).
        if let Some(parts) = json.pointer("/candidates/0/content/parts") {
            if let Some(arr) = parts.as_array() {
                for part in arr {
                    if let Some(fc) = part.get("functionCall") {
                        let mut fc_extra = serde_json::json!({});
                        if let Some(name) = fc.get("name") {
                            fc_extra["name"] = name.clone();
                        }
                        if let Some(args) = fc.get("args").and_then(|a| a.as_object()) {
                            let non_str: serde_json::Map<String, serde_json::Value> = args
                                .iter()
                                .filter(|(_, v)| !v.is_string())
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect();
                            if !non_str.is_empty() {
                                fc_extra["args"] = serde_json::Value::Object(non_str);
                            }
                        }
                        if !fc_extra.as_object().unwrap().is_empty() {
                            extra["candidates"] = serde_json::json!([{"content": {"parts": [{"functionCall": fc_extra}]}}]);
                        }
                    }
                }
            }
        }
        if !extra.as_object().unwrap().is_empty() {
            let frame_str = format!("data: {}\n\n", serde_json::to_string(&extra).unwrap());
            deferred_passthrough.push_back(Bytes::from(frame_str.into_bytes()));
        }
    }

    // Flush any accumulation buffer whose safe prefix is ready.
    for (key, accum) in accumulators.iter_mut() {
        flush_safe_prefix(
            key,
            accum,
            ctx.max_fake_len,
            ctx.entries,
            ctx.session_key,
            output_queue,
        )?;
    }

    Ok(())
}

/// Flushes the safe prefix of one accumulation buffer into `output_queue`.
///
/// The nominal safe length is `accum.len() - max_fake_len`. Before using that
/// boundary, we scan for any fake whose prefix appears as a suffix of the nominal
/// safe region. If one is found, the safe boundary is retracted to just before
/// that partial match, preventing the fake from being split across two restore
/// calls — which would make neither half match the Aho-Corasick automaton.
///
/// During terminal flushes (`max_fake_len == 0`) the entire accumulator is
/// flushed unconditionally; no partial-match check is needed because the
/// complete fake must already be present.
fn flush_safe_prefix(
    key: &FieldKey,
    accum: &mut String,
    max_fake_len: usize,
    entries: &[Entry],
    session_key: &SessionKey,
    output_queue: &mut VecDeque<Bytes>,
) -> io::Result<()> {
    if accum.len() <= max_fake_len {
        return Ok(());
    }
    let nominal_target = accum.len() - max_fake_len;

    // During non-terminal flushes, retract the boundary if any fake prefix
    // appears as a suffix of the nominal safe region. Iterate from the longest
    // possible match downward so we find the leftmost partial-fake start.
    let target = if max_fake_len > 0 {
        let accum_bytes = accum.as_bytes();
        let mut safe_target = nominal_target;
        'entries: for entry in entries {
            let fake = &entry.fake;
            let max_k = fake.len().saturating_sub(1).min(nominal_target);
            for k in (1..=max_k).rev() {
                if accum_bytes[nominal_target - k..nominal_target] == fake[..k] {
                    safe_target = safe_target.min(nominal_target - k);
                    continue 'entries;
                }
            }
        }
        safe_target
    } else {
        nominal_target
    };

    // Round down to the nearest char boundary.
    let safe_len = accum[..target]
        .char_indices()
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    if safe_len == 0 {
        return Ok(());
    }
    let safe_prefix = accum[..safe_len].to_owned();

    let mut input = std::io::Cursor::new(safe_prefix.as_bytes());
    let mut output = Vec::new();
    doppel_restore(&mut input, &mut output, entries, session_key)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let restored = String::from_utf8(output)
        .map_err(|e| io::Error::other(format!("restore produced non-UTF8: {e}")))?;

    if !restored.is_empty() {
        output_queue.push_back(build_synthetic_sse_frame(key, &restored));
    }
    *accum = accum[safe_len..].to_owned();
    Ok(())
}

/// Completely flushes (hold window = 0) every accumulation buffer whose
/// key satisfies `pred`. Used for terminal-event flushes (SPEC.md
/// §Terminal Event Ordering) and, via flush_all_accumulators, at EOF.
fn flush_accumulators_where(
    accumulators: &mut BTreeMap<FieldKey, String>,
    pred: impl Fn(&FieldKey) -> bool,
    entries: &[Entry],
    session_key: &SessionKey,
    output_queue: &mut VecDeque<Bytes>,
) -> io::Result<()> {
    for (key, accum) in accumulators.iter_mut() {
        if accum.is_empty() || !pred(key) {
            continue;
        }
        flush_safe_prefix(key, accum, 0, entries, session_key, output_queue)?;
    }
    Ok(())
}

/// Flushes all accumulation buffers completely (called at stream EOF).
fn flush_all_accumulators(
    accumulators: &mut BTreeMap<FieldKey, String>,
    entries: &[Entry],
    session_key: &SessionKey,
    output_queue: &mut VecDeque<Bytes>,
) -> io::Result<()> {
    flush_accumulators_where(accumulators, |_| true, entries, session_key, output_queue)
}

/// Builds a synthetic SSE frame with `text` embedded in the correct JSON path
/// for the given `FieldKey`. Frame granularity MAY differ from the original
/// per SPEC.md §SSE-Aware Restore: Sliding Window.
fn build_synthetic_sse_frame(key: &FieldKey, text: &str) -> Bytes {
    let frame = match key {
        FieldKey::AnthropicDelta { delta_type, index } => {
            let value = match delta_type.as_str() {
                "text_delta" => serde_json::json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": "text_delta", "text": text}
                }),
                "thinking_delta" => serde_json::json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": "thinking_delta", "thinking": text}
                }),
                "input_json_delta" => serde_json::json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": "input_json_delta", "partial_json": text}
                }),
                other => serde_json::json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": other, "text": text}
                }),
            };
            format!(
                "event: content_block_delta\ndata: {}\n\n",
                serde_json::to_string(&value).unwrap()
            )
        }
        FieldKey::OpenAiContent => {
            let v = serde_json::json!({"choices": [{"delta": {"content": text}}]});
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::OpenAiToolCall { index } => {
            let v = serde_json::json!({
                "choices": [{"delta": {"tool_calls": [{
                    "index": index,
                    "function": {"arguments": text}
                }]}}]
            });
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::OpenAiReasoning => {
            let v = serde_json::json!({"choices": [{"delta": {"reasoning_content": text}}]});
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::OpenAiFunctionCallArgs => {
            let v = serde_json::json!({
                "choices": [{"delta": {"function_call": {"arguments": text}}}]
            });
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::OpenAiRefusal => {
            let v = serde_json::json!({"choices": [{"delta": {"refusal": text}}]});
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::ResponsesApiDelta { event_type } => {
            let v = serde_json::json!({"delta": text});
            format!(
                "event: {}\ndata: {}\n\n",
                event_type,
                serde_json::to_string(&v).unwrap()
            )
        }
        FieldKey::ResponsesApiDone { event_type } => {
            let v = serde_json::json!({"text": text});
            format!(
                "event: {}\ndata: {}\n\n",
                event_type,
                serde_json::to_string(&v).unwrap()
            )
        }
        FieldKey::GeminiText { .. } => {
            let v = serde_json::json!({
                "candidates": [{"content": {"parts": [{"text": text}]}}]
            });
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::GeminiCodeExecOutput { .. } => {
            let v = serde_json::json!({
                "candidates": [{"content": {"parts": [{
                    "codeExecutionResult": {"output": text}
                }]}}]
            });
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
        FieldKey::GeminiFuncCallArg { arg_key, .. } => {
            let v = serde_json::json!({
                "candidates": [{"content": {"parts": [{
                    "functionCall": {"args": {arg_key: text}}
                }]}}]
            });
            format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
        }
    };
    Bytes::from(frame.into_bytes())
}

async fn restore_non_sse(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
) -> Result<VecDeque<Bytes>, io::Error> {
    use futures_util::stream;

    let stream: ResponseStream = Box::pin(stream::once(async move {
        Ok::<Bytes, io::Error>(Bytes::from(raw))
    }));
    let us = restore_stream(stream, entries, session_key)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let mut queue = VecDeque::new();
    futures_util::pin_mut!(us);
    while let Some(chunk) = us.next().await {
        let bytes = chunk.map_err(|e| io::Error::other(e.to_string()))?;
        queue.push_back(bytes);
    }
    Ok(queue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_sse_detects_data_prefix() {
        assert!(is_sse_first_chunk(
            b"data: {\"type\":\"message_start\"}\n\n"
        ));
    }

    #[test]
    fn is_sse_detects_event_prefix() {
        // Real Anthropic SSE streams begin with an `event:` line before `data:`.
        assert!(is_sse_first_chunk(
            b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n"
        ));
    }

    #[test]
    fn is_sse_rejects_json() {
        assert!(!is_sse_first_chunk(b"{\"type\":\"message\"}"));
    }

    #[test]
    fn is_sse_rejects_empty() {
        assert!(!is_sse_first_chunk(b""));
    }

    #[test]
    fn is_sse_detection_skips_empty_first_chunk() {
        // Regression: an empty leading chunk must not latch is_sse to false.
        // The detection should wait for a non-empty chunk.
        // This test documents the intent; the actual guard is in SseRestoreStream::poll_next.
        assert!(!is_sse_first_chunk(b""));
        assert!(is_sse_first_chunk(b"data: "));
        assert!(is_sse_first_chunk(b"event: "));
    }

    #[test]
    fn extract_fields_anthropic_text_delta() {
        let v = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "hello" }
        });
        let fields = extract_fields(&v, Provider::Anthropic, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::AnthropicDelta {
                delta_type: "text_delta".to_owned(),
                index: 0
            }
        );
        assert_eq!(fields[0].text, "hello");
    }

    #[test]
    fn extract_fields_anthropic_skips_message_start() {
        let v = json!({ "type": "message_start" });
        assert!(extract_fields(&v, Provider::Anthropic, None).is_empty());
    }

    #[test]
    fn extract_fields_anthropic_skips_content_block_stop() {
        let v = json!({ "type": "content_block_stop", "index": 0 });
        assert!(extract_fields(&v, Provider::Anthropic, None).is_empty());
    }

    #[test]
    fn extract_fields_openai_delta_content() {
        let v = json!({ "choices": [{ "delta": { "content": "hi" } }] });
        let fields = extract_fields(&v, Provider::OpenAi, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::OpenAiContent);
        assert_eq!(fields[0].text, "hi");
    }

    #[test]
    fn extract_fields_openai_skips_null_content() {
        let v = json!({ "choices": [{ "delta": {} }] });
        assert!(extract_fields(&v, Provider::OpenAi, None).is_empty());
    }

    #[test]
    fn extract_fields_gemini_text() {
        let v = json!({
            "candidates": [{ "content": { "parts": [{ "text": "hi" }] } }]
        });
        let fields = extract_fields(&v, Provider::Gemini, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::GeminiText { thought: false });
        assert_eq!(fields[0].text, "hi");
    }

    #[test]
    fn extract_fields_gemini_skips_empty_parts() {
        let v = json!({
            "candidates": [{ "content": { "parts": [] } }]
        });
        assert!(extract_fields(&v, Provider::Gemini, None).is_empty());
    }

    #[test]
    fn apply_fields_anthropic_text() {
        let mut v = json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "old" }
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::AnthropicDelta {
                    field_name: "text".to_owned(),
                },
                "new".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(v["delta"]["text"], json!("new"));
    }

    #[test]
    fn apply_fields_openai_content() {
        let mut v = json!({ "choices": [{ "delta": { "content": "old" } }] });
        apply_restored_fields(&mut v, &[(WriteBackInfo::OpenAiContent, "new".to_owned())]).unwrap();
        assert_eq!(v["choices"][0]["delta"]["content"], json!("new"));
    }

    #[test]
    fn apply_fields_gemini_text() {
        let mut v = json!({
            "candidates": [{ "content": { "parts": [{ "text": "old" }] } }]
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::GeminiPartText { part_index: 0 },
                "new".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["text"],
            json!("new")
        );
    }

    #[tokio::test]
    async fn sse_restore_stream_passthrough_no_secrets() {
        // SSE stream with no fakes — output bytes must equal input bytes.
        use futures_util::stream;
        let input = b"data: {\"type\":\"message_start\"}\n\ndata: {\"type\":\"message_stop\"}\n\n";
        let stream: ResponseStream = Box::pin(stream::once(async move {
            Ok::<Bytes, io::Error>(Bytes::from_static(input))
        }));
        // Use empty entries + dummy session key → no restoring applied.
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let us = SseRestoreStream::new(stream, vec![], session_key, Provider::Anthropic);
        let out: Vec<Bytes> = futures_util::StreamExt::collect::<Vec<_>>(us)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        let out_bytes: Vec<u8> = out.iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(out_bytes, input);
    }

    #[tokio::test]
    async fn non_sse_passthrough_no_secrets() {
        use futures_util::stream;
        let input = b"{\"result\":\"ok\"}";
        let stream: ResponseStream = Box::pin(stream::once(async move {
            Ok::<Bytes, io::Error>(Bytes::from_static(input))
        }));
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let us = SseRestoreStream::new(stream, vec![], session_key, Provider::OpenAi);
        let out: Vec<u8> = futures_util::StreamExt::collect::<Vec<_>>(us)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .flat_map(|b| b.to_vec())
            .collect();
        assert_eq!(out, input);
    }

    #[tokio::test]
    async fn empty_stream_produces_empty_output() {
        use futures_util::stream;
        let stream: ResponseStream = Box::pin(stream::empty());
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let us = SseRestoreStream::new(stream, vec![], session_key, Provider::Gemini);
        let out: Vec<_> = futures_util::StreamExt::collect::<Vec<_>>(us).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn sse_event_lines_preserved_through_text_frame_reconstruction() {
        // Text frames that have an `event:` prefix line must have that line
        // preserved in the output — reconstruction must not drop non-data fields.
        use futures_util::stream;
        let input = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let stream: ResponseStream = Box::pin(stream::once(async move {
            Ok::<Bytes, io::Error>(Bytes::from(input.as_bytes().to_vec()))
        }));
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let us = SseRestoreStream::new(stream, vec![], session_key, Provider::Anthropic);
        let out: Vec<u8> = futures_util::StreamExt::collect::<Vec<_>>(us)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .flat_map(|b| b.to_vec())
            .collect();
        let out_str = String::from_utf8(out).unwrap();
        // Each event: + data: pair must be in the same SSE frame (no \n\n between them).
        let frames: Vec<&str> = out_str.split("\n\n").filter(|f| !f.is_empty()).collect();
        let text_frame = frames
            .iter()
            .find(|f| f.contains("content_block_delta"))
            .expect("must have a content_block_delta frame");
        assert!(
            text_frame.contains("event: content_block_delta\n") && text_frame.contains("data: "),
            "event: and data: must be in the same frame; got: {text_frame:?}"
        );
        let stop_frame = frames
            .iter()
            .find(|f| f.contains("message_stop"))
            .expect("must have a message_stop frame");
        assert!(
            stop_frame.contains("event: message_stop\n") && stop_frame.contains("data: "),
            "event: and data: must be in the same frame; got: {stop_frame:?}"
        );
    }

    #[test]
    fn extract_fields_anthropic_thinking_delta() {
        let v = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "thinking_delta", "thinking": "Let me reason..." }
        });
        let fields = extract_fields(&v, Provider::Anthropic, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::AnthropicDelta {
                delta_type: "thinking_delta".into(),
                index: 0
            }
        );
        assert_eq!(fields[0].text, "Let me reason...");
    }

    #[test]
    fn extract_fields_anthropic_input_json_delta() {
        let v = json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "{\"key\":" }
        });
        let fields = extract_fields(&v, Provider::Anthropic, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::AnthropicDelta {
                delta_type: "input_json_delta".into(),
                index: 1
            }
        );
        assert_eq!(fields[0].text, "{\"key\":");
    }

    #[test]
    fn extract_fields_anthropic_skips_signature_delta() {
        let v = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "signature_delta", "signature": "bWVzc2FnZV9zaWduYXR1cmU=" }
        });
        let fields = extract_fields(&v, Provider::Anthropic, None);
        assert!(
            fields.is_empty(),
            "signature_delta MUST be passed through unmodified"
        );
    }

    #[test]
    fn apply_fields_anthropic_thinking() {
        let mut v = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "thinking_delta", "thinking": "old" }
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::AnthropicDelta {
                    field_name: "thinking".into(),
                },
                "new thought".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(v["delta"]["thinking"], json!("new thought"));
    }

    #[test]
    fn apply_fields_anthropic_input_json() {
        let mut v = json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "old" }
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::AnthropicDelta {
                    field_name: "partial_json".into(),
                },
                "{\"restored\":true}".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(v["delta"]["partial_json"], json!("{\"restored\":true}"));
    }
    #[test]
    fn extract_fields_openai_tool_calls() {
        let v = json!({
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "function": {"arguments": "{\"q\":"}}]
            }}]
        });
        let fields = extract_fields(&v, Provider::OpenAi, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::OpenAiToolCall { index: 0 });
        assert_eq!(fields[0].text, "{\"q\":");
    }

    #[test]
    fn extract_fields_openai_tool_calls_skips_empty_arguments() {
        let v = json!({
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "id": "call_abc", "type": "function",
                                "function": {"name": "get_secret", "arguments": ""}}]
            }}]
        });
        let fields = extract_fields(&v, Provider::OpenAi, None);
        assert!(fields.is_empty());
    }

    #[test]
    fn extract_fields_openai_reasoning_content() {
        let v = json!({
            "choices": [{"index": 0, "delta": {"reasoning_content": "thinking..."}}]
        });
        let fields = extract_fields(&v, Provider::OpenAi, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::OpenAiReasoning);
        assert_eq!(fields[0].text, "thinking...");
    }

    #[test]
    fn extract_fields_openai_function_call_args() {
        let v = json!({
            "choices": [{"index": 0, "delta": {
                "function_call": {"arguments": "{\"loc\":"}
            }}]
        });
        let fields = extract_fields(&v, Provider::OpenAi, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::OpenAiFunctionCallArgs);
        assert_eq!(fields[0].text, "{\"loc\":");
    }

    #[test]
    fn extract_fields_openai_refusal() {
        let v = json!({"choices": [{"delta": {"refusal": "I cannot do that"}}]});
        let fields = extract_fields(&v, Provider::OpenAi, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::OpenAiRefusal);
        assert_eq!(fields[0].text, "I cannot do that");
    }

    #[test]
    fn extract_fields_openrouter_reasoning_content() {
        let v = json!({
            "choices": [{"index": 0, "delta": {"reasoning_content": "step 1..."}}]
        });
        let fields = extract_fields(&v, Provider::OpenRouter, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::OpenAiReasoning);
    }

    #[test]
    fn apply_fields_openai_tool_calls() {
        let mut v = json!({
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "function": {"arguments": "old"}}]
            }}]
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::OpenAiToolCall { array_pos: 0 },
                "restored".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(
            v["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            json!("restored")
        );
    }

    #[test]
    fn apply_fields_openai_reasoning() {
        let mut v = json!({
            "choices": [{"index": 0, "delta": {"reasoning_content": "old"}}]
        });
        apply_restored_fields(
            &mut v,
            &[(WriteBackInfo::OpenAiReasoning, "restored".to_owned())],
        )
        .unwrap();
        assert_eq!(
            v["choices"][0]["delta"]["reasoning_content"],
            json!("restored")
        );
    }

    #[test]
    fn apply_fields_openai_refusal() {
        let mut v = json!({
            "choices": [{"index": 0, "delta": {"refusal": "old refusal"}}]
        });
        apply_restored_fields(
            &mut v,
            &[(WriteBackInfo::OpenAiRefusal, "restored refusal".to_owned())],
        )
        .unwrap();
        assert_eq!(
            v["choices"][0]["delta"]["refusal"],
            json!("restored refusal")
        );
    }
    #[test]
    fn extract_fields_responses_api_text_delta() {
        let v = json!({
            "type": "response.output_text.delta",
            "output_index": 0,
            "content_index": 0,
            "delta": "Hello world"
        });
        let fields = extract_fields(&v, Provider::OpenAi, Some("response.output_text.delta"));
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::ResponsesApiDelta {
                event_type: "response.output_text.delta".into()
            }
        );
        assert_eq!(fields[0].text, "Hello world");
    }

    #[test]
    fn extract_fields_responses_api_text_done() {
        let v = json!({
            "type": "response.output_text.done",
            "output_index": 0,
            "content_index": 0,
            "text": "Full assembled text here"
        });
        let fields = extract_fields(&v, Provider::OpenAi, Some("response.output_text.done"));
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::ResponsesApiDone {
                event_type: "response.output_text.done".into()
            }
        );
        assert_eq!(fields[0].text, "Full assembled text here");
    }

    #[test]
    fn extract_fields_responses_api_reasoning_delta() {
        let v = json!({
            "type": "response.reasoning_summary_text.delta",
            "output_index": 0,
            "summary_index": 0,
            "delta": "reasoning step"
        });
        let fields = extract_fields(
            &v,
            Provider::OpenAi,
            Some("response.reasoning_summary_text.delta"),
        );
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::ResponsesApiDelta {
                event_type: "response.reasoning_summary_text.delta".into()
            }
        );
    }

    #[test]
    fn extract_fields_responses_api_skips_non_content_events() {
        let v = json!({"type": "response.created", "response": {"id": "resp_test"}});
        let fields = extract_fields(&v, Provider::OpenAi, Some("response.created"));
        assert!(fields.is_empty());
    }

    #[test]
    fn extract_fields_responses_api_does_not_use_chat_completions_paths() {
        let v = json!({
            "type": "response.output_text.delta",
            "delta": "correct",
            "choices": [{"index": 0, "delta": {"content": "wrong"}}]
        });
        let fields = extract_fields(&v, Provider::OpenAi, Some("response.output_text.delta"));
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].text, "correct");
    }

    #[test]
    fn apply_fields_responses_api_delta() {
        let mut v = json!({"type": "response.output_text.delta", "delta": "old"});
        apply_restored_fields(
            &mut v,
            &[(WriteBackInfo::ResponsesApiDelta, "restored".to_owned())],
        )
        .unwrap();
        assert_eq!(v["delta"], json!("restored"));
    }

    #[test]
    fn apply_fields_responses_api_done() {
        let mut v = json!({"type": "response.output_text.done", "text": "old"});
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::ResponsesApiDone,
                "restored full text".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(v["text"], json!("restored full text"));
    }

    #[test]
    fn extract_fields_gemini_multi_part_text() {
        let v = json!({
            "candidates": [{"content": {"parts": [
                {"text": "thought", "thought": true},
                {"text": "answer"}
            ]}}]
        });
        let fields = extract_fields(&v, Provider::Gemini, None);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, FieldKey::GeminiText { thought: true });
        assert_eq!(fields[0].text, "thought");
        assert_eq!(fields[1].key, FieldKey::GeminiText { thought: false });
        assert_eq!(fields[1].text, "answer");
    }

    #[test]
    fn extract_fields_gemini_code_execution_output() {
        let v = json!({
            "candidates": [{"content": {"parts": [
                {"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "result: 42\n"}}
            ]}}]
        });
        let fields = extract_fields(&v, Provider::Gemini, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::GeminiCodeExecOutput { part_index: 0 }
        );
        assert_eq!(fields[0].text, "result: 42\n");
    }

    #[test]
    fn extract_fields_gemini_function_call_args() {
        let v = json!({
            "candidates": [{"content": {"parts": [
                {"functionCall": {"name": "lookup", "args": {"query": "New York", "count": 5}}}
            ]}}]
        });
        let fields = extract_fields(&v, Provider::Gemini, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].key,
            FieldKey::GeminiFuncCallArg {
                part_index: 0,
                arg_key: "query".into()
            }
        );
        assert_eq!(fields[0].text, "New York");
    }

    #[test]
    fn extract_fields_gemini_skips_thought_signature() {
        let v = json!({
            "candidates": [{"content": {"parts": [{"text": "answer"}]}}],
            "thoughtSignature": "base64signature=="
        });
        let fields = extract_fields(&v, Provider::Gemini, None);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, FieldKey::GeminiText { thought: false });
    }

    #[test]
    fn extract_fields_gemini_skips_grounding_metadata() {
        let v = json!({
            "candidates": [{"content": {"parts": [{"text": "answer"}]}}],
            "groundingMetadata": {"searchEntryPoint": {"renderedContent": "<html>"}}
        });
        let fields = extract_fields(&v, Provider::Gemini, None);
        assert_eq!(fields.len(), 1);
    }

    #[test]
    fn apply_fields_gemini_multi_part() {
        let mut v = json!({
            "candidates": [{"content": {"parts": [
                {"text": "old_thought", "thought": true},
                {"text": "old_answer"}
            ]}}]
        });
        apply_restored_fields(
            &mut v,
            &[
                (
                    WriteBackInfo::GeminiPartText { part_index: 0 },
                    "new_thought".to_owned(),
                ),
                (
                    WriteBackInfo::GeminiPartText { part_index: 1 },
                    "new_answer".to_owned(),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["text"],
            json!("new_thought")
        );
        assert_eq!(
            v["candidates"][0]["content"]["parts"][1]["text"],
            json!("new_answer")
        );
    }

    #[test]
    fn apply_fields_gemini_code_exec_output() {
        let mut v = json!({
            "candidates": [{"content": {"parts": [
                {"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "old"}}
            ]}}]
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::GeminiPartCodeExecOutput { part_index: 0 },
                "restored output".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["codeExecutionResult"]["output"],
            json!("restored output")
        );
    }

    #[test]
    fn apply_fields_gemini_func_call_arg() {
        let mut v = json!({
            "candidates": [{"content": {"parts": [
                {"functionCall": {"name": "lookup", "args": {"query": "old", "count": 5}}}
            ]}}]
        });
        apply_restored_fields(
            &mut v,
            &[(
                WriteBackInfo::GeminiPartFuncCallArg {
                    part_index: 0,
                    arg_key: "query".into(),
                },
                "restored query".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["query"],
            json!("restored query")
        );
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["count"],
            json!(5)
        );
    }
    #[test]
    fn flush_where_only_matching_keys() {
        // Two accumulators: index 0 and index 1.
        // Flush with a predicate that matches only index 0.
        // After the call: index-0 buffer drained into output_queue,
        // index-1 buffer untouched and absent from output_queue.
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "text_delta".to_owned(),
                index: 0,
            },
            "hello".to_owned(),
        );
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "input_json_delta".to_owned(),
                index: 1,
            },
            "world".to_owned(),
        );
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        flush_accumulators_where(
            &mut accumulators,
            |k| matches!(k, FieldKey::AnthropicDelta { index: 0, .. }),
            &[],
            &session_key,
            &mut output_queue,
        )
        .unwrap();
        // Index-0 buffer flushed: accumulator is empty, output_queue has one frame.
        assert!(
            accumulators[&FieldKey::AnthropicDelta {
                delta_type: "text_delta".to_owned(),
                index: 0,
            }]
                .is_empty()
        );
        assert_eq!(output_queue.len(), 1);
        let frame = std::str::from_utf8(&output_queue[0]).unwrap();
        assert!(
            frame.contains("hello"),
            "frame should contain 'hello': {frame}"
        );
        // Index-1 buffer untouched.
        assert_eq!(
            accumulators[&FieldKey::AnthropicDelta {
                delta_type: "input_json_delta".to_owned(),
                index: 1,
            }],
            "world"
        );
    }

    #[test]
    fn flush_where_block_scope_drains_all_delta_types_at_same_index() {
        // Two accumulators share index 0 but have different delta_type values.
        // A Block(0) predicate must drain BOTH, ensuring the `..` wildcard over
        // delta_type is load-bearing: narrowing it to a single type would leave
        // the sibling buffer un-flushed, violating VC-SSE-16.
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "text_delta".to_owned(),
                index: 0,
            },
            "hello".to_owned(),
        );
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "input_json_delta".to_owned(),
                index: 0,
            },
            "world".to_owned(),
        );
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        flush_accumulators_where(
            &mut accumulators,
            |k| matches!(k, FieldKey::AnthropicDelta { index: 0, .. }),
            &[],
            &session_key,
            &mut output_queue,
        )
        .unwrap();
        // Both index-0 buffers drained regardless of delta_type.
        for accum in accumulators.values() {
            assert!(accum.is_empty(), "accumulator not empty: {accum:?}");
        }
        assert_eq!(
            output_queue.len(),
            2,
            "expected two frames (one per delta_type)"
        );
        let frames: Vec<&str> = output_queue
            .iter()
            .map(|b| std::str::from_utf8(b).unwrap())
            .collect();
        assert!(
            frames.iter().any(|f| f.contains("hello")),
            "'hello' frame missing from output"
        );
        assert!(
            frames.iter().any(|f| f.contains("world")),
            "'world' frame missing from output"
        );
    }

    #[test]
    fn flush_where_true_flushes_all() {
        // Predicate |_| true must drain every non-empty accumulator.
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "text_delta".to_owned(),
                index: 0,
            },
            "hello".to_owned(),
        );
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "input_json_delta".to_owned(),
                index: 1,
            },
            "world".to_owned(),
        );
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        flush_accumulators_where(
            &mut accumulators,
            |_| true,
            &[],
            &session_key,
            &mut output_queue,
        )
        .unwrap();
        // Both buffers drained.
        for accum in accumulators.values() {
            assert!(accum.is_empty(), "accumulator not empty: {accum:?}");
        }
        assert_eq!(output_queue.len(), 2);
    }

    #[test]
    fn classify_terminal_anthropic_content_block_stop_with_index() {
        let json0 = serde_json::json!({"type": "content_block_stop", "index": 0});
        let json3 = serde_json::json!({"type": "content_block_stop", "index": 3});
        assert!(
            matches!(
                classify_terminal(&json0, None, Provider::Anthropic),
                Some(TerminalScope::Block(0))
            ),
            "index 0 should be Block(0)"
        );
        assert!(
            matches!(
                classify_terminal(&json3, None, Provider::Anthropic),
                Some(TerminalScope::Block(3))
            ),
            "index 3 should be Block(3)"
        );
    }

    #[test]
    fn classify_terminal_anthropic_content_block_stop_missing_index_falls_back_to_stream() {
        let json = serde_json::json!({"type": "content_block_stop"});
        assert!(matches!(
            classify_terminal(&json, None, Provider::Anthropic),
            Some(TerminalScope::Stream)
        ));
        let json_bad_index = serde_json::json!({"type": "content_block_stop", "index": "bad"});
        assert!(matches!(
            classify_terminal(&json_bad_index, None, Provider::Anthropic),
            Some(TerminalScope::Stream)
        ));
    }

    #[test]
    fn classify_terminal_anthropic_stream_scope_events() {
        for ty in ["message_delta", "message_stop"] {
            let json = serde_json::json!({"type": ty});
            assert!(
                matches!(
                    classify_terminal(&json, None, Provider::Anthropic),
                    Some(TerminalScope::Stream)
                ),
                "{ty} should be Stream"
            );
        }
    }

    #[test]
    fn classify_terminal_anthropic_non_terminal_events_return_none() {
        for ty in ["ping", "message_start", "content_block_start"] {
            let json = serde_json::json!({"type": ty});
            assert!(
                classify_terminal(&json, None, Provider::Anthropic).is_none(),
                "{ty} should be None"
            );
        }
    }

    #[test]
    fn classify_terminal_openai_chat_finish_reason_stream() {
        let json = serde_json::json!({
            "choices": [{"delta": {}, "finish_reason": "tool_calls"}]
        });
        assert!(matches!(
            classify_terminal(&json, None, Provider::OpenAi),
            Some(TerminalScope::Stream)
        ));
    }

    #[test]
    fn classify_terminal_openai_chat_null_finish_reason_is_none() {
        let json = serde_json::json!({
            "choices": [{"delta": {"content": "x"}, "finish_reason": null}]
        });
        assert!(classify_terminal(&json, None, Provider::OpenAi).is_none());
    }

    #[test]
    fn classify_terminal_openai_responses_api_terminal_event_types() {
        for et in [
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
            "response.failed",
            "response.cancelled",
            "response.incomplete",
        ] {
            let json = serde_json::json!({});
            assert!(
                matches!(
                    classify_terminal(&json, Some(et), Provider::OpenAi),
                    Some(TerminalScope::Stream)
                ),
                "{et} should be Stream"
            );
        }
    }

    #[test]
    fn classify_terminal_openai_responses_api_non_terminal_event_types() {
        for et in [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
        ] {
            let json = serde_json::json!({});
            assert!(
                classify_terminal(&json, Some(et), Provider::OpenAi).is_none(),
                "{et} should be None"
            );
        }
    }

    #[test]
    fn classify_terminal_openrouter_finish_reason_stream() {
        let json = serde_json::json!({
            "choices": [{"delta": {}, "finish_reason": "stop"}]
        });
        assert!(matches!(
            classify_terminal(&json, None, Provider::OpenRouter),
            Some(TerminalScope::Stream)
        ));
    }

    #[test]
    fn classify_terminal_gemini_with_finish_reason_is_stream() {
        let json = serde_json::json!({
            "candidates": [{"finishReason": "STOP"}]
        });
        assert!(matches!(
            classify_terminal(&json, None, Provider::Gemini),
            Some(TerminalScope::Stream)
        ));
    }

    #[test]
    fn classify_terminal_gemini_without_finish_reason_is_none() {
        let json = serde_json::json!({
            "candidates": [{"content": {"parts": [{"text": "hello"}]}}]
        });
        assert!(classify_terminal(&json, None, Provider::Gemini).is_none());
    }

    #[test]
    fn classify_terminal_cross_provider_guard_anthropic_shape_under_gemini_is_none() {
        // Anthropic message_stop JSON must not trigger a flush under Gemini.
        let json = serde_json::json!({"type": "message_stop"});
        assert!(classify_terminal(&json, None, Provider::Gemini).is_none());
    }

    #[test]
    fn process_one_frame_content_block_stop_flushes_accumulator_before_stop_frame() {
        // Accumulator for AnthropicDelta index 0 holds text "hello".
        // Feed a content_block_stop index-0 frame.
        // The synthetic content frame must appear BEFORE the stop frame in output_queue.
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(
            FieldKey::AnthropicDelta {
                delta_type: "text_delta".to_owned(),
                index: 0,
            },
            "hello".to_owned(),
        );
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let ctx = SseCtx {
            entries: &[],
            session_key: &session_key,
            provider: Provider::Anthropic,
            max_fake_len: 0,
        };
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        let mut deferred: VecDeque<Bytes> = VecDeque::new();
        let frame =
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n";
        process_one_frame(
            frame,
            &mut accumulators,
            &ctx,
            &mut output_queue,
            &mut deferred,
        )
        .unwrap();
        assert_eq!(
            output_queue.len(),
            2,
            "expected synthetic frame + stop frame"
        );
        let first = std::str::from_utf8(&output_queue[0]).unwrap();
        let second = std::str::from_utf8(&output_queue[1]).unwrap();
        assert!(
            first.contains("hello"),
            "first frame should contain accumulator text: {first}"
        );
        assert!(
            second.contains("content_block_stop"),
            "second frame should be the stop frame: {second}"
        );
    }

    #[test]
    fn gemini_deferred_passthrough_drains_before_finish_reason_frame() {
        // Accumulator holds Gemini text "hello"; deferred queue holds one
        // non-text frame (e.g. grounding metadata).  Feeding a finishReason
        // frame must produce: [text flush] [deferred] [finishReason], in that
        // order, so clients that finalize on finishReason receive tool args.
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(FieldKey::GeminiText { thought: false }, "hello".to_owned());
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let ctx = SseCtx {
            entries: &[],
            session_key: &session_key,
            provider: Provider::Gemini,
            max_fake_len: 0,
        };
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        let mut deferred: VecDeque<Bytes> = VecDeque::new();
        deferred.push_back(Bytes::from_static(b"data: {\"groundingMetadata\":{}}\n\n"));
        let frame = "data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n";
        process_one_frame(
            frame,
            &mut accumulators,
            &ctx,
            &mut output_queue,
            &mut deferred,
        )
        .unwrap();
        assert_eq!(
            output_queue.len(),
            3,
            "expected text flush + deferred + finishReason"
        );
        let first = std::str::from_utf8(&output_queue[0]).unwrap();
        let second = std::str::from_utf8(&output_queue[1]).unwrap();
        let third = std::str::from_utf8(&output_queue[2]).unwrap();
        assert!(
            first.contains("hello"),
            "first frame must be text flush: {first}"
        );
        assert!(
            second.contains("groundingMetadata"),
            "second frame must be deferred metadata: {second}"
        );
        assert!(
            third.contains("finishReason"),
            "third frame must be the finishReason terminal: {third}"
        );
        assert!(
            deferred.is_empty(),
            "deferred queue must be empty after drain"
        );
    }

    #[test]
    fn is_sse_detects_spaceless_data_prefix() {
        assert!(is_sse_first_chunk(b"data:{\"type\":\"ping\"}\n\n"));
    }

    #[test]
    fn process_one_frame_data_spaceless_prefix_is_parsed() {
        // A frame with spaceless "data:{...}" MUST be parsed as normal SSE, not
        // silently dropped. message_stop is a stream-scope terminal under Anthropic:
        // flush_all_accumulators (empty) is called, then the frame is forwarded.
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let ctx = SseCtx {
            entries: &[],
            session_key: &session_key,
            provider: Provider::Anthropic,
            max_fake_len: 0,
        };
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        let mut deferred: VecDeque<Bytes> = VecDeque::new();
        process_one_frame(
            "data:{\"type\":\"message_stop\"}\n\n",
            &mut accumulators,
            &ctx,
            &mut output_queue,
            &mut deferred,
        )
        .unwrap();
        assert_eq!(
            output_queue.len(),
            1,
            "spaceless data: frame must produce one passthrough frame"
        );
        let frame = std::str::from_utf8(&output_queue[0]).unwrap();
        assert!(
            frame.contains("message_stop"),
            "frame must contain parsed content: {frame}"
        );
    }

    #[test]
    fn process_one_frame_done_spaceless_flushes_under_openai() {
        // A spaceless "data:[DONE]" MUST flush all accumulators under OpenAI,
        // identical to "data: [DONE]" (with space).
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(FieldKey::OpenAiContent, "held_text".to_owned());
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let ctx = SseCtx {
            entries: &[],
            session_key: &session_key,
            provider: Provider::OpenAi,
            max_fake_len: 0,
        };
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        let mut deferred: VecDeque<Bytes> = VecDeque::new();
        process_one_frame(
            "data:[DONE]\n\n",
            &mut accumulators,
            &ctx,
            &mut output_queue,
            &mut deferred,
        )
        .unwrap();
        assert!(
            accumulators[&FieldKey::OpenAiContent].is_empty(),
            "accumulator must be flushed by spaceless [DONE]"
        );
        assert_eq!(
            output_queue.len(),
            2,
            "expected synthetic frame + [DONE] frame"
        );
        let frames: Vec<&str> = output_queue
            .iter()
            .map(|b| std::str::from_utf8(b).unwrap())
            .collect();
        assert!(
            frames.iter().any(|f| f.contains("held_text")),
            "synthetic frame must carry accumulated text"
        );
        assert!(
            frames.iter().any(|f| f.contains("[DONE]")),
            "[DONE] frame must be present in output"
        );
    }

    #[test]
    fn process_one_frame_done_under_gemini_does_not_flush() {
        // "[DONE]" under Gemini MUST NOT trigger a flush: Gemini terminates
        // with finishReason (JSON), not [DONE]. Guard is intentional.
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(
            FieldKey::GeminiText { thought: false },
            "held_content".to_owned(),
        );
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let ctx = SseCtx {
            entries: &[],
            session_key: &session_key,
            provider: Provider::Gemini,
            max_fake_len: 0,
        };
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        let mut deferred: VecDeque<Bytes> = VecDeque::new();
        process_one_frame(
            "data: [DONE]\n\n",
            &mut accumulators,
            &ctx,
            &mut output_queue,
            &mut deferred,
        )
        .unwrap();
        assert_eq!(
            accumulators[&FieldKey::GeminiText { thought: false }],
            "held_content",
            "[DONE] under Gemini MUST NOT flush the accumulator"
        );
        assert_eq!(output_queue.len(), 1, "[DONE] frame must pass through");
    }

    #[test]
    fn process_one_frame_done_spaceless_under_gemini_does_not_flush() {
        // A spaceless "data:[DONE]" under Gemini MUST NOT trigger a flush,
        // same as the spaced form. Gemini terminates with finishReason (JSON).
        let mut accumulators: BTreeMap<FieldKey, String> = BTreeMap::new();
        accumulators.insert(
            FieldKey::GeminiText { thought: false },
            "held_content".to_owned(),
        );
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let ctx = SseCtx {
            entries: &[],
            session_key: &session_key,
            provider: Provider::Gemini,
            max_fake_len: 0,
        };
        let mut output_queue: VecDeque<Bytes> = VecDeque::new();
        let mut deferred: VecDeque<Bytes> = VecDeque::new();
        process_one_frame(
            "data:[DONE]\n\n",
            &mut accumulators,
            &ctx,
            &mut output_queue,
            &mut deferred,
        )
        .unwrap();
        assert_eq!(
            accumulators[&FieldKey::GeminiText { thought: false }],
            "held_content",
            "spaceless [DONE] under Gemini MUST NOT flush the accumulator"
        );
        assert_eq!(output_queue.len(), 1, "[DONE] frame must pass through");
    }
}
