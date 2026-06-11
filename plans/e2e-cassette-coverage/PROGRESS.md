# E2E Cassette Coverage Plan

## Status

Not started.

## Objective

Build a "cassettes" test tier: real provider API responses are captured once against live
upstream, stored as deterministic fixture files, and replayed by an extended `MockUpstream`
in subsequent test runs. This gives authentic wire-format coverage (real SSE frame shapes,
real event type sequences, real provider quirks) without requiring live API access in CI.

The E2E campaign (Jun 2026) found three real bugs by running against live APIs — all
caused by provider wire formats that diverged from what synthetic unit tests exercised.
Cassettes prevent that class of regression: new provider oddities are captured once,
committed, and covered forever.

## Companion files

| File | Purpose |
|---|---|
| `step-01-cassette-infrastructure.md` | Extend `MockUpstream` + design cassette format |
| `step-02-recording-sessions.md` | Live capture against every provider |
| `step-03-cassette-tests.md` | Write tests for each cassette + assertions |
| `step-04-coverage-gaps.md` | Error paths, concurrent requests, known-gap probes |

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|---|---|---|---|
| 1 | step-01 | No | — |
| 2 | step-02 | No (live API, sequential capture) | Wave 1 |
| 3 | step-03, step-04 | Yes (02 and 03 only after step-02 done) | Wave 2 |

## Steps

- [ ] [step-01-cassette-infrastructure](./step-01-cassette-infrastructure.md) — cassette TOML format + `MockUpstream::Cassette` + `lcp-record` capture binary
- [ ] [step-02-recording-sessions](./step-02-recording-sessions.md) — live capture of 30+ scenarios across all providers
- [ ] [step-03-cassette-tests](./step-03-cassette-tests.md) — integration-tier test for every captured cassette
- [ ] [step-04-coverage-gaps](./step-04-coverage-gaps.md) — error paths, concurrent, admin API, Gemini metadata gap probe
