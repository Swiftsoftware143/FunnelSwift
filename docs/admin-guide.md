# FunnelSwift Admin Guide — System Tags & Auto-Provisioning

> **Who this is for:** Super Admins who manage integrations between FunnelSwift tags and other Swift apps.

---

## What Are System Tags?

System Tags are special tags in FunnelSwift that, when assigned to a lead, automatically trigger an action in another Swift app — like creating a free ADASwift account, adding a contact to CoreSwift CRM, or registering a lead in WorkflowSwift.

They're different from regular tags. Regular tags just organize your leads. System Tags **do something** when applied.

---

## Where to Find System Tags

1. Log into **FunnelSwift** as a Super Admin
2. Go to **Super Admin** → **Tags** → **System Tags**
3. You'll see a table of all existing System Tags

---

## How System Tags Work

| Step | What Happens |
|------|-------------|
| 1. Lead gets tagged | A lead in FunnelSwift receives a System Tag (manually or via automation) |
| 2. Webhook fires | FunnelSwift sends a webhook to the target app with the lead's contact info |
| 3. App provisions | The target app creates a free account/contact for that lead |
| 4. Duplicate protection | If the email already exists, nothing happens — no duplicates |

---

## Creating a System Tag

Click **+ New Tag** and fill in:

| Field | Description | Example |
|-------|-------------|---------|
| **Tag Name** | Display name for this tag | `ADASwift Free` |
| **Slug** | URL-safe identifier (auto-generated from name) | `adaswift_free` |
| **Target Software** | Which app this tag triggers | `adaswift` |
| **Webhook URL** | *(optional)* Override the default webhook URL | Leave blank unless you have a custom endpoint |

### Available Target Software Options

| Value | App | What gets created |
|-------|-----|-------------------|
| `adaswift` | ADASwift | Full ADA widget account (8 pages, free tier) with login credentials |
| `coreswift` | CoreSwift CRM | Contact record with "Free" tag |
| `workflowswift` | WorkflowSwift | Client record |
| `incentiveswift` | IncentiveSwift | Contact record |
| `missedcall` | MissedCall Respondr | Contact record |

---

## Managing Existing System Tags

From the System Tags table you can:

- **Toggle ON/OFF** — Disable a tag without deleting it. While disabled, no webhooks fire.
- **Delete** — Remove the tag entirely. Won't affect leads already tagged.

---

## What the Lead Gets

All leads created through System Tags get a **free-tier account**:

| Feature | Free Tier |
|---------|-----------|
| Pages | **8 pages max** |
| Profiles | Visual, Epilepsy, Hearing, Motor, Cognitive |
| Features | 20 accessibility features |
| Monthly Scan | ✅ Yes (3rd of every month) |
| Widget Embed | ✅ Yes |

> **Leads cannot be upgraded by themselves.** Only an admin can manually upgrade an account to Starter (25 pages), Pro (100 pages), or Enterprise (500 pages).

---

## FAQ

**Q: Can a lead upgrade themselves?**  
No. Free-tier accounts from System Tags can only be upgraded by an admin.

**Q: What if I assign a System Tag and the webhook fails?**  
FunnelSwift logs the attempt. The tag assignment still records in the database, but the target app won't create the account. You can retry by re-assigning the tag.

**Q: Can I have multiple System Tags pointing to the same app?**  
Yes. Each tag independently triggers the same webhook.

**Q: Does the lead get a welcome email?**  
ADASwift sends a welcome email with login credentials and widget embed code. The other apps (CoreSwift, WorkflowSwift, IncentiveSwift, MissedCall) create contact records without emails — they're destination databases.

**Q: What happens if I tag a lead that already exists in the target app?**  
The webhook detects the duplicate email and returns success without creating a duplicate. No error, no data loss.
