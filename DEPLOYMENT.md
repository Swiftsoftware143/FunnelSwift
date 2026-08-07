# Deployment Guide

## VPS Information
- **IP:** 178.156.221.18
- **Provider:** Hetzner
- **User:** root

## Deployment (systemctl, native binary)

FunnelSwift runs as a native Rust binary managed by systemd, behind nginx.

```bash
# Build
cd /opt/swift/funnelswift
CARGO_BUILD_JOBS=1 cargo build --release

# Deploy
cp target/release/funnelswift /opt/swift/funnelswift/funnelswift
systemctl restart funnelswift.service

# Verify
curl -s localhost:8080/api/health
```

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
