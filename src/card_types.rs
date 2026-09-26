//! THE value space of `kinetic_cards.template_type` — decided ONCE, here.
//!
//! ## Decision (kanban t_747b4dd6, 2026-09-25)
//!
//! `kinetic_cards.template_type` stores the **card type** (the card kind), written in the
//! UNDERSCORE ids the card editor's *Template Type* select declares — the list below, which is
//! mirrored by the single declaration `var CARD_TYPES=[…]` in `www-app/index.html` (a unit test in
//! this file fails the build if the two ever drift apart).
//!
//! The OTHER vocabulary measured in the column — the HYPHENATED card kinds API clients send in
//! `type`/`card_type` (`bio-link`, `business-card`, …) — is an **alias space**, not a second
//! value space, and cannot own the column:
//!
//!  1. it cannot represent values that already exist: the column DEFAULT is `'default'`, and
//!     `default` / `hero` / `thank_you` have no hyphenated counterpart anywhere in the app
//!     (the theme catalogue spells the hero *theme* kind `hero-page`, and no client ever sends it).
//!     Canonicalising the column onto it would mean inventing ids.
//!  2. every reader already speaks the underscore ids: the editor select (`www-app/index.html`),
//!     the card badge map, the template-library filter, `admin_handler.rs`'s
//!     `COALESCE(kc.template_type,'default')`, and `card_analytics_handler.rs`'s `card_type` alias.
//!  3. the plan gate does too: `kinetic_handler.rs`'s `create_card` compares `has_mini_funnels`
//!     against the literal `mini_funnel`, so a client that sent `type:"mini-funnel"` stored a
//!     gated card type for free. Normalising at the write boundary closes that hole.
//!
//! Aliases are therefore normalised on the way IN (`create_card` / `update_card`) and the rows that
//! were already hyphenated are canonicalised once by `migrations/057_canonical_card_template_type.sql`.
//! A value that is neither canonical nor a known alias (e.g. the plan matrix's `digital-card`, which
//! no code writes or reads) is **stored unchanged**: inventing a mapping for a spelling the app's own
//! text does not name is the failure mode this card exists to prevent, and the editor appends any
//! unknown stored value to its select so it stays representable.
//!
//! ## Census — every writer and reader of the column (HEAD of the t_747b4dd6 branch)
//!
//! Writers (both through [`stored`]):
//!   * `handlers::kinetic_handler::create_card` — INSERT, `template_type` from
//!     `body["template_type"]` else `body["type"]`/`body["card_type"]` else [`DEFAULT_CARD_TYPE`].
//!   * `handlers::kinetic_handler::update_card` — UPDATE, `template_type=COALESCE($17,template_type)`
//!     from `body["template_type"]` only (an absent key leaves the column alone).
//!
//! Readers:
//!   * `handlers::kinetic_handler::list_cards` — SELECT → JSON `template_type` (the SPA list + editor).
//!   * `www-app/index.html` — `CARD_TYPES` (source of truth for the select, badges and the preview's
//!     archetype styling), `cardTemplates()`'s append clause, `SC2`'s save body.
//!   * `handlers::admin_handler` — `COALESCE(kc.template_type,'default') as template_type`
//!     (rendered unchanged by `www-admin/index.html`).
//!   * `handlers::card_analytics_handler` — `k.template_type as card_type` (same column, same space;
//!     the JSON key keeps its name — see the comment at that SELECT).
//!   * `handlers::kinetic_handler::render_card` — reads the column into nothing: the dead binding was
//!     deleted, the public page renders from theme/colors/`layout_blocks`.
//!   * `features::enforce_template_access` / the `has_mini_funnels` gate — classify the value written
//!     by the two writers above.

/// The canonical card types — the ONE declaration of this column's value space in this service.
/// Mirrored by `var CARD_TYPES=[…]` in `www-app/index.html`; `spa_declares_the_same_ids` asserts it.
pub const CARD_TYPES: &[&str] = &[
    "default",
    "bio_link",
    "business_card",
    "mini_page",
    "mini_funnel",
    "hero",
    "thank_you",
];

/// Every non-canonical spelling measured in a client's `type`/`card_type` (left) → canonical id.
/// The first five are the hyphenated card kinds of the fleet's card APIs and of the theme
/// catalogue's per-theme `card_type`; the last three are the short aliases the SPA's own card-badge
/// map used before this card. Matching is case-insensitive and ignores surrounding whitespace.
pub const CARD_TYPE_ALIASES: &[(&str, &str)] = &[
    ("bio-link", "bio_link"),
    ("business-card", "business_card"),
    ("mini-page", "mini_page"),
    ("mini-funnel", "mini_funnel"),
    ("hero-page", "hero"),
    ("bio", "bio_link"),
    ("business", "business_card"),
    ("mini", "mini_page"),
];

/// The card type stored when a write carries neither `template_type` nor `type`/`card_type`.
/// (This is the previous fallback — the card kind that a bare "links only" POST means — now spelled
/// in the canonical space; before this card it was stored as the hyphenated `bio-link`.)
pub const DEFAULT_CARD_TYPE: &str = "bio_link";

/// The plan-gated card type (`plans.has_mini_funnels`, checked in `create_card`).
pub const MINI_FUNNEL: &str = "mini_funnel";

/// The canonical id for `raw`, or `None` for a value that is neither canonical nor a known alias
/// (the caller keeps such a value verbatim — see the module doc).
pub fn canonical(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if let Some(hit) = CARD_TYPES.iter().find(|id| **id == lower) {
        return Some(hit);
    }
    CARD_TYPE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map(|(_, id)| *id)
}

/// The value to STORE for a card write.
///
/// Precedence matches the editor and the API contract it replaced: an explicit `template_type`
/// (what the editor's *Template Type* select sends) wins; otherwise the card kind the client sent
/// as `type`/`card_type`; otherwise [`DEFAULT_CARD_TYPE`]. The winner is canonicalised, and a value
/// that is neither canonical nor a known alias is stored unchanged.
pub fn stored(template_type: Option<&str>, kind: Option<&str>) -> String {
    let raw = template_type
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| kind.map(str::trim).filter(|s| !s.is_empty()))
        .unwrap_or(DEFAULT_CARD_TYPE);
    canonical(raw)
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids declared by the SPA's `CARD_TYPES`, parsed out of the source that is actually served.
    const SPA: &str = include_str!("../www-app/index.html");

    fn spa_declared_ids() -> Vec<String> {
        const DECL: &str = "var CARD_TYPES=[";
        let start = SPA
            .find(DECL)
            .expect("www-app/index.html must declare `var CARD_TYPES=[...]`")
            + DECL.len();
        let end = SPA[start..]
            .find("];")
            .expect("the CARD_TYPES declaration must be closed with `];`")
            + start;
        let mut rest = &SPA[start..end];
        let mut ids = Vec::new();
        while let Some(pos) = rest.find("v:\"") {
            rest = &rest[pos + 3..];
            match rest.find('"') {
                Some(close) => {
                    ids.push(rest[..close].to_string());
                    rest = &rest[close + 1..];
                }
                None => break,
            }
        }
        ids
    }

    #[test]
    fn spa_declares_the_same_ids() {
        let spa = spa_declared_ids();
        assert_eq!(
            spa,
            CARD_TYPES.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "www-app/index.html's CARD_TYPES and card_types::CARD_TYPES have drifted apart"
        );
    }

    #[test]
    fn aliases_point_at_canonical_ids_and_nothing_else_does() {
        for (alias, id) in CARD_TYPE_ALIASES {
            assert!(
                CARD_TYPES.contains(id),
                "alias {alias} -> {id} is not canonical"
            );
            assert!(
                !CARD_TYPES.contains(alias),
                "alias {alias} is already a canonical id"
            );
            assert_eq!(canonical(alias), Some(*id));
        }
    }

    #[test]
    fn canonical_is_idempotent_and_measures_the_live_spellings() {
        for id in CARD_TYPES {
            assert_eq!(canonical(id), Some(*id));
            assert_eq!(canonical(&id.to_uppercase()), Some(*id));
            assert_eq!(canonical(&format!("  {id}  ")), Some(*id));
        }
        // the two vocabularies measured in the live DB (2026-09-25)
        assert_eq!(canonical("bio-link"), Some("bio_link"));
        assert_eq!(canonical("business_card"), Some("business_card"));
        assert_eq!(canonical("mini-funnel"), Some("mini_funnel"));
        // neither canonical nor a known alias -> kept verbatim by `stored`
        assert_eq!(canonical("digital-card"), None);
        assert_eq!(canonical(""), None);
    }

    #[test]
    fn stored_precedence_and_fallback() {
        assert_eq!(
            stored(Some("business_card"), Some("bio-link")),
            "business_card"
        );
        assert_eq!(stored(Some("  "), Some("bio-link")), "bio_link");
        assert_eq!(stored(None, Some("mini-funnel")), "mini_funnel");
        assert_eq!(stored(None, None), DEFAULT_CARD_TYPE);
        assert_eq!(stored(None, Some("digital-card")), "digital-card");
    }
}
