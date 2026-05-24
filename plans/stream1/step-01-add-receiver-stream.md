# Step 01: Add ReceiverStream Adapter and Imports

## Objective
Add a minimal `ReceiverStream<T>` struct that implements `futures_util::Stream` for `tokio::sync::mpsc::Receiver<T>`. This avoids adding the `tokio-stream` dependency for 15 lines of trivial code.

## Why
- The spawn+channel pattern in step-03 needs to convert an `mpsc::Receiver` into a `Stream` for `Body::from_stream()`.
- This is a prerequisite for step-03 but has no dependencies itself.
- Implementing inline keeps the crate lean.

## File to Modify
`crates/lcp-server/src/proxy.rs`

## Changes

### 1. Add imports at the top of the file

After line 9 (`use futures_util::StreamExt;`), add:

```rust
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
```

Note: `futures_util::Stream` is already available via the `StreamExt` import (it's a re-export), but we need the trait explicitly for the impl. Add it as:

```rust
use futures_util::{Stream, StreamExt};
```

(This replaces the existing `use futures_util::StreamExt;` line.)

### 2. Add ReceiverStream struct

Add this code block right after the imports section, before the `AppState` struct (before line 18):

```rust
/// Adapts `mpsc::Receiver<T>` to `Stream<Item = T>` for use with `Body::from_stream()`.
/// Avoids adding tokio-stream dependency for this single use case.
struct ReceiverStream<T> {
    rx: mpsc::Receiver<T>,
}

impl<T> ReceiverStream<T> {
    fn new(rx: mpsc::Receiver<T>) -> Self {
        Self { rx }
    }
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}
```

## Expected Result After This Step

The file should have:
- Lines 1-9: existing imports
- Line 9: `use futures_util::{Stream, StreamExt};` (modified)
- Lines 10-12: new imports (`std::pin::Pin`, `std::task::{Context, Poll}`, `tokio::sync::mpsc`)
- Lines 14-30 (approx): `ReceiverStream<T>` struct and impls
- Remaining code unchanged

## Acceptance Criteria

1. **Compilation succeeds:**
   ```bash
   cargo build -p lcp-server 2>&1 | head -20
   ```
   Expected: no errors (warnings about unused code are acceptable at this step)

2. **Existing tests pass:**
   ```bash
   cargo nextest run -p lcp-server 2>&1 | tail -10
   ```
   Expected: all tests pass

3. **Clippy clean (with allowance for unused):**
   ```bash
   cargo clippy -p lcp-server -- -A dead_code 2>&1 | grep -E "^error" || echo "No errors"
   ```
   Expected: "No errors"

## Commit Message
```
step-01: add ReceiverStream adapter
```
