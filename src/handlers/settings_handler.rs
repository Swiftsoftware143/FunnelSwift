use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::setting::*;
use crate::state::AppState;

pub async fn get_settings(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TenantSetting>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let mut settings =
        sqlx::query_as::<_, TenantSetting>("SELECT * FROM tenant_settings WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_all(&state.pool)
            .await?;

    // A tenant mail-config row is CIPHERTEXT at rest, so the tenant's own view must render a mask
    // computed from the OPENED value (kanban t_b040a78e). Left raw, `SELECT *` ships the `enc:v1:`
    // envelope to the console — and a mask derived from the ciphertext is itself the defect. Only
    // the three keys `resolve` reads are touched; every other setting (lead_stages, subdomain, …)
    // passes through untouched.
    for setting in settings.iter_mut() {
        if crate::email_provider::TENANT_CONFIG_KEYS.contains(&setting.key.as_str()) {
            setting.value =
                crate::email_provider::open_and_mask_config_secrets(&state.pool, &setting.value)
                    .await;
        }
    }

    // A branding `logo_url` is a POINTER into `tenant_logos`, and the console renders it as the
    // brand thumbnail while every sender embeds it in the mail header. The pointer and the row are
    // written by different paths, so a row removed on its own (restore, hand-run DELETE, residue
    // sweep) would otherwise leave a broken image in both places. Same invariant
    // `crate::branding::load` holds for the senders.
    let points_at_logo = settings.iter().any(|s| {
        s.key == crate::branding::SETTINGS_KEY
            && s.value
                .get("logo_url")
                .and_then(|v| v.as_str())
                .map(|u| !u.trim().is_empty())
                .unwrap_or(false)
    });
    if points_at_logo && !crate::branding::logo_exists(&state.pool, tenant_id).await {
        for setting in settings
            .iter_mut()
            .filter(|s| s.key == crate::branding::SETTINGS_KEY)
        {
            if let Some(obj) = setting.value.as_object_mut() {
                obj.insert("logo_url".to_string(), json!(""));
            }
        }
    }

    Ok(Json(settings))
}

pub async fn update_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateSettingsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // This route takes an ARBITRARY `{key, value}`, so a tenant can write the three keys
    // `email_provider::resolve` reads for it (`email_config` / `mailgun_config` / `smtp_config`),
    // and those carry a provider credential. Seal them before they reach the row (kanban
    // t_b040a78e): left raw, a dump, a backup or a read-only psql yields a working provider key.
    // A masked credential coming back from the tenant's own view restores the STORED (already
    // sealed) value instead of storing the mask as the credential.
    let mut value = req.value.clone();
    if crate::email_provider::TENANT_CONFIG_KEYS.contains(&req.key.as_str()) {
        // Refuse a document the send path could only misread (kanban t_7ac7ceb5). This runs BEFORE
        // the row is touched, so a rejected save stores nothing — the same discipline the API uses
        // for a provider key nothing can read.
        crate::email_provider::validate_config_document(&value).map_err(AppError::BadRequest)?;
        let stored: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT value FROM tenant_settings WHERE tenant_id = $1 AND key = $2",
        )
        .bind(tenant_id)
        .bind(&req.key)
        .fetch_optional(&state.pool)
        .await?;
        crate::email_provider::restore_masked_config_secrets(&mut value, stored.as_ref());
        crate::email_provider::seal_config_secrets(&state.pool, &mut value)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to seal email credentials: {e}")))?;
    }

    // Email branding (kanban t_c06a32eb). The document has two owners: `brand_name` /
    // `brand_color` are written HERE (and validated here), while `logo_url` belongs to the logo
    // endpoints. A panel echo that omits `logo_url` must not un-reference a logo that is still
    // stored, so an omitted key INHERITS the stored value; an explicit "" (= "no logo") is kept.
    if req.key == crate::branding::SETTINGS_KEY {
        crate::branding::validate_value(&value).map_err(AppError::BadRequest)?;
        if let Some(obj) = value.as_object_mut() {
            if !obj.contains_key("logo_url") {
                let stored_logo: Option<String> = sqlx::query_scalar(
                    "SELECT value->>'logo_url' FROM tenant_settings WHERE tenant_id = $1 AND key = $2",
                )
                .bind(tenant_id)
                .bind(crate::branding::SETTINGS_KEY)
                .fetch_optional(&state.pool)
                .await
                .ok()
                .flatten();
                // A stored pointer whose `tenant_logos` row is gone is NOT inherited: inheriting it
                // would re-save a dead URL that answers 404 in every mail. Same invariant the
                // settings read and `branding::load` hold.
                let inherited = match stored_logo {
                    Some(u) if !u.trim().is_empty() => {
                        if crate::branding::logo_exists(&state.pool, tenant_id).await {
                            u
                        } else {
                            String::new()
                        }
                    }
                    other => other.unwrap_or_default(),
                };
                obj.insert("logo_url".to_string(), serde_json::Value::String(inherited));
            }
        }
    }

    // tenant_settings.id is NOT NULL with no default — the insert omitted it, so every
    // settings save died with "null value in column id violates not-null constraint".
    sqlx::query(
        r#"INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES (gen_random_uuid(), $1, $2, $3)
           ON CONFLICT (tenant_id, key) DO UPDATE SET value = $3, updated_at = NOW()"#,
    )
    .bind(tenant_id)
    .bind(&req.key)
    .bind(&value)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Setting updated"})))
}
pub async fn delete_setting(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Don't allow deleting seo settings or lead_stages
    if key.starts_with("seo_") || key == "lead_stages" {
        return Err(AppError::BadRequest(
            "Cannot delete protected setting".into(),
        ));
    }

    let result = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1 AND key = $2")
        .bind(tenant_id)
        .bind(&key)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Setting not found".into()));
    }

    Ok(Json(json!({"message": "Setting deleted"})))
}

/// GET /api/v1/settings/email-config — the workspace's OWN sending identity, and which
/// configuration a send would actually use right now.
///
/// Why a route instead of reading `GET /api/v1/settings` in the browser: the answer to "who sends
/// this workspace's mail" is decided by the SERVER's resolution order (own row → legacy rows →
/// system row, and a half-filled own row FALLS THROUGH). A console that re-implemented that order
/// would eventually disagree with the send path, so the screen asks the same function the send
/// uses (kanban t_7ac7ceb5, the "render the same truth as the API" requirement).
///
/// The credential fields are masked exactly as `GET /api/v1/settings` masks them. The `effective`
/// arm carries no credential at all — only which configuration would speak and the sender it uses.
pub async fn get_email_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let own_raw: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT value FROM tenant_settings WHERE tenant_id = $1 AND key = 'email_config'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let own = match own_raw {
        Some(v) => crate::email_provider::open_and_mask_config_secrets(&state.pool, &v).await,
        None => serde_json::Value::Null,
    };

    let effective = crate::email_provider::resolve_source(&state.pool, Some(tenant_id))
        .await
        .map(|(cfg, src)| {
            json!({
                "source": src.as_str(),
                "provider": cfg.provider,
                "sender": cfg.sender(),
            })
        })
        .unwrap_or_else(|| json!({"source": "none", "provider": null, "sender": null}));

    Ok(Json(json!({
        "own": own,
        "own_set": !own.is_null(),
        "effective": effective,
    })))
}

/// POST /api/v1/settings/email-config/test — send ONE real message through THIS WORKSPACE's own
/// configuration and report what the transport answered.
///
/// Three deliberate constraints (kanban t_7ac7ceb5):
///
/// * the recipient is the CALLER'S OWN sign-in address and is never taken from the request, so this
///   route cannot be aimed at a third party and is not a relay;
/// * it sends only when the workspace's OWN row is the one in use (`ConfigSource::Workspace`).
///   Without that, a signed-in user of any workspace could post to this route repeatedly and spend
///   the installation's shared sending reputation on messages to themselves — the system-mail door
///   already has its own admin-only test;
/// * a 2xx here is the transport's answer, NOT delivery proof — the panel says so, and the fleet's
///   proof of delivery reads the message off the transport (see the card's live-send proof).
pub async fn test_email_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let to = auth.email.trim().to_string();
    if to.is_empty() || !to.contains('@') {
        return Ok(Json(json!({
            "success": false,
            "detail": "Your sign-in address is not a usable recipient for a test message.",
        })));
    }

    let Some((cfg, source)) =
        crate::email_provider::resolve_source(&state.pool, Some(tenant_id)).await
    else {
        return Ok(Json(json!({
            "success": false,
            "detail": "No email provider is configured for this workspace yet.",
        })));
    };
    if source != crate::email_provider::ConfigSource::Workspace {
        return Ok(Json(json!({
            "success": false,
            "source": source.as_str(),
            "detail": "Save your own email provider first — this test sends through YOUR \
                       workspace's setting, not through FunnelSwift's own mail service.",
        })));
    }

    let (body, _) = crate::email::with_support_footer(
        Some(
            "This is a test of YOUR workspace's email provider.\n\nIf you received it, the \
             workspace's own provider is sending.\n\n- FunnelSwift"
                .to_string(),
        ),
        None,
    );

    match crate::email_provider::deliver(
        &cfg,
        &to,
        "FunnelSwift — workspace email test",
        body.as_deref().unwrap_or_default(),
        None,
    )
    .await
    {
        Ok(()) => Ok(Json(json!({
            "success": true,
            "source": source.as_str(),
            "provider": cfg.provider,
            "sender": cfg.sender(),
            "to": to,
            "detail": format!("{} accepted the message", cfg.provider),
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "source": source.as_str(),
            "provider": cfg.provider,
            "to": to,
            "detail": e,
        }))),
    }
}
