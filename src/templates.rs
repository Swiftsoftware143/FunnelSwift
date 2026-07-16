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
        buttons: Vec<BioButton>,
        social_links: Vec<SocialLink>,
    },
    /// Hero section for mini-page — full-width hero with optional portrait image
    /// The image fills the section with a gradient overlay that blends into the page
    Hero {
        title: String,
        subtitle: String,
        cta_text: String,
        cta_url: String,
        /// Full portrait/hero image URL — displayed as a large background image with overlay
        hero_image_url: Option<String>,
        video_url: Option<String>,
        /// Gradient direction: "to-b", "to-r", "to-br", "to-tr", "135deg", etc.
        gradient_angle: Option<String>,
        /// Gradient colors (comma-separated hex values for multi-stop)
        gradient_colors: Option<String>,
    },
    /// Features grid (3-col desktop, 1-col mobile)
    Features {
        items: Vec<FeatureItem>,
    },
    /// Digital business card look — name, title, company, phone, email, website, catchphrase, avatar, logo
    BusinessCard {
        /// Card owner's full name
        name: String,
        /// Job title (optional)
        title: Option<String>,
        /// Company name (optional)
        company: Option<String>,
        /// Company logo URL (optional)
        company_logo_url: Option<String>,
        /// Owner avatar/photo URL (optional) — circular, prominent
        avatar_url: Option<String>,
        /// Catchphrase / tagline / short bio under name
        catchphrase: Option<String>,
        /// Phone number (optional)
        phone: Option<String>,
        /// Email (optional)
        email: Option<String>,
        /// Website URL (optional)
        website: Option<String>,
        /// Additional action buttons (same as BioLink buttons)
        buttons: Vec<BioButton>,
        /// Social links (same as BioLink)
        social_links: Vec<SocialLink>,
    },
    /// Lead capture form
    LeadForm {
        form_title: String,
        placeholder: String,
        button_text: String,
        fields: Vec<FormField>,
    },
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
    /// Extracted from blocks — straight into template for the modal
    pub modal_form_title: &'a str,
    pub modal_button_text: &'a str,
    pub modal_placeholder: &'a str,
    pub modal_fields: Vec<FormField>,
    /// Plan controls
    pub show_branding: bool,
    pub affiliate_code: Option<&'a str>,
}

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
            logo_url: None,
            blocks: Vec::new(),
            modal_form_title: "",
            modal_button_text: "",
            modal_placeholder: "",
            modal_fields: Vec::new(),
            show_branding: true,
            affiliate_code: None,
        }
    }
}
