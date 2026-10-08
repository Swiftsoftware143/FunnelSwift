# FunnelSwift User Guide

## Overview
FunnelSwift is a lead generation and affiliate management platform. Capture leads from multiple sources including manual entry, mobile app scanning, web forms, and API integration.

## Quick Start

1. **Login** at https://funnelswift.net with your credentials
2. **Create an API Key** — go to API Keys → + New Key (needed for web-to-lead)
3. **Set up Web-to-Lead** — go to Web-to-Lead → + New Config → Get embed code
4. **Paste on your website** — add the snippet before `</body>` on any page
5. **Leads auto-capture** — form submissions appear in your FunnelSwift workspace

## Web-to-Lead

### What is Web-to-Lead?
Embed a JavaScript snippet on your external website. When a visitor fills out a form, the data is automatically captured and stored as a lead in your FunnelSwift workspace.

### Setup Steps
1. Navigate to **Web-to-Lead** in the sidebar
2. Click **+ New Config** and fill in:
   - **Name** — descriptive name (e.g., "Website Contact Form")
   - **Default Source** — how to label captured leads (default: "Web Form")
   - **Auto-tag** — optional tag to apply to all captured leads
   - **Rate Limit** — max submissions per hour (default: 100)
3. Click the **link icon (🔗)** on your config to get the embed code
4. If you don't have an API key, go to **API Keys** and create one
5. Replace `YOUR_FULL_API_KEY_HERE` in the embed code with your actual API key
6. Paste the code on your website just before `</body>`

### Field Mapping
The snippet auto-detects form fields:
- `email` / `e-mail`
- `name` / `full_name`
- `first_name` / `fname` / label "First Name"
- `last_name` / `lname` / label "Last Name"
- `phone` / `tel` / `telephone`
- `company` / `organization`
- `message` / `notes` / `comment`
- `website` / `url`
- `title` / `position`
- `linkedin` / `social`
- `address`

Unknown fields pass through as extra data.

### Duplicate Handling
If the email already exists in your workspace, the system returns success but does not create a duplicate. Duplicates are logged for review.

### API Endpoint
Direct POST endpoint: `POST https://funnelswift.net/api/v1/web-to-lead`
Requires: `api_key`, optional `config_id`, and form fields.

## Managing Leads
- **Add Lead** — manual entry with name, email, phone, company, source, tags
- **Edit Lead** — change fields, add optional fields, update tags
- **Lead Stages** — New → Contacted → Qualified → Converted → Lost

## Mobile App (Android)
Download the APK from: https://funnelswift.net/download-app
- Scan business cards via camera (OCR)
- Import phone contacts in bulk
- Search, select, and batch upload

## Account & Plan

### Plans
- **Plans** are managed by admins with configurable features and limits
- **Every new account starts on a free plan** — assigned by the signup handler itself, and the plan a signup request asks for is ignored. The handler no longer names a slug: it binds the free tier of the product being signed up (`plans.price = 0` with `plans.side = 'main'` for `/api/v1/auth/register`, `plans.side = 'kinetic'` for `/api/v1/auth/signup`, `'both'` accepted by either — see `src/plan_resolver.rs`). On a database whose free tiers are named `capture-free` / `kinetic-free` each path gets its own; on an install that still ships only the generic price-0 tier both paths start there and the server logs a warning naming the fix. The response carries `plan_assignment` — when `assigned` is `false` this install has no price-0 plan at all, so the account was created with no plan and an admin must add a free plan.
- **Changing plan is an admin action**, stored as the active row in `tenant_plan_subscriptions`; a plan can also carry an optional `purchase_url` / `payment_provider` field (admin plans API) — nothing in the app turns either into a checkout
- **There is no in-app checkout.** FunnelSwift integrates no payment provider and registers no payment webhook receiver, so no payment can be taken and nothing is triggered by one: `POST /api/v1/checkout/create` always refuses and creates nothing — `503 payment_provider_not_configured` while the account has no provider configured (every account today), `501 checkout_not_implemented` once one is configured
- **Accounts are created at signup** (or by an admin) and the plan is applied as above — no purchase is involved

### Email Templates
Transactional emails are rendered from database-stored templates in `email_templates` with `{{variable}}` placeholders:

| Template Type | When Sent | Merge Fields |
|---|---|---|
| `credentials` | **The new-account email** — sent when you sign up, and when an admin creates a user. It carries the first password the system generated for you | `{{name}}`, `{{email}}`, `{{password}}`, `{{login_url}}` |
| `password_reset` | You ask for a password reset — carries the reset code | `{{name}}`, `{{token}}` |
| `welcome` | **Not sent by any code path** — stored and editable, but nothing sends it; your new-account email is `credentials` | `{{name}}`, `{{email}}` |
| `purchase_confirmed` | **Not sent — there is no payment flow**; `send_purchase_confirmed_email()` has no caller | `{{name}}`, `{{plan_name}}` |

All types also bind `{{app_name}}`, `{{app_url}}` and `{{login_url}}`. Placeholders must be written with
**double braces** — `{name}` is treated as literal text and is sent to the recipient as-is.

Admins can edit these templates in the admin panel (`Email Templates`) — modify subject lines, the plain-text body and the HTML body (two separate boxes, so neither overwrites the other), and tick *Default for this type* to make a template the one that type's emails are rendered from. The panel lists the merge fields each type supports (read from `GET /api/v1/admin/email-templates/types`), but the editor is plain text: the placeholders are typed by hand.

## Affiliate Program

Earn a commission by referring people to the SwiftSoftware products. When a lead that flows through your account upgrades to a paid plan in any Swift product, you're credited — every upgrade, forever, no expiry.

- **No separate login** — your existing account is your affiliate account.
- **Opt in** from the Affiliate section of your portal, accept the terms, and you're auto-approved.
- **Payout rate** is tied to your plan (Free 20% → Agency 50%).

For the full, plain-language guide see **AFFILIATE_GUIDE.md**.

## Tags & Tag Groups
Organize leads by categories. Tags can be grouped into: Source, Status, Events, Services, Engagement, Custom.
