# FunnelSwift Admin Guide

## System Architecture
- **Backend**: Rust (Axum) at port 8080, systemd service `funnelswift.service`
- **Database**: PostgreSQL on Docker, container `swift-postgres-1`
- **Web App**: Single-page HTML/JS at `/var/www/funnelswift/`
- **Mobile App**: React Native / Expo at `/opt/swift/FunnelSwift-Mobile/`
- **VPS**: Miami (ReliableSite) Ubuntu 24.04 — root@209.222.97.179

## Web-to-Lead Feature

### Database Schema
Two new tables:
- **`web_to_lead_configs`** — per-tenant configs: name, active status, tag assignment, field mapping, allowed domains, rate limit
- **`web_to_lead_logs`** — audit trail: IP, origin domain, field count, lead ID, status (received/imported/duplicate/rejected)

### Multi-Tag Support
Configs now support assigning **multiple tags** per widget. When a lead is captured via web-to-lead, all configured tags are applied to the lead automatically. In the UI, hold Ctrl/Cmd to select multiple tags in the form.

### API Endpoints (all under `/api/v1/`)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/web-to-lead` | API key (public) | Receive lead from external form |
| GET | `/web-to-lead/configs` | JWT | List tenant configs |
| POST | `/web-to-lead/configs` | JWT | Create new config |
| PUT | `/web-to-lead/configs/:id` | JWT | Update config |
| DELETE | `/web-to-lead/configs/:id` | JWT | Delete config |
| GET | `/web-to-lead/configs/:id/embed` | JWT | Get embed code |

### JS Snippet
Served from: `/var/www/funnelswift/funnelswift-capture.js`
Accessible at: `https://funnelswift.net/funnelswift-capture.js`

The snippet:
- Reads `window.FunnelSwiftConfig.apiKey` and `.configId`
- Auto-attaches to all `<form>` elements on submit
- Smart-maps field names/labels to lead fields
- Posts JSON to `/api/v1/web-to-lead`
- Rate-limited by tenant (100/hr default)

### Security
- API keys hashed with Argon2id (reuses existing `api_keys` table)
- Web-to-lead permission check on API key (`permissions.web_to_lead`)
- Rate limiting at tenant level
- Duplicate email detection

### Admin UI
Web-to-Lead section added to the main SPA at `/var/www/funnelswift/funnelswift.js`:
- List configs with status, source, rate limit
- Create/edit/delete configs (modal pop-outs)
- Get embed code modal
- Walkthrough text added

## Deployment

### Backend
```bash
cd /opt/swift/funnelswift
export PATH=/root/.cargo/bin:/usr/bin:/usr/local/bin:$PATH
cargo build --release
systemctl restart funnelswift
```

### JS Snippet
Update `/var/www/funnelswift/funnelswift-capture.js` and reference from HTTPS.

## Affiliate System (Tag-Based)

FunnelSwift is the affiliate hub. Affiliate products represent the Swift products (FunnelSwift, CoreSwift, WorkflowSwift, IncentiveSwift, ADASwift, MissedCallRespondr, MultiDirectory). A product may carry a **system tag** link (`affiliate_products.system_tag_id`), and attribution is tracked on the **user account** a lead flows through — not on a cookie.

### The Model

1. **Affiliate products are the Swift products** — they sync in from each app's plan sync (below), and a product may carry a `system_tag_id` link to a System Tag.
2. When a lead is tagged with that System Tag, `tag_logic::attribute_affiliate_on_tags` (`src/tag_logic.rs:319`, fired from `src/handlers/lead_handler.rs:633`) matches it against `affiliate_products.system_tag_id WHERE is_active = true` and records the pending $0 commission. **Assigning a System Tag does not create an account, contact or client in the other app** — see [System Tags](#system-tags). The link is API-only today: the product form has no picker and 0 of 11 products carry one (measured 2026-09-26).
3. When a lead — created under a user account — is tagged, a pending $0 commission is recorded (the free-plan attribution anchor).
4. When that lead later **upgrades to a paid plan** in the product, the other app fires an upgrade event back to FunnelSwift, and the referring affiliate is credited. **No expiry — every upgrade, forever.**

### Data Model

| Table / Column | Purpose |
|---|---|
| `affiliate_products.system_tag_id` | Optional link to the System Tag whose assignment attributes this product (`src/tag_logic.rs:319`) — set from the product form's System Tag picker (`www-app/index.html` `LPAFS`), which is fed by `GET /api/v1/admin/system-tags`; un-set with `clear_system_tag: true` (a bare `null` keeps the stored tag) |
| `leads.created_by` | The user account a lead flowed through (the affiliate anchor) |
| `affiliates.user_id` | First-class link: which user account an affiliate is |
| `affiliate_commissions.product_id` | Which product a commission is for |
| `affiliate_commissions.metadata` | Upgrade-event traceability + idempotency key (`event_id`) |

Attribution resolution: `leads.created_by` → `affiliates.user_id`. Admin / non-affiliate users resolve to no affiliate.

### Admin Endpoints

| Method | Path | Description |
|---|---|---|
| POST | `/api/v1/affiliate-products` | Create a product (admin), accepts `system_tag_id` |
| PUT | `/api/v1/affiliate-products/:id` | Update a product (admin), incl. `system_tag_id` (`clear_system_tag: true` removes it) |
| DELETE | `/api/v1/affiliate-products/:id` | Delete a product (admin) |
| GET | `/api/v1/admin/system-tags` | List System Tags as `{id, name, color}` (`src/handlers/affiliate_product_handler.rs:187`) — answers 200 live; its caller is the product form's System Tag picker (`www-app/index.html` `LPAFS`) |

### Cross-App Upgrade Event

`POST /api/v1/internal/affiliate/upgrade-event` (protected by `x-internal-key`)

Every other Swift app calls this when a user upgrades to a paid plan:

```json
{
  "source_app": "workflowswift",
  "email": "lead@example.com",
  "plan_name": "Pro",
  "plan_price": 79.0,
  "event_id": "unique-per-upgrade"
}
```

FunnelSwift resolves the lead by email → `leads.created_by` → affiliate, and records `commission = plan_price × affiliate_rate / 100`. `event_id` makes it idempotent (replays return `already-credited`).

### Per-Plan Payout

Each plan carries a `commission_rate` — the payout % an affiliate earns. Admin-adjustable per plan:

| Plan | Default Payout % |
|---|---|
| Capture Free / Kinetic Free | 20% |
| Capture Starter / Kinetic Pro | 30% |
| Suite | 40% |
| Agency / Scale | 50% |

Set a plan's rate via `PUT /api/v1/plans/:id` with `{"commission_rate": 50}`. An affiliate's effective rate is stamped from their plan at signup; upgrade the affiliate's plan and their rate follows.

### Plan Sync (unchanged)

Plans from all Swift apps still auto-sync into `affiliate_products` via `POST /api/v1/internal/sync-affiliate-plan` (each app fires it on plan create/update/delete with its `source_app`):

- CoreSwift (`source_app: coreswift`)
- WorkflowSwift (`source_app: workflowswift`)
- AdaSwift (`source_app: adaswift`)
- IncentiveSwift (`source_app: incentiveswift`)
- MissedCallRespondr (`source_app: missedcallrespondr`)

A product can additionally carry a `system_tag_id` link — the API accepts it on create/update (`POST`/`PUT /api/v1/affiliate-products`, `src/handlers/affiliate_product_handler.rs:221`/`:298`) — but no shipped form offers a picker, so 0 of 11 products carry one (measured 2026-09-26). Product categories are seeded in migration `026_affiliate_product_auto_sync.sql`.

## MultiDirectory Integration (CTA Slots)

FunnelSwift's SMS/email funnels can appear as **CTA buttons** on MultiDirectory business listing pages.

**Configuration (Business → Integrations tab in MultiDirectory):**
- Toggle "SMS Funnel" integration on
- Select a CTA: "Text Us", "Get Started", "Send a Message"
- Assign a FunnelSwift funnel to trigger when the CTA is clicked
- The button renders on the business listing; clicking opens FunnelSwift's modal for SMS/funnel capture
- **Controlled vocabulary only** — business owners pick from pre-approved CTAs

## Monitoring
- Check logs: `journalctl -u funnelswift --no-pager -n 50`
- Health endpoint: `GET /api/health`
- Database: `psql postgres://swift:SwiftSecure2026!@localhost:5432/funnelswift`

## Plan Features Editor

Access at: **https://funnelswift.net/admin/plans** (must be logged in as an admin)

### How Plans Work
- Each plan has a JSONB `features` column in the database
- Signing up routes to a plan based on the `plan` field sent from the signup form
  - **funnelswift.net signup** → no plan field → defaults to `free` plan
  - **funnelswift.net/kinetic signup** → sends `kinetic_free` → maps to `kinetic_free` plan
- The plan editor lets you toggle features on/off without writing code

### Feature Definitions

| UI Label | JSON Key | Type | Description |
|----------|----------|------|-------------|
| Max Kinetic Cards | `max_kinetic_cards` | number | Max bio-link cards per account |
| Custom Colors | `kinetic_custom_colors` | checkbox | Allow custom primary/accent colors |
| Video Embeds | `kinetic_video` | checkbox | Allow video embeds on cards |
| Source Tracking | `kinetic_source_tracking` | checkbox | Track click sources via UTM params |
| Mini-Page Layout | `kinetic_minipage` | checkbox | Enable mini-page (extended layout) |
| Show Tenant Name in Footer | `kinetic_branding` | checkbox | Show tenant name above CTA (free plan locked off) |
| Mini Funnels | `kinetic_minifunnel` | checkbox | Allow mini-funnel multi-page sequences |
| Custom Domain | `kinetic_custom_domain` | checkbox | Attach a custom domain to kinetic cards |
| Analytics / Insights | `kinetic_analytics` | checkbox | Show card analytics dashboard |
| CTA Buttons | `kinetic_cta_buttons_max` | number | Max CTA buttons per card (0=disabled, -1=unlimited) |
| Social Links | `kinetic_social_links_max` | number | Max social link buttons (0=disabled, -1=unlimited) |
| Theme Templates | `kinetic_theme_templates` | number | Number of theme presets (1=default only, -1=all) |
| Footer CTA Text | `kinetic_cta_text` | text | Custom footer CTA (use `{type}` as placeholder, e.g. "Get Your Free {type}") |

### Permission Model
| Feature | Admin | Paid User | Free User |
|---------|-------|-----------|-----------|
| Edit CTA text | ✅ Plan editor | ❌ | ❌ |
| Toggle tenant name in footer | ✅ Per plan | ✅ (if plan allows) | ❌ |
| Change card colors/styling | ✅ | ✅ | ✅ |

### Defaults (Free Plan)
```json
{
  "max_kinetic_cards": 1,
  "kinetic_custom_colors": false,
  "kinetic_video": false,
  "kinetic_source_tracking": false,
  "kinetic_minipage": true,          // free can create 1 mini page
  "kinetic_branding": false,         // no tenant name in footer (CTA only)
  "kinetic_minifunnel": false,       // no mini funnels on free
  "kinetic_custom_domain": false,
  "kinetic_analytics": false,
  "kinetic_cta_buttons_max": 0,
  "kinetic_social_links_max": 0,
  "kinetic_theme_templates": 1,
  "kinetic_cta_text": "Claim Your {type}"
}
```

## Email Templates

Transactional emails use database-stored templates in the `email_templates` table. Templates support `{{variable}}` placeholders in `subject`, `body` and `html_body`; the HTML part is sent when `html_body` is non-empty, otherwise the plain-text `body` is used.

### Template Types

`template_type` is a free-form string (the API validates nothing); `GET /api/v1/admin/email-templates/types` advertises three types — and only one of them is ever sent by this app:

| Type | When Sent | Merge Fields (what the sender binds) |
|---|---|---|
| `password_reset` | User requests a password reset (`src/auth/handlers.rs:448`) — **the only send path in the codebase** | `{{name}}`, `{{token}}` |
| `welcome` | **Never sent** — `send_welcome_email()` (`src/email.rs`) has no caller; signup does not email anybody | `{{name}}`, `{{email}}` |
| `purchase_confirmed` | **Never sent — there is no purchase flow** (see *Account & Plan Flow*); `send_purchase_confirmed_email()` (`src/email.rs`) has no caller | `{{name}}`, `{{plan_name}}` |

Every type also binds `{{app_name}}` (`FunnelSwift`), `{{app_url}}` (`https://app.funnelswift.net`) and
`{{login_url}}` (`https://app.funnelswift.net/login`) — the same names
`GET /api/v1/admin/email-templates/types` advertises, so a body built from that list always renders.

**Only the double-brace form is a placeholder.** `{{name}}` is substituted; `{name}` is not — it reaches
the recipient verbatim. The renderer logs a warning naming every placeholder left unsubstituted, so a
half-rendered email appears in the app log instead of going out silently (kanban t_2349e6ce).

### API Endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/api/v1/admin/email-templates` | List all templates |
| POST | `/api/v1/admin/email-templates` | Create template (`template_type`, `name`, `subject` are required) |
| GET | `/api/v1/admin/email-templates/:id` | Get single template |
| PUT | `/api/v1/admin/email-templates/:id` | Update template |
| DELETE | `/api/v1/admin/email-templates/:id` | Delete template |
| GET | `/api/v1/admin/email-templates/types` | Template types **and** their merge fields |

There is no `merge-fields` route: `/api/email-templates/merge-fields` answers 404 and
`/api/v1/admin/email-templates/merge-fields` is rejected 400 (it matches `/:id` and is not a UUID).

### Template Fields

- **name** — display label
- **template_type** — free-form string; the admin panel offers `welcome`, `password_reset` and `purchase_confirmed` from `GET /api/v1/admin/email-templates/types`, and the API accepts any value
- **subject** — subject line with `{{variable}}` insertion
- **body** — plain-text body
- **html_body** — HTML body; when it is non-empty the email is sent as HTML, otherwise the plain-text `body` is used (there is no `is_html` column)
- **is_default** — serves as fallback for this type (an account-specific row wins over the default; lookup is `WHERE template_type = $1 AND (aid = $2 OR is_default = true)`)
- **aid** — account the template belongs to (nullable; a default row carries none)

### Merge Fields Available

Every template type binds the shared `{{app_name}}`, `{{app_url}}`, `{{login_url}}` plus its own fields
(table above). There is **no `{{password}}` merge field anywhere in the codebase** — no sender holds the
plaintext password, so a body that uses it sends that literal text.

### Account & Plan Flow (there is no purchase flow)

FunnelSwift has no checkout and no payment webhook receiver (see README's *Checkout & Payments*), so
nothing in this app is triggered by a payment. What actually happens:

1. An account is created by signing up (`POST /api/v1/auth/signup`, or `POST /api/v1/auth/register`) or by an admin creating the tenant
2. The system creates the tenant and seeds `tenant_settings` (e.g. the default lead stages)
3. The plan is a **hardcoded free slug** — `kinetic-free` on the public signup path (`src/handlers/public_signup_handler.rs:112`), `capture-free` on the register path (`src/auth/handlers.rs:141`) — a plan slug sent in the request is ignored either way
4. Only an admin can change it: `POST /api/v1/admin/plans/assign` writes the active `tenant_plan_subscriptions` row (`set_active_plan`, `src/handlers/plan_handler.rs`), cancelling the previous one with `status = 'cancelled'` first
5. **No email is sent at any of these steps.** `send_welcome_email()` and `send_purchase_confirmed_email()` have no callers; the only email this app sends on its own is `password_reset`

### Fallback Content (not database seeds)

No migration seeds `email_templates` — the table is edited by admins. When no matching row is found,
the app renders hardcoded fallback content from `get_inline()` in `src/email.rs` for `welcome`,
`purchase_confirmed` and `password_reset`.

## Per-Tenant Email (Mailgun) Resolution

Transactional email delivery can be **resolved per tenant**, so a tenant can send from its own configured email/Mailgun/SMTP identity instead of the platform default. Everything comes from the **database** — `src/email_provider.rs` reads no environment variables (`EMAIL_API_URL` / `EMAIL_API_KEY` / `EMAIL_FROM` are ignored).

### How Resolution Works

1. `email_provider::resolve` reads `tenant_settings` for the sending tenant, in this order: `email_config` (explicit `provider`: `smtp` | `mailgun` | `sendgrid` | `sendiio`), then the legacy rows `mailgun_config`, then `smtp_config`. The first row that is actually configured (`is_configured()`) wins.
2. With no usable tenant row it falls back to the global **`admin_settings` key `email`** row — the system-mail identity the admin panel edits.
3. With neither, the send is **skipped with a logged warning** and the caller gets `Email provider not configured. Set it in Admin > Settings > Email Provider.` There is no env-var fallback and no server-wide default.

### Admin Bypass

For system/platform emails with no tenant context (e.g. admin-triggered sends), `send_template_email` passes `tenant_id = None` and goes straight to the global `admin_settings.email` row — same DB provider, no environment variables.

### Admin UI

The admin panel (**Settings > Email Provider**, and the **Email Templates** editor) writes those rows through `GET/POST /api/v1/admin/email-config` (+ `/test`) and `GET/POST /api/v1/admin/email-templates`.

### API Notes

- No new public endpoint is required — resolution happens inside the transactional send path.
- Live state: the global `admin_settings.email` row is configured (provider `mailgun`); **0 tenants** have their own `email_config` / `mailgun_config` / `smtp_config` row, so every tenant currently resolves to the global identity.
- Templates still prioritize account-specific templates, then defaults, then inline fallback, before dispatch.

## Kinetic Cards Management

Kinetic Cards are the digital business cards, bio-links, landing pages, and mini funnels that each tenant creates and manages. As an admin, you can view, create, edit, and delete cards for any tenant.

### Where to Find Kinetic Cards

1. Log into the FunnelSwift admin panel
2. Click **Kinetic Cards** in the left sidebar
3. You'll see all cards belonging to your tenant

### Card Management from the UI

- **Create** — Click "+ New Card" to open the template gallery. Choose from 54 templates across 5 card types.
- **Edit** — Click any card to open the edit modal. Modify the slug, title, description, avatar, buttons, social links, theme, and template.
- **Delete** — Click the delete button on any card row. This is permanent.
- **Preview** — Each card has a public URL you can share: `https://funnelswift.net/{prefix}/{slug}`

### Card Types (URL Prefixes)

| Prefix | Card Type |
|--------|-----------|
| `/k/` | Kinetic Card (original) |
| `/b/` | Bio Link |
| `/c/` | Digital Business Card |
| `/m/` | Micro Page |
| `/f/` | Mini Funnel |
| `/h/` | Hero Page |

### Per-Tenant Card Visibility

Cards are **per-tenant** — each portfolio company has its own set of cards. The `list_cards` endpoint filters by `auth.tenant_id`, so when you're logged in as a tenant (or impersonating one), you'll only see that tenant's cards.

### Card Features by Plan

| Feature | Free | Paid |
|---------|------|------|
| Max Cards | 1 | Unlimited |
| Templates | 8 | 54 |
| Custom Colors | ❌ | ✅ |
| Video Embeds | ❌ | ✅ |
| Source Tracking | ❌ | ✅ |
| Mini Funnels | ❌ | ✅ |
| Custom Domain | ❌ | ✅ |
| Analytics | ❌ | ✅ |

---

## Workspace Status (`Active` / `Inactive`)

**Admin → Tenants → Edit → Status** writes `tenants.status` (migration 053: `NOT NULL DEFAULT 'active'`,
`CHECK (status IN ('active','inactive'))`; any other value is rejected as a 400 by the API, never as a
raw constraint error). New workspaces start `active`, and every workspace in the fleet does today.

**What `Inactive` actually does** — decided and enforced server-side 2026-09-25 (kanban `t_af890bbf`):

| Surface | An `inactive` workspace |
|---------|------------------------|
| Login (`POST /api/v1/auth/login`) | **403** `This workspace is inactive. Contact support to restore access.` — checked only *after* the password is verified, so a wrong password is still the usual 401 and the endpoint reveals nothing about which accounts exist |
| Every authenticated `/api/v1` call — including a token minted **before** the workspace was retired (`/auth/me`, cards, leads, tags, settings, integrations, …) | **403**, same message. A JWT lives 30 days and cannot be recalled, so the flag is re-checked on every request: retiring a workspace also ends its live sessions |
| `POST /api/v1/auth/forgot-password` | 200 with the usual generic line, but **no reset mail is sent** |
| `POST /api/v1/auth/reset-password` | **403** — no new credential is written into a retired workspace |
| Admin → Tenants → 👤 **Login As** (impersonate) | **403** — the session it would mint is refused by the gate above, so handing one out would only look like a console bug |
| Public cards `/k/ /b/ /c/ /m/ /f/ /h/`, `/card/:id/track`, public card analytics, public lead capture | **unchanged — still served.** This is deliberate: Kinetic cards are physical artifacts whose QR codes are already in customers' hands, and the card is addressed by *their* audience, not by us. Retiring a workspace stops the workspace from being *used*; it must not silently break a live campaign that neither you nor the customer would notice. Take a card down with its own Delete control |
| A token for a workspace whose row no longer exists (deleted workspace, stale JWT) | **403** `Workspace no longer exists` — fail closed |
| `role = admin` (platform operator, i.e. this console) | **exempt**, so a workspace retired by mistake can always be switched back from this very screen. Tenant staff (`user`, `company_admin`) are **not** exempt — they are the customer |

### Switching a Workspace Back On

Edit the tenant → Status → **Active** → Save. There is no grace period, no warning mail and no
scheduled reactivation: the 403 above is the customer's first signal. Treat the dropdown as a
lockout switch, not a label.

---

## Admin Impersonation

Admins can impersonate any tenant to view and manage that tenant's data as if logged into their account. This is useful for troubleshooting, verifying data, or performing actions on behalf of a tenant. An `inactive` workspace cannot be impersonated (403 — see **Workspace Status** above).

### How to Impersonate a Tenant

1. Log into FunnelSwift as a **Super Admin**
2. Go to **Admin** → **Tenants** in the left sidebar
3. Find the tenant you want to impersonate
4. Click the **👤 Login As** button on their row
5. The page reloads with the impersonated tenant's view

### During Impersonation

- A **red banner** appears at the top of the page: `⚠️ Impersonating: [Company Name] | [Stop]`
- You can view and manage the tenant's data: **Kinetic Cards, Leads, Tags, Integrations**, and all other tenant-scoped resources
- The admin sidebar links (Users, Plans, System Tags, etc.) are hidden — you see what the tenant sees
- **Impersonation expires after 1 hour** (the JWT token has a 1-hour TTL)

### Stopping Impersonation

Click the **Stop** button in the red banner at the top of the page. This:
1. Restores your original admin token from `localStorage`
2. Removes the impersonation banner
3. Reloads the page so you're back to your admin view

### API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/v1/admin/impersonate` | Admin JWT | Generate an impersonation token. Body: `{"account_id": "..."}` |
| POST | `/api/v1/admin/stop-impersonation` | Any JWT | Confirms the client should stop impersonating |

### Technical Details

- The impersonation JWT includes an `impersonating` claim set to the admin's `user_id`
- The `tenant_id` in the JWT is set to the target tenant's ID, so all tenant-scoped queries automatically filter to the impersonated tenant
- The `sub` claim is set to the target tenant's first active user
- Kinetic cards, leads, tags, integrations, and all other tenant-scoped resources are filtered by `tenant_id` — they'll show the impersonated tenant's data automatically

---

## Pending Items
- iOS mobile app (blocked on Apple Developer account renewal)
- Field mapping UI (manual override for web-to-lead)
- Domain whitelist validation (currently logged but not enforced)

---

## URL Routing — Card Types (HARD-WIRED)

This map is authoritative. Defined in `kinetic_handler.rs` (`resolve_canonical_url` + `cta_for_prefix`) and mirrored in `dashboard.js` (`CARD_ROUTES`). Both locations must stay in sync.

| Prefix | Card Type | Branding CTA |
|--------|-----------|-------------|
| `/k/` | Kinetic Card | Claim your free Kinetic Card → |
| `/b/` | Bio Link | Claim your free Bio Link → |
| `/c/` | Digital Business Card | Claim your free Digital Business Card → |
| `/m/` | Micro Page | Claim your free Micro Page → |
| `/f/` | Mini Funnel | Claim your free Mini Funnel → |
| `/h/` | Hero Page | Claim your free Hero Page → |

**How it works:** The Rust handler reads the URL prefix from the request path (`/{prefix}/{slug}`), maps it to a card type label, and injects the correct canonical URL + branded CTA. The branding badge at the bottom of every public card auto-adjusts based on the URL prefix — a `/c/myjones` card shows "Claim your free Digital Business Card →" while `/b/mylinks` shows "Claim your free Bio Link →".

**Rules:**
- `funnelswift.net/{prefix}/{slug}` → canonical redirects to `kntcrd.com/{prefix}/{slug}`
- `{tenant}.kntcrd.com/{prefix}/{slug}` → canonical uses the subdomain directly
- Custom domains → canonical uses the custom domain

---

## SEO Settings (Admin → SEO tab)

Access at: **https://app.funnelswift.net/app** → Admin → SEO

All fields stored in `site_settings` table with `seo_` prefix. Public cards and funnels auto-inject these tags via SSR.

### Configurable Fields

| Field | Key | Injected As |
|-------|-----|------------|
| Site Name | `seo_site_name` | `<meta property="og:site_name">` |
| Meta Description | `seo_description` | `<meta name="description">` + OG version |
| Keywords | `seo_keywords` | `<meta name="keywords">` |
| OG Image | `seo_og_image` | `<meta property="og:image">` + Twitter image |
| Twitter Handle | `seo_twitter_handle` | `<meta name="twitter:site">` + creator |
| Google Analytics | `seo_google_analytics` | Full gtag.js script block |
| Facebook Pixel | `seo_facebook_pixel` | Full fbq() init + noscript fallback |
| Site Verification | `seo_site_verification` | `<meta name="google-site-verification">` |
| Schema Type | `seo_schema_type` | `<script type="application/ld+json">` |

### Public Endpoints

| URL | Description | Cache |
|-----|-------------|-------|
| `/api/v1/seo/sitemap.xml` | Dynamic sitemap with all public cards + funnels | 1 hour |
| `/robots.txt` | Configurable crawl rules (via `seo_robots` setting) | 24 hours |
| `/api/v1/seo/inject` | Returns current meta + script tags as JSON | — |

### API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/v1/seo/settings` | JWT (admin) | Get all SEO settings |
| PUT | `/api/v1/seo/settings` | JWT (admin) | Update SEO settings |
| GET | `/api/v1/seo/sitemap.xml` | Public | Dynamic XML sitemap |
| GET | `/api/v1/seo/inject` | Public | JSON of meta/script tags |
| GET | `/robots.txt` | Public | Dynamic crawl rules |

### CSP Header

Content-Security-Policy injected via `middleware/security.rs`. Allows:
- `script-src`: self, Tailwind CDN, Google Tag Manager, Facebook
- `img-src`: self, data: URIs, all HTTPS sources
- `frame-src`: self, YouTube, Vimeo, Facebook

### Security Headers (via Nginx + Rust middleware)

| Header | Value |
|--------|-------|
| Strict-Transport-Security | max-age=63072000; includeSubDomains; preload |
| X-Frame-Options | DENY (allow on card pages) |
| X-Content-Type-Options | nosniff |
| Referrer-Policy | strict-origin-when-cross-origin |
| Permissions-Policy | camera=(), microphone=(), geolocation=(), interest-cohort=() |
| Content-Security-Policy | (see above) |

---

## Sidebar Accordions

The admin sidebar uses accordion groups for cleaner navigation:
- **Tags** — collapses System Tags + System Tag Groups
- **Affiliates** — collapses Affiliates + Affiliate Tiers + Affiliate Payouts

State persisted in `localStorage` keys: `fs_acc_tags`, `fs_acc_affiliates`. Clicking a child tab auto-opens its parent accordion.

## System Tags

> **Who this is for:** Super Admins who manage the tags every workspace shares.
> These are the same System Tags that affiliate products reference via `system_tag_id` (see the [Affiliate System](#affiliate-system-tag-based) section above).

### What Are System Tags?

A System Tag is a tag an **admin** creates once (`tags.is_system = true`). Every workspace sees it in its own tag list — `GET /api/v1/tags` answers `WHERE tenant_id = $1 OR is_system = true` (`src/handlers/tag_handler.rs:22`) — and only an admin can create, change or delete it.

### Where to Find System Tags

1. Log into **FunnelSwift** as a Super Admin
2. Go to **Admin** → **Tags** → **System Tags** (the *Tags* accordion in the sidebar)
3. You'll see a table of all existing System Tags, with **+ Add System Tag**, per-row **Edit** / **Del**, and a bulk delete

### What an Admin Can Do

Writes are **admin-only in the handler**, not merely hidden in the UI (create and delete measured live 2026-09-26):

| Action | Route | Non-admin answer |
|--------|-------|------------------|
| Create a System Tag | `POST /api/v1/tags` with `is_system: true` (`src/handlers/tag_handler.rs:65`) | `403 Only admins can create system tags` |
| Rename / recolor | `PUT /api/v1/tags/:id` (`src/handlers/tag_handler.rs:111`) | `403` |
| Delete it for every workspace | `DELETE /api/v1/tags/:id` (`src/handlers/tag_handler.rs:171`) | `403 Only admins can delete system tags` |

The form itself has three fields — **Name**, **Tag Group**, **Color** (`RFT`, `www-app/index.html`). There is no target app, no webhook URL and no on/off switch on a System Tag.

### What System Tags Do NOT Do

- They do **not** create a free account, contact or client in another Swift app. There is no per-tag target, no provisioning webhook and no cross-app caller: the `tags` table carries no target/webhook/payload column (its nine columns are `id`, `tenant_id`, `name`, `color`, `group_id`, `is_system`, `metadata`, `created_at`, `updated_at`), and `to_regclass('public.system_tags')` is NULL (measured 2026-09-26).
- They do **not** hand a lead a free tier of anything. The free/starter/pro/enterprise tiers of ADASwift, CoreSwift, WorkflowSwift, IncentiveSwift and MissedCallRespondr live in those apps' own plans, which each workspace buys separately.
- They are **not** a separate object type: the System Tags screen is the same `tags` table filtered on `is_system = true` (0 of 5 rows today).

### Assigning a System Tag

Applying one to a lead writes the tag name into `leads.tags` like any other tag, and visibility is already covered by `GET /api/v1/tags`. Its one extra effect is **affiliate attribution**: FunnelSwift matches the freshly-applied tag ids against `affiliate_products.system_tag_id WHERE is_active = true` (`tag_logic::attribute_affiliate_on_tags`, `src/tag_logic.rs:319`, fired from `src/handlers/lead_handler.rs:633`) and records a **pending $0 commission** for the affiliate the lead flowed through (`leads.created_by` → `affiliates.user_id`). Nothing is written to another app.

Two conditions have to hold on the tagging path: the tag id must resolve for the workspace doing the tagging (`src/handlers/lead_handler.rs:556` resolves that workspace's own tags; rule-driven adds at `:625` also resolve shared system tags), and the product must still be active. No product carries a `system_tag_id` live today (0 of 11, measured 2026-09-26), so no tag assignment attributes anything yet.

The pending commission is filled in later, when the product itself fires the upgrade event back (see [Cross-App Upgrade Event](#cross-app-upgrade-event)).

### FAQ

**Q: Can a workspace create, rename or delete a System Tag?**
No. All three are refused with `403` for a non-admin (`src/handlers/tag_handler.rs:65`, `:111`, `:171`). A workspace sees System Tags in its tag list and can apply them to its own leads.

**Q: Does assigning a System Tag create anything in another app?**
No. Nothing is provisioned and no webhook fires — the only write is the tag name on the lead, plus the pending commission described above.

**Q: Why is the System Tags list empty?**
Nothing seeds System Tags; an admin creates the first one (0 rows live today).

**Q: A product's tag link is stale — how do I stop it attributing?**
Deactivate the product or clear its `system_tag_id`: both attribution readers resolve a product `WHERE is_active = true` only, so a retired product stops producing commissions.
