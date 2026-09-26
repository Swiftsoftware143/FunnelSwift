use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::error::Result;

/// Boot-time verdict on the schema this binary ships vs the schema the database carries
/// (kanban t_c3823fd1).
///
/// The migration error arm is still non-fatal: schema changes are also applied out-of-band, and a
/// ledger-shape complaint (an already-applied file whose checksum changed) would otherwise turn a
/// perfectly served app into an outage — measured on CoreSwift-CRM, where a duplicate `080_*.sql`
/// made EVERY boot raise `VersionMismatch(80)` while the schema was fully present.
///
/// What changed is that the failure is now RECORDED and QUERYABLE instead of being one line in
/// `docker logs`: `GET /api/health` reports the schema component, and answers **503** whenever the
/// database's ledger is short of the versions this binary ships — i.e. whenever the service is
/// serving traffic on a schema it never applied. The fleet uptime watchdog probes exactly that
/// route (scripts/fleet-uptime-watch.sh: `funnelswift:8080:/api/v1/health`, non-200 => down), so
/// the degraded boot now raises an alert without a human reading the container log.
#[derive(Clone, Debug)]
pub struct SchemaStatus {
    inner: Arc<RwLock<SchemaVerdict>>,
}

/// One boot-time migration verdict. Everything here is a fact about the run, never a guess.
#[derive(Clone, Debug, Default)]
pub struct SchemaVerdict {
    /// `migrate()` has produced a verdict. Always true by the time the server binds, since
    /// `main` awaits `Database::migrate()` before serving; a `false` here is an unreachable state
    /// reported as `pending` rather than as a failure.
    pub decided: bool,
    /// `sqlx::migrate!().run()` returned `Ok`.
    pub applied: bool,
    /// Highest version compiled into THIS binary (`sqlx::migrate!` embeds the `.sql` files at
    /// build time — the runtime never reads `./migrations`, so the binary is the source of truth
    /// for what it expects).
    pub expected_version: i64,
    /// Highest version recorded in `_sqlx_migrations`. `None` = the ledger itself could not be
    /// read (missing table or an unreachable pool), which is treated as "short" on purpose.
    pub ledger_version: Option<i64>,
    /// Stable machine token for the failure shape, safe to publish on a public route (it is a
    /// fixed vocabulary, not SQL text): `version_mismatch`, `version_missing`, `dirty`,
    /// `execute`, ... The verbatim message stays in the log line.
    pub error_class: Option<&'static str>,
    /// The migrator's own message, single-lined and capped, for the log/diagnostics.
    pub error: Option<String>,
}

impl SchemaStatus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SchemaVerdict::default())),
        }
    }

    pub fn snapshot(&self) -> SchemaVerdict {
        self.inner
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| SchemaVerdict::default())
    }

    fn record(&self, verdict: SchemaVerdict) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = verdict;
        }
    }

    /// The service is serving on a schema it never applied: the migration run failed AND the
    /// database does not carry every version this binary ships.
    pub fn schema_missing(&self) -> bool {
        let v = self.snapshot();
        if !v.decided || v.applied {
            return false;
        }
        // A ledger-shape complaint is NOT "the schema is absent": the versions are recorded, the
        // files just do not match the ledger anymore (a modified or foreign file). Measured on
        // CoreSwift-CRM, a duplicate `080` raised `VersionMismatch(80)` on every boot for hours
        // while the app kept serving the correct schema; fail-closing on that shape would have
        // been an outage. It is still reported (health + WARN), just not fatal.
        let ledger_shape = matches!(
            v.error_class,
            Some("version_mismatch")
                | Some("version_missing")
                | Some("version_not_present")
                | Some("version_too_old")
                | Some("version_too_new")
        );
        if ledger_shape && v.ledger_version.is_some_and(|l| l >= v.expected_version) {
            return false;
        }
        true
    }
}

impl Default for SchemaStatus {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Database {
    pool: Pool<Postgres>,
    schema: SchemaStatus,
}

impl Database {
    pub async fn new() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

        let pool = PgPoolOptions::new()
            .max_connections(20)
            .min_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .connect(&database_url)
            .await?;

        tracing::info!("Database connection pool established");

        Ok(Self {
            pool,
            schema: SchemaStatus::new(),
        })
    }

    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// The boot-time migration verdict, read by `GET /api/health` (kanban t_c3823fd1).
    pub fn schema(&self) -> &SchemaStatus {
        &self.schema
    }

    pub async fn migrate(&self) -> Result<()> {
        tracing::info!("Running database migrations...");
        // `sqlx::migrate!` EMBEDS the migrations at build time (the generated code is
        // `include_str!` per file — sqlx-macros-core-0.8.6/src/migrate.rs:63), so the runtime never
        // reads `./migrations` and the compiled-in set is exactly the schema this binary expects.
        let migrator = sqlx::migrate!("./migrations");
        let expected_version = migrator.iter().map(|m| m.version).max().unwrap_or(0);

        match migrator.run(&self.pool).await {
            Ok(_) => {
                tracing::info!(
                    "Database migrations applied successfully (ledger version {expected_version})"
                );
                self.schema.record(SchemaVerdict {
                    decided: true,
                    applied: true,
                    expected_version,
                    ledger_version: Some(expected_version),
                    error_class: None,
                    error: None,
                });
            }
            Err(e) => {
                let ledger_version = self.ledger_version().await;
                let error_class = classify_migration_error(&e);
                // The same predicate `GET /api/health` uses, computed here so the log line and the
                // route can never disagree.
                self.schema.record(SchemaVerdict {
                    decided: true,
                    applied: false,
                    expected_version,
                    ledger_version,
                    error_class: Some(error_class),
                    error: Some(one_line(&e.to_string())),
                });
                let verdict = self.schema.snapshot();
                let ledger = ledger_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unreadable".to_string());
                if self.schema.schema_missing() {
                    // The loud arm: the app is up but serving on a schema it never applied.
                    tracing::error!(
                        "SCHEMA-GATE FAILED {error_class}: migrations did not apply and the \
                         database ledger ({ledger}) is short of the schema this binary ships \
                         ({expected_version}) — serving DEGRADED, GET /api/health answers 503 \
                         until this is resolved: {e}"
                    );
                } else {
                    // The benign arm: the ledger carries everything this binary ships, so the
                    // schema is present and the complaint is about the files, not the schema.
                    tracing::warn!(
                        "SCHEMA-GATE: migration run failed ({error_class}) but the ledger \
                         ({ledger}) is complete for this binary ({expected_version}) — the schema \
                         this binary expects is present, continuing to serve; GET /api/health \
                         reports schema.status={} : {e}",
                        health_schema_status(&verdict)
                    );
                }
            }
        }
        Ok(())
    }

    /// Highest version in `_sqlx_migrations`. `None` = the ledger could not be read at all
    /// (no table yet, or the pool is unusable) — deliberately treated as "short" by the gate.
    async fn ledger_version(&self) -> Option<i64> {
        sqlx::query_scalar::<_, Option<i64>>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&self.pool)
            .await
            .ok()
            .flatten()
    }
}

/// The single source of truth for the `schema.status` token in `GET /api/health` and in the
/// SCHEMA-GATE log line.
pub fn health_schema_status(v: &SchemaVerdict) -> &'static str {
    if !v.decided {
        "pending"
    } else if v.applied {
        "applied"
    } else if v
        .error_class
        .is_some_and(|c| matches!(c, "version_mismatch" | "version_missing"))
        && v.ledger_version.is_some_and(|l| l >= v.expected_version)
    {
        "error-ledger-complete"
    } else {
        "missing"
    }
}

/// Map a `sqlx::MigrateError` onto a small, publishable vocabulary. `MigrateError` is
/// `#[non_exhaustive]`, so the catch-all is required by the compiler and not a guess.
fn classify_migration_error(e: &sqlx::migrate::MigrateError) -> &'static str {
    use sqlx::migrate::MigrateError as M;
    match e {
        M::VersionMismatch(_) => "version_mismatch",
        M::VersionMissing(_) => "version_missing",
        M::VersionNotPresent(_) => "version_not_present",
        M::VersionTooOld(_, _) => "version_too_old",
        M::VersionTooNew(_, _) => "version_too_new",
        M::Dirty(_) => "dirty",
        M::Execute(_) => "execute",
        M::Source(_) => "source",
        _ => "error",
    }
}

/// One line, no ANSI, capped — an error message must not be able to flood a log line or a
/// response body.
fn one_line(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let squashed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if squashed.chars().count() > 400 {
        squashed.chars().take(400).collect::<String>() + "..."
    } else {
        squashed
    }
}
