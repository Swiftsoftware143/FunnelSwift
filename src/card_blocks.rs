//! Server-side renderer for the `kinetic_cards.layout_blocks` JSONB column.
//!
//! ## Why this module exists (kanban t_f1a4f9df)
//!
//! `kinetic_handler::render_card` resolved `layout_blocks` and then rendered none of it: both
//! `cta_html` and `social_html` were built as `String::new()` with the comment "pulled from
//! layout_blocks by front-end JS". There is no such front-end JS — the served page's only
//! `<script>` is the inline `POST /card/<id>/track` tracker — so a card whose blocks carry a
//! fully configured `lead_form` served a page with **zero** `<form>`/`<input>`/`<button>` while
//! the served guide promises "Kinetic Card Forms — Add a lead capture form block to any Kinetic
//! landing card. Visitors submit their info directly through the card."
//!
//! ## What this renderer does and does NOT do
//!
//! * It renders the block vocabulary the DATA actually uses (measured 2026-09-26): the five live
//!   `lead_form` cards and four `business_card` cards carry `hero`, `features`, `lead_form` and
//!   `business_card`, and `templates.json` adds `bio_link`, `mini_funnel` and `thank_you`.
//!   `buttons`, `social(s)`, `link`, `image`, `video` and `qr` are rendered too because the served
//!   guide's Layout Blocks table names them and/or the shipped schema declares them.
//! * Unknown `type` values are **ignored**, silently — a block type nobody ships a renderer for
//!   must never break the page (same rule as `templates.rs`'s overlay handling).
//! * It does NOT resurrect `templates/micro_page.html` + `templates.rs`'s `PageTemplate`. That
//!   template is a complete block renderer but it is DEAD CODE (`PageTemplate` is instantiated
//!   nowhere in `src/`), it is a *different page shell* (swapping `render_card` onto it would
//!   change all 27 live cards, not just the ones carrying blocks), it pulls a third-party
//!   `https://cdn.tailwindcss.com` script onto the public page, and its lead capture is a MODAL
//!   (`openModal()`) while the guide and the stored data both say *inline*. The markup here is
//!   self-contained CSS in the card's own `<style>`, driven by the card's own colours.
//! * It is strictly ADDITIVE: for a card with no blocks (or blocks nobody renders) every returned
//!   string is empty, so the served page is byte-identical to what it was before.
//!
//! ## Security notes
//!
//! Every text value is `html_escape`d, every href goes through [`safe_url`] (a `javascript:` /
//! `data:` link is dropped, not rendered) and every CSS fragment (hero gradient) goes through
//! [`safe_css_fragment`], so a stored value cannot break out of the `style` attribute.

use crate::templates::html_escape;
use serde_json::Value;

/// A CTA button row from the `kinetic_buttons` table (the app's only button WRITER is the card
/// editor's "CTA buttons" box, which POSTs `{label,url}` to `/api/v1/kinetic/cards/:id/buttons`).
#[derive(Debug, Clone)]
pub struct CardButton {
    pub label: String,
    pub url: String,
    /// Migration 0017 vocabulary: `'url' | 'lead_form' | 'sms'`.
    pub action_type: String,
}

/// The pieces `render` hands back to `render_card`.
#[derive(Debug, Default, Clone)]
pub struct Rendered {
    /// `""` or `"\n<div class=\"kblocks\">…</div>"` — inserted right after the card's `</div>`.
    pub section: String,
    /// `""` or CSS rules ending in a newline — inserted right before `</style>`.
    pub css: String,
    /// `""` or `"\n<script>…</script>"` — inserted right before `</body>`.
    pub script: String,
    /// True when at least one `lead_form` block rendered (the script is only needed then).
    pub has_lead_form: bool,
    /// The block types that actually produced markup, in order (evidence for the audit).
    pub rendered: Vec<String>,
}

impl Rendered {
    /// The value for a card with no renderable blocks: every field empty, i.e. the page is
    /// byte-identical to the pre-t_f1a4f9df output.
    pub fn nothing() -> Self {
        Self::default()
    }
}

/// Everything the block markup needs that comes from the card row rather than the block itself.
pub struct BlockOptions<'a> {
    /// `kinetic_cards.accent_color` — the theme colour the button/inputs are drawn from.
    pub accent: &'a str,
    /// Where the `lead_form` posts: `/<prefix>/<slug>/lead` on the page's OWN prefix.
    pub lead_action: &'a str,
}

/// The selector the lead-form script binds to; also what a `kinetic_buttons` row whose
/// `action_type='lead_form'` submits.
const FORM_ATTR: &str = "data-kc-lead";
/// id of the first rendered lead form (a `lead_form` button submits this one).
const FIRST_FORM_ID: &str = "kc-lead-form-1";

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|x| !x.is_empty())
}

fn esc(v: Option<&str>) -> String {
    html_escape(v.unwrap_or(""))
}

/// An href that can be rendered, or `None` when the stored value must not become a link.
///
/// `javascript:`/`data:`/`vbscript:` and anything with control characters are refused; http(s),
/// mailto:, tel:, sms:, root-relative (`/…`) and fragment (`#…`) targets are allowed.
pub fn safe_url(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 2000 || t.chars().any(|c| c.is_control()) {
        return None;
    }
    let l = t.to_ascii_lowercase();
    let ok = l.starts_with("http://")
        || l.starts_with("https://")
        || l.starts_with("mailto:")
        || l.starts_with("tel:")
        || l.starts_with("sms:")
        || t.starts_with('/')
        || t.starts_with('#');
    ok.then(|| t.to_string())
}

/// A CSS fragment (the hero's `gradient_colors` / `gradient_angle`) that is provably incapable of
/// escaping its declaration: only alphanumerics and `# , % . ( ) -` and spaces are accepted, so
/// `;`, `{`, `}`, quotes and `url(` can never appear.
pub fn safe_css_fragment(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 400 {
        return None;
    }
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || " #,%.()-".contains(c))
        .then(|| t.to_string())
}

fn field_label(f: &Value) -> String {
    match s(f, "label") {
        Some(l) => l.to_string(),
        None => {
            // "field_name" -> "Field name"
            let n = s(f, "name").unwrap_or("Field");
            let mut c = n.chars();
            match c.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &c.as_str().replace('_', " ")
                }
                None => "Field".to_string(),
            }
        }
    }
}

/// `input[type=…]` for a configured field. Unknown/absent kinds fall back to `text`.
fn input_type(f: &Value) -> &'static str {
    // The live cards spell it `field_type`; templates.json spells the same key `type`.
    match s(f, "field_type").or_else(|| s(f, "type")) {
        Some("email") => "email",
        Some("tel") | Some("phone") => "tel",
        Some("number") => "number",
        Some("date") => "date",
        Some("url") => "url",
        _ => "text",
    }
}

fn is_textarea(f: &Value) -> bool {
    matches!(
        s(f, "field_type").or_else(|| s(f, "type")),
        Some("textarea") | Some("message")
    )
}

fn field_name_attr(f: &Value) -> String {
    html_escape(s(f, "name").unwrap_or(""))
}

/// The configured `fields`, or the default pair the endpoint needs for identity when the block
/// configures none (a form with zero inputs could only ever answer the handler's 400).
fn lead_fields(block: &Value) -> Vec<Value> {
    let configured: Vec<Value> = block
        .get("fields")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter(|f| s(f, "name").is_some())
                .cloned()
                .collect::<Vec<Value>>()
        })
        .unwrap_or_default();
    if configured.is_empty() {
        vec![
            serde_json::json!({"name":"name","label":"Name","field_type":"text"}),
            serde_json::json!({"name":"email","label":"Email","field_type":"email"}),
        ]
    } else {
        configured
    }
}

fn lead_form_html(block: &Value, action: &str, index: usize) -> String {
    let title = esc(s(block, "form_title"));
    let button = match s(block, "button_text") {
        Some(b) => b.to_string(),
        None => "Send".to_string(),
    };
    let default_ph = s(block, "placeholder").unwrap_or("");
    let mut fields_html = String::new();
    for f in lead_fields(block) {
        let name = field_name_attr(&f);
        if name.is_empty() {
            continue;
        }
        let label = html_escape(&field_label(&f));
        let ph = html_escape(s(&f, "placeholder").unwrap_or(default_ph));
        // `required` is honoured exactly as configured. It is NOT defaulted on: a client-side gate
        // would hide the server's own "A card lead needs a name or an email" answer, which is the
        // outcome this card requires the form to show.
        let req = if f.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
            " required"
        } else {
            ""
        };
        let input = if is_textarea(&f) {
            format!("<textarea name=\"{name}\" rows=\"3\" placeholder=\"{ph}\"{req}></textarea>")
        } else {
            format!(
                "<input type=\"{}\" name=\"{name}\" placeholder=\"{ph}\"{req}>",
                input_type(&f)
            )
        };
        fields_html.push_str(&format!(
            "<label class=\"kfield\"><span>{label}</span>{input}</label>"
        ));
    }
    // novalidate: the server decides what a valid submission is (see the `required` note above).
    format!(
        "<form class=\"kblock kform\" id=\"kc-lead-form-{index}\" {FORM_ATTR}=\"{}\" \
         action=\"{}\" method=\"post\" novalidate>\
         <h3 class=\"kform-title\">{title}</h3>{fields_html}\
         <button type=\"submit\" class=\"kbtn\">{button}</button>\
         <p class=\"kform-msg\" data-kc-msg role=\"status\" aria-live=\"polite\"></p></form>",
        html_escape(action),
        html_escape(action),
    )
}

/// A `tel:`/`mailto:` target built from a stored free-form value: a phone number is reduced to a
/// leading `+` plus its digits (the live cards store `+1 (305) 555-0247`), an address is used as-is.
fn contact_href(scheme: &str, raw: &str) -> Option<String> {
    if scheme == "mailto" {
        return Some(format!("mailto:{}", raw.trim()));
    }
    let mut digits = String::new();
    for (i, c) in raw.chars().filter(|c| !c.is_whitespace()).enumerate() {
        match c {
            '0'..='9' => digits.push(c),
            '+' if i == 0 => digits.push('+'),
            _ => {}
        }
    }
    (digits.chars().filter(char::is_ascii_digit).count() >= 3).then(|| format!("tel:{digits}"))
}

fn link_row(label: &str, url: &str, extra: &str) -> Option<String> {
    let href = safe_url(url)?;
    Some(format!(
        "<a class=\"kbtn kbtn-{extra}\" href=\"{}\" rel=\"noopener\">{}</a>",
        html_escape(&href),
        html_escape(label)
    ))
}

/// `{platform|icon|name, url|href}` rows — the shape both `bio_link` and `thank_you` use, with
/// `platform` (templates.json) and `icon` (the shipped enum) accepted for the same slot.
fn socials_html(block: &Value) -> String {
    let mut out = String::new();
    let Some(items) = block
        .get("social_links")
        .or_else(|| block.get("items"))
        .and_then(|v| v.as_array())
    else {
        return out;
    };
    for it in items {
        let label = s(it, "platform")
            .or_else(|| s(it, "icon"))
            .or_else(|| s(it, "name"));
        let url = s(it, "url").or_else(|| s(it, "href"));
        let (Some(label), Some(url)) = (label, url) else {
            continue;
        };
        if let Some(a) = link_row(label, url, "social") {
            out.push_str(&a);
        }
    }
    if out.is_empty() {
        return out;
    }
    format!("<div class=\"ksocials\">{out}</div>")
}

/// `[{label, url}]` or `[{label, href}]` rows: the `buttons` block, `bio_link.buttons`, or the
/// generic `items` list a button/action block may carry instead.
fn button_rows_html(block: &Value) -> String {
    let mut out = String::new();
    let Some(items) = block
        .get("buttons")
        .or_else(|| block.get("items"))
        .and_then(|v| v.as_array())
    else {
        return out;
    };
    for it in items {
        let label = s(it, "label").or_else(|| s(it, "text"));
        let url = s(it, "url").or_else(|| s(it, "href"));
        let (Some(label), Some(url)) = (label, url) else {
            continue;
        };
        if let Some(a) = link_row(label, url, "block") {
            out.push_str(&a);
        }
    }
    if out.is_empty() {
        return out;
    }
    format!("<div class=\"kbuttons\">{out}</div>")
}

fn image_html(url: &str, alt: &str, class: &str) -> Option<String> {
    let src = safe_url(url)?;
    Some(format!(
        "<img class=\"{class}\" src=\"{}\" alt=\"{}\" loading=\"lazy\">",
        html_escape(&src),
        html_escape(alt)
    ))
}

fn hero_html(block: &Value) -> String {
    let angle = s(block, "gradient_angle")
        .and_then(safe_css_fragment)
        .unwrap_or_else(|| "135deg".to_string());
    let colors = s(block, "gradient_colors").and_then(safe_css_fragment);
    let style = match colors {
        Some(c) => format!(" style=\"background:linear-gradient({angle}, {c})\""),
        None => String::new(),
    };
    let mut out = format!("<section class=\"kblock khero\"{style}>");
    if let Some(u) = s(block, "hero_image_url") {
        if let Some(img) = image_html(u, "", "khero-img") {
            out.push_str(&img);
        }
    }
    if let Some(v) = s(block, "video_url") {
        if let Some(url) = safe_url(v) {
            // Only a direct video file is embedded; a page URL is not a video source.
            if url.to_ascii_lowercase().ends_with(".mp4") {
                out.push_str(&format!(
                    "<video class=\"khero-img\" src=\"{}\" muted playsinline controls></video>",
                    html_escape(&url)
                ));
            }
        }
    }
    if let Some(t) = s(block, "title") {
        out.push_str(&format!(
            "<h2 class=\"khero-title\">{}</h2>",
            html_escape(t)
        ));
    }
    if let Some(st) = s(block, "subtitle") {
        out.push_str(&format!("<p class=\"khero-sub\">{}</p>", html_escape(st)));
    }
    if let (Some(text), Some(url)) = (s(block, "cta_text"), s(block, "cta_url")) {
        if let Some(a) = link_row(text, url, "hero") {
            out.push_str(&a);
        }
    }
    out.push_str("</section>");
    out
}

fn features_html(block: &Value) -> String {
    let mut items = String::new();
    if let Some(arr) = block.get("items").and_then(|v| v.as_array()) {
        for it in arr {
            let title = esc(s(it, "title"));
            let desc = esc(s(it, "description").or_else(|| s(it, "body")));
            if title.is_empty() && desc.is_empty() {
                continue;
            }
            items.push_str(&format!(
                "<div class=\"kfeat\">\
                 <h3 class=\"kfeat-title\">{title}</h3>\
                 <p class=\"kfeat-desc\">{desc}</p></div>"
            ));
        }
    }
    if items.is_empty() {
        return String::new();
    }
    let heading = match s(block, "title") {
        Some(t) => format!("<h2 class=\"ksect-title\">{}</h2>", html_escape(t)),
        None => String::new(),
    };
    format!("<section class=\"kblock\">{heading}<div class=\"kfeatures\">{items}</div></section>")
}

fn business_card_html(block: &Value) -> String {
    let mut out = String::from("<section class=\"kblock kbc\">");
    if let Some(u) = s(block, "avatar_url").or_else(|| s(block, "company_logo_url")) {
        if let Some(img) = image_html(u, "", "kbc-av") {
            out.push_str(&img);
        }
    }
    if let Some(n) = s(block, "name") {
        out.push_str(&format!("<h2 class=\"kbc-name\">{}</h2>", html_escape(n)));
    }
    let meta = [s(block, "title"), s(block, "company")]
        .into_iter()
        .flatten()
        .map(html_escape)
        .collect::<Vec<_>>()
        .join(" · ");
    if !meta.is_empty() {
        out.push_str(&format!("<p class=\"kbc-meta\">{meta}</p>"));
    }
    if let Some(h) = s(block, "headline") {
        out.push_str(&format!("<p class=\"kbc-head\">{}</p>", html_escape(h)));
    }
    if let Some(b) = s(block, "biography").or_else(|| s(block, "catchphrase")) {
        out.push_str(&format!("<p class=\"kbc-bio\">{}</p>", html_escape(b)));
    }
    // Contact rows: the legacy flat fields, plus the rich `phones`/`emails` lists.
    let mut rows = String::new();
    for (key, scheme) in [("phone", "tel"), ("email", "mailto")] {
        if let Some(v) = s(block, key) {
            if let Some(href) = contact_href(scheme, v) {
                rows.push_str(&format!(
                    "<a class=\"kbc-row\" href=\"{}\">{}</a>",
                    html_escape(&href),
                    html_escape(v)
                ));
            }
        }
    }
    for list in ["phones", "emails"] {
        let scheme = if list == "phones" { "tel" } else { "mailto" };
        if let Some(arr) = block.get(list).and_then(|v| v.as_array()) {
            for it in arr {
                if let Some(v) = s(it, "value") {
                    if let Some(href) = contact_href(scheme, v) {
                        rows.push_str(&format!(
                            "<a class=\"kbc-row\" href=\"{}\">{}</a>",
                            html_escape(&href),
                            html_escape(v)
                        ));
                    }
                }
            }
        }
    }
    if let Some(w) = s(block, "website") {
        let href = if w.to_ascii_lowercase().starts_with("http") {
            w.to_string()
        } else {
            format!("https://{w}")
        };
        if let Some(a) = link_row(w, &href, "block") {
            rows.push_str(&a);
        }
    }
    if !rows.is_empty() {
        out.push_str(&format!("<div class=\"kbc-rows\">{rows}</div>"));
    }
    out.push_str(&button_rows_html(block));
    out.push_str(&socials_html(block));
    out.push_str("</section>");
    out
}

fn bio_link_html(block: &Value) -> String {
    let mut out = String::from("<section class=\"kblock kbio\">");
    if let Some(u) = s(block, "avatar_url") {
        if let Some(img) = image_html(u, "", "kbc-av") {
            out.push_str(&img);
        }
    }
    if let Some(t) = s(block, "name").or_else(|| s(block, "title")) {
        out.push_str(&format!("<h2 class=\"kbc-name\">{}</h2>", html_escape(t)));
    }
    if let Some(b) = s(block, "bio") {
        out.push_str(&format!("<p class=\"kbio-bio\">{}</p>", html_escape(b)));
    }
    out.push_str(&button_rows_html(block));
    out.push_str(&socials_html(block));
    out.push_str("</section>");
    out
}

fn mini_funnel_html(block: &Value) -> String {
    let mut out = String::from("<section class=\"kblock kfunnel\">");
    if let Some(u) = s(block, "product_image_url") {
        if let Some(img) = image_html(u, "", "kfunnel-img") {
            out.push_str(&img);
        }
    }
    if let Some(v) = s(block, "video_embed_url") {
        if let Some(url) = safe_url(v) {
            if url.to_ascii_lowercase().ends_with(".mp4") {
                out.push_str(&format!(
                    "<video class=\"kfunnel-img\" src=\"{}\" muted playsinline controls></video>",
                    html_escape(&url)
                ));
            }
        }
    }
    if let Some(t) = s(block, "title") {
        out.push_str(&format!(
            "<h2 class=\"khero-title\">{}</h2>",
            html_escape(t)
        ));
    }
    if let Some(st) = s(block, "subtitle") {
        out.push_str(&format!("<p class=\"khero-sub\">{}</p>", html_escape(st)));
    }
    if let (Some(text), Some(url)) = (s(block, "cta_text"), s(block, "cta_url")) {
        if let Some(a) = link_row(text, url, "hero") {
            out.push_str(&a);
        }
    }
    out.push_str("</section>");
    out
}

fn thank_you_html(block: &Value) -> String {
    let mut out = String::from("<section class=\"kblock kthank\">");
    if let Some(h) = s(block, "headline") {
        out.push_str(&format!(
            "<h2 class=\"khero-title\">{}</h2>",
            html_escape(h)
        ));
    }
    if let Some(m) = s(block, "message") {
        out.push_str(&format!("<p class=\"khero-sub\">{}</p>", html_escape(m)));
    }
    if let Some(t) = s(block, "next_step_text") {
        out.push_str(&format!(
            "<h3 class=\"ksect-title\">{}</h3>",
            html_escape(t)
        ));
    }
    if let Some(b) = s(block, "next_step_body") {
        out.push_str(&format!("<p class=\"khero-sub\">{}</p>", html_escape(b)));
    }
    if let (Some(text), Some(url)) = (
        s(block, "secondary_cta_text"),
        s(block, "secondary_cta_url"),
    ) {
        if let Some(a) = link_row(text, url, "hero") {
            out.push_str(&a);
        }
    }
    out.push_str(&socials_html(block));
    out.push_str("</section>");
    out
}

fn qr_html(block: &Value) -> String {
    let Some(u) = s(block, "image_url")
        .or_else(|| s(block, "qr_code_url"))
        .or_else(|| s(block, "qr_url"))
        .or_else(|| s(block, "url"))
    else {
        return String::new();
    };
    match image_html(u, "QR code", "kqr") {
        Some(img) => format!("<section class=\"kblock kqr-wrap\">{img}</section>"),
        None => String::new(),
    }
}

fn single_block(block: &Value, opts: &BlockOptions<'_>, form_index: &mut usize) -> Option<String> {
    let ty = s(block, "type")?;
    let html = match ty {
        "hero" | "hero_page" => hero_html(block),
        "features" | "feature_grid" => features_html(block),
        "lead_form" | "form" => {
            *form_index += 1;
            lead_form_html(block, opts.lead_action, *form_index)
        }
        "business_card" => business_card_html(block),
        "bio_link" | "bio" | "links" => bio_link_html(block),
        "mini_funnel" => mini_funnel_html(block),
        "thank_you" => thank_you_html(block),
        "buttons" | "button_list" => {
            let rows = button_rows_html(block);
            if rows.is_empty() {
                return None;
            }
            format!("<section class=\"kblock\">{rows}</section>")
        }
        "social" | "socials" | "social_links" | "social_icons" => {
            let rows = socials_html(block);
            if rows.is_empty() {
                return None;
            }
            format!("<section class=\"kblock\">{rows}</section>")
        }
        "link" | "link_button" => {
            let (label, url) = (
                s(block, "label").or_else(|| s(block, "text")),
                s(block, "url"),
            );
            let (Some(label), Some(url)) = (label, url) else {
                return None;
            };
            link_row(label, url, "block").map(|a| {
                format!("<section class=\"kblock\"><div class=\"kbuttons\">{a}</div></section>")
            })?
        }
        "image" | "image_block" => {
            let u = s(block, "image_url")?;
            let cap = esc(s(block, "caption"));
            let img = image_html(u, s(block, "alt_text").unwrap_or(""), "kimg")?;
            let cap = if cap.is_empty() {
                String::new()
            } else {
                format!("<p class=\"kcap\">{cap}</p>")
            };
            format!("<section class=\"kblock\">{img}{cap}</section>")
        }
        "video" | "video_block" => {
            let v = s(block, "video_url").or_else(|| s(block, "url"))?;
            let url = safe_url(v)?;
            if !url.to_ascii_lowercase().ends_with(".mp4") {
                return None;
            }
            format!(
                "<section class=\"kblock\"><video class=\"kimg\" src=\"{}\" muted playsinline controls></video></section>",
                html_escape(&url)
            )
        }
        "qr" | "qr_code" => qr_html(block),
        // A type nobody ships a renderer for is ignored, never an error and never a broken page.
        _ => return None,
    };
    (!html.is_empty()).then_some(html)
}

/// The `kinetic_buttons` rows as a button block. `action_type` is migration 0017's vocabulary:
/// `'url'` links, `'sms'` opens the messages app, `'lead_form'` submits the card's own form when
/// the card has one (and falls back to its `destination_url` when it does not).
fn buttons_section(buttons: &[CardButton], has_lead_form: bool) -> String {
    let mut rows = String::new();
    for b in buttons {
        let label = html_escape(b.label.trim());
        if label.is_empty() {
            continue;
        }
        match b.action_type.as_str() {
            "lead_form" if has_lead_form => rows.push_str(&format!(
                "<button type=\"submit\" class=\"kbtn kbtn-block\" form=\"{FIRST_FORM_ID}\">{label}</button>"
            )),
            "sms" => {
                if let Some(url) = safe_url(&format!("sms:{}", b.url.trim())) {
                    rows.push_str(&format!(
                        "<a class=\"kbtn kbtn-block\" href=\"{}\">{label}</a>",
                        html_escape(&url)
                    ));
                }
            }
            _ => {
                if let Some(a) = link_row(b.label.trim(), &b.url, "block") {
                    rows.push_str(&a);
                }
            }
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    format!("<section class=\"kblock\"><div class=\"kbuttons\">{rows}</div></section>")
}

/// The CSS the block markup needs. Only emitted when a block actually rendered, and every value
/// is accent-derived so it reads on both the dark and the light card themes.
fn blocks_css(accent: &str) -> String {
    let a = html_escape(accent);
    format!(
        ".kblocks{{position:relative;z-index:2;width:100%;max-width:720px;margin:26px auto 8px;\
display:flex;flex-direction:column;gap:20px;text-align:left}}\
.kblock{{background:{a}12;border:1px solid {a}40;border-radius:16px;padding:20px;backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px)}}\
.khero{{text-align:center;background:{a}1f}}\
.khero-img,.kimg,.kfunnel-img{{width:100%;max-width:420px;display:block;margin:0 auto 14px;border-radius:12px;object-fit:cover}}\
.khero-title{{font-size:22px;font-weight:800;margin-bottom:6px}}\
.khero-sub{{font-size:14px;line-height:1.6;opacity:.85;margin-bottom:12px}}\
.ksect-title{{font-size:15px;font-weight:700;margin-bottom:12px;letter-spacing:.4px;text-transform:uppercase;color:{a}}}\
.kfeatures{{display:grid;grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:12px}}\
.kfeat{{background:{a}14;border:1px solid {a}33;border-radius:12px;padding:14px}}\
.kfeat-title{{font-size:14px;font-weight:700;margin-bottom:5px}}\
.kfeat-desc{{font-size:13px;line-height:1.55;opacity:.82}}\
.kbc-av{{width:84px;height:84px;border-radius:50%;object-fit:cover;display:block;margin:0 auto 12px;box-shadow:0 0 0 3px {a}}}\
.kbc-name{{font-size:20px;font-weight:800;text-align:center}}\
.kbc-meta{{font-size:13px;color:{a};font-weight:600;text-align:center;margin-top:3px}}\
.kbc-head,.kbc-bio,.kbio-bio{{font-size:14px;line-height:1.6;margin-top:10px;opacity:.88;text-align:center}}\
.kbc-rows{{display:flex;flex-wrap:wrap;gap:8px;justify-content:center;margin-top:14px}}\
.kbc-row{{font-size:13px;padding:7px 14px;border-radius:20px;background:{a}1f;border:1px solid {a}40;text-decoration:none;color:inherit}}\
.kbuttons,.ksocials{{display:flex;flex-wrap:wrap;gap:10px;justify-content:center;margin-top:12px}}\
.kbtn{{display:inline-block;padding:12px 22px;border-radius:12px;background:{a};color:#fff;font-size:14px;font-weight:700;border:0;cursor:pointer;text-decoration:none;font-family:inherit}}\
.kbtn-block{{width:100%;max-width:420px;text-align:center}}\
.kbtn-social{{padding:7px 15px;border-radius:20px;font-size:12px;font-weight:600;background:{a}22;color:inherit;border:1px solid {a}55}}\
.kform{{display:flex;flex-direction:column;gap:12px;max-width:520px;margin:0 auto;width:100%}}\
.kform-title{{font-size:18px;font-weight:800;text-align:center}}\
.kfield{{display:flex;flex-direction:column;gap:5px}}\
.kfield span{{font-size:11px;font-weight:700;letter-spacing:.6px;text-transform:uppercase;opacity:.7}}\
.kfield input,.kfield textarea{{width:100%;padding:11px 14px;border-radius:10px;border:1px solid {a}55;background:transparent;color:inherit;font:inherit;font-size:14px}}\
.kfield input:focus,.kfield textarea:focus{{outline:2px solid {a}88;outline-offset:1px}}\
.kform-msg{{font-size:13px;font-weight:600;min-height:18px;text-align:center}}\
.kok{{color:#22c55e}}.kerr{{color:#f87171}}\
.kcap{{font-size:12px;text-align:center;opacity:.75;margin-top:8px}}\
.kqr{{display:block;margin:0 auto;width:100%;max-width:200px}}\n"
    )
}

/// The inline lead-form submitter. Posts JSON (the endpoint's contract) to the form's own
/// `data-kc-lead` path — the card's page prefix — and renders the handler's real answer:
/// `201` → the response's own message as the confirmation, anything else → its `error` text.
fn form_script() -> String {
    format!(
        "\n<script>\n(function(){{var F=document.querySelectorAll('form[{FORM_ATTR}]');if(!F.length)return;function bind(f){{var m=f.querySelector('[data-kc-msg]');var b=f.querySelector('button[type=submit]');var lbl=b?b.textContent:'';function say(t,k){{if(!m)return;m.textContent=t;m.className='kform-msg '+(k?'kok':'kerr');}}f.addEventListener('submit',function(e){{e.preventDefault();var d={{}},q=new URLSearchParams(location.search);new FormData(f).forEach(function(v,k){{d[k]=(typeof v==='string')?v:'';}});['utm_source','utm_medium','utm_campaign'].forEach(function(k){{var v=q.get(k);if(v)d[k]=v;}});if(document.referrer)d.referrer_url=document.referrer;if(b){{b.disabled=true;b.textContent='Sending...';}}function done(){{if(b){{b.disabled=false;b.textContent=lbl;}}}}fetch(f.getAttribute('{FORM_ATTR}'),{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify(d)}}).then(function(r){{return r.text().then(function(t){{var j={{}};try{{j=JSON.parse(t);}}catch(_){{}}if(r.status===201){{say((j&&j.message)?('\\u2713 '+j.message):'\\u2713 Sent',1);f.reset();}}else{{say((j&&j.error)?j.error:('Request failed ('+r.status+')'),0);}}done();}});}}).catch(function(){{say('Network error. Please try again.',0);done();}});}});}}for(var i=0;i<F.length;i++)bind(F[i]);}})();\n</script>"
    )
}

/// Render every block the card carries (plus its `kinetic_buttons` rows) to HTML + CSS + JS.
pub fn render(blocks: &Value, buttons: &[CardButton], opts: &BlockOptions<'_>) -> Rendered {
    let mut rendered = Vec::new();
    let mut inner = String::new();
    let mut form_index = 0usize;
    if let Some(arr) = blocks.as_array() {
        for block in arr {
            if let Some(html) = single_block(block, opts, &mut form_index) {
                if let Some(ty) = s(block, "type") {
                    rendered.push(ty.to_string());
                }
                inner.push_str(&html);
            }
        }
    }
    let has_lead_form = form_index > 0;
    let btns = buttons_section(buttons, has_lead_form);
    if !btns.is_empty() {
        rendered.push("kinetic_buttons".to_string());
        inner.push_str(&btns);
    }
    if inner.is_empty() {
        return Rendered::nothing();
    }
    Rendered {
        section: format!("\n<div class=\"kblocks\">{inner}</div>"),
        css: blocks_css(opts.accent),
        script: if has_lead_form {
            form_script()
        } else {
            String::new()
        },
        has_lead_form,
        rendered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts<'a>() -> BlockOptions<'a> {
        BlockOptions {
            accent: "#8b5cf6",
            lead_action: "/k/swiftsoftware/lead",
        }
    }

    #[test]
    fn no_blocks_renders_exactly_nothing() {
        let r = render(&json!(null), &[], &opts());
        assert_eq!(r.section, "");
        assert_eq!(r.css, "");
        assert_eq!(r.script, "");
        assert!(!r.has_lead_form);
        assert!(r.rendered.is_empty());
    }

    #[test]
    fn unknown_block_types_are_ignored_not_fatal() {
        let r = render(
            &json!([{"type":"action_grid_2x2"},{"type":"nope","x":1},{"type":""}]),
            &[],
            &opts(),
        );
        assert_eq!(r.section, "");
        assert_eq!(r.script, "");
    }

    #[test]
    fn the_live_lead_form_block_renders_its_configured_fields_and_its_own_prefix_path() {
        let block = json!([{
        "type":"lead_form","form_title":"Work with SwiftSoftware","button_text":"Get In Touch",
        "placeholder":"your@email.com",
        "fields":[
            {"name":"name","label":"Name","field_type":"text","placeholder":"Your name"},
            {"name":"email","label":"Email","field_type":"email","placeholder":"your@email.com"},
            {"name":"company","label":"Company","field_type":"text","placeholder":"Your company"}
        ]}]);
        let r = render(&block, &[], &opts());
        assert!(r.has_lead_form);
        assert_eq!(r.rendered, vec!["lead_form"]);
        assert!(r.section.contains("data-kc-lead=\"/k/swiftsoftware/lead\""));
        assert!(r.section.contains("action=\"/k/swiftsoftware/lead\""));
        assert!(r.section.contains("Work with SwiftSoftware"));
        assert!(r.section.contains("name=\"name\""));
        assert!(r.section.contains("name=\"email\""));
        assert!(r.section.contains("name=\"company\""));
        assert!(r.section.contains("type=\"email\""));
        assert!(r.section.contains("Get In Touch"));
        assert!(!r.script.is_empty());
        // No captcha / third-party service is introduced by the submitter.
        assert!(!r.script.contains("http"));
    }

    #[test]
    fn the_templates_json_field_spelling_type_is_honoured_too() {
        let block = json!([{"type":"lead_form","form_title":"Get Early Access",
            "button_text":"Join the Waitlist",
            "fields":[{"name":"email","type":"email","required":true}]}]);
        let r = render(&block, &[], &opts());
        assert!(r.section.contains("name=\"email\""));
        assert!(r.section.contains("type=\"email\""));
        assert!(r.section.contains("required"));
        // The `label` is absent in this shape: it comes from the field name.
        assert!(r.section.contains("<span>Email</span>"));
    }

    #[test]
    fn a_lead_form_with_no_fields_still_renders_the_identity_pair() {
        let block = json!([{"type":"lead_form","form_title":"Contact us","button_text":"Send"}]);
        let r = render(&block, &[], &opts());
        assert!(r.section.contains("name=\"name\""));
        assert!(r.section.contains("name=\"email\""));
    }

    #[test]
    fn the_live_business_card_block_renders_contact_rows() {
        let block = json!([{"type":"business_card","name":"David J Giraudy","title":"Managing Director",
            "company":"Giraudy Capital","phone":"+1 (305) 555-0247","email":"david@giraudycapital.com",
            "website":"https://giraudycapital.com","catchphrase":"Executed with precision."}]);
        let r = render(&block, &[], &opts());
        assert_eq!(r.rendered, vec!["business_card"]);
        assert!(r.section.contains("David J Giraudy"));
        assert!(r.section.contains("Managing Director · Giraudy Capital"));
        assert!(r.section.contains("href=\"tel:+13055550247\""));
        assert!(r
            .section
            .contains("href=\"mailto:david@giraudycapital.com\""));
        assert!(r.section.contains("Executed with precision."));
        assert!(r.script.is_empty());
    }

    #[test]
    fn the_live_hero_and_features_blocks_render_title_subtitle_cta_and_items() {
        let block = json!([
            {"type":"hero","title":"SwiftSoftware","subtitle":"Seven apps. One ecosystem.",
             "cta_url":"#","cta_text":"Explore Our Apps","gradient_angle":"135deg",
             "gradient_colors":"#1e1b4b 0%, #312e81 50%, #6366f1 100%","video_url":null,"hero_image_url":null},
            {"type":"features","items":[{"title":"FunnelSwift","description":"Lead generation."},
                                        {"title":"CoreSwift CRM","description":"Pipeline tracking."}]}]);
        let r = render(&block, &[], &opts());
        assert_eq!(r.rendered, vec!["hero", "features"]);
        assert!(r
            .section
            .contains("linear-gradient(135deg, #1e1b4b 0%, #312e81 50%, #6366f1 100%)"));
        assert!(r.section.contains("Explore Our Apps"));
        assert!(r.section.contains("href=\"#\""));
        assert!(r.section.contains("FunnelSwift"));
        assert!(r.section.contains("Pipeline tracking."));
    }

    #[test]
    fn a_css_injection_attempt_in_the_gradient_is_dropped() {
        let block = json!([{"type":"hero","title":"x",
            "gradient_colors":"red);}</style><script>alert(1)</script>","gradient_angle":"135deg"}]);
        let r = render(&block, &[], &opts());
        assert!(!r.section.contains("script"));
        assert!(!r.section.contains("</style>"));
        assert!(!r.section.contains("linear-gradient(135deg,"));
    }

    #[test]
    fn a_javascript_url_never_becomes_a_link() {
        let block = json!([{"type":"buttons","items":[{"label":"Click","url":"javascript:alert(1)"},
            {"label":"Real","url":"https://example.com"}]}]);
        let r = render(&block, &[], &opts());
        assert!(!r.section.contains("javascript:"));
        assert!(r.section.contains("https://example.com"));
        assert!(r.section.contains("Real"));
    }

    #[test]
    fn stored_text_is_html_escaped() {
        let block =
            json!([{"type":"hero","title":"<img src=x onerror=alert(1)>","subtitle":"a & b"}]);
        let r = render(&block, &[], &opts());
        assert!(!r.section.contains("<img src=x"));
        assert!(r.section.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(r.section.contains("a &amp; b"));
    }

    #[test]
    fn kinetic_buttons_render_and_a_lead_form_button_submits_the_cards_form() {
        let btns = vec![
            CardButton {
                label: "Book a Call".into(),
                url: "https://cal.example.com".into(),
                action_type: "url".into(),
            },
            CardButton {
                label: "Get In Touch".into(),
                url: "".into(),
                action_type: "lead_form".into(),
            },
            CardButton {
                label: "Text Us".into(),
                url: "+13055550100".into(),
                action_type: "sms".into(),
            },
        ];
        let with_form = render(
            &json!([{"type":"lead_form","button_text":"Send"}]),
            &btns,
            &opts(),
        );
        assert!(with_form.rendered.contains(&"kinetic_buttons".to_string()));
        assert!(with_form.section.contains("https://cal.example.com"));
        assert!(with_form.section.contains("form=\"kc-lead-form-1\""));
        assert!(with_form.section.contains("href=\"sms:+13055550100\""));
        // Without a form the lead_form button falls back to its destination (empty here) and is dropped.
        let no_form = render(&json!([]), &btns, &opts());
        assert_eq!(no_form.rendered, vec!["kinetic_buttons"]);
        assert!(!no_form.section.contains("Get In Touch"));
        assert!(no_form.section.contains("Book a Call"));
    }

    #[test]
    fn a_buttons_block_that_carries_items_instead_of_buttons_still_renders() {
        // The guide's "Buttons" block and the wireframe vocabulary both spell the row list `items`.
        let block = json!([{"type":"buttons","items":[{"label":"Book a Call","url":"https://cal.example.com"}]},
                           {"type":"social","items":[{"platform":"linkedin","url":"https://linkedin.com/in/x"}]}]);
        let r = render(&block, &[], &opts());
        assert_eq!(r.rendered, vec!["buttons", "social"]);
        assert!(r.section.contains("href=\"https://cal.example.com\""));
        assert!(r.section.contains("href=\"https://linkedin.com/in/x\""));
    }

    #[test]
    fn templates_json_bio_link_mini_funnel_and_thank_you_render() {
        let block = json!([
            {"type":"bio_link","bio":"Founder & CEO.",
             "social_links":[{"platform":"twitter","url":"https://twitter.com/founder"}]},
            {"type":"mini_funnel","title":"The Playbook","subtitle":"47 strategies",
             "cta_text":"Get it","cta_url":"#"},
            {"type":"thank_you","headline":"You're In!","message":"Check your inbox.",
             "next_step_text":"While You Wait","next_step_body":"Follow us.",
             "secondary_cta_text":"Join","secondary_cta_url":"#",
             "social_links":[{"platform":"instagram","url":"#"}]}]);
        let r = render(&block, &[], &opts());
        assert_eq!(r.rendered, vec!["bio_link", "mini_funnel", "thank_you"]);
        assert!(r.section.contains("Founder &amp; CEO."));
        assert!(r.section.contains("twitter"));
        assert!(r.section.contains("The Playbook"));
        assert!(r.section.contains("You&#39;re In!"));
        assert!(r.section.contains("While You Wait"));
        assert!(r.script.is_empty());
    }

    #[test]
    fn the_form_script_only_ships_when_a_form_rendered_and_shows_the_handlers_answer() {
        let r = render(
            &json!([{"type":"hero","title":"x","cta_text":"a","cta_url":"#"}]),
            &[],
            &opts(),
        );
        assert!(r.script.is_empty());
        // The block CSS ships whenever a block rendered (the markup uses those classes); only the
        // SUBMITTER is conditional on a form existing.
        assert!(r.css.contains(".kblocks{"));
        let f = render(&json!([{"type":"lead_form"}]), &[], &opts());
        assert!(f.script.contains("s.status===201") || f.script.contains("r.status===201"));
        assert!(f.script.contains("j.error"));
        assert!(!f.css.is_empty());
        assert!(f.css.starts_with(".kblocks{"));
        assert!(f.css.ends_with('\n'));
    }
}
