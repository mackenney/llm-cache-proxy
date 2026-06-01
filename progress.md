# Progress

## Status
In Progress

## Tasks
- [x] fc-cluster2: Fact-check 3 claims about proxy.rs caching behavior — DONE (artifacts/fc-cluster2.md)
- [x] fc-cluster1: Fact-check 3 claims about extensions.rs/proxy.rs — DONE (artifacts/fc-cluster1.md)

## Files Changed
- artifacts/fc-cluster2.md (created)
- artifacts/fc-cluster1.md (created)

## Notes

### fc-cluster2 (proxy.rs caching)
All 3 claims CONFIRMED.
- Claim 1: per-chunk from_utf8 at line 251 does split multibyte sequences; guard at line 284 skips cache
- Claim 2: do_cache (never mutated in loop) still passes after body_is_valid_utf8=false; chunks allocated but discarded
- Claim 3: exchange.clone() at line 300; original drops at line 323; only clone enters spawn_blocking

### fc-cluster1 (extensions.rs + proxy.rs header/stream/zip)
All 3 claims CONFIRMED.
- Claim 1: run_phase3 uses zip (extensions.rs:205); silently truncates if states.len() < extensions.len(); invariant held by calling convention only
- Claim 2: `stream` not explicitly dropped after loop; remains live through drop(tx) and all spawn_blocking calls (proxy.rs:239–324)
- Claim 3: header strip uses exhaustive matches! list; only x-lcp-bypass and x-lcp-trace stripped; other x-lcp-* headers forwarded (proxy.rs:160–178)
