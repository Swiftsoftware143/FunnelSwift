//! Askama templates for FunnelSwift SSR pages (kinetic bio-links / mini-pages)

use askama::Template;
use serde::{Deserialize, Serialize};

/// Dynamic layout blocks that power both bio-link and mini-page layouts.
/// Uses serde tag for JSON (de)serialization matching the DB layout_blocks column.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutBlock {
    /// Bio-link style: centered column, avatar, buttons, socials
    BioLink {
        avatar_url: Option<String>,
        video_url: Option<String>,
        bio: String,
        #[serde(default)]
        buttons: Vec<BioButton>,
        #[serde(default)]
        social_links: Vec<SocialLink>,
    },
    /// Hero section for mini-page — full-width hero with optional portrait image
    Hero {
        title: String,
        subtitle: String,
        cta_text: String,
        cta_url: String,
        hero_image_url: Option<String>,
        video_url: Option<String>,
        gradient_angle: Option<String>,
        gradient_colors: Option<String>,
    },
    /// Features grid (3-col desktop, 1-col mobile)
    Features {
        items: Vec<FeatureItem>,
    },
    /// Digital business card — rich schema: multi-contact, actions, geo, QR
    BusinessCard {
        name: String,
        title: Option<String>,
        company: Option<String>,
        company_logo_url: Option<String>,
        avatar_url: Option<String>,
        banner_url: Option<String>,
        catchphrase: Option<String>,
        headline: Option<String>,
        biography: Option<String>,
        /// Legacy flat fields (backward compat)
        #[serde(default)]
        phone: Option<String>,
        #[serde(default)]
        email: Option<String>,
        website: Option<String>,
        /// Rich multi-contact lists
        #[serde(default)]
        phones: Vec<BizContact>,
        #[serde(default)]
        emails: Vec<BizContact>,
        /// Location
        location: Option<String>,
        geo: Option<BizGeo>,
        /// Generic action buttons
        #[serde(default)]
        actions: Vec<BizAction>,
        /// QR code
        qr_code_url: Option<String>,
        /// Lead capture endpoint
        lead_capture_url: Option<String>,
        #[serde(default)]
        buttons: Vec<BioButton>,
        #[serde(default)]
        social_links: Vec<SocialLink>,
    },
    /// Mini funnel — centered card with product image/video, title, big CTA
    MiniFunnel {
        title: String,
        subtitle: Option<String>,
        product_image_url: Option<String>,
        video_embed_url: Option<String>,
        cta_text: String,
        cta_url: String,
        theme_style: Option<String>,
    },
    /// Lead capture form
    LeadForm {
        form_title: String,
        placeholder: String,
        button_text: String,
        fields: Vec<FormField>,
    },
    /// --- NEW BLOCK TYPES ---

    /// Single image block — full-width or card-style
    ImageBlock {
        image_url: String,
        caption: Option<String>,
        alt_text: Option<String>,
        link_url: Option<String>,
        /// "full" | "card" | "rounded"
        style: Option<String>,
        /// Optional password gate — content hidden until password entered
        password_hash: Option<String>,
    },
    /// Single video block (embed or direct mp4)
    VideoBlock {
        /// "youtube" | "vimeo" | "mp4"
        provider: String,
        video_id: String,
        caption: Option<String>,
        autoplay: Option<bool>,
        muted: Option<bool>,
        loop_play: Option<bool>,
        /// Optional password gate
        password_hash: Option<String>,
    },
    /// Gallery / carousel of images and/or videos
    GalleryBlock {
        title: Option<String>,
        items: Vec<GalleryItem>,
        /// "grid" | "carousel" | "masonry"
        layout: Option<String>,
        /// Optional password gate for whole gallery
        password_hash: Option<String>,
    },
    /// Standalone link button (for multi-link layouts)
    LinkBlock {
        label: String,
        url: String,
        /// "primary" | "secondary" | "outline" | "ghost"
        style: Option<String>,
        icon: Option<String>,
        /// Whether this link requires unlock
        require_unlock: Option<bool>,
    },
    /// Rich text / HTML content block
    RichText {
        content: String, // HTML allowed
        align: Option<String>, // "left" | "center" | "right"
    },
    /// Divider / spacer
    Divider {
        style: Option<String>, // "solid" | "dashed" | "dotted"
        thickness: Option<i32>,
        color: Option<String>,
        margin: Option<i32>,
    },
    /// Cookie consent banner (always visible, dismissible)
    CookieConsent {
        message: Option<String>,
        button_text: Option<String>,
        policy_url: Option<String>,
        /// "banner" | "modal" | "floating"
        position: Option<String>,
        ga_tracking_id: Option<String>,
    },
    /// Consent / age gate — appears on load, must accept to view content
    ConsentGate {
        title: String,
        message: String,
        button_text: String,
        decline_text: Option<String>,
        /// "overlay" | "fullscreen" | "modal"
        style: Option<String>,
        /// "age_18" | "age_21" | "custom"
        gate_type: Option<String>,
        /// Redirect URL on decline
        redirect_url: Option<String>,
    },
    /// Password gate — protects a section or the whole page
    PasswordGate {
        title: Option<String>,
        message: Option<String>,
        password_hash: String, // bcrypt hash
        placeholder: Option<String>,
        button_text: Option<String>,
        /// "section" | "page" — section = gate protects sections below, page = gates entire page
        scope: Option<String>,
        /// Tag to apply when unlocked (for tracking)
        unlock_tag_id: Option<String>,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GalleryItem {
    pub media_type: String, // "image" | "video" | "youtube" | "vimeo"
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BioButton {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SocialLink {
    pub icon: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BizContact {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BizGeo {
    pub lat: f64,
    pub lng: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BizAction {
    pub label: String,
    pub url: String,
    #[serde(rename = "type", default)]
    pub action_type: Option<String>,
    #[serde(default)]
    pub is_featured: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FeatureItem {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FormField {
    pub name: String,
    pub label: String,
    pub field_type: String,
    pub placeholder: String,
}

/// Template context for /k/:slug pages
#[derive(Template)]
#[template(path = "micro_page.html")]
pub struct PageTemplate<'a> {
    pub tenant_name: &'a str,
    pub page_title: &'a str,
    pub meta_description: &'a str,
    pub primary_color: &'a str,
    pub accent_color: &'a str,
    pub custom_css: &'a str,
    pub slug: &'a str,
    pub logo_url: Option<&'a str>,
    pub blocks: Vec<LayoutBlock>,
    /// Extracted from blocks for the modal
    pub modal_form_title: &'a str,
    pub modal_button_text: &'a str,
    pub modal_placeholder: &'a str,
    pub modal_fields: Vec<FormField>,
    /// Plan controls
    pub show_branding: bool,
    pub affiliate_code: Option<&'a str>,
    /// Page-level password gate
    pub page_password_hash: Option<&'a str>,
    /// Page-level consent requirement
    pub page_consent_required: bool,
    /// Branding CTA label — "Free Bio Link", "Free Mini Page", etc.
    pub cta_label: &'a str,
    /// True if bg_color is a dark color (auto-detected)
    pub is_dark: bool,
    /// Theme pattern: "gradient" | "bubbles" | "mesh" | "dots" | "circuit" | "brick" | "gradient_dots"
    pub theme: Option<&'a str>,
}

impl<'a> PageTemplate<'a> {
    /// Returns the CSS class for the body's background theme pattern.
    pub fn theme_class(&self) -> &str {
        match self.theme {
            Some("bubbles") => "bg-bubbles",
            Some("mesh") => "bg-mesh",
            Some("dots") => "bg-dots",
            Some("circuit") => "bg-circuit",
            Some("brick") => "bg-brick",
            Some("gradient_dots") => "bg-gradient-dots",
            Some("gradient") | None => "bg-gradient-accent",
            _ => "bg-gradient-accent",
        }
    }
}

/// Template for the admin plans feature-management page
#[derive(Template)]
#[template(path = "admin_plans.html")]
pub struct AdminPlansTemplate;

impl Default for PageTemplate<'_> {
    fn default() -> Self {
        Self {
            tenant_name: "",
            page_title: "",
            meta_description: "",
            primary_color: "#3B82F6",
            accent_color: "#8B5CF6",
            custom_css: "",
            slug: "",
            is_dark: false,
            logo_url: None,
            blocks: Vec::new(),
            modal_form_title: "",
            modal_button_text: "",
            modal_placeholder: "",
            modal_fields: Vec::new(),
            show_branding: true,
            affiliate_code: None,
            page_password_hash: None,
            page_consent_required: false,
            cta_label: "Free Kinetic Card",
            theme: None,
        }
    }
}
