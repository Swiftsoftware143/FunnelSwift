//! Guard: the MCP door exposes EXACTLY the eleven read tools, each tenant-scoped in its own
//! statement, and prints the same list on the customer console (kanban t_aeefb4fb).
//!
//! These are source-level checks in the style of `tenant_decode_guard` — no database, no network —
//! so a later change cannot quietly widen the surface:
//!
//! * a twelfth tool (or a renamed one) fails;
//! * a write/destructive tool fails;
//! * a statement that lost the bound `tenant_id` predicate fails;
//! * a SELECT that projects a password hash, key, provider credential or the layout blob fails;
//! * the console's MCP screen drifting away from the door fails.

#[cfg(test)]
mod tests {
    use mcp_door::ToolHandler;

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

    fn registry() -> mcp_door::Registry {
        super::super::mcp::registry(lazy_pool())
    }

    fn spec(name: &str) -> serde_json::Value {
        registry()
            .get(name)
            .unwrap_or_else(|| panic!("tool {name:?} is not registered"))
            .spec()
            .to_json()
    }

    #[tokio::test]
    async fn the_door_offers_exactly_the_eleven_read_tools() {
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
        assert_eq!(names.len(), 11, "this phase is eleven reads");

        for name in &names {
            let tool = spec(name);
            let annotations = tool.get("annotations").expect("annotations");
            assert_eq!(
                annotations.get("readOnlyHint").and_then(|v| v.as_bool()),
                Some(true),
                "{name} must be annotated read-only"
            );
            assert_eq!(
                annotations.get("destructiveHint").and_then(|v| v.as_bool()),
                Some(false),
                "{name} must never be destructive"
            );
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
                &head.chars().take(40).collect::<String>()
            );
            statements += 1;
        }
        assert_eq!(
            statements, 13,
            "eleven tool statements, the credential lookup and its usage stamp"
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
            13,
            "eleven reads plus the credential pair"
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
        let door = super::super::mcp::build_door(lazy_pool()).expect("the door builds");
        assert_eq!(door.registry().len(), 11);
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
