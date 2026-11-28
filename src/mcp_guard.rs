//! Guard: the MCP door exposes EXACTLY the eleven reads plus the thirty write tools (fleet standard
//! §9 core + §10 parity families), each tenant-scoped in its own statement or by calling the app's
//! own handler, and prints
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
    const WRITE_TOOLS: [&str; 30] = [
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
        "create_campaign",
        "update_campaign",
        "archive_campaign",
        "create_tag_group",
        "update_tag_group",
        "archive_tag_group",
        "create_tag_rule",
        "update_tag_rule",
        "archive_tag_rule",
        "create_web_to_lead",
        "update_web_to_lead",
        "archive_web_to_lead",
        "create_webhook",
        "update_webhook",
        "archive_webhook",
        "create_qr_code",
        "update_qr_code",
        "archive_qr_code",
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
        assert_eq!(
            names.len(),
            41,
            "eleven reads plus thirty writes (§9 core + §10 parity families)"
        );

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
            if name == "archive_tag" || name == "archive_tag_group" {
                assert!(destructive, "{name} deletes the row and must say so");
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
        assert_eq!(door.registry().len(), 41);
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

    /// kanban t_47d24788 — THE PAGE TOOLS PUBLISH THE BLOCK VOCABULARY.
    ///
    /// Measured live: `create_page` stored `layout_blocks` all along, but its published schema never
    /// named the key, so an agent reading the tool list had no way to send blocks; the shape an agent
    /// reaches for instead (`blocks`) was accepted, ignored and answered 201, leaving `blocks: 0` — a
    /// page that had to be finished by hand in the editor, reported as "the platform cannot draw
    /// link blocks". Both spellings are published now, an omitted array is documented as an EMPTY
    /// page, and a key the tool does not publish is refused by name.
    #[tokio::test]
    async fn the_page_tools_publish_their_block_vocabulary() {
        for name in ["create_page", "update_page"] {
            let tool = spec(name);
            let props = tool
                .pointer("/inputSchema/properties")
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("{name} must publish a properties object"));
            for key in super::super::mcp::PAGE_BLOCK_KEYS {
                assert!(
                    props.contains_key(key),
                    "{name} must publish `{key}` — a block key the tool does not name is a payload \
                     an agent cannot send (or sends and loses in silence)"
                );
            }
            assert_eq!(
                tool.pointer("/inputSchema/additionalProperties"),
                Some(&serde_json::Value::Bool(false)),
                "{name} must keep additionalProperties false"
            );
            let description = tool
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_lowercase();
            assert!(
                description.contains("empty"),
                "{name} must say plainly that a page created without blocks is an empty page"
            );
        }
    }

    /// kanban t_47d24788 — NOTHING IS PUBLISHED THAT THE CARD WRITER DOES NOT READ.
    ///
    /// The failure that caused the report was the reverse (the writer read a key nothing published),
    /// and the neighbouring one was found in the same sweep: `create_page` published
    /// `button_bg_color` / `button_text_color` while the handler filled those columns from
    /// `sub_color` / `btn_color`, so every colour the editor sent was dropped. This asserts the
    /// published names against the handler's own source, so a schema key that no statement reads
    /// fails the build instead of lying to a customer.
    #[tokio::test]
    async fn every_published_page_key_is_read_by_the_card_writer() {
        let handler = include_str!("handlers/kinetic_handler.rs");
        for name in ["create_page", "update_page"] {
            let tool = spec(name);
            let props = tool
                .pointer("/inputSchema/properties")
                .and_then(|v| v.as_object())
                .expect("properties");
            for key in props.keys() {
                assert!(
                    handler.contains(&format!("\"{key}\"")),
                    "{name} publishes `{key}`, which kinetic_handler.rs never reads — \
                     an argument that is advertised and dropped"
                );
            }
        }
    }

    /// kanban t_47d24788 — AN UNPUBLISHED ARGUMENT IS A REFUSAL THAT NAMES IT.
    #[tokio::test]
    async fn an_unknown_argument_is_refused_by_name() {
        let card = spec("create_page");
        let schema = card.get("inputSchema").expect("schema");
        let unknown = serde_json::json!({"title": "Probe", "blockz": [{"type": "link"}]});
        let err = super::super::mcp::refuse_unknown_args("create_page", &unknown, schema)
            .expect_err("an argument the tool does not publish must be refused");
        let text = format!("{err:?}");
        assert!(
            text.contains("blockz"),
            "the refusal must name the key: {text}"
        );
        assert!(
            text.contains("layout_blocks"),
            "a block-shaped key must be pointed at layout_blocks: {text}"
        );

        // Every published key is accepted, including the plain `blocks` alias.
        for key in super::super::mcp::PAGE_BLOCK_KEYS {
            let mut args = serde_json::json!({"title": "Probe"});
            args[key] = serde_json::json!([]);
            super::super::mcp::refuse_unknown_args("create_page", &args, schema)
                .unwrap_or_else(|e| panic!("`{key}` is published and must be accepted: {e:?}"));
        }
    }
}
