//! Guard: `tenant_id` is decoded as an **Option** everywhere a row can carry a NULL.
//!
//! Measured live 2026-11-24 (kanban t_b7c13d76): two requests in one morning killed their tokio
//! worker with
//!
//! ```text
//! panicked at sqlx-core-0.7.4/src/row.rs:72:37:
//! called `Result::unwrap()` on an `Err` value:
//!   ColumnDecode { index: "\"tenant_id\"", source: UnexpectedNullError }
//! ```
//!
//! `Row::get` is `try_get(..).unwrap()`, so a NULL in a column decoded as a bare `Uuid` is not a
//! 500 — it is a panic that drops the connection with no status line. The source was
//! `invoice_handler::row_to_json`: an erasure DETACHES a workspace's invoices (keeps them for tax
//! purposes, `tenant_id -> NULL`, `src/auth/handlers.rs`), and the operator's invoice book then died
//! the moment it was opened. `tenant_id` is NULLABLE on EIGHT tables in this schema — invoices,
//! product_categories, payment_webhook_events, affiliate_plan_movements, affiliate_products,
//! affiliate_selections, affiliate_users, support_requests — so the decode must be an `Option` on any
//! row that can come from one of them.
//!
//! This test runs in `cargo test` with no database and no network (this fleet's source-level census
//! style): it walks `src/` and fails when a LATER change reintroduces the panicking form. The four
//! files in [`NON_NULL_COLUMN_READERS`] are the only places allowed to decode `tenant_id` into a bare
//! `Uuid`, and each one reads a column the schema declares `NOT NULL`.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// The fix itself, read at COMPILE time so the test can never look at a stale copy.
    const INVOICE_HANDLER: &str = include_str!("handlers/invoice_handler.rs");
    const CATEGORY_HANDLER: &str = include_str!("handlers/product_category_handler.rs");
    const AFFILIATE_PRODUCT_HANDLER: &str = include_str!("handlers/affiliate_product_handler.rs");

    /// Files that may decode `tenant_id` into a bare `Uuid` with `Row::get` (which unwraps), because
    /// the column they read is `NOT NULL` in `information_schema` — a NULL cannot reach the decode.
    /// Every other reader must say `Option<Uuid>` (or use `try_get`, which cannot panic).
    const NON_NULL_COLUMN_READERS: [(&str, &str); 4] = [
        ("src/feedback.rs", "feedback.tenant_id is NOT NULL"),
        (
            "src/handlers/email_template_handler.rs",
            "users.tenant_id is NOT NULL",
        ),
        ("src/handlers/api_keys.rs", "api_keys.tenant_id is NOT NULL"),
        (
            "src/handlers/plan_request_handler.rs",
            "plan_requests.tenant_id is NOT NULL",
        ),
    ];

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => panic!("cannot read {}: {e}", dir.display()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// Is this code line a `Row::get("tenant_id")` whose target type is a bare `Uuid`?
    ///
    /// * `get::<Uuid, _>("tenant_id")` — explicit turbofish, panics on NULL.
    /// * `row.get("tenant_id")` (receiver is a row) — `Uuid` inferred from the field it feeds,
    ///   panics on NULL. `serde_json::Value::get("tenant_id")` is NOT this shape, hence the receiver
    ///   check: `req`/`body`/`params`/`value` access a JSON object, not a `PgRow`.
    fn panics_on_null_tenant_id(line: &str) -> bool {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        if !code.contains("\"tenant_id\"") {
            return false;
        }
        // A non-panicking decode, whichever spelling.
        if code.contains("try_get") || code.contains("get::<Option<Uuid>") {
            return false;
        }
        if code.contains("get::<Uuid") {
            return true;
        }
        let mut rest = code;
        while let Some(at) = rest.find(".get(") {
            let before = rest[..at].trim_end();
            let receiver = before
                .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            let arg = rest[at + 5..].trim_start();
            if matches!(receiver, "r" | "row" | "rec" | "record" | "pgrow")
                && arg.starts_with("\"tenant_id\"")
            {
                return true;
            }
            rest = &rest[at + 5..];
        }
        false
    }

    /// The census: no line in `src/` may decode `tenant_id` into a bare `Uuid` outside the NOT NULL
    /// allow-list, and the nullable-column readers must keep their defensive decodes.
    #[test]
    fn tenant_id_is_never_decoded_into_a_bare_uuid() {
        let mut files = Vec::new();
        rust_files(Path::new("src"), &mut files);
        assert!(
            files.len() > 40,
            "the census walked only {} files — a broken walk must never look like a clean tree",
            files.len()
        );

        let mut offences: Vec<String> = Vec::new();
        for path in &files {
            let shown = path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            let allowed = NON_NULL_COLUMN_READERS
                .iter()
                .any(|(file, _)| shown.ends_with(file));
            let source = fs::read_to_string(path).expect("source file is readable");
            for (n, line) in source.lines().enumerate() {
                if panics_on_null_tenant_id(line) && !allowed {
                    offences.push(format!("{shown}:{}: {}", n + 1, line.trim()));
                }
            }
        }

        assert!(
            offences.is_empty(),
            "tenant_id is NULLABLE on eight tables in this schema (an erasure DETACHES invoices, \
             the fleet catalogue rows carry a NULL tenant_id, a plan movement may have no \
             workspace). `Row::get` on a NULL PANICS and drops the request with no status line \
             (measured live 2026-11-24: GET /api/v1/admin/invoices). Decode `Option<Uuid>` or use \
             `try_get`, or add the file to NON_NULL_COLUMN_READERS with the column's NOT NULL \
             evidence:\n{}",
            offences.join("\n")
        );
    }

    /// The two decodes this card fixed must stay fixed — a revert to `r.get::<Uuid, _>` would be
    /// caught by the census above, but the census allow-list could be widened, so pin the shape.
    #[test]
    fn detached_invoice_and_null_tenant_category_decode_defensively() {
        assert!(
            INVOICE_HANDLER.contains("try_get::<Option<Uuid>, _>(\"tenant_id\")"),
            "invoice_handler::row_to_json must read tenant_id as Option<Uuid> — an erased \
             workspace leaves its invoices with tenant_id NULL"
        );
        assert!(
            !INVOICE_HANDLER.lines().any(panics_on_null_tenant_id),
            "invoice_handler must not contain a panicking tenant_id decode"
        );
        assert!(
            CATEGORY_HANDLER.contains("tenant_id: Option<Uuid>"),
            "product_categories.tenant_id is NULL on the fleet catalogue rows — the row struct \
             must decode it as Option<Uuid>"
        );
        assert!(
            AFFILIATE_PRODUCT_HANDLER.contains("tenant_id: Option<Uuid>"),
            "affiliate_products.tenant_id is NULLABLE — the row struct must decode it as Option"
        );
    }
}
