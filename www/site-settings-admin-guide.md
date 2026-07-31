# Site Settings — Admin Guide

## Overview

Every product's marketing page (SEO, analytics, custom scripts, homepage content)
is now configurable from the admin panel. No more code changes for minor updates.

## Where to Find It

1. Go to **funnelswift.net/admin**
2. Log in with your admin account
3. In the left sidebar, scroll to the bottom — click **Sites**
4. You'll see cards for each product:
   - **FunnelSwift** ⚡ (funnelswift.net)
   - **IncentiveSwift** 🎯 (incentiveswift.com)
   - **MissedCallRespondr** 📞 (missedcallrespondr.com)
   - **ADASwift** ♿ (adaswift.com)
5. Click any card to open its editor

## The Three Tabs

### 1. SEO & Meta
- **Meta Title** — the browser tab text and Google search title
- **Meta Description** — the snippet under your link in search results
- **Meta Keywords** — search keywords (comma-separated)
- **OG Title / Description / Image** — how your page looks when shared on social media (Facebook, LinkedIn, Twitter)
- **Canonical URL** — tells search engines which URL is the "real" one (prevents duplicate content penalties)
- **Favicon URL** — the small icon in browser tabs

### 2. Tracking & Scripts
- **Google Analytics ID** — your G-XXXXXXXXXX code. Save it here and GA4 loads on every page
- **Google Tag Manager ID** — your GTM-XXXXXXX code
- **Head Scripts** — injects before `</head>`. Use for:
  - ADA compliance widget code
  - Chatbot widget (Tidio, Intercom, etc.)
  - Facebook/Meta pixel
  - LinkedIn Insight tag
  - Any custom `<script>` or `<style>` tags
- **Body End Scripts** — injects before `</body>`. For footer scripts, popup widgets, etc.

### 3. Homepage
Edit the visible content of each product's homepage:
- **Navigation** — logo text, sign-in URL, CTA button text
- **Hero** — main headline, subheadline, button text
- **Features Section** — heading, subheading, visibility toggle
- **CTA Section** — call-to-action heading, text, button
- **Footer** — footer text

## Saving

Click **Save Changes** at the top right. The HTML file is updated immediately on the product's domain.

## FAQ

**Q: How long does it take for changes to go live?**
A: Instantly. The HTML file is written to disk immediately on save.

**Q: Will my changes be lost if something breaks?**
A: No. Settings are stored in the database. The HTML file is regenerated from DB on each save.

**Q: Can team members edit site settings?**
A: No — only the admin account (`swiftsoftware143@yahoo.com`) can access the Sites tab.

**Q: What about WorkflowSwift?**
A: Builder Bot is handling that. The spec has been prepared for handoff.
