# FunnelSwift — Lead Capture & Affiliate Hub

Rust backend for FunnelSwift — multi-tenant lead capture, kinetic cards, funnel builder, affiliate management, plan management, and cross-app lead/tag sync with CoreSwift CRM and WorkflowSwift.

## Architecture

- **Framework:** Axum (Tokio-based)
- **Database:** PostgreSQL with SQLx
- **Auth:** Local JWT (JWT_SECRET env var)
- **Deployment:** systemctl via native binary + nginx reverse proxy
- **Memory:** ~50-100MB

## Project Structure

```
funnelswift/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, create_router()
│   ├── api_router.rs        # All /api/v1/ routes (161 endpoints)
│   ├── db.rs                # PostgreSQL connection pool (AppState)
│   ├── auth.rs              # JWT validation & AuthUser extractor
│   ├── middleware/           # Auth, CORS, security headers
│   ├── handlers/            # Route handlers (50+ modules)
│   └── models/              # Database models
├── www/                     # Static HTML guides + admin pages
├── docs/                    # Markdown documentation
└── n8n-templates/           # Cross-app workflow templates
```

## Quick Start

```bash
# Set environment
cp .env.example .env

# Build
cargo build

# Run
cargo run
# Server starts on port 8080
```

## Core API Endpoints

All routes are under `/api/v1/`. 161 endpoints total.

### Auth
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/v1/auth/login` | Login |
| POST | `/api/v1/auth/signup` | Public signup |
| POST | `/api/v1/auth/register` | Registration |
| POST | `/api/v1/auth/forgot-password` | Forgot password |
| POST | `/api/v1/auth/reset-password` | Reset password |
| GET | `/api/v1/auth/me` | Current user |
| PUT | `/api/v1/auth/password` | Change password |
| PUT | `/api/v1/auth/profile` | Update profile |

### Leads (web-to-lead)
| Method | Path | Purpose |
|--------|------|---------|
| GET/POST | `/api/v1/leads` | List / Create leads |
| GET/PUT/DELETE | `/api/v1/leads/:id` | Get / Update / Delete lead |
| POST | `/api/v1/leads/:id/assign` | Assign lead |
| POST | `/api/v1/leads/:id/stage` | Update stage |
| POST | `/api/v1/leads/:id/tags` | Assign tags |
| GET | `/api/v1/leads/export` | Export leads |
| POST | `/api/v1/web-to-lead` | Public web-to-lead submission |

### Tags & Tag Rules
| Method | Path | Purpose |
|--------|------|---------|
| GET/POST | `/api/v1/tags` | List / Create tags |
| GET/PUT/DELETE | `/api/v1/tags/:id` | Get / Update / Delete tag |
| GET/POST | `/api/v1/tag-rules` | Auto-tagging rules |
| GET/POST | `/api/v1/tag-groups` | Tag groups |
| GET/POST | `/api/v1/tag-change-log` | Tag change audit log |

### Affiliate Hub
| Method | Path | Purpose |
|--------|------|---------|
| GET/POST | `/api/v1/affiliates` | List / Create affiliates |
| GET/PUT/DELETE | `/api/v1/affiliates/:id` | Get / Update / Delete affiliate |
| GET | `/api/v1/affiliates/:id/commissions` | Affiliate commissions |
| GET/POST | `/api/v1/affiliate-products` | Affiliate products |
| POST | `/api/v1/affiliate/signup` | Affiliate portal signup |
| POST | `/api/v1/affiliate/login` | Affiliate portal login |
| POST | `/api/v1/affiliate/dashboard` | Affiliate dashboard |
| GET/POST | `/api/v1/affiliate-stats` | Affiliate statistics |
| GET/POST | `/api/v1/affiliate-conversions` | Conversion tracking |
| POST | `/api/v1/check-affiliate-email` | Check affiliate by email |
| POST | `/api/v1/log-lead-movement` | Lead movement logging |

### Checkout & Payments

**FunnelSwift takes no payments.** No payment provider is integrated, so nothing here can create a
session or a charge — and that is deliberate rather than broken (the handler used to return a fake
`cs_test_placeholder` session id, which made a dead checkout look successful):

* `503 payment_provider_not_configured` when the calling account has no active `payment_providers`
  row (the state of every account today — the table is empty),
* `501 checkout_not_implemented` once one is configured: the provider is known, live session
  creation is not built.

Plans are therefore **never** bought in-app: a new account is put on the free plan its signup handler
hardcodes (`capture-free` on `/api/v1/auth/register`, `kinetic-free` on `/api/v1/auth/signup` — a
caller-supplied plan slug is ignored) and an admin assigns any other plan with
`POST /api/v1/admin/plans/assign`, recorded as the active row in `tenant_plan_subscriptions`.

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/v1/checkout/create` | Create checkout session — **always refuses, creates nothing**: `503 payment_provider_not_configured` / `501 checkout_not_implemented` |
| GET | `/api/v1/checkout/sessions` | List this tenant's checkout sessions — always `[]`: no code in `src/` writes `checkout_sessions` |
| GET/POST | `/api/v1/payment-providers` | Payment provider config (`api_key` stored as `enc:v1:` ciphertext, returned masked). 0 rows live; saving a provider is what turns `checkout/create` from 503 into 501 |
| DELETE | `/api/v1/payment-providers/:provider_type` | Delete a configured provider |

> No payment webhook receivers are registered (`/api/v1/webhooks/stripe`,
> `/api/v1/webhooks/paypal` were removed in kanban `t_6e746b75`): with no live checkout and no writer
> for `checkout_sessions`, a receiver could only answer `200 {"received":true}` to an event it never
> verified and never processed. Re-add them together with a real, signature-verified checkout flow.

### Kinetic Cards & Funnels
| Method | Path | Purpose |
|--------|------|---------|
| GET/POST | `/api/v1/kinetic/cards` | List / Create cards |
| GET/PUT/DELETE | `/api/v1/kinetic/cards/:id` | Get / Update / Delete card |
| GET/POST | `/api/v1/kinetic/cards/:id/buttons` | Card buttons |
| GET/POST | `/api/v1/kinetic/cards/:id/sources` | Card traffic sources |
| GET | `/api/v1/kinetic/metrics` | Card metrics |
| GET/PUT | `/api/v1/kinetic/subdomain` | Subdomain config |
| GET/PUT | `/api/v1/kinetic/custom-domain` | Custom domain config |
| GET/POST | `/api/v1/kinetic/qr` | QR codes |
| GET | `/api/v1/kinetic/qr/:id/svg` | QR SVG |
| GET | `/api/v1/kinetic/qr/:id/png` | QR PNG |
| GET/POST | `/api/v1/funnels` | Funnels CRUD |
| GET/PUT/DELETE | `/api/v1/funnels/:id` | Get / Update / Delete funnel |

### Tenants & Plans (Multi-tenant)
| Method | Path | Purpose |
|--------|------|---------|
| GET/POST | `/api/v1/tenants` | List / Create tenants |
| GET/PUT/DELETE | `/api/v1/tenants/:id` | Get / Update / Delete tenant |
| POST | `/api/v1/tenants/:id/credits` | Assign credits |
| POST | `/api/v1/tenants/:id/plan` | Assign plan |
| GET/POST | `/api/v1/admin/plans` | Admin plan management |
| POST | `/api/v1/admin/plans/assign` | Admin assign plan |

### Cross-App Push & Integration
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/v1/push/coreswift` | Push lead to CoreSwift |
| POST | `/api/v1/push/workflowswift` | Push lead to WorkflowSwift |
| POST | `/api/v1/push/adaswift` | Push lead to ADASwift |
| POST | `/api/v1/webhooks/conversion` | Cross-app conversion receiver (requires `x-internal-key`; writes `affiliate_commissions`) |

`/api/v1/webhooks/conversion` is posted by sibling apps (WorkflowSwift, ADASwift, missedcallrespondr)
when a paid checkout carries referral data. It requires the `x-internal-key` header, resolves the
affiliate through `leads.created_by -> affiliates.user_id` (or a direct `affiliate_id`), and writes the
commission row the affiliate portal and payout handler read. A 2xx always means the row was written:
an unattributable conversion answers 422 and records nothing. `/api/v1/track/lead` (t_f408b7cc) and
`/api/v1/track-click` (t_813d51d4) were removed — both were no-ops with zero callers in the fleet,
and neither had a consumer. Click tracking in this product is the Kinetic card tracker
`POST /card/:id/track` (`event_type=click`), which writes `kinetic_card_events`.

### Web-to-Lead Configs
| Method | Path | Purpose |
|--------|------|---------|
| GET/POST | `/api/v1/web-to-lead/configs` | List / Create configs |
| PUT/DELETE | `/api/v1/web-to-lead/configs/:id` | Update / Delete config |
| GET | `/api/v1/web-to-lead/configs/:id/embed` | Get embed code |

### Public Facing (no auth)
| Method | Path | Purpose |
|--------|------|---------|
| GET | `/` | API status |
| GET | `/api/health` | Health check |
| GET | `/api/v1/health` | Health check (v1) |
| GET | `/api/v1/insights` | Dashboard insights |
| GET | `/api/v1/campaigns` | Campaign listing |
| GET | `/api/v1/incentiveswift/config` | IncentiveSwift config |
| GET | `/funnel/:slug` | Public funnel page |
| POST | `/k/:slug/lead` | Kinetic card lead submit (also `/b`, `/m`, `/c`, `/f`, `/h` — all six card prefixes) |

## Deployment

```bash
# Build
CARGO_BUILD_JOBS=1 cargo build --release

# Deploy
cp target/release/funnelswift /opt/swift/funnelswift/funnelswift
systemctl restart funnelswift.service

# Verify
curl -s localhost:8080/api/health
```

## Documentation

- `www/guide.html` — Full user guide
- `www/guide-admin.html` — Admin guide
- `www/guide-affiliate.html` — Affiliate program guide
- `docs/ADMIN_GUIDE.md` — Admin API reference
- `ARCHITECTURE.md` — Cross-app architecture
- `GUARDRAILS.md` — Code quality standards

## License

Proprietary — SwiftSoftware
