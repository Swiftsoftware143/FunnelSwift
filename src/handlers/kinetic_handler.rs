use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

// ─────────────────────────────────────────────────────────────────────────────────────────────
// RECORDED DECISION (kanban t_8ccc8e6a): a numeric plan limit means "you may not EXCEED it", so
// `enforce_feature_limit(…, "max_cards", …)` belongs on the route that ADDS a card and nowhere
// else. A tenant AT its limit must still be able to list, read, edit and DELETE what it owns —
// DELETE is the only way to free a slot, so guarding it wedges the workspace until the plan
// changes. The same rule holds for the workspace settings on this handler: `max_cards` counts
// kinetic_cards rows, and `set_subdomain` / `set_site_meta` / `set_custom_domain` add none, so
// they are not card operations and must not be gated by a card count. The upsell (402
// UpgradeRequired -> the SPA's upgrade modal) is carried by the ADD that actually exceeds the
// plan, and by a FIRST claim of a feature the plan does not grant (max_custom_domains=0).
//
// Measured on the running container before this change (2026-09-25, binary 48f54fff): a
// kinetic-free workspace at 1/1 got 402 `Cards limit reached (1/1)` on GET /kinetic/cards,
// PUT /kinetic/cards/:id, DELETE /kinetic/cards/:id, GET+PUT /kinetic/subdomain,
// GET /kinetic/custom-domain, GET+PUT /kinetic/site-meta; on kinetic-pro at 3/3 the same card
// routes 402'd, and a pro tenant that had claimed its one custom domain got
// `Custom domains limit reached (1/1)` when CHANGING or CLEARING it. Census of the fleet's
// `enforce_feature_limit` call sites: every other one is on a create handler (or, in
// WorkflowSwift's set_account_industry, behind an explicit `if is_new` check — the precedent for
// the first-claim shape used in `set_custom_domain` below).
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Resolve canonical URL from Host header
/// kntcrd.com subdomain → https://{tenant}.kntcrd.com/k/{slug}
/// funnelswift.net → 301 redirect to kntcrd.com (never canonical)
/// custom domain → https://{domain}/k/{slug}
/// Resolve canonical URL from Host header with correct prefix
/// kntcrd.com subdomain → https://{tenant}.kntcrd.com/{prefix}/{slug}
/// funnelswift.net → redirect to kntcrd.com root
/// custom domain → https://{domain}/{prefix}/{slug}
fn resolve_canonical_url(host: &str, prefix: &str, slug: &str) -> String {
    let host_clean = host.split(':').next().unwrap_or(host).to_lowercase();
    if host_clean.ends_with("funnelswift.net") {
        return format!("https://kntcrd.com/{}/{}", prefix, slug);
    }
    if host_clean.ends_with("kntcrd.com") {
        return format!("https://{}/{}/{}", host_clean, prefix, slug);
    }
    format!("https://{}/{}/{}", host_clean, prefix, slug)
}

/// Map URL prefix to branded CTA label + card type label
fn cta_for_prefix(prefix: &str) -> (&'static str, &'static str) {
    match prefix {
        "b" => ("Claim your free Bio Link →", "Bio Link"),
        "c" => (
            "Claim your free Digital Business Card →",
            "Digital Business Card",
        ),
        "m" => ("Claim your free Micro Page →", "Micro Page"),
        "f" => ("Claim your free Mini Funnel →", "Mini Funnel"),
        "h" => ("Claim your free Hero Page →", "Hero Page"),
        _ => ("Claim your free Kinetic Card →", "Kinetic Card"),
    }
}

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::templates::html_escape;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CardQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
    pub type_: Option<String>,
}

pub async fn list_cards(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(_q): Query<CardQuery>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let rows = sqlx::query(
        "SELECT id, tenant_id, user_id, title, slug, bio, bg_color, accent_color, text_color, button_bg_color, button_text_color, template_type, tagline, meta_description, avatar_url, layout_blocks, theme, video_provider, video_id, is_active, consent_required, age_gate_type, age_gate_message, consent_decline_redirect, created_at, updated_at FROM kinetic_cards WHERE tenant_id = $1 ORDER BY created_at DESC"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    use sqlx::Row;
    let cards: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").unwrap_or_default().to_string(),
        "title": r.try_get::<String, _>("title").unwrap_or_default(),
        "slug": r.try_get::<String, _>("slug").unwrap_or_default(),
        "bio": r.try_get::<Option<String>, _>("bio").unwrap_or_default(),
        "bg_color": r.try_get::<Option<String>, _>("bg_color").unwrap_or_default(),
        "accent_color": r.try_get::<Option<String>, _>("accent_color").unwrap_or_default(),
        "text_color": r.try_get::<Option<String>, _>("text_color").unwrap_or_default(),
        "button_bg_color": r.try_get::<Option<String>, _>("button_bg_color").unwrap_or_default(),
        "button_text_color": r.try_get::<Option<String>, _>("button_text_color").unwrap_or_default(),
        "template_type": r.try_get::<Option<String>, _>("template_type").unwrap_or_default(),
        "tagline": r.try_get::<Option<String>, _>("tagline").unwrap_or_default(),
        "meta_description": r.try_get::<Option<String>, _>("meta_description").unwrap_or_default(),
        "avatar_url": r.try_get::<Option<String>, _>("avatar_url").unwrap_or_default(),
        "layout_blocks": r.try_get::<Option<Value>, _>("layout_blocks").unwrap_or_default(),
        "theme": r.try_get::<Option<String>, _>("theme").unwrap_or_default(),
        "video_provider": r.try_get::<Option<String>, _>("video_provider").unwrap_or_default(),
        "video_id": r.try_get::<Option<String>, _>("video_id").unwrap_or_default(),
        "is_template": r.try_get::<bool, _>("is_template").unwrap_or(false),
        // Consent / age gate half of `has_card_gating` — the editor prefills its
        // controls from these, and they are what the renderer acts on.
        "consent_required": r.try_get::<Option<bool>, _>("consent_required").unwrap_or(None).unwrap_or(false),
        "age_gate_type": r.try_get::<Option<String>, _>("age_gate_type").unwrap_or(None),
        "age_gate_message": r.try_get::<Option<String>, _>("age_gate_message").unwrap_or(None),
        "consent_decline_redirect": r.try_get::<Option<String>, _>("consent_decline_redirect").unwrap_or(None),
        "template_category": r.try_get::<Option<String>, _>("template_category").unwrap_or_default(),
        "category": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "created_at": r
            .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
        "updated_at": r
            .try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
            .map(|t| t.to_rfc3339())
            .unwrap_or_default()
    })).collect();
    Ok(Json(json!({"cards": cards})))
}

pub async fn create_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let id = Uuid::new_v4();
    let title = body["title"].as_str().unwrap_or("New Card");
    let id_str = id.to_string();
    let slug = body["slug"].as_str().unwrap_or(&id_str[..8]);
    let bio = body["bio"].as_str().unwrap_or("");
    let bg_color = body["bg_color"].as_str().unwrap_or("#0f172a");
    let accent_color = body["accent_color"].as_str().unwrap_or("#6366f1");
    let text_color = body["text_color"].as_str().unwrap_or("#ffffff");
    let sub_color = body["sub_color"].as_str().unwrap_or("#94a3b8");
    let btn_color = body["btn_color"].as_str().unwrap_or("#6366f1");
    let card_kind = body["type"].as_str().or(body["card_type"].as_str());
    // kanban t_747b4dd6 — ONE value space for `kinetic_cards.template_type`: the card TYPE, spelled
    // in the editor's underscore ids (`crate::card_types::CARD_TYPES`). The hyphenated card kinds
    // clients send in `type`/`card_type` ("bio-link") are ALIASES and are normalised here, so the
    // second vocabulary cannot reach the column again; an unknown value is stored unchanged.
    // Precedence is the one t_8151c83f established: an explicit `template_type` (what the editor's
    // Template Type select sends) wins, else the card kind the client sent, else the fallback.
    let template_type = crate::card_types::stored(body["template_type"].as_str(), card_kind);
    // Mini funnels are a plan-gated card type (has_mini_funnels). The gate reads the NORMALISED
    // value now: before t_747b4dd6 it compared the literal "mini_funnel", so a client sending the
    // hyphenated kind stored a gated card type on a plan without the flag (measured: 201).
    if template_type == crate::card_types::MINI_FUNNEL {
        crate::features::enforce_feature_flag(
            &state,
            tenant_id,
            "has_mini_funnels",
            "Mini funnels",
        )
        .await?;
    }
    let tagline = body["tagline"].as_str().unwrap_or("");
    let meta_desc = body["meta_description"].as_str().unwrap_or("");
    let avatar = body["avatar_url"].as_str();
    let social = body.get("social_links").cloned();
    let theme = body["theme_slug"].as_str();
    let video_provider = body["video_provider"].as_str();
    let video_id = body["video_id"].as_str();

    // Plan gate: premium themes + premium catalogue templates require the
    // `premium_themes` feature. Free themes (midnight/ocean/rose) and free
    // templates (bio_*) pass through untouched, as do plain card types.
    crate::features::enforce_theme_access(
        &state,
        tenant_id,
        body["theme_slug"].as_str().or(body["theme"].as_str()),
    )
    .await?;
    // A card carries its catalogue template in `template_type`, while `type`/`card_type`
    // describe the card kind. Chaining these with `.or()` short-circuits on the FIRST `Some`,
    // so `{"type":"bio-link","template_type":"biz_executive"}` was never checked against
    // `template_type` at all and a premium template got created for free. Every candidate is
    // checked now, so whichever field carries the template, the gate sees it.
    for candidate in [
        body["template_type"].as_str(),
        body["type"].as_str(),
        body["card_type"].as_str(),
    ]
    .into_iter()
    .flatten()
    {
        crate::features::enforce_template_access(&state, tenant_id, Some(candidate)).await?;
    }

    sqlx::query("INSERT INTO kinetic_cards (id, tenant_id, user_id, title, slug, bio, bg_color, accent_color, text_color, button_bg_color, button_text_color, template_type, tagline, meta_description, avatar_url, layout_blocks, theme, video_provider, video_id, is_active) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,true)")
        .bind(id).bind(tenant_id).bind(id).bind(title).bind(slug).bind(bio).bind(bg_color).bind(accent_color).bind(text_color).bind(sub_color).bind(btn_color).bind(&template_type).bind(tagline).bind(meta_desc).bind(avatar).bind(&social).bind(theme).bind(video_provider).bind(video_id)
        .execute(&state.pool).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "slug": slug, "message": "Card created"})),
    ))
}

pub async fn update_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Plan gate: changing to a premium theme / premium catalogue template
    // requires the caller's plan to grant `premium_themes`.
    crate::features::enforce_theme_access(
        &state,
        tenant_id,
        body["theme"].as_str().or(body["theme_slug"].as_str()),
    )
    .await?;
    // Same reasoning as `create_card`: check every field that can carry a catalogue template,
    // never just the first one that happens to be present.
    for candidate in [
        body["template_type"].as_str(),
        body["type"].as_str(),
        body["card_type"].as_str(),
    ]
    .into_iter()
    .flatten()
    {
        crate::features::enforce_template_access(&state, tenant_id, Some(candidate)).await?;
    }
    let social = body.get("social_links").cloned();
    // kanban t_8151c83f — TWO columns this screen edits were not writable through this route:
    //  * `template_type` was missing from the SET list entirely, so the editor's Template Type select
    //    could never change it (measured: PUT {"template_type":"hero"} left the row at business_card).
    //  * `video_provider` used COALESCE, and `body["video_provider"].as_str()` is None for BOTH "the
    //    key is absent" (leave the column) and "the client sent null" (the editor's None option ->
    //    CLEAR it), so a provider could be set but never cleared (measured: PUT {"video_provider":null}
    //    left the row at vimeo).
    // Both are written only when the body carries them: an ABSENT key leaves the column alone, an
    // explicit null (or the trimmed "" a select sends) clears `video_provider`. Note the mini-funnel
    // plan gate stays on CREATE only — gating here would make an already-stored mini_funnel card
    // unsavable for a plan that no longer grants the feature.
    // kanban t_747b4dd6: an explicit `template_type` is normalised to the canonical card type
    // (`crate::card_types`) before it is written, so the editor cannot re-introduce the hyphenated
    // card-kind vocabulary either; a value that is neither canonical nor a known alias is written
    // verbatim. An ABSENT key still leaves the column alone (COALESCE below).
    let template_type = body["template_type"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|raw| crate::card_types::canonical(raw).unwrap_or(raw));
    let vp_change: Option<Option<String>> = match body.get("video_provider") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.trim().to_string()).filter(|v| !v.is_empty())),
        Some(_) => {
            return Err(AppError::BadRequest(
                "video_provider must be a string or null".into(),
            ))
        }
    };
    sqlx::query("UPDATE kinetic_cards SET title=COALESCE($3,title), bio=COALESCE($4,bio), bg_color=COALESCE($5,bg_color), accent_color=COALESCE($6,accent_color), text_color=COALESCE($7,text_color), button_bg_color=COALESCE($8,button_bg_color), button_text_color=COALESCE($9,button_text_color), avatar_url=COALESCE($10,avatar_url), layout_blocks=COALESCE($11,layout_blocks), tagline=COALESCE($12,tagline), meta_description=COALESCE($13,meta_description), video_id=COALESCE($14,video_id), slug=COALESCE($15,slug), theme=COALESCE($16,theme), template_type=COALESCE($17::varchar,template_type), video_provider=CASE WHEN $18 THEN $19::varchar ELSE video_provider END WHERE id=$1 AND tenant_id=$2")
        .bind(id).bind(tenant_id)
        .bind(body["title"].as_str()).bind(body["bio"].as_str())
        .bind(body["bg_color"].as_str()).bind(body["accent_color"].as_str())
        .bind(body["text_color"].as_str()).bind(body["sub_color"].as_str())
        .bind(body["btn_color"].as_str()).bind(body["avatar_url"].as_str())
        .bind(&social)
        .bind(body["tagline"].as_str()).bind(body["meta_description"].as_str())
        .bind(body["video_id"].as_str())
        // `cta_text` was dropped: `kinetic_cards` has no such column and nothing reads one —
        // the footer CTA is the plan feature `kinetic_cta_text` and per-block CTAs live inside
        // `layout_blocks`. Naming it made this UPDATE fail 42703 on every card edit.
        .bind(body["slug"].as_str())
        .bind(body["theme"].as_str())
        .bind(template_type)
        .bind(vp_change.is_some())
        .bind(vp_change.flatten())
        .execute(&state.pool).await?;
    Ok(Json(json!({"message": "Card updated"})))
}

pub async fn delete_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM kinetic_cards WHERE id=$1 AND tenant_id=$2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Card deleted"})))
}

pub async fn list_buttons(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // `kinetic_buttons` has no `url` column — migration 0017 declares `destination_url` plus a
    // NOT NULL `action_type`. The statement named `url`, so this reader raised ERROR 42703 and
    // `.unwrap_or_default()` turned it into "No buttons yet." for every card. Errors now
    // propagate (the reason this defect could hide in the first place).
    let rows = sqlx::query(
        "SELECT b.id, b.card_id, b.label, b.destination_url, b.action_type, b.sort_order, b.created_at FROM kinetic_buttons b JOIN kinetic_cards c ON c.id = b.card_id WHERE b.card_id=$1 AND c.tenant_id=$2 ORDER BY b.sort_order"
    ).bind(card_id).bind(tenant_id).fetch_all(&state.pool).await?;
    use sqlx::Row;
    let buttons: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid, _>("id").unwrap_or_default().to_string(),
                "card_id": r.try_get::<Uuid, _>("card_id").unwrap_or_default().to_string(),
                "label": r.try_get::<String, _>("label").unwrap_or_default(),
                "url": r.try_get::<Option<String>, _>("destination_url").unwrap_or_default().unwrap_or_default(),
                "destination_url": r.try_get::<Option<String>, _>("destination_url").unwrap_or_default().unwrap_or_default(),
                "action_type": r.try_get::<String, _>("action_type").unwrap_or_default(),
                "sort_order": r.try_get::<i32, _>("sort_order").unwrap_or(0),
                "created_at": r
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default()
            })
        })
        .collect();
    Ok(Json(json!(buttons)))
}

pub async fn create_button(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // Verify the card belongs to this tenant before attaching a button.
    let owns_card: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kinetic_cards WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(false);
    if !owns_card {
        return Err(AppError::NotFound("Card not found".into()));
    }
    crate::features::enforce_action_button_limit(&state, tenant_id, card_id).await?;
    let id = Uuid::new_v4();
    let label = body["label"].as_str().unwrap_or("Button");
    // `kinetic_buttons` columns are `destination_url` and a NOT NULL `action_type` (migration
    // 0017: 'url' | 'lead_form' | 'sms' — no default). The old statement named `url` (42703) and
    // omitted `action_type` (23502), so no button could ever be created. The SPA posts
    // {label, url}; the DB names are accepted too.
    let url = body["url"]
        .as_str()
        .or_else(|| body["destination_url"].as_str())
        .unwrap_or("");
    let action_type = body["action_type"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("url");
    let sort = body["sort_order"].as_i64().unwrap_or(0) as i32;
    sqlx::query(
        "INSERT INTO kinetic_buttons (id, card_id, label, action_type, destination_url, sort_order) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(id)
    .bind(card_id)
    .bind(label)
    .bind(action_type)
    .bind(url)
    .bind(sort)
    .execute(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

/// Set / clear a card's viewer password (plan feature `has_card_gating`).
///
/// `{"password": "..."}` stores an argon2 PHC hash; an empty or absent password
/// clears the gate (NULL). The stored hash is what `render_card` compares against
/// at serve time and what `card_unlock_token` binds the unlock cookie to, so
/// rotating the password invalidates every outstanding unlock cookie.
pub async fn set_card_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Selling point of Suite/Agency — a plan without it gets 402 UpgradeRequired
    // rather than a silently ignored write.
    crate::features::enforce_feature_flag(&state, tenant_id, "has_card_gating", "Card gating")
        .await?;
    // Same ownership check create_button uses: never touch another tenant's card.
    let owns_card: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kinetic_cards WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(false);
    if !owns_card {
        return Err(AppError::NotFound("Card not found".into()));
    }

    let password = body["password"].as_str().unwrap_or("").trim();
    let new_hash: Option<String> = if password.is_empty() {
        None
    } else {
        if password.chars().count() < 8 {
            return Err(AppError::BadRequest(
                "Password must be at least 8 characters".into(),
            ));
        }
        // Same hashing scheme as tenant/user auth (argon2 PHC string).
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
            .to_string();
        Some(hash)
    };

    sqlx::query("UPDATE kinetic_cards SET password_hash = $3 WHERE id = $1 AND tenant_id = $2")
        .bind(card_id)
        .bind(tenant_id)
        .bind(new_hash.as_deref())
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({
        "id": card_id,
        "password_protected": new_hash.is_some(),
        "message": if new_hash.is_some() { "Card password set" } else { "Card password cleared" }
    })))
}

pub async fn delete_button(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query(
        "DELETE FROM kinetic_buttons b USING kinetic_cards c WHERE b.id=$1 AND b.card_id = c.id AND c.tenant_id=$2",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"message": "Deleted"})))
}

pub async fn get_metrics(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"views":0,"clicks":0,"leads":0})))
}

pub async fn get_subdomain(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // NOTE: this used to read a `settings` table that does not exist in this database
    // ('relation "settings" does not exist'), so GET and PUT both 500'd and no tenant could
    // ever claim a subdomain. tenant_settings is the real table; value is jsonb.
    let row = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(value #>> '{}', '') FROM tenant_settings WHERE tenant_id=$1 AND key='subdomain'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(json!({"subdomain": row.unwrap_or_default()})))
}
pub async fn set_subdomain(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let val = body["subdomain"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if val.is_empty() {
        return Err(AppError::BadRequest("Subdomain is required".into()));
    }
    if val.len() < 3
        || val.len() > 63
        || !val
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || val.starts_with('-')
        || val.ends_with('-')
    {
        return Err(AppError::BadRequest(
            "Subdomain must be 3-63 chars of lowercase letters, numbers, or hyphens (cannot start or end with a hyphen)".into(),
        ));
    }
    // Reserved names: hostnames that already mean something on kntcrd.com (or would break the
    // zone) must never be claimable by a tenant — otherwise a user could take `www`/`admin`
    // and shadow infrastructure hostnames.
    const RESERVED: [&str; 24] = [
        "www",
        "admin",
        "api",
        "app",
        "mail",
        "smtp",
        "imap",
        "ftp",
        "ns",
        "ns1",
        "ns2",
        "cdn",
        "static",
        "assets",
        "dashboard",
        "portal",
        "support",
        "help",
        "billing",
        "status",
        "test",
        "dev",
        "kntcrd",
        "funnelswift",
    ];
    if RESERVED.contains(&val.as_str()) {
        return Err(AppError::Conflict(format!(
            "'{val}' is reserved — pick another subdomain"
        )));
    }
    // Reject if another tenant already claimed this subdomain.
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tenant_settings WHERE key = 'subdomain' AND value #>> '{}' = $1 AND tenant_id != $2)",
    )
    .bind(&val)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    if taken {
        return Err(AppError::Conflict("Subdomain already in use".into()));
    }
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value, created_at, updated_at)
         VALUES ($1,$2,'subdomain', to_jsonb($3::text), NOW(), NOW())
         ON CONFLICT (tenant_id, key) DO UPDATE SET value = to_jsonb($3::text), updated_at = NOW()",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(&val)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        json!({"message": "Saved", "subdomain": val, "url": format!("https://{val}.kntcrd.com")}),
    ))
}
pub async fn get_custom_domain(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // tenant_settings, not the non-existent `settings` table (same bug as the subdomain pair).
    let row = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(value #>> '{}', '') FROM tenant_settings WHERE tenant_id=$1 AND key='custom_domain'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(json!({"custom_domain": row.unwrap_or_default()})))
}
pub async fn set_custom_domain(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // `max_custom_domains` counts the tenant's custom-domain SETTING, and that setting is a
    // singleton row (`tenant_settings` key='custom_domain'), so the count is 0 or 1 — never a
    // collection. Enforcing the limit unconditionally refused the tenant's own second write:
    // measured on the pre-fix binary, a kinetic-pro tenant (max_custom_domains=1) that had claimed
    // its one domain got 402 `Custom domains limit reached (1/1)` when CHANGING it and when
    // CLEARING it, so the setting was frozen for as long as the plan lasted. The limit is a
    // first-claim gate, the same shape WorkflowSwift's `set_account_industry` already uses
    // (`if is_new { enforce… }`): it applies only while this tenant has no domain to manage.
    let has_domain: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tenant_settings WHERE tenant_id=$1 AND key='custom_domain' AND value IS NOT NULL)",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    if !has_domain {
        crate::features::enforce_feature_limit(
            &state,
            tenant_id,
            "max_custom_domains",
            "Custom domains",
        )
        .await?;
    }
    let val = body["custom_domain"].as_str().unwrap_or("");
    if !val.is_empty()
        && (val.contains("://")
            || val.contains('/')
            || !val.contains('.')
            || val.starts_with('.')
            || val.ends_with('.'))
    {
        return Err(AppError::BadRequest("Invalid custom domain".into()));
    }
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value, created_at, updated_at)
         VALUES ($1,$2,'custom_domain', to_jsonb($3::text), NOW(), NOW())
         ON CONFLICT (tenant_id, key) DO UPDATE SET value = to_jsonb($3::text), updated_at = NOW()",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(val)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"message": "Saved", "custom_domain": val})))
}

pub async fn get_site_meta(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let row = sqlx::query_as::<_, (Value,)>(
        "SELECT value FROM tenant_settings WHERE tenant_id=$1 AND key='site_meta'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(row.map(|r| r.0).unwrap_or(json!({}))))
}

pub async fn set_site_meta(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let allowed_keys = [
        "og_title",
        "og_description",
        "og_image",
        "favicon_url",
        "twitter_handle",
        "google_analytics_id",
        "facebook_pixel_id",
        "theme_color",
    ];
    let mut cleaned = serde_json::Map::new();
    if let Some(obj) = body.as_object() {
        for k in allowed_keys {
            if let Some(v) = obj.get(k) {
                cleaned.insert(k.to_string(), v.clone());
            }
        }
    }
    let val = serde_json::Value::Object(cleaned);
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1,$2,'site_meta',$3) ON CONFLICT (tenant_id, key) DO UPDATE SET value=$3"
    ).bind(Uuid::new_v4()).bind(tenant_id).bind(&val).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Saved"})))
}

pub async fn render_card(
    axum::extract::Path(slug): axum::extract::Path<String>,
    axum::extract::Host(host): axum::extract::Host,
    OriginalUri(uri): OriginalUri,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    use sqlx::Row;
    // Extract URL prefix (k/b/c/m/f/h) from the request path
    let path = uri.path().to_string();
    let prefix = path.split('/').nth(1).unwrap_or("k");
    let prefix = if prefix.is_empty() || prefix == slug {
        "k"
    } else {
        prefix
    };
    let row = sqlx::query(
        // tenant_plan_subscriptions is the real table — this query used to join a
        // `tenant_plans` table that does not exist, and the error was swallowed by
        // `.unwrap_or(None)` below, so EVERY card page rendered "Card not found"
        // (with HTTP 200) and no Kinetic card has ever been publicly viewable.
        "SELECT k.id, k.password_hash, k.consent_required, k.age_gate_type, k.age_gate_message, k.consent_decline_redirect, k.tenant_id, k.title, k.slug, k.bio, k.bg_color, k.accent_color, k.text_color, k.tagline, k.meta_description, k.avatar_url, k.template_type, k.video_provider, k.video_id, k.layout_blocks, k.created_at, k.updated_at, t.affiliate_code, t.settings as tenant_settings, COALESCE(p.features->>'white_label','false') as white_label, COALESCE(p.features->>'remove_branding','false') as remove_branding FROM kinetic_cards k LEFT JOIN tenants t ON t.id = k.tenant_id LEFT JOIN tenant_plan_subscriptions tp ON tp.tenant_id = k.tenant_id AND tp.status = 'active' LEFT JOIN plans p ON p.id = tp.plan_id WHERE k.slug = $1 LIMIT 1"
    ).bind(&slug).fetch_optional(&state.pool).await.unwrap_or_else(|e| {
        // Never swallow this again: a failed lookup is indistinguishable from a missing
        // card on the public page, which is exactly how the phantom-table bug hid.
        tracing::error!("render_card lookup failed for slug '{}': {}", slug, e);
        None
    });

    // ── Load global SEO settings for SSO injection ──
    // site_settings.value is NULLABLE with no DEFAULT and this tuple's element is a plain
    // String, so a single NULL row broke the whole read (silently: unwrap_or_default).
    // kanban t_98f5292f.
    let seo_rows: Vec<(String, Value)> = sqlx::query_as::<_, (String, String)>(
        "SELECT COALESCE(key, ''), COALESCE(value::text, '') FROM site_settings WHERE key LIKE 'seo_%'",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| tracing::error!("render_card seo settings query failed: {:?}", e))
    .unwrap_or_default()
    .into_iter()
    .map(|(k, raw)| {
        (
            k,
            crate::handlers::site_settings_handler::value_from_text(&raw),
        )
    })
    .collect();
    let mut seo_meta = String::new();
    let mut seo_scripts = String::new();
    for (k, v) in &seo_rows {
        let short = k.strip_prefix("seo_").unwrap_or(k);
        match short {
            "site_name" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!(
                        "<meta property=\"og:site_name\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "description" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta name=\"description\" content=\"{}\">\n", s));
                    seo_meta.push_str(&format!(
                        "<meta property=\"og:description\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "keywords" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta name=\"keywords\" content=\"{}\">\n", s));
                }
            }
            "og_image" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta property=\"og:image\" content=\"{}\">\n", s));
                    seo_meta.push_str(&format!(
                        "<meta property=\"twitter:image\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "twitter_handle" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta name=\"twitter:site\" content=\"{}\">\n", s));
                    seo_meta.push_str(&format!(
                        "<meta name=\"twitter:creator\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "site_verification" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!(
                        "<meta name=\"google-site-verification\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "google_analytics" => {
                if let Some(s) = v.as_str() {
                    seo_scripts.push_str(&format!("<script async src=\"https://www.googletagmanager.com/gtag/js?id={}\"></script><script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{}');</script>\n", s, s));
                }
            }
            "facebook_pixel" => {
                if let Some(s) = v.as_str() {
                    seo_scripts.push_str(&format!("<script>!function(f,b,e,v,n,t,s){{if(f.fbq)return;n=f.fbq=function(){{n.callMethod?n.callMethod.apply(n,arguments):n.queue.push(arguments)}};if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';n.queue=[];t=b.createElement(e);t.async=!0;t.src=v;s=b.getElementsByTagName(e)[0];s.parentNode.insertBefore(t,s)}}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','{}');fbq('track','PageView');</script><noscript><img height=\"1\" width=\"1\" src=\"https://www.facebook.com/tr?id={}&ev=PageView&noscript=1\"/></noscript>\n", s, s));
                }
            }
            "schema_type" => {
                let schema_json = serde_json::to_string(v).unwrap_or_default();
                seo_scripts.push_str(&format!(
                    "<script type=\"application/ld+json\">{}</script>\n",
                    schema_json
                ));
            }
            _ => {}
        }
    }
    // Twitter card type always set
    seo_meta.push_str("<meta property=\"twitter:card\" content=\"summary_large_image\">\n");
    // Canonical URL — resolves from Host header (tenant.kntcrd.com → canonical, custom domain, or fallback)
    let canonical = html_escape(&resolve_canonical_url(&host, prefix, &slug));
    seo_meta.push_str(&format!(
        "<link rel=\"canonical\" href=\"{}\">\n",
        canonical
    ));

    // Font preconnect for speed
    seo_meta.push_str("<link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">\n<link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\n");

    if row.is_none() {
        return axum::response::Html("<html><body style='background:#0f172a;color:#fff;display:flex;align-items:center;justify-content:center;height:100vh;font-family:sans-serif'><div style='text-align:center'><h1 style='font-size:48px;margin-bottom:8px'>404</h1><p>Card not found</p><a href='https://funnelswift.net/kinetic' style='color:#a855f7'>Create your own →</a></div></body></html>".to_string());
    }

    let r = row.unwrap();
    // ── Load tenant-level site meta (overrides card-level OG tags for their subdomain) ──
    let tenant_id_for_meta: Uuid = r.try_get::<Uuid, _>("tenant_id").unwrap_or_default();
    let tenant_site_meta: Value = sqlx::query_as::<_, (Value,)>(
        "SELECT value FROM tenant_settings WHERE tenant_id=$1 AND key='site_meta'",
    )
    .bind(tenant_id_for_meta)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None)
    .map(|r| r.0)
    .unwrap_or(json!({}));
    // kanban t_747b4dd6: the card TYPE (`kinetic_cards.template_type`) is deliberately NOT read in
    // this renderer. The public page draws from `theme` / the colour columns / `layout_blocks`, and
    // the per-archetype styling lives in the served SPA's editor preview; wiring a kind-dependent
    // layout here would invent rendering behaviour, so the dead binding that used to sit on this
    // line (`let _template_type: String = …`, read by nothing) was DELETED, not wired. The column's
    // value space is declared once in `crate::card_types`.
    // Branding badge logic:
    //   the plan grants badge removal via `remove_branding` (Kinetic Pro, Suite, Agency/Scale)
    //   OR via `white_label`; the tenant's site_meta.hide_branding_badge is the actual switch.
    //   Otherwise the badge is forced ON (free plans cannot hide it).
    // Before this, the check required white_label alone — an Agency/Scale-only flag — so
    // Kinetic Pro customers who had paid specifically for "hide branding" could never remove
    // the badge. A granted, paid-for feature was silently not delivered.
    let is_white_label: String = r.try_get("white_label").unwrap_or_else(|_| "false".into());
    let has_remove_branding: String = r
        .try_get("remove_branding")
        .unwrap_or_else(|_| "false".into());
    let can_hide_badge = is_white_label == "true" || has_remove_branding == "true";
    let tenant_settings: Option<Value> = r.try_get("tenant_settings").unwrap_or(None);
    let tenant_hides_badge = can_hide_badge
        && tenant_settings
            .as_ref()
            .and_then(|s| s.get("hide_branding_badge"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let show_branding = !tenant_hides_badge; // always show unless explicitly hidden
    let affiliate_code: Option<String> = r.try_get("affiliate_code").unwrap_or(None);
    let title: String = html_escape(&r.try_get::<String, _>("title").unwrap_or_default());
    let bio: String = html_escape(&r.try_get::<String, _>("bio").unwrap_or_default());
    let bg: String = r.try_get("bg_color").unwrap_or_default();
    let accent: String = r.try_get("accent_color").unwrap_or_default();
    let text: String = r.try_get("text_color").unwrap_or_default();
    let avatar: Option<String> = r
        .try_get::<Option<String>, _>("avatar_url")
        .unwrap_or(None)
        .map(|a| html_escape(&a));
    let tagline: Option<String> = r
        .try_get::<Option<String>, _>("tagline")
        .unwrap_or(None)
        .map(|t| html_escape(&t));
    let meta_desc: Option<String> = r
        .try_get::<Option<String>, _>("meta_description")
        .unwrap_or(None)
        .map(|d| html_escape(&d));
    let card_id: Uuid = r.try_get("id").unwrap_or_default();

    // ── Card-level password gate (plan feature `has_card_gating`) ──────────────────
    // Migration 024 added kinetic_cards.password_hash / consent_required / age_gate_*,
    // but nothing ever read them: the "Card Gating" feature sold on Suite + Agency was
    // never delivered. This honours the password half of it.
    // NULL or empty (the default for every pre-existing card) => no gate, so behaviour
    // is byte-identical for cards that never set a password.
    // NOTE: the *setter* (and the editor UI) is still open — see
    // /opt/swift/audits/funnelswift/FunnelSwift-feature-verification-2026-09-20.md.
    // The plan flag is enforced by the setter; rendering honours any hash that exists.
    if let Ok(Some(phc)) = r.try_get::<Option<String>, _>("password_hash") {
        if !phc.trim().is_empty() {
            let cookie_name = format!("kc_gate_{}", card_id.simple());
            let expected = card_unlock_token(&state.jwt_secret, &card_id, &phc);
            let presented = headers
                .get(axum::http::header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|raw| cookie_lookup(raw, &cookie_name));
            if presented.as_deref() != Some(expected.as_str()) {
                return axum::response::Html(render_password_gate(&title));
            }
        }
    }

    // ── Consent + age gate — the other half of the same `has_card_gating` feature ──
    // Migration 024 added consent_required / age_gate_type / age_gate_message /
    // consent_decline_redirect and nothing ever read them either, so the feature sold on
    // Suite + Agency shipped with only its password half. Every pre-existing card has
    // consent_required = false and age_gate_type NULL, which is why this whole block is
    // skipped for them and their pages render exactly as before.
    // Consent is evaluated first and the age gate only after it passes, so a card with
    // both is two sequential interstitials (consent -> reload -> age -> reload -> card).
    let consent_required: bool = r
        .try_get::<Option<bool>, _>("consent_required")
        .unwrap_or(None)
        .unwrap_or(false);
    let age_gate_type: String = r
        .try_get::<Option<String>, _>("age_gate_type")
        .unwrap_or(None)
        .unwrap_or_default()
        .trim()
        .to_string();
    // The only message column migration 024 defined; shown on whichever interstitial
    // renders when the owner set one, with a per-gate default otherwise.
    let gate_message: String = r
        .try_get::<Option<String>, _>("age_gate_message")
        .unwrap_or(None)
        .unwrap_or_default();
    // Never trust a stored redirect: `safe_redirect_target` accepts only http(s), so a
    // javascript:/data: value that somehow reached the column is rendered as "no target".
    let decline_redirect: String = r
        .try_get::<Option<String>, _>("consent_decline_redirect")
        .unwrap_or(None)
        .unwrap_or_default();
    let decline_target: Option<&str> = safe_redirect_target(&decline_redirect);

    // Interstitial copy: the owner's message when they set one, otherwise the per-gate
    // default (18+ and 21+ get their own wording).
    let consent_msg = if gate_message.trim().is_empty() {
        "We need your consent before showing this card.".to_string()
    } else {
        gate_message.clone()
    };
    let age_msg = if gate_message.trim().is_empty() {
        age_gate_default_message(&age_gate_type).to_string()
    } else {
        gate_message.clone()
    };

    // Consent gate — cookie value is bound to the gate's *configuration* (the configured
    // decline target), so reconfiguring the gate invalidates outstanding consent cookies
    // exactly like rotating a password invalidates the password cookie.
    if consent_required {
        let cookie_name = format!("kc_consent_{}", card_id.simple());
        let expected = card_unlock_token(
            &state.jwt_secret,
            &card_id,
            &consent_gate_signature(&decline_redirect),
        );
        let presented = headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|raw| cookie_lookup(raw, &cookie_name));
        if presented.as_deref() != Some(expected.as_str()) {
            return axum::response::Html(render_gate_interstitial(
                &title,
                "consent",
                &consent_msg,
                &card_id,
                decline_target,
            ));
        }
    }

    // Age gate — 'none' (the default) and any unexpected value mean "no age gate", so a
    // bad value can never silently keep rendering an interstitial.
    if is_age_gate_type(&age_gate_type) {
        let cookie_name = format!("kc_age_{}", card_id.simple());
        let expected = card_unlock_token(
            &state.jwt_secret,
            &card_id,
            &age_gate_signature(&age_gate_type),
        );
        let presented = headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|raw| cookie_lookup(raw, &cookie_name));
        if presented.as_deref() != Some(expected.as_str()) {
            return axum::response::Html(render_gate_interstitial(
                &title,
                "age",
                &age_msg,
                &card_id,
                decline_target,
            ));
        }
    }

    let _video_provider: Option<String> = r.try_get("video_provider").unwrap_or(None);
    let _video_id: Option<String> = r.try_get("video_id").unwrap_or(None);

    // Dynamic branding badge — uses prefix to determine card type label
    let (branding_cta, _card_label) = cta_for_prefix(prefix);
    let (branding_text, branding_url) = if show_branding {
        let url = if let Some(ref code) = affiliate_code {
            format!("https://funnelswift.net/kinetic?ref={}", code)
        } else {
            "https://funnelswift.net/kinetic".to_string()
        };
        (branding_cta.to_string(), url)
    } else {
        (String::new(), String::new())
    };
    let branding_html = if !branding_text.is_empty() {
        format!(
            "<a href='{}' class='branding-badge'>{}</a>",
            branding_url, branding_text
        )
    } else {
        String::new()
    };

    let av_html = if let Some(ref a) = avatar {
        if a.is_empty() {
            String::new()
        } else {
            format!(
                "<img src='{}' class='av' alt='' onerror=\"this.style.display='none'\">",
                a
            )
        }
    } else {
        String::new()
    };
    let tag_html = if let Some(ref t) = tagline {
        if t.is_empty() {
            String::new()
        } else {
            format!("<p class='tag'>{}</p>", t)
        }
    } else {
        String::new()
    };
    let bio_html = if !bio.is_empty() {
        format!("<p class='bio'>{}</p>", bio)
    } else {
        String::new()
    };
    let cta_html = String::new(); // CTA pulled from layout_blocks by front-end JS

    let bg_gradient = format!(
        "radial-gradient(circle at 50% 25%, {}44 0%, {} 70%)",
        accent, bg
    );

    let social_html = String::new(); // social links rendered by front-end JS from layout_blocks

    // ── Tenant site meta overrides for OG tags ──
    let og_title = html_escape(
        tenant_site_meta
            .get("og_title")
            .and_then(|v| v.as_str())
            .unwrap_or(&title),
    );
    let og_desc = html_escape(
        tenant_site_meta
            .get("og_description")
            .and_then(|v| v.as_str())
            .unwrap_or(meta_desc.as_deref().unwrap_or("")),
    );
    let og_image_html = tenant_site_meta.get("og_image")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("<meta property=\"og:image\" content=\"{}\">\n<meta property=\"twitter:image\" content=\"{}\">\n", s, s))
        .or_else(|| {
            let default_img = "<meta property=\"og:image\" content=\"https://funnelswift.net/assets/og-funnelswift-card.png\">\n<meta property=\"og:image:width\" content=\"1200\">\n<meta property=\"og:image:height\" content=\"630\">\n<meta property=\"twitter:image\" content=\"https://funnelswift.net/assets/og-funnelswift-card.png\">\n".to_string();
            Some(default_img)
        })
        .unwrap_or_default();
    let favicon_html = tenant_site_meta
        .get("favicon_url")
        .and_then(|v| v.as_str())
        .map(|s| format!("<link rel=\"icon\" href=\"{}\">\n", s))
        .unwrap_or_default();
    let ga_html = tenant_site_meta.get("google_analytics_id")
        .and_then(|v| v.as_str())
        .map(|s| format!("<script async src=\"https://www.googletagmanager.com/gtag/js?id={0}\"></script><script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{0}');</script>\n", s))
        .unwrap_or_default();
    let fb_html = tenant_site_meta.get("facebook_pixel_id")
        .and_then(|v| v.as_str())
        .map(|s| format!("<script>!function(f,b,e,v,n,t,s){{if(f.bq)return;n=f.bq=function(){{n.callMethod?n.callMethod.apply(n,arguments):n.queue.push(arguments)}};if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';n.queue=[];t=b.createElement(e);t.async=!0;t.src=v;s=b.getElementsByTagName(e)[0];s.parentNode.insertBefore(t,s)}}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','{0}');fbq('track','PageView');</script><noscript><img height=\"1\" width=\"1\" src=\"https://www.facebook.com/tr?id={0}&ev=PageView&noscript=1\"/></noscript>\n", s))
        .unwrap_or_default();
    let twitter_site_meta = tenant_site_meta.get("twitter_handle")
        .and_then(|v| v.as_str())
        .map(|s| format!("<meta name=\"twitter:site\" content=\"@{}\">\n<meta name=\"twitter:creator\" content=\"@{}\">\n", s, s))
        .unwrap_or_default();
    let theme_color_meta = tenant_site_meta
        .get("theme_color")
        .and_then(|v| v.as_str())
        .map(|s| format!("<meta name=\"theme-color\" content=\"{}\">\n", s))
        .unwrap_or_default();

    let page_title_display = og_title.to_string();

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{page_title_display}</title>
<meta name="description" content="{og_desc}">
<meta property="og:title" content="{page_title_display}">
<meta property="og:description" content="{og_desc}">
<meta property="og:type" content="website">
<meta property="og:url" content="{canonical}">
{og_image_html}
{favicon_html}
{twitter_site_meta}
{theme_color_meta}
{seo_meta}
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:'Plus Jakarta Sans','Inter',system-ui,-apple-system,sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;padding:24px 16px;color:{text};overflow-x:hidden}}
.bg{{position:fixed;inset:0;z-index:-1;background:{bg_gradient}}}
.noise{{position:fixed;inset:0;z-index:0;opacity:.035;background-image:url("data:image/svg+xml,%3Csvg viewBox='0 0 256 256' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.8' numOctaves='4' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");pointer-events:none}}
.glow{{position:fixed;top:40%;left:50%;transform:translate(-50%,-50%);width:200px;height:200px;border-radius:50%;background:radial-gradient(circle,{accent}44 0%,transparent 70%);pointer-events:none;z-index:0}}
.card{{position:relative;z-index:2;max-width:360px;width:100%;text-align:center;display:flex;flex-direction:column;align-items:center;gap:12px;animation:fadeIn .6s ease-out}}
.av{{width:80px;height:80px;border-radius:50%;object-fit:cover;box-shadow:0 0 0 4px {accent},0 0 24px {accent}66;animation:pulse-glow 2.5s ease-in-out infinite}}
h1{{font-size:26px;font-weight:800;text-shadow:0 2px 8px rgba(0,0,0,.3)}}
.tag{{font-size:14px;color:{accent};font-weight:600;letter-spacing:.5px}}
.bio{{font-size:14px;line-height:1.6;opacity:.85;max-width:300px}}
.cta{{display:inline-block;padding:14px 36px;background:{accent};color:{text};border-radius:14px;font-size:15px;font-weight:700;text-decoration:none;box-shadow:0 4px 18px {accent}44;transition:all .2s;margin-top:4px}}
.cta:hover{{transform:translateY(-2px);box-shadow:0 6px 24px {accent}66}}
.socials{{display:flex;gap:10px;flex-wrap:wrap;justify-content:center;margin-top:4px}}
.s-icon{{display:inline-flex;align-items:center;gap:4px;padding:7px 16px;background:rgba(255,255,255,.08);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px);border:1px solid rgba(255,255,255,.12);border-radius:20px;font-size:12px;color:{text};text-decoration:none;transition:all .2s}}
.s-icon:hover{{background:rgba(255,255,255,.15);border-color:rgba(255,255,255,.25)}}
.branding-badge{{position:absolute;bottom:16px;right:16px;font-size:11px;padding:6px 14px;border-radius:100px;background:rgba(255,255,255,.1);backdrop-filter:blur(12px);-webkit-backdrop-filter:blur(12px);border:1px solid rgba(255,255,255,.15);color:{text};text-decoration:none;z-index:10;transition:all .2s;font-weight:600;letter-spacing:.3px}}
.branding-badge:hover{{background:rgba(255,255,255,.18);border-color:{accent};color:{accent};transform:translateY(-1px);box-shadow:0 4px 16px {accent}33}}
@keyframes fadeIn{{from{{opacity:0;transform:translateY(16px)}}to{{opacity:1;transform:translateY(0)}}}}
@keyframes pulse-glow{{0%,100%{{box-shadow:0 0 0 4px {accent},0 0 24px {accent}66}}50%{{box-shadow:0 0 0 5px {accent},0 0 36px {accent}88}}}}
</style>
{seo_scripts}
{ga_html}
{fb_html}
<link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<div class="bg"></div>
<div class="noise"></div>
<div class="glow"></div>
{branding_html}
<div class="card">
{av_html}
<h1>{title}</h1>
{tag_html}
{bio_html}
{cta_html}
{social_html}
</div>
<script>
(function(){{try{{var c="{card_id_tracker}";if(!c||c.length<10)return;var a="https://funnelswift.net";var u=navigator.userAgent||"";if(/bot|crawler|spider/i.test(u))return;function t(e,x){{var b={{event_type:e||"view",user_agent:u.substring(0,500),referrer_url:document.referrer||"",device_type:screen.width<768?"mobile":screen.width<1024?"tablet":"desktop",screen_size:(screen.width||0)+"x"+(screen.height||0)}};var p=new URLSearchParams(location.search);["utm_source","utm_medium","utm_campaign","utm_content","utm_term"].forEach(function(k){{var v=p.get(k);if(v)b[k]=v}});if(x)Object.assign(b,x);var r=new XMLHttpRequest();r.open("POST",a+"/card/"+c+"/track",!0);r.setRequestHeader("Content-Type","application/json");r.send(JSON.stringify(b))}}setTimeout(function(){{t("view")}},100);document.addEventListener("visibilitychange",function(){{document.visibilityState==="hidden"&&t("leave")}});document.querySelectorAll("a[href]").forEach(function(e){{e.addEventListener("click",function(){{t("click",{{click_label:(e.textContent||"").trim().substring(0,100),click_url:e.getAttribute("href")||""}})}})}})}}catch(e){{}})}})();
</script>
</body>
</html>"#,
        page_title_display = page_title_display,
        og_desc = og_desc,
        text = text,
        bg_gradient = bg_gradient,
        accent = accent,
        title = title,
        card_id_tracker = card_id,
        canonical = canonical,
        seo_meta = seo_meta,
        seo_scripts = seo_scripts,
        og_image_html = og_image_html,
        favicon_html = favicon_html,
        twitter_site_meta = twitter_site_meta,
        theme_color_meta = theme_color_meta,
        ga_html = ga_html,
        fb_html = fb_html,
        av_html = av_html,
        tag_html = tag_html,
        bio_html = bio_html,
        cta_html = cta_html,
        social_html = social_html,
        branding_html = branding_html
    ))
}
/// The card `lead_form`'s field names this handler stores by hand. A form configures its own
/// fields (`layout_blocks[].fields`), so every name NOT in here is kept in `custom_fields.extra`
/// rather than being dropped on the floor.
const CARD_LEAD_NAMED_FIELDS: [&str; 12] = [
    "name",
    "first_name",
    "last_name",
    "email",
    "phone",
    "company",
    "message",
    "notes",
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "referrer_url",
];

/// A trimmed, non-empty string field of a card form submission.
fn card_lead_field(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The lead's display name (`leads.name` is NOT NULL): the form's `name`, else
/// `first_name`/`last_name`, else the email local part — one shipped `lead_form` template
/// configures an email-only waitlist form, and a nameless row is worse than a derived one.
fn card_lead_name(body: &Value, email: Option<&str>) -> Option<String> {
    let joined = [
        card_lead_field(body, "first_name"),
        card_lead_field(body, "last_name"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    card_lead_field(body, "name")
        .or_else(|| (!joined.is_empty()).then_some(joined))
        .or_else(|| {
            email?
                .split('@')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

/// Every field the form configured that this handler does not name by hand.
fn card_lead_extra_fields(body: &Value) -> serde_json::Map<String, Value> {
    body.as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| !CARD_LEAD_NAMED_FIELDS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Public card lead capture — `POST /{k,b,m,c,f,h}/:slug/lead`.
///
/// kanban t_c7673a05. This used to be a no-op constant that bound neither the state nor the
/// body — `Json(json!({"message":"Lead submitted","slug":slug}))` — so every submission from a
/// card's `lead_form` block answered a success and was never recorded, while the live
/// funnelswift.net vhost proxies this path (`location ~ ^/(k|b|c|f|m|h)/`) and five live cards
/// already carry a configured `lead_form` layout block. It is now implemented against the app's
/// real public capture path (`web_to_lead_handler::handle_web_to_lead`): the durable row lands
/// in `leads` with `source = 'kinetic_card'`, the raw card event lands in `lead_events`
/// (migration 0017's card-lead event table: `card_id` + `lead_id` + `event_type='form_submit'`),
/// and the tenant's inbound CoreSwift push is fired.
///
/// The card — and with it the tenant — is resolved from the slug, never from the body: all six
/// prefixes render the same card, so a body-supplied `tenant_id` would be a cross-tenant write.
/// `leads.created_by` takes the card owner, which is the affiliate attribution anchor
/// (`leads.created_by -> affiliates.user_id`), so a card lead attributes to the card's owner.
///
/// `leads.name` is NOT NULL: the display name comes from `name`, else `first_name`/`last_name`,
/// else the email local part (one shipped `lead_form` template configures an email-only waitlist
/// form), and a submission with no identity at all is refused with a 400 rather than stored as a
/// nameless row. No plan gate and no duplicate-email refusal, deliberately: the sibling public
/// capture path (`/api/v1/web-to-lead`) has neither, and a public form that silently refuses a
/// visitor because of a quota is the same defect class this card closes.
pub async fn submit_lead(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let card: Option<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("SELECT id, tenant_id, user_id FROM kinetic_cards WHERE slug = $1 LIMIT 1")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await?;
    let Some((card_id, tenant_id, owner_id)) = card else {
        return Err(AppError::NotFound("Card not found".into()));
    };

    let email = card_lead_field(&body, "email");
    let Some(name) = card_lead_name(&body, email.as_deref()) else {
        return Err(AppError::Validation(
            "A card lead needs a name or an email".into(),
        ));
    };
    let phone = card_lead_field(&body, "phone");
    let company = card_lead_field(&body, "company");
    let notes = card_lead_field(&body, "message").or_else(|| card_lead_field(&body, "notes"));
    let utm_source = card_lead_field(&body, "utm_source");
    let utm_medium = card_lead_field(&body, "utm_medium");
    let utm_campaign = card_lead_field(&body, "utm_campaign");
    let referrer_url = card_lead_field(&body, "referrer_url");

    let custom_fields = json!({
        "card_id": card_id.to_string(),
        "card_slug": slug,
        "utm_source": &utm_source,
        "utm_medium": &utm_medium,
        "utm_campaign": &utm_campaign,
        "referrer_url": &referrer_url,
        "extra": Value::Object(card_lead_extra_fields(&body)),
    });

    // `kinetic_cards.user_id` is NOT an FK to `users(id)` and 25 of the 27 live cards carry an
    // owner id with no `users` row behind it, so binding it straight into `leads.created_by`
    // (which IS an FK to `users(id)`) raised 23503 and 500'd every submission — found by the
    // first live probe. Resolve it once: a real user is used, a dangling id becomes NULL, which
    // is also what the app's other public capture path (`/api/v1/web-to-lead`) leaves there.
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
        .bind(owner_id)
        .fetch_optional(&state.pool)
        .await?;

    let lead_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO leads (id, tenant_id, name, email, phone, company, source, status, notes, custom_fields, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, 'kinetic_card', 'new', $7, $8, $9)"#,
    )
    .bind(lead_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(&email)
    .bind(&phone)
    .bind(&company)
    .bind(&notes)
    .bind(&custom_fields)
    .bind(owner)
    .execute(&state.pool)
    .await?;

    // The raw card event (migration 0017's `lead_events`) — this is what ties the lead to the
    // card it was submitted from. `source_param` carries the traffic source, as in the table's
    // own historical rows ('ig'/'fb'); `ip_hash` stays NULL exactly like those rows. `user_id` is
    // NOT NULL and has no FK here, so it keeps the card's recorded owner even when that user row
    // is gone (the durable `leads` row above is what the FK-constrained column follows).
    sqlx::query(
        "INSERT INTO lead_events (id, user_id, tenant_id, lead_id, card_id, event_type, source_param) VALUES ($1, $2, $3, $4, $5, 'form_submit', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(owner_id)
    .bind(tenant_id)
    .bind(lead_id)
    .bind(card_id)
    .bind(
        utm_source
            .as_deref()
            .map(|s| s.chars().take(30).collect::<String>()),
    )
    .execute(&state.pool)
    .await?;

    // INBOUND CoreSwift push (fleet standard 2026-09-20 §R2): every captured opt-in must be able
    // to land in CoreSwift as a contact. Fire-and-forget, a no-op without a `coreswift` BYOK key.
    crate::coreswift::spawn_lead_push(
        state.pool.clone(),
        tenant_id,
        crate::coreswift::LeadPayload {
            email,
            phone,
            name: Some(name),
            company,
            source: Some("kinetic_card".to_string()),
            ..Default::default()
        },
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": lead_id.to_string(), "message": "Lead captured", "card": slug})),
    ))
}
// track_click (GET /track/click) was removed here — kanban t_65849ce9. It was a no-op stub
// that persisted nothing and answered a constant `{}`, with zero callers in src/ or in any
// served www*/ root. Click tracking is POST /card/:id/track (event_type=click), handled by
// card_analytics_handler::track_card_event.

// ─────────────────────────────────────────────────────────────────────────────
// Card password gate (`has_card_gating` — Suite + Agency)
// ─────────────────────────────────────────────────────────────────────────────

/// Stateless unlock token: HMAC-SHA256(jwt_secret, "<card_id>:<password_hash>").
/// Bound to the card AND to the current hash, so rotating a card's password
/// invalidates every outstanding unlock cookie. The password itself never leaves
/// the server, and the token is not reversible into the stored hash.
pub fn card_unlock_token(secret: &str, card_id: &Uuid, password_hash: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    let msg = format!("{}:{}", card_id.simple(), password_hash);
    hex::encode(ring::hmac::sign(&key, msg.as_bytes()).as_ref())
}

/// Pull one cookie value out of a raw `Cookie:` header.
fn cookie_lookup(raw: &str, name: &str) -> Option<String> {
    raw.split(';').find_map(|part| {
        let mut it = part.trim().splitn(2, '=');
        match (it.next(), it.next()) {
            (Some(k), Some(v)) if k == name => Some(v.to_string()),
            _ => None,
        }
    })
}

/// Verify a card password against an argon2 PHC string (project standard) or a
/// bcrypt hash (what migration 024's comment assumed). Returns false — never an
/// error — for an unparseable/unknown hash format.
pub fn verify_card_password(password: &str, stored: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    if let Ok(parsed) = PasswordHash::new(stored) {
        if argon2::Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
        {
            return true;
        }
    }
    bcrypt::verify(password, stored).unwrap_or(false)
}

/// HTML served instead of a gated card. The form posts JSON to `<current
/// path>/unlock` (built from `location.pathname`, so nothing from the URL is
/// interpolated into the script) and reloads on success.
fn render_password_gate(title: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex,nofollow">
<title>{title}</title>
<style>
*{{box-sizing:border-box}}body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
background:#0f172a;color:#e5e7eb;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;padding:24px}}
.box{{width:100%;max-width:380px;background:#111827;border:1px solid #1f2937;border-radius:16px;padding:28px;text-align:center}}
h1{{font-size:20px;margin:0 0 6px}}p{{color:#9ca3af;font-size:14px;margin:0 0 18px}}
input{{width:100%;padding:12px 14px;border-radius:10px;border:1px solid #374151;background:#0b1220;color:#e5e7eb;font-size:15px}}
button{{width:100%;margin-top:12px;padding:12px;border:0;border-radius:10px;background:#a855f7;color:#fff;font-size:15px;font-weight:600;cursor:pointer}}
.err{{color:#f87171;font-size:13px;margin-top:12px}}
</style></head>
<body><div class="box">
<h1>{title}</h1>
<p>This card is password protected. Enter the password to view it.</p>
<form id="gate"><input id="pw" type="password" autocomplete="current-password" placeholder="Password" required autofocus>
<button type="submit">Unlock</button></form>
<p id="err" class="err" style="display:none">Incorrect password. Try again.</p>
</div>
<script>
(function(){{var f=document.getElementById('gate'),e=document.getElementById('err');
f.addEventListener('submit',function(ev){{ev.preventDefault();
var u=location.pathname.replace(/\/+$/,'')+'/unlock';
fetch(u,{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{password:document.getElementById('pw').value}})}})
.then(function(r){{if(r.ok){{location.reload()}}else{{e.style.display='block'}}}})
.catch(function(){{e.style.display='block'}});}});}})();
</script>
</body></html>"#,
        title = title
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Consent + age gate (the other half of `has_card_gating` — Suite + Agency)
// ─────────────────────────────────────────────────────────────────────────────

/// The age gate values the renderer honours. 'none' (the column default, i.e. NULL)
/// and anything outside this allowlist mean "no age gate" — the setter 400s on a bad
/// value precisely so a typo can never leave a gate silently not rendering.
fn is_age_gate_type(t: &str) -> bool {
    matches!(t, "age_18" | "age_21" | "custom")
}

/// Default copy for an age gate the owner gave no message for.
fn age_gate_default_message(t: &str) -> &'static str {
    match t {
        "age_21" => "You must be 21 or over to view this card.",
        "age_18" => "You must be 18 or over to view this card.",
        _ => "Please confirm you are old enough to view this card.",
    }
}

/// Configuration binding for the consent cookie: the gate's own configuration is part
/// of the HMAC input, so reconfiguring the gate invalidates every outstanding consent
/// cookie — the property the password cookie gets from hashing the password.
fn consent_gate_signature(decline_redirect: &str) -> String {
    format!("consent:{}", decline_redirect.trim())
}

/// Configuration binding for the age cookie (its configuration is the gate type).
fn age_gate_signature(age_gate_type: &str) -> String {
    format!("age:{}", age_gate_type.trim())
}

/// Accept a decline-redirect target only when it is an absolute http(s) URL.
/// `javascript:` and `data:` are an XSS sink the moment the interstitial follows the
/// value, and a bare/relative path is not something the owner deliberately configured.
/// Whitespace and control characters are refused too, so the value can never break out
/// of the attribute it is rendered into. Returns the trimmed URL when it is safe.
pub fn safe_redirect_target(raw: &str) -> Option<&str> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 2048 {
        return None;
    }
    if t.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let scheme_ok = ["https://", "http://"]
        .iter()
        .any(|p| lower.starts_with(p) && t.len() > p.len());
    if scheme_ok {
        Some(t)
    } else {
        None
    }
}

/// Canonicalise an incoming `age_gate_type`: `none`/empty clear the gate (NULL), the
/// three real types are stored verbatim, everything else is a 400 so that a bad value
/// can never be persisted (which is how a gate silently stops rendering).
fn normalize_age_gate_type(v: &str) -> AppResult<Option<String>> {
    match v.trim() {
        "" | "none" => Ok(None),
        s if is_age_gate_type(s) => Ok(Some(s.to_string())),
        _ => Err(AppError::BadRequest(
            "age_gate_type must be one of none, age_18, age_21, custom".into(),
        )),
    }
}

/// Read an optional string field, distinguishing "absent" from "wrong type" — a
/// non-string value is a client bug worth reporting, not something to ignore.
fn optional_json_str<'a>(body: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(AppError::BadRequest(format!("{key} must be a string"))),
    }
}

/// Read an optional boolean field (same absent-vs-wrong-type distinction).
fn optional_json_bool(body: &Value, key: &str) -> AppResult<Option<bool>> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(AppError::BadRequest(format!("{key} must be true or false"))),
    }
}

/// HTML served instead of a consent- or age-gated card.
///
/// `title` arrives already html-escaped (as with `render_password_gate`); everything
/// else is escaped here. The confirm button POSTs `{"gate": <gate>}` to the public,
/// id-addressed gate route and reloads. Decline follows the owner's redirect when one
/// is configured and validated, otherwise it shows a neutral "nothing to see" panel.
/// No owner-supplied text ever reaches the script: the only interpolated attribute is
/// an already-validated absolute http(s) URL.
fn render_gate_interstitial(
    title: &str,
    gate: &str,
    message: &str,
    card_id: &Uuid,
    decline_target: Option<&str>,
) -> String {
    // `gate` is a literal from the two call sites below.
    let (heading, confirm_label) = if gate == "age" {
        ("Age verification", "Yes, I'm old enough")
    } else {
        ("Before you continue", "Continue")
    };
    let action = format!("/api/v1/kinetic/cards/{}/gate", card_id);
    let redirect_attr = match decline_target {
        Some(url) => format!(" data-redirect=\"{}\"", html_escape(url)),
        None => String::new(),
    };
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex,nofollow">
<title>{title}</title>
<style>
*{{box-sizing:border-box}}body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
background:#0f172a;color:#e5e7eb;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;padding:24px}}
.box{{width:100%;max-width:380px;background:#111827;border:1px solid #1f2937;border-radius:16px;padding:28px;text-align:center}}
h1{{font-size:20px;margin:0 0 6px}}p{{color:#9ca3af;font-size:14px;margin:0 0 18px}}
button{{width:100%;margin-top:12px;padding:12px;border:0;border-radius:10px;background:#a855f7;color:#fff;font-size:15px;font-weight:600;cursor:pointer}}
button.ghost{{background:transparent;border:1px solid #374151;color:#9ca3af;font-weight:500}}
.decl{{color:#9ca3af;font-size:14px;margin:0}}
.err{{color:#f87171;font-size:13px;margin-top:12px}}
</style></head>
<body><div class="box"{redirect_attr}>
<h1>{heading}</h1>
<p>{message}</p>
<button id="acc" type="button">{confirm_label}</button>
<button id="dec" type="button" class="ghost">No thanks</button>
<p id="err" class="err" style="display:none">Something went wrong. Please try again.</p>
<div id="declined" style="display:none"><p class="decl">No problem — this card will not be shown.</p></div>
</div>
<script>
(function(){{var box=document.querySelector('.box'),acc=document.getElementById('acc'),
dec=document.getElementById('dec'),err=document.getElementById('err'),dn=document.getElementById('declined');
var redirect=box.getAttribute('data-redirect');
dec.addEventListener('click',function(){{
if(redirect){{location.href=redirect;return;}}
acc.style.display='none';dec.style.display='none';err.style.display='none';dn.style.display='block';}});
acc.addEventListener('click',function(){{
acc.disabled=true;
fetch('{action}',{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{gate:'{gate}'}})}})
.then(function(r){{if(r.ok){{location.reload()}}else{{acc.disabled=false;err.style.display='block'}}}})
.catch(function(){{acc.disabled=false;err.style.display='block'}});}});
}})();
</script>
</body></html>"#,
        title = title,
        heading = heading,
        message = html_escape(message),
        confirm_label = confirm_label,
        action = action,
        redirect_attr = redirect_attr,
        gate = gate
    )
}

/// Set a card's consent / age gate. Mirror of `set_card_password`: same plan gate
/// (`has_card_gating` ⇒ 402 without it), same tenant-scoped ownership check, same
/// 400 shapes. Absent keys leave the current value untouched (partial update), and an
/// empty string for the message or the redirect clears it (NULL).
pub async fn set_card_gating(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    use sqlx::Row;
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Selling point of Suite/Agency — a plan without it gets 402 UpgradeRequired
    // rather than a silently ignored write.
    crate::features::enforce_feature_flag(&state, tenant_id, "has_card_gating", "Card gating")
        .await?;
    // Same ownership check create_button / set_card_password use.
    let owns_card: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kinetic_cards WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(false);
    if !owns_card {
        return Err(AppError::NotFound("Card not found".into()));
    }

    let current = sqlx::query(
        "SELECT consent_required, age_gate_type, age_gate_message, consent_decline_redirect FROM kinetic_cards WHERE id = $1 AND tenant_id = $2",
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Card not found".into()))?;
    let current_consent: bool = current
        .try_get::<Option<bool>, _>("consent_required")
        .unwrap_or(None)
        .unwrap_or(false);
    let current_age_type: Option<String> = current
        .try_get::<Option<String>, _>("age_gate_type")
        .unwrap_or(None);
    let current_message: Option<String> = current
        .try_get::<Option<String>, _>("age_gate_message")
        .unwrap_or(None);
    let current_redirect: Option<String> = current
        .try_get::<Option<String>, _>("consent_decline_redirect")
        .unwrap_or(None);

    // Validate everything BEFORE the UPDATE: nothing unvalidated may reach the columns.
    let consent_required = match optional_json_bool(&body, "consent_required")? {
        Some(v) => v,
        None => current_consent,
    };
    let age_gate_type = match optional_json_str(&body, "age_gate_type")? {
        Some(v) => normalize_age_gate_type(v)?,
        None => current_age_type,
    };
    let age_gate_message = match optional_json_str(&body, "age_gate_message")? {
        Some(v) => {
            let v = v.trim();
            if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            }
        }
        None => current_message,
    };
    let consent_decline_redirect = match optional_json_str(&body, "consent_decline_redirect")? {
        Some(v) => {
            let v = v.trim();
            if v.is_empty() {
                None
            } else if safe_redirect_target(v).is_some() {
                Some(v.to_string())
            } else {
                return Err(AppError::BadRequest(
                    "consent_decline_redirect must be an http:// or https:// URL".into(),
                ));
            }
        }
        None => current_redirect,
    };

    sqlx::query(
        "UPDATE kinetic_cards SET consent_required = $3, age_gate_type = $4, age_gate_message = $5, consent_decline_redirect = $6 WHERE id = $1 AND tenant_id = $2",
    )
    .bind(card_id)
    .bind(tenant_id)
    .bind(consent_required)
    .bind(age_gate_type.as_deref())
    .bind(age_gate_message.as_deref())
    .bind(consent_decline_redirect.as_deref())
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({
        "id": card_id,
        "consent_required": consent_required,
        "age_gate_type": age_gate_type.unwrap_or_else(|| "none".to_string()),
        "age_gate_message": age_gate_message,
        "consent_decline_redirect": consent_decline_redirect,
        "message": "Card gating saved"
    })))
}

#[derive(serde::Deserialize)]
pub struct CardGateConfirmRequest {
    #[serde(default)]
    pub gate: String,
}

/// POST /api/v1/kinetic/cards/:id/gate — an anonymous viewer's own confirmation of a
/// consent or age gate; sets the matching cookie so the interstitial can reload into
/// the real card.
///
/// Public (`auth/global_auth.rs` lets this one path through unauthenticated) and
/// deliberately tenant-agnostic: the viewer is anonymous, the card is addressed by id,
/// and the only thing issued is the same HMAC the renderer recomputes for that card's
/// *current* gate configuration. A gate that is not configured answers 400, so a stale
/// page can never mint a cookie for a gate the owner has switched off.
pub async fn confirm_card_gate(
    axum::extract::Path(card_id): axum::extract::Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<CardGateConfirmRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT consent_required, age_gate_type, consent_decline_redirect FROM kinetic_cards WHERE id = $1 LIMIT 1",
    )
    .bind(card_id)
    .fetch_optional(&state.pool)
    .await;
    let r = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"confirmed": false, "error": "Card not found"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(
                "confirm_card_gate lookup failed for card {}: {}",
                card_id,
                e
            );
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"confirmed": false, "error": "Lookup failed"})),
            )
                .into_response();
        }
    };

    let (cookie_name, token) = match req.gate.trim() {
        "consent" => {
            let required: bool = r
                .try_get::<Option<bool>, _>("consent_required")
                .unwrap_or(None)
                .unwrap_or(false);
            if !required {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(
                        json!({"confirmed": false, "error": "This card does not ask for consent"}),
                    ),
                )
                    .into_response();
            }
            let decline: String = r
                .try_get::<Option<String>, _>("consent_decline_redirect")
                .unwrap_or(None)
                .unwrap_or_default();
            (
                format!("kc_consent_{}", card_id.simple()),
                card_unlock_token(
                    &state.jwt_secret,
                    &card_id,
                    &consent_gate_signature(&decline),
                ),
            )
        }
        "age" => {
            let t: String = r
                .try_get::<Option<String>, _>("age_gate_type")
                .unwrap_or(None)
                .unwrap_or_default();
            let t = t.trim().to_string();
            if !is_age_gate_type(&t) {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({"confirmed": false, "error": "This card has no age gate"})),
                )
                    .into_response();
            }
            (
                format!("kc_age_{}", card_id.simple()),
                card_unlock_token(&state.jwt_secret, &card_id, &age_gate_signature(&t)),
            )
        }
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"confirmed": false, "error": "gate must be \"consent\" or \"age\""})),
            )
                .into_response()
        }
    };

    let cookie = format!("{cookie_name}={token}; Path=/; Max-Age=604800; HttpOnly; SameSite=Lax");
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(json!({"confirmed": true})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct CardUnlockRequest {
    pub password: String,
}

/// POST /<k|b|c|m|f|h|thank>/:slug/unlock — verifies the card password and sets the
/// unlock cookie. Response shape is JSON; the gate page reloads on 2xx.
pub async fn unlock_card(
    axum::extract::Path(slug): axum::extract::Path<String>,
    State(state): State<AppState>,
    Json(req): Json<CardUnlockRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use sqlx::Row;
    let row = sqlx::query("SELECT id, password_hash FROM kinetic_cards WHERE slug = $1 LIMIT 1")
        .bind(&slug)
        .fetch_optional(&state.pool)
        .await;
    let r = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"unlocked": false, "error": "Card not found"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("unlock_card lookup failed for slug '{}': {}", slug, e);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"unlocked": false, "error": "Lookup failed"})),
            )
                .into_response();
        }
    };
    let card_id: Uuid = r.try_get("id").unwrap_or_default();
    let stored: Option<String> = r.try_get("password_hash").unwrap_or(None);
    let stored = stored.filter(|h| !h.trim().is_empty());
    let stored = match stored {
        Some(h) => h,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"unlocked": false, "error": "This card is not password protected"})),
            )
                .into_response()
        }
    };
    if !verify_card_password(&req.password, &stored) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({"unlocked": false, "error": "Incorrect password"})),
        )
            .into_response();
    }
    let token = card_unlock_token(&state.jwt_secret, &card_id, &stored);
    let cookie = format!(
        "kc_gate_{}={}; Path=/; Max-Age=604800; HttpOnly; SameSite=Lax",
        card_id.simple(),
        token
    );
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(json!({"unlocked": true})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid() -> Uuid {
        Uuid::parse_str("8f1a1a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b").unwrap()
    }

    #[test]
    fn consent_cookie_is_bound_to_the_gate_configuration() {
        let base = card_unlock_token("secret", &cid(), &consent_gate_signature(""));
        // stable for one configuration, so a viewer's cookie survives a reload
        assert_eq!(
            base,
            card_unlock_token("secret", &cid(), &consent_gate_signature(""))
        );
        // reconfiguring the gate invalidates outstanding consent cookies
        assert_ne!(
            base,
            card_unlock_token(
                "secret",
                &cid(),
                &consent_gate_signature("https://declined.test/bye")
            )
        );
        // scoped to the card and to the server secret
        assert_ne!(
            base,
            card_unlock_token("secret", &Uuid::new_v4(), &consent_gate_signature(""))
        );
        assert_ne!(
            base,
            card_unlock_token("other-secret", &cid(), &consent_gate_signature(""))
        );
    }

    #[test]
    fn age_cookie_is_bound_to_the_age_gate_type() {
        let t18 = card_unlock_token("secret", &cid(), &age_gate_signature("age_18"));
        let t21 = card_unlock_token("secret", &cid(), &age_gate_signature("age_21"));
        assert_ne!(t18, t21);
        assert_eq!(
            t18,
            card_unlock_token("secret", &cid(), &age_gate_signature("age_18"))
        );
        // an age cookie and a consent cookie never collide, even on the same input
        assert_ne!(
            t18,
            card_unlock_token("secret", &cid(), &consent_gate_signature("age_18"))
        );
    }

    #[test]
    fn age_gate_allowlist_accepts_the_four_legal_values_and_rejects_age_16() {
        for ok in ["age_18", "age_21", "custom"] {
            assert!(is_age_gate_type(ok), "{ok} should be a real age gate");
        }
        for bad in [
            "none", "", "age_16", "AGE_18", "age18", "custom ", "yes", "true",
        ] {
            assert!(!is_age_gate_type(bad), "{bad} must not render an age gate");
        }
        // the setter's normaliser: legal values stored, 'none'/empty cleared, junk 400s
        assert_eq!(
            normalize_age_gate_type("age_18").unwrap(),
            Some("age_18".to_string())
        );
        assert_eq!(
            normalize_age_gate_type("custom").unwrap(),
            Some("custom".to_string())
        );
        assert_eq!(normalize_age_gate_type("none").unwrap(), None);
        assert_eq!(normalize_age_gate_type("").unwrap(), None);
        assert!(normalize_age_gate_type("age_16").is_err());
    }

    #[test]
    fn redirect_validator_rejects_javascript_and_data_schemes() {
        for bad in [
            "",
            "   ",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "ftp://example.com/x",
            "//evil.example.com",
            "/local/path",
            "https://",
            "http://",
            "https://example.com/a b",
            "https://example.com/a\nb",
        ] {
            assert_eq!(
                safe_redirect_target(bad),
                None,
                "{bad:?} must not be an accepted decline target"
            );
        }
        assert_eq!(
            safe_redirect_target("https://example.com/declined"),
            Some("https://example.com/declined")
        );
        assert_eq!(
            safe_redirect_target("  http://example.com/x  "),
            Some("http://example.com/x")
        );
        assert_eq!(
            safe_redirect_target("HTTPS://Example.com/x"),
            Some("HTTPS://Example.com/x")
        );
    }

    #[test]
    fn interstitial_escapes_owner_text_and_only_emits_a_validated_redirect() {
        let html = render_gate_interstitial(
            "Card &amp; Co",
            "age",
            "You must be 18+ <script>alert(1)</script>",
            &cid(),
            safe_redirect_target("https://ok.test/bye"),
        );
        assert!(html.contains("You must be 18+ &lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains(" data-redirect=\"https://ok.test/bye\""));
        assert!(html.contains("Age verification"));
        // no valid target => no attribute at all, so decline shows the neutral panel
        let html2 = render_gate_interstitial(
            "T",
            "consent",
            "hello",
            &cid(),
            safe_redirect_target("javascript:alert(1)"),
        );
        // no valid target => no attribute is emitted at all (the script's own
        // getAttribute('data-redirect') lookup is still there and reads null), so
        // decline shows the neutral panel instead of navigating
        assert!(!html2.contains("data-redirect=\""));
        assert!(html2.contains(&format!("/api/v1/kinetic/cards/{}/gate", cid())));
        assert!(html2.contains("Before you continue"));
    }

    /// kanban t_c7673a05 — the card lead form's submission parsing. `leads.name` is NOT NULL and
    /// the name may legitimately arrive in three shapes; a submission with no identity at all must
    /// be refused (the handler turns `None` into a 400, never a nameless row).
    #[test]
    fn card_lead_name_prefers_typed_name_then_parts_then_the_email_local_part() {
        let named = json!({"name": "  Ada Lovelace  ", "email": "ada@example.com"});
        assert_eq!(
            card_lead_name(&named, card_lead_field(&named, "email").as_deref()),
            Some("Ada Lovelace".to_string())
        );
        // parts alone, in either order, and never a stray space when only one part is present
        let parts = json!({"first_name": "Ada", "last_name": "Lovelace"});
        assert_eq!(
            card_lead_name(&parts, None),
            Some("Ada Lovelace".to_string())
        );
        let one_part = json!({"last_name": "Lovelace"});
        assert_eq!(
            card_lead_name(&one_part, None),
            Some("Lovelace".to_string())
        );
        // the email-only waitlist template: the local part becomes the display name
        let email_only = json!({"email": "ada.lovelace@example.com"});
        assert_eq!(
            card_lead_name(
                &email_only,
                card_lead_field(&email_only, "email").as_deref()
            ),
            Some("ada.lovelace".to_string())
        );
        // a blank name is not a name, and no identity at all stays absent
        assert_eq!(card_lead_name(&json!({"name": "   "}), None), None);
        assert_eq!(
            card_lead_name(&json!({"name": "", "email": ""}), None),
            None
        );
        assert_eq!(card_lead_name(&json!({}), None), None);
        assert_eq!(
            card_lead_name(&json!({"email": "@example.com"}), Some("@example.com")),
            None
        );
    }

    /// The form's field list is configurable, so a field this handler does not name by hand has to
    /// survive on the lead — that is `custom_fields.extra`, asserted here against a real card's
    /// `lead_form` field names.
    #[test]
    fn card_lead_extra_fields_keeps_configured_fields_and_drops_the_handled_ones() {
        let body = json!({
            "name": "Ada", "email": "ada@example.com", "company": "Analytical Engines",
            "budget": "5000", "message": "hello", "referrer_url": "https://x.test",
            "extra": {"nested": true},
        });
        let extra = card_lead_extra_fields(&body);
        assert_eq!(extra.get("budget").and_then(|v| v.as_str()), Some("5000"));
        assert_eq!(extra.get("extra"), Some(&json!({"nested": true})));
        for handled in [
            "name",
            "email",
            "company",
            "message",
            "referrer_url",
            "first_name",
            "last_name",
            "phone",
            "notes",
            "utm_source",
            "utm_medium",
            "utm_campaign",
        ] {
            assert!(
                !extra.contains_key(handled),
                "{handled} is stored by hand and must not be duplicated"
            );
        }
        assert_eq!(card_lead_extra_fields(&json!({})).len(), 0);
        assert_eq!(card_lead_extra_fields(&json!("not an object")).len(), 0);
    }

    /// Every named field is trimmed and blank-as-absent, and `notes` falls back to `message` —
    /// the two rules the handler's insert and its 400 rest on.
    #[test]
    fn card_lead_field_trims_and_treats_blank_as_absent() {
        assert_eq!(
            card_lead_field(&json!({"x": "  y  "}), "x"),
            Some("y".to_string())
        );
        assert_eq!(card_lead_field(&json!({"x": "   "}), "x"), None);
        assert_eq!(card_lead_field(&json!({"x": 7}), "x"), None);
        assert_eq!(card_lead_field(&json!({}), "x"), None);
    }
}
