# GUARDRAILS.md — FunnelSwift

**Rust Guardrails — Vibe Engineering Standard**

## Non-Negotiable
- No `unwrap()` or `expect()` in production code paths. Use `?`, `.ok_or_else()`, `.unwrap_or_default()`.
- Every `parse_str()` constant: define as `lazy_static` or `const UUID`.
- All DB errors must be handled — no silent failures.
- New routes must register in axum Router and include JWT middleware.
- `cargo clippy -- -D warnings` must pass before any task is declared done.
- All SQL queries use sqlx compile-time checking where possible.
- Build exclusively through `/opt/swift/funnelswift/deploy-from-vps.sh`.
- **Build Mutex:** ALL builds MUST acquire `/tmp/rust-build.lock` before spawning `cargo build`.
  Use `/opt/swift/scripts/swift-build-guard.sh <project>` to gate. One build at a time on this 2GB VPS — no exceptions.
  If lock is held, wait (poll every 2s) or fail gracefully. Never spawn a second parallel build.

## Verification Before Deploy
1. `cargo check`
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. `sqlx migrate run` (migrations sequential, never re-use numbers)
5. `curl localhost:8080/api/health`
