# Deployment Guide

## VPS Information
- **IP:** 209.222.97.179
- **Provider:** ReliableSite (Miami)
- **User:** root

## Deployment (Docker container, bind-mounted binary)

FunnelSwift runs in the Docker container `funnelswift` (compose project `funnelswift`,
`/opt/swift/docker/funnelswift/docker-compose.yml`), behind nginx. The release binary is
bind-mounted, so there is **no systemd unit** and **no `/opt/swift/funnelswift/` copy step**
(measured 2026-10-02: neither the directory nor `funnelswift.service` exists — an earlier revision
of this file named both).

```bash
# Build (release) — one build at a time, through the lock
cd /opt/swift/apps/FunnelSwift
/opt/swift/build-lock.sh FunnelSwift cargo build --release

# Deploy (bind mount -> restart is the deploy)
docker restart funnelswift
# ...or the full gated path (pre-build gate + 3-way parity + boundary + health):
/opt/swift/deploy.sh FunnelSwift

# Verify
curl -s localhost:8080/api/health   # -> {"schema":{"expected_version":76,"ledger_version":"76","status":"applied"},...}
```

Container facts: user `funnelswift` (uid 999), `network_mode: host`, port 8080, env from
`/etc/swift/env/funnelswift.env`, healthcheck against `/api/health` every 30s.

**Migrations are embedded into the binary at build time** (`sqlx::migrate!("./migrations")`), so a
migration change is a release build + restart, never a file drop. The compose file's
`migrations -> /app/migrations` bind is diagnostic-only. Full detail: `README.md` § Migrations.

## Domain
- `funnelswift.net` → FunnelSwift (nginx reverse proxy to :8080)

## Environment

Key env vars (in .env or systemd EnvironmentFile):
- `DATABASE_URL` — PostgreSQL connection string
- `JWT_SECRET` — Local JWT signing secret (not Supabase)
- `PORT=8080`

## Database
- PostgreSQL on localhost:5432
- Docker container: `swift-postgres-1`
- User: `swift`, DB: `funnelswift`

## Related Services

| Service | Port | Status |
|---------|------|--------|
| FunnelSwift | 8080 | Active |
| CoreSwift CRM | 8084 | Active (coreswiftcrm.com) |
| IncentiveSwift | 8083 | Active |
| MultiDirectory | 3001 | Active |
| WorkflowSwift | 8085 | Active |
| ADA Swift | 8087 | Active |
| MissedCall Respondr | 8088 | Active |
