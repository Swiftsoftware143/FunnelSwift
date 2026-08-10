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

## Account & Purchases

### Plans & Checkout
- **Plans** are managed by admins with configurable features and limits
- Checkout is handled via Stripe or PayPal — payment provider selectable per plan
- After successful payment, an account is auto-created with credential delivery via email

### Email Templates
Transactional emails (welcome, purchase confirmation) use database-stored templates with `{{variable}}` placeholders:

| Template Type | When Sent | Merge Fields |
|---|---|---|
| `welcome` | Account created | `{{name}}`, `{{email}}`, `{{password}}`, `{{app_url}}` |
| `purchase_confirmed` | Payment confirmed | `{{name}}`, `{{plan_name}}`, `{{app_url}}` |

Admins can edit these templates in the admin panel — modify subject lines, HTML body, or plain text fallback. Merge field buttons insert placeholders automatically.

## Tags & Tag Groups
Organize leads by categories. Tags can be grouped into: Source, Status, Events, Services, Engagement, Custom.
