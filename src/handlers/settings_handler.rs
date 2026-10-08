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
                obj.insert(
                    "logo_url".to_string(),
                    serde_json::Value::String(stored_logo.unwrap_or_default()),
                );
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
