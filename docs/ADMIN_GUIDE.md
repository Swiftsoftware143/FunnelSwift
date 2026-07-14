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

## Pending Items
- iOS mobile app (blocked on Apple Developer account renewal)
- Field mapping UI (manual override for web-to-lead)
- Domain whitelist validation (currently logged but not enforced)
