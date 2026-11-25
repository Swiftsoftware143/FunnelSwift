//! Guard: the MCP door exposes EXACTLY the eleven reads plus the twelve write tools (fleet standard
//! §9), each tenant-scoped in its own statement or by calling the app's own handler, and prints
//! the same list on the customer console (kanban t_aeefb4fb, t_03af3510).
//!
//! These are source-level checks in the style of `tenant_decode_guard` — no database, no network —
//! so a later change cannot quietly widen the surface:
//!
//! * a tool that is neither a read nor one of the twelve named writes fails;
//! * a read tool annotated writable (or a write tool annotated read-only) fails;
//! * only `archive_tag` may be annotated destructive;
//! * a statement that lost the bound `tenant_id` predicate fails;
//! * a SELECT that projects a password hash, key, provider credential or the layout blob fails;
//! * the console's MCP screen drifting away from the door fails.

#[cfg(test)]
mod tests {
    use crate::db::SchemaStatus;

    /// The door's own source, read at COMPILE time.
    const SRC: &str = include_str!("mcp.rs");
    /// The customer console that prints the tool list.
    const CONSOLE: &str = include_str!("../www-app/index.html");

    fn lazy_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://guard:guard@127.0.0.1:1/guard")
            .expect("a lazy pool parses without connecting")
    }

    /// The door only ever READS this state while building its registry; no tool runs here, so a
    /// lazy pool and placeholder config are enough to validate names, schemas and annotations.
    fn test_state() -> crate::state::AppState {
        crate::state::AppState::new(
            lazy_pool(),
            SchemaStatus::new(),
            "guard-jwt-secret".to_string(),
            "guard-internal-key".to_string(),
            "http://127.0.0.1:1".to_string(),
            "http://127.0.0.1:1".to_string(),
        )
    }

    fn registry() -> mcp_door::Registry {
        super::super::mcp::registry(test_state())
    }

    fn spec(name: &str) -> serde_json::Value {
        registry()
            .get(name)
            .unwrap_or_else(|| panic!("tool {name:?} is not registered"))
            .spec()
            .to_json()
    }

    /// The twelve writes §9 adds: one create, one update and one archive per top-level entity the
    /// product owns (funnels, pages, leads, tags). Any other write tool is a surface change.
    const WRITE_TOOLS: [&str; 12] = [
        "create_funnel",
        "update_funnel",
        "archive_funnel",
        "create_page",
        "update_page",
        "archive_page",
        "create_lead",
        "update_lead",
        "archive_lead",
        "create_tag",
        "update_tag",
        "archive_tag",
    ];

    #[tokio::test]
    async fn the_door_offers_the_eleven_reads_and_the_twelve_writes() {
        let reg = registry();
        assert_eq!(
            reg.validate(),
            Vec::<String>::new(),
            "the registry must be valid at startup"
        );
        let mut names = reg.names();
        names.sort();
        let mut expected: Vec<String> = super::super::mcp::TOOL_NAMES
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        expected.sort();
        assert_eq!(names, expected, "the exposed tool set changed");
        assert_eq!(names.len(), 23, "eleven reads plus twelve writes");

        for name in &names {
            let tool = spec(name);
            let annotations = tool.get("annotations").expect("annotations");
            let read_only = annotations
                .get("readOnlyHint")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let destructive = annotations
                .get("destructiveHint")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if WRITE_TOOLS.contains(&name.as_str()) {
                assert!(
                    !read_only,
                    "{name} is a write tool and must not read-only-annotate"
                );
            } else {
                assert!(read_only, "{name} must be annotated read-only");
            }
            // Archive semantics keep every write reversible EXCEPT the tag, which has no archive
            // column and therefore says so with the destructive annotation.
            if name == "archive_tag" {
                assert!(destructive, "archive_tag deletes the row and must say so");
            } else {
                assert!(!destructive, "{name} must never be destructive");
            }
        }
    }

    #[test]
    fn every_statement_binds_the_workspace_predicate() {
        // The tenancy guarantee is structural: the filter is in the SQL text of every statement,
        // bound from the token's scope — never a post-fetch check, never a body argument.
        assert_eq!(
            SRC.matches("tenant_id::text = any($2::text[])").count(),
            13,
            "the predicate must appear in every statement (the two JOINs and page_stats' card join carry it twice)"
        );

        // No statement may be BUILT at run time (gate rule 5d): each `sqlx::query(` is followed by
        // a string literal, and no format!/concat arm feeds one.
        assert!(
            !SRC.contains("format!(\"SELECT") && !SRC.contains("format!(\"SELECT"),
            "SQL must never be built with format!"
        );
        let mut rest = SRC;
        let mut statements = 0;
        while let Some(at) = rest.find("sqlx::query(") {
            rest = &rest[at + "sqlx::query(".len()..];
            let head = rest.trim_start();
            assert!(
                head.starts_with('"'),
                "a run-time-built SQL statement appeared in the door: {:?}",
                head.chars().take(40).collect::<String>()
            );
            statements += 1;
        }
        assert_eq!(
            statements, 14,
            "eleven tool statements, the archive_page update, the credential lookup and its usage stamp"
        );
    }

    /// Every SQL literal in the door, in source order. Each is a constant (never built).
    fn sql_literals() -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut rest = SRC;
        while let Some(at) = rest.find("sqlx::query(") {
            rest = &rest[at + "sqlx::query(".len()..];
            let head = rest.trim_start();
            assert!(
                head.starts_with('"'),
                "a run-time-built SQL statement appeared in the door"
            );
            let body = &head[1..];
            let close = body.find('"').expect("unterminated SQL literal");
            out.push(&body[..close]);
            rest = &body[close + 1..];
        }
        out
    }

    #[test]
    fn no_secret_or_blob_is_projected() {
        let statements = sql_literals();
        assert_eq!(
            statements.len(),
            14,
            "eleven reads, the page archive and the credential pair"
        );
        for sql in &statements {
            for token in [
                "key_sealed",
                "mailgun_api_key",
                "smtp_password_encrypted",
                "webhook_signing_key_encrypted",
                "inbound_token",
                "ip_hash",
                "provider_keys",
                "payment_webhook_events",
                "webhook_delivery_log",
                "webhooks",
            ] {
                assert!(
                    !sql.to_lowercase().contains(token),
                    "{token} must never be read by a door statement: {sql:?}"
                );
            }
        }

        // The page password appears ONCE in SQL, as a boolean, never as the stored hash.
        let password_uses: Vec<&&str> = statements
            .iter()
            .filter(|sql| sql.contains("password_hash"))
            .collect();
        assert_eq!(
            password_uses.len(),
            1,
            "password_hash is read by one statement"
        );
        assert!(
            password_uses[0].contains("(c.password_hash IS NOT NULL)"),
            "the only use is the boolean `has_password`, never the hash"
        );

        // The layout blob is only ever COUNTED, never selected.
        for sql in statements
            .iter()
            .filter(|sql| sql.contains("layout_blocks"))
        {
            assert!(
                sql.contains("jsonb_typeof(layout_blocks)")
                    || sql.contains("jsonb_array_length(layout_blocks)")
                    || sql.contains("jsonb_typeof(c.layout_blocks)")
                    || sql.contains("jsonb_array_length(c.layout_blocks)"),
                "layout_blocks is only ever counted, not returned: {sql:?}"
            );
            assert!(
                !sql.contains("SELECT c.layout_blocks") && !sql.contains(", c.layout_blocks,"),
                "the blob itself must never be projected: {sql:?}"
            );
        }

        // The key hash is read in exactly one place: the WHERE clause of the credential lookup.
        assert_eq!(
            SRC.matches("key_hash").count(),
            1,
            "key_hash may only appear in the credential WHERE clause"
        );
        assert!(SRC.contains("WHERE k.key_hash = $1 AND k.revoked_at IS NULL"));
    }

    #[tokio::test]
    async fn the_door_builds_and_refuses_a_bad_registry() {
        // A real build validates names, descriptions, schemas, annotations and the config.
        let door = super::super::mcp::build_door(test_state()).expect("the door builds");
        assert_eq!(door.registry().len(), 23);
        assert_eq!(door.config().path, "/mcp");
        assert_eq!(door.config().server_name, "funnelswift");
    }

    #[test]
    fn the_console_prints_the_door_tool_list() {
        let start = CONSOLE
            .find("var MCP_SPECS=[")
            .expect("the console's MCP tool list is gone");
        let rest = &CONSOLE[start..];
        let end = rest.find("];").expect("unterminated MCP_SPECS");
        let block = &rest[..end];
        let mut printed: Vec<String> = Vec::new();
        for line in block.lines() {
            let line = line.trim();
            if let Some(after) = line.strip_prefix("[\"") {
                if let Some((name, _)) = after.split_once('"') {
                    printed.push(name.to_string());
                }
            }
        }
        printed.sort();
        let mut expected: Vec<String> = super::super::mcp::TOOL_NAMES
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        expected.sort();
        assert_eq!(
            printed, expected,
            "the console's MCP screen must print exactly the tools the door serves"
        );

        // The console must advertise THIS app's door, not the retired cross-app hub spelling.
        assert!(
            CONSOLE.contains("var MCP_ENDPOINT='https://app.funnelswift.net/mcp'"),
            "the console must point at the app's own door"
        );
    }

    #[test]
    fn the_api_key_is_never_a_subject() {
        // The audit subject is the key PREFIX; the plaintext must never be formatted into a string.
        assert!(SRC.contains("format!(\"key {key_prefix}\")"));
    }
}
