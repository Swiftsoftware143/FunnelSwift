use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::models::tag::*;
use crate::state::AppState;

/// kanban t_847f9d63: a tag's optional mapping fields are TEXT, and a panel operator clears a field
/// by emptying it. `None` (key absent) and `Some("")` (box emptied) both mean "no mapping", so both
/// fold to SQL NULL — a zero-length `source_app` must never reach `app_provision`, which would
/// otherwise log an `unknown_app` attempt for a field the operator simply left blank.
fn clean_opt(v: Option<&str>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub async fn list_tags(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Tag>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let tags = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE tenant_id = $1 OR is_system = true ORDER BY name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(tags))
}

pub async fn get_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Tag>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let tag = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE id = $1 AND (tenant_id = $2 OR is_system = true)",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tag not found".into()))?;

    Ok(Json(tag))
}

pub async fn create_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_tags", "Tags").await?;
    let tag_id = Uuid::new_v4();

    let is_system = req.is_system.unwrap_or(false);
    if is_system && !auth.is_admin {
        return Err(AppError::Forbidden(
            "Only admins can create system tags".into(),
        ));
    }

    sqlx::query(
        "INSERT INTO tags (id, tenant_id, name, color, group_id, metadata, is_system, source_app, plan_slug, provisions_account) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(tag_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.color)
    .bind(req.group_id)
    .bind(&req.metadata)
    .bind(is_system)
    // kanban t_847f9d63: the tag → free account mapping is writable from the panel, not seed-only.
    // `clean_opt` folds an empty/whitespace box to NULL so a blank field stores "no mapping" rather
    // than a zero-length slug that `app_provision` would try to call.
    .bind(clean_opt(req.source_app.as_deref()))
    .bind(clean_opt(req.plan_slug.as_deref()))
    .bind(req.provisions_account.unwrap_or(false))
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": tag_id, "message": "Tag created"})),
    ))
}

pub async fn update_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let tag = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE id = $1 AND (tenant_id = $2 OR is_system = true)",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tag not found".into()))?;

    if tag.is_system && !auth.is_admin {
        return Err(AppError::Forbidden(
            "Only admins can modify system tags".into(),
        ));
    }
    // Admin CAN modify system tags — but only on system tags, not tenant-scoped
    let update_query = if tag.is_system && auth.is_admin {
        "UPDATE tags SET name=$1, color=$2, group_id=$3, metadata=$4, source_app=$5, plan_slug=$6, provisions_account=$7 WHERE id=$8"
    } else {
        "UPDATE tags SET name=$1, color=$2, group_id=$3, metadata=$4, source_app=$5, plan_slug=$6, provisions_account=$7 WHERE id=$8 AND tenant_id=$9"
    };

    // kanban t_e6991a42. `leads.tags` stores tag NAMES and every resolution in the tag pipeline
    // matches those names against `tags.name`, so a rename used to leave every lead carrying the OLD
    // string: the tag came off the lead on remove but resolved to zero tag ids, so
    // `tag_logic::reverse_affiliate_on_tags` retracted no money (measured live, see
    // audits/fs-lead-tags-remove-t_769b93ba/60-renamed-tag-residual.md). ARM (a): the RENAME is the
    // event that makes the stored name wrong, so the rename is what rewrites it — in the same
    // transaction as the tag row, so a failure leaves neither half behind.
    let old_name = tag.name.clone();
    let name = req.name.unwrap_or(tag.name);
    let color = req.color.or(tag.color);
    // kanban t_f94a8a00: `clear_group` is the ONLY way to ungroup a tag. Without it a group can be
    // SET and CHANGED but never CLEARED, because `req.group_id.or(tag.group_id)` reads both an absent
    // key and an explicit null as "keep the stored group" (and a bare "" is an axum 422). The flag is
    // additive on purpose: every body shape a shipped caller already sends (including
    // `group_id: null` on a rename) keeps meaning exactly what it means today, so nothing can wipe a
    // tag's group by accident. Explicit clear wins over a group_id sent in the same body.
    let group_id = if req.clear_group.unwrap_or(false) {
        None
    } else {
        req.group_id.or(tag.group_id)
    };
    let metadata = req.metadata.or(tag.metadata);

    // ── the tag → free account mapping (kanban t_847f9d63) ───────────────────────────────────────
    // ABSENT key = keep what the row carries (every shipped caller that knows nothing about the
    // mapping, including the tenant Tags editor, keeps its exact current meaning). An explicitly
    // sent EMPTY string = CLEAR to NULL, which is what the admin form posts when the operator
    // empties the box; see `UpdateTagRequest`. `provisions_account` is a plain bool: absent keeps,
    // `false` really turns the shipped-ON mapping OFF.
    let source_app = match req.source_app.as_deref() {
        None => tag.source_app.clone(),
        Some(s) => clean_opt(Some(s)),
    };
    let plan_slug = match req.plan_slug.as_deref() {
        None => tag.plan_slug.clone(),
        Some(s) => clean_opt(Some(s)),
    };
    let provisions_account = req.provisions_account.unwrap_or(tag.provisions_account);

    let renamed = name != old_name;

    // One transaction: the tag row and the stored names it invalidated are one change, so a failure
    // in either half rolls both back — no lead is left pointing at a name the tags table no longer
    // has, and no tag is renamed with the leads still carrying the old string.
    let mut tx = state.pool.begin().await?;

    if tag.is_system && auth.is_admin {
        sqlx::query(update_query)
            .bind(&name)
            .bind(&color)
            .bind(group_id)
            .bind(&metadata)
            .bind(&source_app)
            .bind(&plan_slug)
            .bind(provisions_account)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query(update_query)
            .bind(&name)
            .bind(&color)
            .bind(group_id)
            .bind(&metadata)
            .bind(&source_app)
            .bind(&plan_slug)
            .bind(provisions_account)
            .bind(id)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await?;
    }

    // ── the rename rewrite ──────────────────────────────────────────────────────────────────────
    // Every lead carrying the OLD name gets the NEW one, so the name-keyed resolutions above
    // (`resolve_tag_ids`, the retraction block of `apply_lead_tags`, `evaluate_tag_rules`) keep
    // resolving. Scope: a tenant tag is rewritten for its OWN tenant only; a SYSTEM tag is the
    // shared vocabulary every tenant is offered (`GET /api/v1/tags` returns `tenant_id = $1 OR
    // is_system = true`), so its rename is rewritten for every lead that carries the name — the
    // same scope the resolution itself uses. Matched on the stored jsonb array, so a NULL/empty
    // `leads.tags` can never be touched, and `updated_at` moves with the write.
    let mut leads_migrated: i64 = 0;
    if renamed {
        let affected: Vec<(Uuid, Uuid)> = if tag.is_system && auth.is_admin {
            sqlx::query_as(
                "UPDATE leads
                    SET tags = COALESCE((SELECT jsonb_agg(CASE WHEN e = $1 THEN $2 ELSE e END)
                                           FROM jsonb_array_elements_text(tags) e), '[]'::jsonb),
                        updated_at = NOW()
                  WHERE tags @> to_jsonb($1::text)
                  RETURNING id, tenant_id",
            )
            .bind(&old_name)
            .bind(&name)
            .fetch_all(&mut *tx)
            .await?
        } else {
            sqlx::query_as(
                "UPDATE leads
                    SET tags = COALESCE((SELECT jsonb_agg(CASE WHEN e = $1 THEN $2 ELSE e END)
                                           FROM jsonb_array_elements_text(tags) e), '[]'::jsonb),
                        updated_at = NOW()
                  WHERE tenant_id = $3 AND tags @> to_jsonb($1::text)
                  RETURNING id, tenant_id",
            )
            .bind(&old_name)
            .bind(&name)
            .bind(tenant_id)
            .fetch_all(&mut *tx)
            .await?
        };
        leads_migrated = affected.len() as i64;

        // `tag_change_log` honoured: the stored tag really did change on each lead, so each lead
        // gets the audit row a tag change gets (added = the new name, removed = the old one). One
        // set-based INSERT, and every row carries the lead's OWN tenant (a system-tag rewrite spans
        // tenants while `tag_change_log.tenant_id` is scoped per lead).
        if !affected.is_empty() {
            let lead_ids: Vec<Uuid> = affected.iter().map(|(lead, _)| *lead).collect();
            let tenant_ids: Vec<Uuid> = affected.iter().map(|(_, tenant)| *tenant).collect();
            sqlx::query(
                "INSERT INTO tag_change_log (tenant_id, lead_id, added_tags, removed_tags, triggered_by)
                 SELECT tenant_id, lead_id, jsonb_build_array($1::text), jsonb_build_array($2::text), 'tag_rename'
                   FROM unnest($3::uuid[], $4::uuid[]) AS x(lead_id, tenant_id)",
            )
            .bind(&name)
            .bind(&old_name)
            .bind(&lead_ids)
            .bind(&tenant_ids)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;

    if renamed {
        tracing::info!(%id, %old_name, new_name = %name, leads_migrated,
            "tag renamed — stored lead tag names rewritten so the name-keyed tag pipeline still resolves");
    }

    // Outbound webhook delivery (kanban t_431faa99): `tag.updated` fires AFTER the row is written,
    // so a subscriber never sees a change the console could still refuse.
    crate::webhooks::spawn(
        state.pool.clone(),
        tenant_id,
        crate::webhooks::TAG_UPDATED,
        json!({
            "tag_id": id.to_string(),
            "name": name,
            "color": color,
            "group_id": group_id,
        }),
    );

    // Additive on the response (kanban t_e6991a42): the count of leads whose stored tag name the
    // rename rewrote. Every shipped caller reads `message` only; the number is what makes the
    // backfill measurable from the API instead of only from the DB.
    Ok(Json(
        json!({ "message": "Tag updated", "leads_migrated": leads_migrated }),
    ))
}

/// DELETE /api/v1/tags/:id — kanban t_ca4693e3.
///
/// ARM (a), decided by measurement (the card's own `checks-delete-residual.json` measured on the
/// pre-fix binary, plus this card's `checks-before.json`): the DELETE now does, for EVERY lead that
/// carries the tag's name, exactly what the removal arm (t_769b93ba) does for one lead — because after
/// the delete there is no longer any code path that could ever do it again.
///
/// The measurements that chose the arm:
///   * arm (b) — "a deleted label stays on the lead as free text" — is not a neutral choice. Every
///     resolution in the tag pipeline is name-keyed (`resolve_tag_ids`, the retraction block of
///     `apply_lead_tags`, `tag_logic::attribute_affiliate_on_tags`), so the leftover name resolves to
///     ZERO tag ids, the pending `affiliate_commissions` row the tag produced is never retracted, and
///     the FK `affiliate_products.system_tag_id` is `ON DELETE SET NULL` — the product link dies with
///     the tag, so a re-created tag of the same name cannot resolve it either (measured: `before` B5/B7,
///     the row stays `pending` forever). The money would be unretractable from the app permanently.
///   * arm (c) — refuse with 409 while a lead still carries the tag — pushes the operator through the
///     removal arm, but nothing guarantees the sweep, and the served Del buttons report ANY non-2xx as
///     "N failed" with no reason (`www-app/index.html`), so the refusal is invisible and the money
///     stays pending.
///   * arm (a) — CHOSEN: the DELETE means "this tag is gone from this workspace", so in ONE
///     transaction it (1) strips the name from every lead carrying it, in the resolution's OWN scope
///     (a tenant tag: its tenant; a SYSTEM tag: every tenant — the same scope as the recorded rename
///     fix, and what the served confirm already promises, "This removes it for ALL tenants"),
///     (2) writes the `tag_change_log` row each lead's own tag change gets (`added=[]`,
///     `removed=[name]`, `triggered_by='tag_delete'`), (3) REVERSES the pending attribution of the
///     products this tag pointed at exactly the way the removal arm does
///     (`tag_logic::reverse_affiliate_on_tags_bulk`: reversed, never deleted, never settled money,
///     marked `reversed_by='tag_removed'` so a re-tag can re-open the SAME row), and only then
///     (4) deletes the tag row — last, because the FK would erase the link step (3) resolves.
///
/// Additive on the response: `leads_updated` and `commissions_reversed`. Every shipped caller (the
/// served System Tags / Tags screens' Del buttons) ignores the body and re-renders the list.
pub async fn delete_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let tag = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE id = $1 AND (tenant_id = $2 OR is_system = true)",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tag not found".into()))?;

    if tag.is_system && !auth.is_admin {
        return Err(AppError::Forbidden(
            "Only admins can delete system tags".into(),
        ));
    }

    // The resolution's own scope, the same one the rename rewrite uses (see `update_tag`): a SYSTEM tag
    // is the shared vocabulary every workspace is offered (`GET /api/v1/tags` returns
    // `tenant_id = $1 OR is_system = true`), so deleting it is not a tenant-scoped act; a tenant tag is
    // stripped only for its own tenant's leads.
    let system_scope = tag.is_system && auth.is_admin;
    let name = tag.name.clone();

    // One transaction: the tag row, the stored names it invalidated and the attribution they produced
    // are one change. A failure in any half rolls all of them back — no lead is left pointing at a name
    // the tags table no longer has, and no attribution is withdrawn without the tag actually going.
    let mut tx = state.pool.begin().await?;

    // ── 1. strip the name from every lead carrying it ─────────────────────────────────────────────
    // `tags - $1::text` removes every occurrence of that string element from the jsonb array and keeps
    // the order; `tags @> to_jsonb($1::text)` is the containment test the name-keyed resolvers use, so a
    // NULL/empty `leads.tags` can never be touched. `updated_at` moves with the write.
    let affected: Vec<(Uuid, Uuid)> = if system_scope {
        sqlx::query_as(
            "UPDATE leads
                SET tags = tags - $1::text, updated_at = NOW()
              WHERE tags @> to_jsonb($1::text)
              RETURNING id, tenant_id",
        )
        .bind(&name)
        .fetch_all(&mut *tx)
        .await?
    } else {
        sqlx::query_as(
            "UPDATE leads
                SET tags = tags - $1::text, updated_at = NOW()
              WHERE tenant_id = $2 AND tags @> to_jsonb($1::text)
              RETURNING id, tenant_id",
        )
        .bind(&name)
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await?
    };
    let leads_updated = affected.len() as i64;

    // ── 2. `tag_change_log` honoured ──────────────────────────────────────────────────────────────
    // The stored tag really did come off each lead, so each lead gets the audit row a tag change gets.
    // One set-based INSERT, and every row carries the lead's OWN tenant (a system-tag delete spans
    // tenants while `tag_change_log.tenant_id` is scoped per lead).
    if !affected.is_empty() {
        let lead_ids: Vec<Uuid> = affected.iter().map(|(lead, _)| *lead).collect();
        let tenant_ids: Vec<Uuid> = affected.iter().map(|(_, tenant)| *tenant).collect();
        sqlx::query(
            "INSERT INTO tag_change_log (tenant_id, lead_id, added_tags, removed_tags, triggered_by)
             SELECT tenant_id, lead_id, '[]'::jsonb, jsonb_build_array($1::text), 'tag_delete'
               FROM unnest($2::uuid[], $3::uuid[]) AS x(lead_id, tenant_id)",
        )
        .bind(&name)
        .bind(&lead_ids)
        .bind(&tenant_ids)
        .execute(&mut *tx)
        .await?;
    }

    // ── 3. REVERSE the attribution the deleted tag produced ───────────────────────────────────────
    // THE DECISION (the same contract the removal arm recorded): REVERSE the pending row — never delete
    // it, never touch a row that has already settled. This must run BEFORE the tag row goes, because
    // `affiliate_products.system_tag_id` is `ON DELETE SET NULL`.
    let lead_ids: Vec<Uuid> = affected.iter().map(|(lead, _)| *lead).collect();
    let commissions_reversed =
        crate::tag_logic::reverse_affiliate_on_tags_bulk(&mut tx, &lead_ids, &[id]).await?;

    // ── 4. delete the tag row, in the same scope as before ────────────────────────────────────────
    if auth.is_admin {
        sqlx::query("DELETE FROM tags WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM tags WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    tracing::info!(%id, %name, system_scope, leads_updated, commissions_reversed,
        "tag deleted — the stored lead tag names and the attribution the tag produced were retired with it");

    // Additive on the response (kanban t_ca4693e3): the number of leads whose stored tag name the delete
    // retired, and the number of pending attributions it reversed. Every shipped caller reads `message`
    // only; the numbers are what make the retraction measurable from the API instead of only from the DB.
    Ok(Json(json!({
        "message": "Tag deleted",
        "leads_updated": leads_updated,
        "commissions_reversed": commissions_reversed,
    })))
}
