# Deployment Guide

## VPS Information
- **IP:** 178.156.221.18
- **Provider:** Hetzner
- **User:** root

## API Keys
API keys are stored in GitHub Secrets. Contact admin for access.

## Quick Deploy

Run on VPS as root:
```bash
curl -fsSL https://raw.githubusercontent.com/SwiftSoftware204/FunnelSwift/main/deployment/setup.sh | bash
```

Then:
```bash
cd /opt/swift
docker-compose up -d --build
```

## Files Location
- `deployment/docker-compose.yml` - Docker compose config
- `deployment/nginx.conf` - Nginx reverse proxy config
- `deployment/setup.sh` - One-time VPS setup script
- `.github/workflows/deploy.yml` - Auto-deploy on git push

## Domains
- `app.funnelswift.net` → FunnelSwift API
- `crm.funnelswift.net` → CoreSwift CRM API

## GitHub Secrets Required
- `VPS_SSH_KEY` - Private key for VPS access
- `CF_API_TOKEN` - Cloudflare API token
- `CF_ZONE_ID` - Cloudflare zone ID
- `CF_DNS_RECORD_ID` - DNS record ID for app.funnelswift.net
- `CF_DNS_RECORD_ID_CRM` - DNS record ID for crm.funnelswift.net

## Manual Setup Commands

If auto-deploy fails, run these on VPS:

```bash
# Install Docker
curl -fsSL https://get.docker.com | sh
systemctl enable docker && systemctl start docker

# Install Docker Compose
curl -L "https://github.com/docker/compose/releases/download/v2.24.0/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
chmod +x /usr/local/bin/docker-compose

# Setup directories
mkdir -p /opt/swift && cd /opt/swift

# Clone repos
git clone https://github.com/SwiftSoftware204/FunnelSwift.git funnelswift-rust
git clone https://github.com/SwiftSoftware204/CRMSwift.git crmswift-source

# Copy deployment files
cp funnelswift-rust/deployment/docker-compose.yml .
cp funnelswift-rust/deployment/nginx.conf .

# Create env file and edit with real values
cat > .env << 'EOF'
POSTGRES_USER=swift
POSTGRES_PASSWORD=SwiftSecure2026!
FUNNELSWIFT_SUPABASE_URL=https://emklnfkjfnzlfjtnxccy.supabase.co
FUNNELSWIFT_SUPABASE_ANON_KEY=YOUR_KEY_HERE
FUNNELSWIFT_PORT=8080
CRM_PORT=8081
CRM_JWT_SECRET=CoreSwiftSecure2026!
EOF

# Deploy
docker-compose up -d --build
```