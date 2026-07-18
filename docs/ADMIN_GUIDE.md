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
| Hide Branding | `kinetic_branding` | checkbox | Hide "Powered by FunnelSwift" (checked = hidden) |
| Mini Funnels | `kinetic_minifunnel` | checkbox | Allow mini-funnel multi-page sequences |
| Custom Domain | `kinetic_custom_domain` | checkbox | Attach a custom domain to kinetic cards |
| Analytics / Insights | `kinetic_analytics` | checkbox | Show card analytics dashboard |
| CTA Buttons | `kinetic_cta_buttons_max` | number | Max CTA buttons per card (0=disabled, -1=unlimited) |
| Social Links | `kinetic_social_links_max` | number | Max social link buttons (0=disabled, -1=unlimited) |
| Theme Templates | `kinetic_theme_templates` | number | Number of theme presets (1=default only, -1=all) |

### Defaults (Free Plan)
```json
{
  "max_kinetic_cards": 1,
  "kinetic_custom_colors": false,
  "kinetic_video": false,
  "kinetic_source_tracking": false,
  "kinetic_minipage": false,
  "kinetic_branding": true,
  "kinetic_minifunnel": true,
  "kinetic_custom_domain": false,
  "kinetic_analytics": false,
  "kinetic_cta_buttons_max": 0,
  "kinetic_social_links_max": 0,
  "kinetic_theme_templates": 1
}
```

## Pending Items
- iOS mobile app (blocked on Apple Developer account renewal)
- Field mapping UI (manual override for web-to-lead)
- Domain whitelist validation (currently logged but not enforced)
