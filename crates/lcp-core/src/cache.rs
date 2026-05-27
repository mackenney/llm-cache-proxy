use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::types::{CacheEntry, Exchange, FullEntry};

/// Thread-safe SQLite-backed cache for LLM exchanges.
#[derive(Clone)]
pub struct Cache {
    // Debug impl is manual below to avoid exposing Connection internals.
    inner: Arc<Mutex<Connection>>,
    ttl_seconds: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct CacheStats {
    pub hits: i64,
    pub misses: i64,
    pub bytes_served_from_cache: i64,
    pub entries: i64,
    pub by_model: std::collections::HashMap<String, i64>,
}

impl Cache {
    /// Open (or create) the cache database at `path`.
    ///
    /// `ttl_seconds = 0` means entries never expire.
    pub fn open(path: &PathBuf, ttl_seconds: u64) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening cache db at {}", path.display()))?;
        init_schema(&conn)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
            ttl_seconds,
        })
    }

    /// Look up a cached exchange by key. Returns `None` on miss or expired entry.
    pub fn get(&self, key: &str) -> Result<Option<Exchange>> {
        let conn = self.inner.lock().expect("cache mutex poisoned");

        let result = conn.query_row(
            "SELECT created_at, resp_bytes, exchange_json FROM entries WHERE key = ?1",
            rusqlite::params![key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );

        match result {
            Ok((created_at, resp_bytes, json)) => {
                if self.ttl_seconds > 0 {
                    let age = age_seconds(&created_at);
                    if age > self.ttl_seconds {
                        // Expired entries are treated as misses per spec.
                        conn.execute(
                            "INSERT INTO stats(k, v) VALUES('misses', 1)
                             ON CONFLICT(k) DO UPDATE SET v = v + 1",
                            [],
                        )?;
                        return Ok(None);
                    }
                }
                conn.execute(
                    "UPDATE entries SET hit_count = hit_count + 1 WHERE key = ?1",
                    rusqlite::params![key],
                )?;
                conn.execute(
                    "INSERT INTO stats(k, v) VALUES('hits', 1)
                     ON CONFLICT(k) DO UPDATE SET v = v + 1",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO stats(k, v) VALUES('bytes_served_from_cache', ?1)
                     ON CONFLICT(k) DO UPDATE SET v = v + ?1",
                    rusqlite::params![resp_bytes],
                )?;
                let exchange: Exchange =
                    serde_json::from_str(&json).context("deserializing cached exchange")?;
                Ok(Some(exchange))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO stats(k, v) VALUES('misses', 1)
                     ON CONFLICT(k) DO UPDATE SET v = v + 1",
                    [],
                )?;
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Store an exchange. Overwrites any existing entry for the same key.
    pub fn put(
        &self,
        key: &str,
        provider: &str,
        model: Option<&str>,
        exchange: &Exchange,
    ) -> Result<()> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let now = iso_now();
        let exchange_json = serde_json::to_string(exchange)?;
        let resp_bytes: usize = exchange.chunks.iter().map(|c| c.data.len()).sum();
        conn.execute(
            "INSERT OR REPLACE INTO entries
             (key, created_at, status, content_type, exchange_json, provider, model,
              req_bytes, resp_bytes, hit_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            rusqlite::params![
                key,
                now,
                exchange.status,
                exchange.content_type,
                exchange_json,
                provider,
                model,
                exchange.request.body.len() as i64,
                resp_bytes as i64,
            ],
        )?;
        Ok(())
    }

    /// Persist a (trace_id, cache_key) association.
    pub fn record_trace(&self, trace_id: &str, cache_key: &str) -> Result<()> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        conn.execute(
            "INSERT OR IGNORE INTO trace_entries(trace_id, cache_key) VALUES (?1, ?2)",
            rusqlite::params![trace_id, cache_key],
        )?;
        Ok(())
    }

    /// Return all cache entries associated with a trace, ordered by created_at.
    pub fn get_trace(&self, trace_id: &str) -> Result<Vec<CacheEntry>> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT e.key, e.created_at, e.provider, e.model, e.status,
                    e.hit_count, e.req_bytes, e.resp_bytes
             FROM entries e
             JOIN trace_entries t ON t.cache_key = e.key
             WHERE t.trace_id = ?1
             ORDER BY e.created_at ASC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![trace_id], |r| {
                Ok(CacheEntry {
                    key: r.get(0)?,
                    created_at: r.get(1)?,
                    provider: r.get(2)?,
                    model: r.get(3)?,
                    status: r.get(4)?,
                    hit_count: r.get(5)?,
                    req_bytes: r.get(6)?,
                    resp_bytes: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Return aggregate statistics.
    pub fn stats(&self) -> Result<CacheStats> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let stat_rows: Vec<(String, i64)> = {
            let mut stmt = conn.prepare("SELECT k, v FROM stats")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let stat_map: std::collections::HashMap<String, i64> = stat_rows.into_iter().collect();

        let entries: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;

        let by_model: Vec<(Option<String>, i64)> = {
            let mut stmt = conn.prepare("SELECT model, COUNT(*) FROM entries GROUP BY model")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };

        Ok(CacheStats {
            hits: *stat_map.get("hits").unwrap_or(&0),
            misses: *stat_map.get("misses").unwrap_or(&0),
            bytes_served_from_cache: *stat_map.get("bytes_served_from_cache").unwrap_or(&0),
            entries,
            by_model: by_model
                .into_iter()
                .map(|(m, c)| (m.unwrap_or_else(|| "unknown".into()), c))
                .collect(),
        })
    }

    /// Delete all cache and trace entries.
    pub fn clear_entries(&self) -> Result<i64> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        conn.execute("DELETE FROM trace_entries", [])?;
        conn.execute("DELETE FROM entries", [])?;
        Ok(n)
    }

    /// Reset all stat counters.
    pub fn clear_stats(&self) -> Result<()> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        conn.execute("DELETE FROM stats", [])?;
        Ok(())
    }

    pub fn list_entries(&self) -> Result<Vec<CacheEntry>> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT key, created_at, provider, model, status, hit_count, req_bytes, resp_bytes
             FROM entries ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CacheEntry {
                    key: r.get(0)?,
                    created_at: r.get(1)?,
                    provider: r.get(2)?,
                    model: r.get(3)?,
                    status: r.get(4)?,
                    hit_count: r.get(5)?,
                    req_bytes: r.get(6)?,
                    resp_bytes: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Fetch the full exchange for a key without incrementing any counter.
    /// Returns `None` if the key does not exist.
    pub fn inspect(&self, key: &str) -> Result<Option<FullEntry>> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let result = conn.query_row(
            "SELECT created_at, provider, model, status, content_type, \
             hit_count, req_bytes, resp_bytes, exchange_json \
             FROM entries WHERE key = ?1",
            rusqlite::params![key],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, u16>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, String>(8)?,
                ))
            },
        );
        match result {
            Ok((
                created_at,
                provider,
                model,
                status,
                content_type,
                hit_count,
                req_bytes,
                resp_bytes,
                json,
            )) => {
                let exchange: Exchange =
                    serde_json::from_str(&json).context("deserializing inspect exchange")?;
                Ok(Some(FullEntry {
                    key: key.to_owned(),
                    created_at,
                    provider,
                    model,
                    status,
                    content_type,
                    hit_count,
                    req_bytes,
                    resp_bytes,
                    request: exchange.request,
                    chunks: exchange.chunks,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Fetch full entries for all keys in a trace, ordered by `created_at`.
    /// No counters are modified.
    pub fn inspect_trace(&self, trace_id: &str) -> Result<Vec<FullEntry>> {
        let conn = self.inner.lock().expect("cache mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT e.key, e.created_at, e.provider, e.model, e.status, e.content_type, \
             e.hit_count, e.req_bytes, e.resp_bytes, e.exchange_json \
             FROM entries e \
             JOIN trace_entries t ON t.cache_key = e.key \
             WHERE t.trace_id = ?1 \
             ORDER BY e.created_at ASC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![trace_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, u16>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, String>(9)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    key,
                    created_at,
                    provider,
                    model,
                    status,
                    content_type,
                    hit_count,
                    req_bytes,
                    resp_bytes,
                    json,
                )| {
                    let exchange: Exchange = serde_json::from_str(&json)
                        .context("deserializing inspect_trace exchange")?;
                    Ok(FullEntry {
                        key,
                        created_at,
                        provider,
                        model,
                        status,
                        content_type,
                        hit_count,
                        req_bytes,
                        resp_bytes,
                        request: exchange.request,
                        chunks: exchange.chunks,
                    })
                },
            )
            .collect()
    }
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache")
            .field("ttl_seconds", &self.ttl_seconds)
            .finish_non_exhaustive()
    }
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (
             key             TEXT PRIMARY KEY,
             created_at      TEXT NOT NULL,
             status          INTEGER NOT NULL,
             content_type    TEXT NOT NULL,
             exchange_json   TEXT NOT NULL,
             provider        TEXT NOT NULL,
             model           TEXT,
             req_bytes       INTEGER NOT NULL DEFAULT 0,
             resp_bytes      INTEGER NOT NULL DEFAULT 0,
             hit_count       INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS trace_entries (
             trace_id  TEXT NOT NULL,
             cache_key TEXT NOT NULL,
             PRIMARY KEY (trace_id, cache_key)
         );
         CREATE TABLE IF NOT EXISTS stats (
             k TEXT PRIMARY KEY,
             v INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

fn iso_now() -> String {
    Utc::now().to_rfc3339()
}

/// Compute seconds elapsed since an ISO-8601 timestamp. Returns 0 on parse failure.
fn age_seconds(created_at: &str) -> u64 {
    DateTime::parse_from_rfc3339(created_at)
        .map(|dt| {
            Utc::now()
                .signed_duration_since(dt.with_timezone(&Utc))
                .num_seconds()
                .max(0) as u64
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RequestRecord, ResponseChunk};

    fn temp_cache() -> Cache {
        Cache::open(&":memory:".into(), 0).unwrap()
    }

    fn make_exchange(status: u16, chunks: Vec<(&str, u64)>) -> Exchange {
        Exchange {
            request: RequestRecord {
                method: "POST".into(),
                path: "/anthropic/v1/messages".into(),
                body: "{}".into(),
            },
            status,
            content_type: "text/event-stream".into(),
            chunks: chunks
                .into_iter()
                .map(|(data, offset_ms)| ResponseChunk {
                    offset_ms,
                    data: data.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn miss_on_empty_cache() {
        let cache = temp_cache();
        assert!(cache.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn put_then_get_roundtrip() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![("data: hello\n\n", 0), ("data: world\n\n", 50)]);
        cache
            .put("key1", "anthropic", Some("claude-opus-4"), &ex)
            .unwrap();

        let got = cache.get("key1").unwrap().unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.chunks.len(), 2);
        assert_eq!(got.chunks[0].data, "data: hello\n\n");
        assert_eq!(got.chunks[1].offset_ms, 50);
    }

    #[test]
    fn hit_count_increments() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![("data: ok\n\n", 0)]);
        cache.put("key2", "anthropic", None, &ex).unwrap();

        cache.get("key2").unwrap();
        cache.get("key2").unwrap();

        let entries = cache.list_entries().unwrap();
        assert_eq!(entries[0].hit_count, 2);
    }

    #[test]
    fn stats_track_hits_and_misses() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![("data: ok\n\n", 0)]);
        cache.put("key3", "openai", Some("gpt-4o"), &ex).unwrap();

        cache.get("key3").unwrap(); // hit
        cache.get("missing").unwrap(); // miss

        let stats = cache.stats().unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn clear_entries_removes_all() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![]);
        cache.put("k1", "anthropic", None, &ex).unwrap();
        cache.put("k2", "anthropic", None, &ex).unwrap();

        let n = cache.clear_entries().unwrap();
        assert_eq!(n, 2);
        assert!(cache.get("k1").unwrap().is_none());
    }

    #[test]
    fn record_and_get_trace() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![("data: ok\n\n", 0)]);
        cache
            .put("k1", "anthropic", Some("claude-opus-4"), &ex)
            .unwrap();
        cache.put("k2", "openai", Some("gpt-4o"), &ex).unwrap();

        cache.record_trace("trace-abc", "k1").unwrap();
        cache.record_trace("trace-abc", "k2").unwrap();

        let entries = cache.get_trace("trace-abc").unwrap();
        assert_eq!(entries.len(), 2);
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"k1"));
        assert!(keys.contains(&"k2"));
    }

    #[test]
    fn get_trace_unknown_returns_empty() {
        let cache = temp_cache();
        let entries = cache.get_trace("no-such-trace").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn bytes_served_from_cache_incremented() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![("hello", 0)]);
        cache.put("k1", "anthropic", None, &ex).unwrap();
        cache.get("k1").unwrap();
        let stats = cache.stats().unwrap();
        assert_eq!(stats.bytes_served_from_cache, 5);
    }

    #[test]
    fn inspect_returns_full_entry() {
        let cache = temp_cache();
        let ex = make_exchange(200, vec![("data: hi\n\n", 0)]);
        cache
            .put("k1", "anthropic", Some("claude-opus-4"), &ex)
            .unwrap();
        cache.get("k1").unwrap(); // hit to increment counters

        let entry = cache.inspect("k1").unwrap().unwrap();
        assert_eq!(entry.key, "k1");
        assert_eq!(entry.status, 200);
        assert_eq!(entry.provider, "anthropic");
        assert_eq!(entry.model.as_deref(), Some("claude-opus-4"));
        assert_eq!(entry.chunks.len(), 1);
        assert_eq!(entry.request.method, "POST");

        // inspect MUST NOT increment counters
        let stats_before = cache.stats().unwrap().hits;
        cache.inspect("k1").unwrap();
        assert_eq!(cache.stats().unwrap().hits, stats_before);
    }

    #[test]
    fn inspect_unknown_returns_none() {
        let cache = temp_cache();
        assert!(cache.inspect("no-such-key").unwrap().is_none());
    }

    #[test]
    fn inspect_trace_returns_full_entries() {
        let cache = temp_cache();
        let ex = make_exchange(201, vec![("chunk", 0)]);
        cache.put("k1", "openai", Some("gpt-4o"), &ex).unwrap();
        cache.record_trace("t1", "k1").unwrap();

        let entries = cache.inspect_trace("t1").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "k1");
        assert_eq!(entries[0].status, 201);
        assert_eq!(entries[0].chunks[0].data, "chunk");
    }
}
