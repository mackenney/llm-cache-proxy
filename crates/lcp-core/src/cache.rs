use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::types::{CacheEntry, Exchange, ResponseChunk};

/// Thread-safe SQLite-backed cache for LLM exchanges.
#[derive(Clone)]
pub struct Cache {
    // Debug impl is manual below to avoid exposing Connection internals.
    inner: Arc<Mutex<Connection>>,
    ttl_seconds: u64,
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
        let conn = self.inner.lock().unwrap();
        let now = unix_now();

        let result = conn.query_row(
            "SELECT created_at, status, content_type, exchange_json FROM entries WHERE key = ?1",
            rusqlite::params![key],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, u16>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        );

        match result {
            Ok((created_at, status, content_type, json)) => {
                if self.ttl_seconds > 0 && now.saturating_sub(created_at as u64) > self.ttl_seconds
                {
                    return Ok(None);
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
                let chunks: Vec<ResponseChunk> =
                    serde_json::from_str(&json).context("deserializing cached chunks")?;
                Ok(Some(Exchange {
                    request: Default::default(),
                    status,
                    content_type,
                    chunks,
                }))
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
        let conn = self.inner.lock().unwrap();
        let now = unix_now() as i64;
        let chunks_json = serde_json::to_string(&exchange.chunks)?;
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
                chunks_json,
                provider,
                model,
                exchange.request.body.len() as i64,
                resp_bytes as i64,
            ],
        )?;
        Ok(())
    }

    /// Return aggregate statistics.
    pub fn stats(&self) -> Result<CacheStats> {
        let conn = self.inner.lock().unwrap();
        let stat_rows: Vec<(String, i64)> = {
            let mut stmt = conn.prepare("SELECT k, v FROM stats")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let stat_map: std::collections::HashMap<String, i64> = stat_rows.into_iter().collect();

        let (entries, resp_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(resp_bytes), 0) FROM entries",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

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
            cached_response_bytes: resp_bytes,
            by_model: by_model
                .into_iter()
                .map(|(m, c)| (m.unwrap_or_else(|| "unknown".into()), c))
                .collect(),
        })
    }

    /// Delete all entries.
    pub fn clear_entries(&self) -> Result<i64> {
        let conn = self.inner.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        conn.execute("DELETE FROM entries", [])?;
        Ok(n)
    }

    /// Reset all stat counters.
    pub fn clear_stats(&self) -> Result<()> {
        let conn = self.inner.lock().unwrap();
        conn.execute("DELETE FROM stats", [])?;
        Ok(())
    }

    pub fn list_entries(&self) -> Result<Vec<CacheEntry>> {
        let conn = self.inner.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT key, created_at, provider, model, hit_count, req_bytes, resp_bytes
             FROM entries ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CacheEntry {
                    key: r.get(0)?,
                    created_at: r.get(1)?,
                    provider: r.get(2)?,
                    model: r.get(3)?,
                    hit_count: r.get(4)?,
                    req_bytes: r.get(5)?,
                    resp_bytes: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
}

#[derive(Debug, serde::Serialize)]
pub struct CacheStats {
    pub hits: i64,
    pub misses: i64,
    pub bytes_served_from_cache: i64,
    pub entries: i64,
    pub cached_response_bytes: i64,
    pub by_model: std::collections::HashMap<String, i64>,
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
             created_at      INTEGER NOT NULL,
             status          INTEGER NOT NULL,
             content_type    TEXT NOT NULL,
             exchange_json   TEXT NOT NULL,
             provider        TEXT NOT NULL,
             model           TEXT,
             req_bytes       INTEGER NOT NULL DEFAULT 0,
             resp_bytes      INTEGER NOT NULL DEFAULT 0,
             hit_count       INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS stats (
             k TEXT PRIMARY KEY,
             v INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RequestRecord;

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
}
