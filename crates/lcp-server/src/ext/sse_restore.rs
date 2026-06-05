//! SSE-aware restore helpers for `DoppelExt`.
//!
//! These functions are the building blocks for semantic-level secret
//! restoration in `text/event-stream` responses, where raw-byte Aho-Corasick
//! fails because the fake key never appears contiguously in the byte stream.

use std::collections::{HashMap, VecDeque};
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
// Variants not yet extracted are defined here for future steps; suppress dead_code
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    GeminiCodeExecOutput,
    GeminiFuncCallArg { arg_key: String },
}

/// A content-bearing field extracted from one SSE event.
struct ExtractedField {
    key: FieldKey,
    text: String,
    /// Write-back location: the information needed to set the restored text
    /// back into the correct JSON path. This is separate from `key` because
    /// the accumulation identity may differ from the write-back path (e.g.,
    /// Gemini parts keyed by `thought` but written back by `part_index`).
    write_back: WriteBackInfo,
}

/// Information needed to write a restored value back into its JSON location.
// Variants not yet used are defined here for future steps; suppress dead_code
#[allow(dead_code)]
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
    /// OpenAI: `json["choices"][0]["delta"]["function_call"]["arguments"]`
    OpenAiFunctionCallArgs,
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

/// Records what a single frame contributed to each accumulation buffer.
struct FrameFieldContribution {
    key: FieldKey,
    byte_len: usize,
    write_back: WriteBackInfo,
}

/// Returns `true` if the first bytes of a response chunk look like an SSE stream.
///
/// Detects both `data: ` and `event: ` line starters. Providers that emit only
/// `data:` lines (OpenAI, OpenRouter, Gemini) start with `data: `; Anthropic's
/// real API prefixes each data line with a named `event:` line, so the stream
/// starts with `event: ` instead. Non-SSE JSON responses begin with `{`.
pub fn is_sse_first_chunk(bytes: &[u8]) -> bool {
    bytes.starts_with(b"data: ") || bytes.starts_with(b"event: ")
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
                            write_back: WriteBackInfo::AnthropicDelta {
                                field_name: "text".into(),
                            },
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
                            write_back: WriteBackInfo::AnthropicDelta {
                                field_name: "thinking".into(),
                            },
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
                            write_back: WriteBackInfo::AnthropicDelta {
                                field_name: "partial_json".into(),
                            },
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
                                        event_type: "output_text".into(),
                                    },
                                    text: text.to_owned(),
                                    write_back: WriteBackInfo::ResponsesApiDelta,
                                }]
                            } else {
                                vec![]
                            }
                        }
                        "response.output_text.done" => {
                            if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                                vec![ExtractedField {
                                    key: FieldKey::ResponsesApiDone {
                                        event_type: "output_text".into(),
                                    },
                                    text: text.to_owned(),
                                    write_back: WriteBackInfo::ResponsesApiDone,
                                }]
                            } else {
                                vec![]
                            }
                        }
                        "response.reasoning_summary_text.delta" => {
                            if let Some(text) = json.get("delta").and_then(|v| v.as_str()) {
                                vec![ExtractedField {
                                    key: FieldKey::ResponsesApiDelta {
                                        event_type: "reasoning_summary_text".into(),
                                    },
                                    text: text.to_owned(),
                                    write_back: WriteBackInfo::ResponsesApiDelta,
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
                fields.push(ExtractedField {
                    key: FieldKey::OpenAiContent,
                    text: text.to_owned(),
                    write_back: WriteBackInfo::OpenAiContent,
                });
            }

            if let Some(text) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                fields.push(ExtractedField {
                    key: FieldKey::OpenAiReasoning,
                    text: text.to_owned(),
                    write_back: WriteBackInfo::OpenAiReasoning,
                });
            }

            if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for (array_pos, tc) in tool_calls.iter().enumerate() {
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
                                write_back: WriteBackInfo::OpenAiToolCall { array_pos },
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
                        write_back: WriteBackInfo::OpenAiFunctionCallArgs,
                    });
                }
            }

            if let Some(text) = delta.get("refusal").and_then(|v| v.as_str()) {
                fields.push(ExtractedField {
                    key: FieldKey::OpenAiRefusal,
                    text: text.to_owned(),
                    write_back: WriteBackInfo::OpenAiRefusal,
                });
            }

            fields
        }
        Provider::Gemini => {
            let Some(text) = json
                .pointer("/candidates/0/content/parts/0/text")
                .and_then(Value::as_str)
            else {
                return vec![];
            };
            vec![ExtractedField {
                key: FieldKey::GeminiText { thought: false },
                text: text.to_owned(),
                write_back: WriteBackInfo::GeminiPartText { part_index: 0 },
            }]
        }
    }
}

/// Writes restored values back into their JSON locations.
///
/// Returns `Err` if the target path doesn't exist in the JSON (signals
/// extract/apply mismatch).
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
            WriteBackInfo::OpenAiFunctionCallArgs => {
                let target = json
                    .get_mut("choices")
                    .and_then(|c| c.get_mut(0))
                    .and_then(|c| c.get_mut("delta"))
                    .and_then(|d| d.get_mut("function_call"))
                    .and_then(|fc| fc.get_mut("arguments"));
                match target {
                    Some(v) => *v = Value::String(text.clone()),
                    None => return Err(
                        "apply_restored_fields: choices[0].delta.function_call.arguments not found"
                            .to_owned(),
                    ),
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

/// Response stream wrapper that auto-detects SSE and applies the correct
/// restore strategy.
///
/// For SSE (`text/event-stream`): buffers all frames, accumulates provider text
/// fields, runs `restore_stream` on the concatenated text, redistributes the
/// restored text back into the original SSE frames, then drains the queue.
///
/// For non-SSE: buffers all bytes, runs `restore_stream` on the full buffer,
/// then drains.
///
/// Trade-off: complete buffering is required because a fake may span any number
/// of SSE events and its boundaries are unknown until the stream ends.
pub struct SseRestoreStream {
    state: SseState,
}

enum SseState {
    /// Collecting all raw bytes from the inner stream.
    Collecting {
        inner: ResponseStream,
        raw_buf: Vec<u8>,
        is_sse: Option<bool>,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
    },
    /// Processing the fully-collected buffer (async).
    Processing(BoxFuture<'static, Result<VecDeque<Bytes>, io::Error>>),
    /// Draining the processed output queue.
    Emitting(VecDeque<Bytes>),
    /// Terminal: stream exhausted.
    Done,
}

impl SseRestoreStream {
    /// Wrap `stream` in SSE-aware restoring. `provider` is used to locate the
    /// text field in SSE events.
    pub fn new(
        stream: ResponseStream,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
    ) -> Self {
        Self {
            state: SseState::Collecting {
                inner: stream,
                raw_buf: Vec::new(),
                is_sse: None,
                entries,
                session_key,
                provider,
            },
        }
    }
}

impl Stream for SseRestoreStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match std::mem::replace(&mut self.state, SseState::Done) {
                SseState::Collecting {
                    mut inner,
                    mut raw_buf,
                    mut is_sse,
                    entries,
                    session_key,
                    provider,
                } => {
                    match inner.as_mut().poll_next(cx) {
                        Poll::Ready(Some(Ok(chunk))) => {
                            if is_sse.is_none() && !chunk.is_empty() {
                                is_sse = Some(is_sse_first_chunk(&chunk));
                            }
                            raw_buf.extend_from_slice(&chunk);
                            // Restore state and loop — drain inner before processing.
                            self.state = SseState::Collecting {
                                inner,
                                raw_buf,
                                is_sse,
                                entries,
                                session_key,
                                provider,
                            };
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => {
                            // state is already Done
                            return Poll::Ready(Some(Err(e)));
                        }
                        Poll::Ready(None) => {
                            let is_sse_flag = is_sse.unwrap_or(false);
                            let fut = process_buffer(
                                raw_buf,
                                entries,
                                session_key,
                                provider,
                                is_sse_flag,
                            )
                            .boxed();
                            self.state = SseState::Processing(fut);
                            continue;
                        }
                        Poll::Pending => {
                            // Restore state — more data coming.
                            self.state = SseState::Collecting {
                                inner,
                                raw_buf,
                                is_sse,
                                entries,
                                session_key,
                                provider,
                            };
                            return Poll::Pending;
                        }
                    }
                }
                SseState::Processing(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(queue)) => {
                        self.state = SseState::Emitting(queue);
                        continue;
                    }
                    Poll::Ready(Err(e)) => {
                        // state is already Done
                        return Poll::Ready(Some(Err(e)));
                    }
                    Poll::Pending => {
                        self.state = SseState::Processing(fut);
                        return Poll::Pending;
                    }
                },
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

async fn process_buffer(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
    provider: Provider,
    is_sse: bool,
) -> Result<VecDeque<Bytes>, io::Error> {
    // Nothing to process for an empty buffer.
    if raw.is_empty() {
        return Ok(VecDeque::new());
    }
    if !is_sse {
        // Non-SSE: run restore_stream on the raw bytes as a single-chunk in-memory stream.
        return restore_non_sse(raw, entries, session_key).await;
    }
    restore_sse(raw, entries, session_key, provider).await
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

async fn restore_sse(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
    provider: Provider,
) -> Result<VecDeque<Bytes>, io::Error> {
    // Step 1: Split into frames. SSE frames are separated by "\n\n".
    // Include the "\n\n" terminator in each frame for round-trip fidelity.
    let raw_str = String::from_utf8(raw)
        .map_err(|e| io::Error::other(format!("SSE response contained non-UTF8 bytes: {e}")))?;
    // Normalize \r\n → \n before splitting so both line-ending styles are handled.
    // The WHATWG EventSource spec permits \r, \n, or \r\n line endings; providers
    // that use \r\n\r\n frame separators would otherwise produce no split boundaries.
    let normalized = raw_str.replace("\r\n", "\n");
    let frames: Vec<&str> = normalized.split_inclusive("\n\n").collect();

    struct ParsedFrame {
        fields: Vec<FrameFieldContribution>,
        json: Option<serde_json::Value>,
        raw: String,
    }

    let mut parsed: Vec<ParsedFrame> = Vec::with_capacity(frames.len());
    let mut accumulators: HashMap<FieldKey, String> = HashMap::new();

    for frame in &frames {
        let event_type = frame.lines().find_map(|l| l.strip_prefix("event: "));
        let data_content = frame.lines().find_map(|l| l.strip_prefix("data: "));

        let Some(data_str) = data_content else {
            // No data line (comment-only frame or keep-alive). Pass through.
            parsed.push(ParsedFrame {
                fields: vec![],
                json: None,
                raw: frame.to_string(),
            });
            continue;
        };

        match serde_json::from_str::<serde_json::Value>(data_str) {
            Err(_) => {
                // Not JSON (e.g., "data: [DONE]"). Pass through.
                parsed.push(ParsedFrame {
                    fields: vec![],
                    json: None,
                    raw: frame.to_string(),
                });
            }
            Ok(json) => {
                let extracted = extract_fields(&json, provider, event_type);
                let mut contribs: Vec<FrameFieldContribution> = Vec::with_capacity(extracted.len());
                for field in extracted {
                    let byte_len = field.text.len();
                    accumulators
                        .entry(field.key.clone())
                        .or_default()
                        .push_str(&field.text);
                    contribs.push(FrameFieldContribution {
                        key: field.key,
                        byte_len,
                        write_back: field.write_back,
                    });
                }
                parsed.push(ParsedFrame {
                    fields: contribs,
                    json: Some(json),
                    raw: frame.to_string(),
                });
            }
        }
    }

    // If no text events found, nothing to restore — pass frames through unchanged.
    if accumulators.is_empty() {
        let mut queue = VecDeque::new();
        for f in parsed {
            queue.push_back(Bytes::from(f.raw.into_bytes()));
        }
        return Ok(queue);
    }

    // Run sync doppel::restore per accumulation buffer.
    let mut restored_buffers: HashMap<FieldKey, String> = HashMap::new();
    for (key, buf) in &accumulators {
        let mut input = std::io::Cursor::new(buf.as_bytes());
        let mut output = Vec::new();
        doppel_restore(&mut input, &mut output, &entries, &session_key)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let restored = String::from_utf8(output)
            .map_err(|e| io::Error::other(format!("restore produced non-UTF8: {e}")))?;
        restored_buffers.insert(key.clone(), restored);
    }

    debug_assert!(
        accumulators
            .iter()
            .all(|(k, buf)| buf.len() == restored_buffers[k].len()),
        "structural-equivalence invariant violated: accumulated and restored buffer lengths differ"
    );
    // Verify that each key's byte_len contributions sum to the accumulated buffer length.
    debug_assert!(
        accumulators.keys().all(|key| {
            let sum: usize = parsed
                .iter()
                .flat_map(|f| f.fields.iter())
                .filter(|c| &c.key == key)
                .map(|c| c.byte_len)
                .sum();
            sum == accumulators[key].len()
        }),
        "per-key byte_len sum must equal accumulated buffer length"
    );

    // Redistribute restored text back into each frame.
    // First frame for each key receives the full restored text; subsequent frames
    // for the same key receive an empty string. This preserves the invariant that
    // all restored content appears contiguously in the output while supporting
    // multiple independent field keys.
    let mut emitted: HashMap<FieldKey, bool> = HashMap::new();
    let mut queue = VecDeque::new();

    for mut frame in parsed {
        if frame.fields.is_empty() {
            queue.push_back(Bytes::from(frame.raw.into_bytes()));
            continue;
        }
        let Some(mut json) = frame.json.take() else {
            queue.push_back(Bytes::from(frame.raw.into_bytes()));
            continue;
        };

        let mut write_pairs: Vec<(WriteBackInfo, String)> = Vec::new();
        for contrib in &frame.fields {
            let already_emitted = emitted.entry(contrib.key.clone()).or_insert(false);
            let text = if !*already_emitted {
                *already_emitted = true;
                restored_buffers[&contrib.key].clone()
            } else {
                String::new()
            };
            write_pairs.push((contrib.write_back.clone(), text));
        }
        apply_restored_fields(&mut json, &write_pairs).map_err(io::Error::other)?;

        // Reconstruct frame preserving non-data lines
        let prefix_lines: String = frame
            .raw
            .lines()
            .filter(|l| !l.starts_with("data:") && !l.is_empty())
            .map(|l| format!("{l}\n"))
            .collect();
        let reconstructed = format!(
            "{}data: {}\n\n",
            prefix_lines,
            serde_json::to_string(&json).map_err(|e| io::Error::other(e.to_string()))?
        );
        queue.push_back(Bytes::from(reconstructed.into_bytes()));
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
                event_type: "output_text".into()
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
                event_type: "output_text".into()
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
                event_type: "reasoning_summary_text".into()
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
}
