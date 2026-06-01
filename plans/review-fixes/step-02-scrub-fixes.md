# Step 02 — scrub.rs fixes (I3, S2, S3)

**File:** `crates/lcp-server/src/ext/scrub.rs` (+ `Cargo.toml` files for S2)
**Wave:** 0 (no deps)

---

## I3 — error_stream when entries present but session_key missing (line 140-142)

### Root cause

If `SensitiveState` has `entries` but no `session_key`, the current code returns
`stream` (silent passthrough). This is an internal-error state: secrets were scrubbed
in Phase 2 but Phase 3 cannot unscrub. Fakes reach the client and get cached.

The first guard (line 136-138, `entries` absent) is correct: no entries → no secrets
→ passthrough is safe.

### Change

**Before (lines 140-141):**
```rust
        let Some(key_hex) = state.get("session_key").map(str::to_owned) else {
            return stream;
        };
```

**After:**
```rust
        let Some(key_hex) = state.get("session_key").map(str::to_owned) else {
            return error_stream("SensitiveState has entries but no session_key".into());
        };
```

No other changes to the first guard — it stays as passthrough.

---

## S2 — Replace custom hex encode/decode with `hex` crate (lines 166-195)

### Background

`hex` 0.4.3 is already a transitive dep via `its-classified` (confirmed in Cargo.lock).
Custom `encode_key_hex`, `decode_key_hex`, and `hex_nibble` (~30 lines) can be replaced.

### Changes

**1. Root `Cargo.toml` — add workspace dep (after line 61):**

Add to `[workspace.dependencies]`:
```toml
hex = "0.4"
```

**2. `crates/lcp-server/Cargo.toml` — add dep (after `its-classified` line 22):**

Add to `[dependencies]`:
```toml
hex = { workspace = true }
```

**3. `crates/lcp-server/src/ext/scrub.rs` — replace functions:**

Add import at top:
```rust
use hex;
```

Replace usage at line ~118 (in `on_request_body`, the encode call). Find the call site:
```rust
encode_key_hex(&result.session_key.as_bytes())
```
Replace with:
```rust
hex::encode(result.session_key.as_bytes())
```

Replace usage at line 150 (in `on_response_stream`, the decode call):
```rust
let key_bytes = match decode_key_hex(&key_hex) {
```
Replace with:
```rust
let key_bytes: [u8; 32] = match hex::decode(&key_hex)
    .map_err(|e| e.to_string())
    .and_then(|v| v.try_into().map_err(|_| format!("expected 32 bytes, got wrong length")))
{
```

**4. Delete the three private functions** `encode_key_hex` (lines 165-172), `decode_key_hex` (lines 174-186), `hex_nibble` (lines 188-195).

**5. Delete the `key_hex_round_trip` test** at lines 313-323 — the `hex` crate is well-tested externally.

---

## S3 — Update module doc (lines 15-16)

### Change

**Before (lines 15-16):**
```rust
//! - **Phase 3:** `its_classified::unscrub_stream` restores originals in every
//!   response chunk.  The cache stores the restored content; the wire carried
//!   only fakes.
```

**After:**
```rust
//! - **Phase 3:** For SSE responses, `SseUnscrubStream` performs semantic-level
//!   unscrubbing (accumulate text across events, unscrub, redistribute). For
//!   non-SSE responses, `its_classified::unscrub_stream` performs raw-byte
//!   Aho-Corasick restoration. The cache stores restored content; the wire
//!   carried only fakes.
```

---

## Acceptance

```bash
cargo nextest run -p lcp-server --lib -- scrub
cargo nextest run --test integration -- scrub
cargo clippy --workspace --all-targets -- -D warnings
```

All existing scrub tests pass. Deleted `key_hex_round_trip` is acceptable since `hex` crate is proven.
