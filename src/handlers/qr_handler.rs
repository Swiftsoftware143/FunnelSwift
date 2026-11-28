use crate::auth::middleware::AuthUser;
use crate::card_types;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderName, StatusCode},
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// The public origin a card is published under. `kntcrd.com` is the canonical host the card
/// renderer, the sitemap (`seo_handler`) and the console's "your cards are published at" line all
/// use; a tenant subdomain or a custom domain is a rewrite of the SAME card, so the QR's target
/// stays the canonical address.
const CARD_ORIGIN: &str = "https://kntcrd.com";

/// The public origin a FUNNEL page is published under — the same host `render_funnel` builds its
/// canonical link from, so a printed QR opens exactly the page the console's "your page" link does.
const FUNNEL_ORIGIN: &str = "https://funnelswift.net";

/// The address a card QR code opens, in the spelling that ANSWERS 200 (kanban t_c4fcafa7).
///
/// Every card vhost 301s `kntcrd.com/k/<slug>` onto `kntcrd.com/<slug>` (sites/kntcrd.conf), so a
/// QR printed on the `/k/` spelling sent every scanner through a redirect to reach the very page
/// the code was for. The default card type's address IS the short form; the other five types are
/// not redirected by any vhost and keep their prefix.
fn card_public_url(template_type: Option<&str>, slug: &str) -> String {
    let prefix = card_types::url_prefix(template_type.unwrap_or_default());
    if prefix == "k" {
        format!("{CARD_ORIGIN}/{slug}")
    } else {
        format!("{CARD_ORIGIN}/{prefix}/{slug}")
    }
}

/// The URL a QR code `id` should open, and whether the row is switched on. `None` when the row or
/// its card is gone.
async fn qr_target_url(state: &AppState, id: Uuid) -> AppResult<Option<(String, bool)>> {
    // Either end of the link, never both (migration 143's CHECK): a card code opens the card's
    // canonical host, a funnel code opens the funnel's public page.
    let row: Option<(Option<String>, Option<String>, Option<String>, bool)> = sqlx::query_as(
        "SELECT c.slug, c.template_type, f.slug, q.is_active \
         FROM kinetic_qr_codes q \
         LEFT JOIN kinetic_cards c ON c.id = q.card_id \
         LEFT JOIN funnels f ON f.id = q.funnel_id \
         WHERE q.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((card_slug, template_type, funnel_slug, is_active)) = row else {
        return Ok(None);
    };
    if let Some(slug) = card_slug {
        return Ok(Some((
            card_public_url(template_type.as_deref(), &slug),
            is_active,
        )));
    }
    Ok(funnel_slug.map(|slug| (format!("{FUNNEL_ORIGIN}/funnel/{slug}"), is_active)))
}

/// Build the SVG for `url`. A QR renderer failure is a 422: the only input is the URL we just
/// built, so a refusal means the payload itself is out of range (a slug longer than the encoder
/// accepts), not a server fault.
fn qr_svg(url: &str) -> AppResult<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| AppError::UnprocessableEntity(format!("could not encode a QR code: {e}")))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// Build the PNG for `url` at 512px, so a printed code stays sharp.
fn qr_png(url: &str) -> AppResult<Vec<u8>> {
    use image::{codecs::png::PngEncoder, ImageEncoder, Luma};
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| AppError::UnprocessableEntity(format!("could not encode a QR code: {e}")))?;
    let img = code
        .render::<Luma<u8>>()
        .min_dimensions(512, 512)
        .quiet_zone(true)
        .build();
    let mut out: Vec<u8> = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::L8,
        )
        .map_err(|e| AppError::Internal(format!("could not write the QR png: {e}")))?;
    Ok(out)
}

/// `GET /api/v1/kinetic/qr` — every QR in this workspace.
///
/// The rows are OBJECTS, not the four-element tuples this used to return: a client cannot find
/// the QR that belongs to a card when the response has no `card_id` field (measured 2026-10-21 —
/// nothing consumed the tuple shape, since no screen has ever called this route).
pub async fn list_qr_codes(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // A QR code belongs to a kinetic CARD. The canonical model is `kinetic_qr_codes`
    // (migration 0015) — the table `features::count_usage` counts for the `max_qr_codes`
    // plan gate. Title/slug are the card's, which is what a QR's display name is.
    // A code belongs to a kinetic CARD or a FUNNEL page (migration 143). `kind` says which end it
    // opens so a client never has to infer it from two nullable ids.
    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        String,
        String,
        Option<String>,
        bool,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT q.id, q.card_id, q.funnel_id, q.name, COALESCE(c.title, f.name, ''), \
                COALESCE(c.slug, f.slug), q.is_active, q.created_at \
             FROM kinetic_qr_codes q \
             LEFT JOIN kinetic_cards c ON c.id = q.card_id \
             LEFT JOIN funnels f ON f.id = q.funnel_id \
             WHERE q.tenant_id = $1 ORDER BY q.created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, card_id, funnel_id, name, card_title, card_slug, is_active, created_at)| {
                json!({
                    "id": id,
                    "kind": if funnel_id.is_some() { "funnel" } else { "card" },
                    "card_id": card_id,
                    "funnel_id": funnel_id,
                    "name": name,
                    "card_title": card_title,
                    "card_slug": card_slug,
                    "is_active": is_active,
                    "created_at": created_at,
                    "png_url": format!("/api/v1/kinetic/qr/{id}/png"),
                    "svg_url": format!("/api/v1/kinetic/qr/{id}/svg"),
                })
            },
        )
        .collect();
    Ok(Json(json!(out)))
}

/// `POST /api/v1/kinetic/qr` — the QR for a card, creating it once.
///
/// IDEMPOTENT per card: a workspace on `kinetic-free` may hold exactly ONE QR code, so pressing
/// the button twice must return the same row rather than spend the plan's single slot and then
/// refuse the second press.
pub async fn create_qr_code(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let user_id = Uuid::parse_str(&auth.user_id).unwrap_or_default();

    // A FUNNEL page's code (migration 143). Same object, other end of the link: the tenant check
    // keeps a caller from minting a code for another workspace's page (the FK is not tenant-aware),
    // and the mint is idempotent per page so a second press costs no second slot.
    if let Some(funnel_id) = payload["funnel_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        let owned: Option<String> =
            sqlx::query_scalar("SELECT name FROM funnels WHERE id = $1 AND tenant_id = $2")
                .bind(funnel_id)
                .bind(tenant_id)
                .fetch_optional(&state.pool)
                .await?;
        let Some(funnel_name) = owned else {
            return Err(AppError::NotFound("Funnel not found".into()));
        };
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM kinetic_qr_codes WHERE funnel_id = $1 AND tenant_id = $2 LIMIT 1",
        )
        .bind(funnel_id)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?;
        if let Some(id) = existing {
            sqlx::query(
                "UPDATE kinetic_qr_codes SET is_active = true, updated_at = NOW() WHERE id = $1",
            )
            .bind(id)
            .execute(&state.pool)
            .await?;
            return Ok((
                StatusCode::OK,
                Json(json!({
                    "id": id.to_string(),
                    "existing": true,
                    "kind": "funnel",
                    "png_url": format!("/api/v1/kinetic/qr/{id}/png"),
                    "svg_url": format!("/api/v1/kinetic/qr/{id}/svg"),
                })),
            ));
        }
        features::enforce_feature_limit(&state, tenant_id, "max_qr_codes", "QR codes").await?;
        let id = Uuid::new_v4();
        // name is varchar(100): truncate rather than let a long page name 500 on the insert.
        let title: String = payload["title"]
            .as_str()
            .or_else(|| payload["name"].as_str())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .unwrap_or_else(|| format!("{funnel_name} — funnel page"))
            .chars()
            .take(100)
            .collect();
        sqlx::query(
            "INSERT INTO kinetic_qr_codes (id, user_id, tenant_id, funnel_id, name) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(funnel_id)
        .bind(&title)
        .execute(&state.pool)
        .await?;
        return Ok((
            StatusCode::CREATED,
            Json(json!({
                "id": id.to_string(),
                "existing": false,
                "kind": "funnel",
                "png_url": format!("/api/v1/kinetic/qr/{id}/png"),
                "svg_url": format!("/api/v1/kinetic/qr/{id}/svg"),
            })),
        ));
    }

    // The card is the request's real argument: `kinetic_qr_codes.card_id` is NOT NULL and
    // FKs kinetic_cards, and the tenant check below keeps a caller from attaching a QR to
    // another workspace's card (the FK itself is not tenant-aware).
    let card_id = payload["card_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("card_id is required".into()))?;
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM kinetic_cards WHERE id = $1 AND tenant_id = $2")
            .bind(card_id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound("Card not found".into()));
    }
    // Already minted for this card? Return it (and re-activate a disabled one, which is what
    // pressing the button means).
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM kinetic_qr_codes WHERE card_id = $1 AND tenant_id = $2 LIMIT 1",
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(id) = existing {
        sqlx::query(
            "UPDATE kinetic_qr_codes SET is_active = true, updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&state.pool)
        .await?;
        return Ok((
            StatusCode::OK,
            Json(json!({
                "id": id.to_string(),
                "existing": true,
                "png_url": format!("/api/v1/kinetic/qr/{id}/png"),
                "svg_url": format!("/api/v1/kinetic/qr/{id}/svg"),
            })),
        ));
    }
    features::enforce_feature_limit(&state, tenant_id, "max_qr_codes", "QR codes").await?;
    let id = Uuid::new_v4();
    // name is varchar(100): truncate rather than let a long title 500 on the insert.
    let title: String = payload["title"]
        .as_str()
        .or_else(|| payload["name"].as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or("QR Code")
        .chars()
        .take(100)
        .collect();
    sqlx::query(
        "INSERT INTO kinetic_qr_codes (id, user_id, tenant_id, card_id, name) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(card_id)
    .bind(&title)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id.to_string(),
            "existing": false,
            "png_url": format!("/api/v1/kinetic/qr/{id}/png"),
            "svg_url": format!("/api/v1/kinetic/qr/{id}/svg"),
        })),
    ))
}

/// `PUT /api/v1/kinetic/qr/:id` — rename, or switch off. This used to answer
/// `{"message":"QR code updated"}` without touching a row: a 200 that changed nothing, so a
/// rename looked saved and was gone on the next load.
pub async fn update_qr_code(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let name: Option<String> = payload["name"]
        .as_str()
        .or_else(|| payload["title"].as_str())
        .map(|t| t.trim().chars().take(100).collect::<String>())
        .filter(|t| !t.is_empty());
    let is_active: Option<bool> = payload["is_active"].as_bool();
    if name.is_none() && is_active.is_none() {
        return Err(AppError::BadRequest(
            "send a name or is_active to change".into(),
        ));
    }
    let updated: Option<(Uuid, String, bool)> = sqlx::query_as(
        "UPDATE kinetic_qr_codes \
         SET name = COALESCE($3, name), is_active = COALESCE($4, is_active), updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 \
         RETURNING id, name, is_active",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(name)
    .bind(is_active)
    .fetch_optional(&state.pool)
    .await?;
    match updated {
        Some((id, name, is_active)) => Ok(Json(json!({
            "id": id, "name": name, "is_active": is_active,
        }))),
        None => Err(AppError::NotFound("QR code not found".into())),
    }
}

pub async fn delete_qr_code(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let result = sqlx::query("DELETE FROM kinetic_qr_codes WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("QR code not found".into()));
    }
    Ok(Json(json!({ "message": "QR code deleted" })))
}

/// `GET /api/v1/kinetic/qr/:id/svg` — the scannable code, PUBLIC by design: it is fetched by a
/// bare `<img src>` in a page or an email with no credential. Until 2026-10-21 this returned a
/// solid black square, so the plan's `max_qr_codes` limit, the dashboard usage counter and the
/// card QR block all pointed at a feature that could not produce a single scannable code.
pub async fn get_qr_svg(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<(StatusCode, [(HeaderName, &'static str); 2], String)> {
    // A QR image is fetched by a SCANNER or a print job, never by a programmer: a stale, truncated
    // or hand-typed id must answer the same clean 404 as an id with no row, not axum's `Path<Uuid>`
    // refusal (400 "Invalid URL: UUID parsing failed…") which shows a parser sentence instead of an
    // image. Fleet contract measured 2026-10-26; same fix as the public avatar/logo reads.
    let id = Uuid::parse_str(raw_id.trim())
        .map_err(|_| AppError::NotFound("QR code not found".into()))?;
    let (url, is_active) = qr_target_url(&state, id)
        .await?
        .ok_or_else(|| AppError::NotFound("QR code not found".into()))?;
    // A switched-off QR must stop resolving: an image that keeps serving after the owner turned
    // it off is the same false PASS as a 200 that changes nothing.
    if !is_active {
        return Err(AppError::NotFound("QR code is switched off".into()));
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        qr_svg(&url)?,
    ))
}

/// `GET /api/v1/kinetic/qr/:id/png` — the same code as PNG. This returned ZERO bytes before
/// 2026-10-21: a 200 with `Content-Type: image/png` and an empty body, which a browser renders as
/// a broken image and a print shop cannot use at all.
pub async fn get_qr_png(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<(StatusCode, [(HeaderName, &'static str); 2], Vec<u8>)> {
    // Same contract as the SVG twin above: a bad id is "not found", never a 400 parser sentence.
    let id = Uuid::parse_str(raw_id.trim())
        .map_err(|_| AppError::NotFound("QR code not found".into()))?;
    let (url, is_active) = qr_target_url(&state, id)
        .await?
        .ok_or_else(|| AppError::NotFound("QR code not found".into()))?;
    // A switched-off QR must stop resolving: an image that keeps serving after the owner turned
    // it off is the same false PASS as a 200 that changes nothing.
    if !is_active {
        return Err(AppError::NotFound("QR code is switched off".into()));
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        qr_png(&url)?,
    ))
}

#[cfg(test)]
mod card_public_url_tests {
    use super::*;

    /// A printed QR must open the address that ANSWERS 200 (kanban t_c4fcafa7). A card that
    /// publishes under the `/k/` slot is the case that was broken: `kntcrd.com/k/<slug>` answers
    /// 301, so its code sent every scanner through a redirect. Two live cards carry the
    /// `default` type, which is what lands in that slot.
    #[test]
    fn a_k_slot_card_prints_the_short_address() {
        assert_eq!(
            card_public_url(Some("default"), "david"),
            "https://kntcrd.com/david"
        );
        assert_eq!(
            card_public_url(Some("thank_you"), "david"),
            "https://kntcrd.com/david"
        );
    }

    /// An untyped card stores the DEFAULT type, which is a Bio Link — its `/b/` address is not
    /// redirected by any vhost, so that address is already the 200 one.
    #[test]
    fn an_untyped_card_keeps_the_default_types_prefix() {
        assert_eq!(card_public_url(None, "david"), "https://kntcrd.com/b/david");
    }

    /// The other five types are redirected by no vhost, so their prefixed address IS the 200 one.
    #[test]
    fn the_other_card_types_keep_their_prefix() {
        assert_eq!(
            card_public_url(Some("bio_link"), "david"),
            "https://kntcrd.com/b/david"
        );
        assert_eq!(
            card_public_url(Some("business_card"), "david"),
            "https://kntcrd.com/c/david"
        );
    }
}
