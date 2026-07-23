# FunnelSwift Admin Guide

## System Architecture
- **Backend**: Rust (Axum) at port 8080, systemd service `funnelswift.service`
- **Database**: PostgreSQL on Docker, container `swift-postgres-1`
- **Web App**: Single-page HTML/JS at `/var/www/funnelswift/`
- **Mobile App**: React Native / Expo at `/opt/swift/FunnelSwift-Mobile/`
- **VPS**: Hetzner Debian 12 — root@178.156.221.18

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

## Affiliate Product Auto-Sync

FunnelSwift is the central hub for affiliate product management. Plans from all Swift apps are automatically synced into `affiliate_products` so they appear as commissionable products.

### Product Categories

Each source app maps to a product category:

| Source App | Category Slug | Category Name |
|---|---|---|
| FunnelSwift | `funnelswift-plans` | FunnelSwift Plans |
| Kinetic Cards | `kinetic-cards` | Kinetic Cards |
| IncentiveSwift | `incentiveswift-plans` | IncentiveSwift Plans |
| MultiDirectory | `multidirectory-plans` | MultiDirectory Plans |
| CoreSwift | `coreswift-plans` | CoreSwift Plans |
| WorkflowSwift | `workflowswift-plans` | WorkflowSwift Plans |
| AdaSwift | `adaswift-plans` | AdaSwift Plans |

Categories are seeded in migration `026_affiliate_product_auto_sync.sql`.

### Cross-App Sync Endpoint

`POST /api/v1/internal/sync-affiliate-plan`

Internal endpoint called by other Swift apps to sync plan changes. Protected by `api_key` field matching `INTERNAL_SYNC_KEY`.

**Payload:**
```json
{
  "action": "create|update|deactivate",
  "plan_name": "Pro Plan",
  "plan_price": 29.99,
  "source_app": "coreswift",
  "is_active": true,
  "owner_name": "SwiftSoftware",
  "product_type": "software",
  "api_key": "..."
}
```

**Behavior:**
- `create`/`update` — upserts an `affiliate_products` record by `source_app` + `plan_name`
- `deactivate` — sets `is_active = false` on the matching affiliate product

### Admin Sync Endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/api/v1/admin/affiliate-products/sync` | POST | Manually syncs all FunnelSwift plans to affiliate_products |
| `/api/v1/admin/affiliate-products/:id` | PUT | Super admin only — update owner_name and product_type |

### Which Apps Sync Automatically

All 5 apps now fire async sync events on plan create/update/delete:
- FunnelSwift (own plans)
- CoreSwift (`source_app: coreswift`)
- WorkflowSwift (`source_app: workflowswift`)
- AdaSwift (`source_app: adaswift`)
- IncentiveSwift (`source_app: incentiveswift`)

Each app needs `FUNNELSWIFT_URL` set in its environment (default `http://localhost:8080`).

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

## Email Templates (New)

Transactional emails use database-stored templates in the `email_templates` table. Templates support `{{variable}}` placeholders for dynamic content in both subject and body fields.

### Template Types

| Type | When Sent | Merge Fields |
|---|---|---|
| `welcome` | New account created | `{{name}}`, `{{email}}`, `{{password}}`, `{{app_url}}` |
| `purchase_confirmed` | Successful payment | `{{name}}`, `{{plan_name}}`, `{{app_url}}` |

### API Endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/api/email-templates` | List all templates (paginated) |
| POST | `/api/email-templates` | Create template |
| GET | `/api/email-templates/:id` | Get single template |
| PUT | `/api/email-templates/:id` | Update template |
| DELETE | `/api/email-templates/:id` | Delete template |
| GET | `/api/email-templates/merge-fields` | List merge fields by type |

### Template Fields

- **name** — display label
- **template_type** — `welcome` or `purchase_confirmed`
- **subject** — subject line with `{{variable}}` insertion
- **body** — plain text fallback
- **html_body** — rich HTML content
- **is_html** — if true, sends HTML; otherwise plain text
- **is_default** — serves as fallback for this type

### Merge Fields Available

All templates: `{{name}}`, `{{email}}`, `{{password}}`, `{{app_url}}`, `{{plan_name}}`

### Purchase Flow

1. User completes checkout via Stripe/PayPal
2. Webhook fires `POST /api/checkout/webhook`
3. System creates account, seeds tenant settings, applies plan
4. `send_purchase_confirmed_email()` queues email via `email_templates`
5. If template exists with `template_type = 'purchase_confirmed'`, it's rendered and sent; otherwise hardcoded fallback
6. `send_welcome_email()` sends credential details

### Default Seeds

- **Welcome Email** — credentials + login URL + next steps
- **Purchase Confirmation** — plan name + thank-you + login link

## Pending Items
- iOS mobile app (blocked on Apple Developer account renewal)
- Field mapping UI (manual override for web-to-lead)
- Domain whitelist validation (currently logged but not enforced)
