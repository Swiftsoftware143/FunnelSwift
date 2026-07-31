use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::error::AppResult;
use crate::state::AppState;

pub async fn list_themes(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        {
            "id": "cyber_dark",
            "name": "Cyber Dark",
            "primary": "#a855f7",
            "gradient": "linear-gradient(135deg, #0f172a 0%, #1e1b4b 50%, #311042 100%)",
            "colors": {
                "background": "linear-gradient(135deg, #0f172a 0%, #1e1b4b 50%, #311042 100%)",
                "card_bg": "rgba(255, 255, 255, 0.05)",
                "backdrop_filter": "blur(16px) saturate(180%)",
                "border": "1px solid rgba(168, 85, 247, 0.25)",
                "accent_glow": "0 0 25px rgba(168, 85, 247, 0.4)",
                "text": "#ffffff", "text_secondary": "#c4b5fd",
                "button_bg": "rgba(168, 85, 247, 0.25)", "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.4), 0 0 0 1px rgba(168, 85, 247, 0.15)"
            }
        },
        {
            "id": "sunset_kinetic",
            "name": "Sunset Kinetic",
            "primary": "#ff4b2b",
            "gradient": "linear-gradient(45deg, #ff416c 0%, #ff4b2b 100%)",
            "colors": {
                "background": "linear-gradient(45deg, #180325 0%, #4a0e2e 50%, #7c1d23 100%)",
                "card_bg": "rgba(0, 0, 0, 0.2)",
                "backdrop_filter": "blur(12px)",
                "border": "1px solid rgba(255, 115, 92, 0.25)",
                "accent_glow": "0 10px 30px rgba(255, 75, 43, 0.5)",
                "text": "#ffffff", "text_secondary": "#ffe4e6",
                "button_bg": "rgba(255, 75, 43, 0.3)", "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(255, 75, 43, 0.25)"
            }
        },
        {
            "id": "emerald_glass",
            "name": "Emerald Glass",
            "primary": "#10b981",
            "gradient": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
            "colors": {
                "background": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
                "card_bg": "rgba(6, 78, 59, 0.3)",
                "backdrop_filter": "blur(20px)",
                "border": "1px solid rgba(52, 211, 153, 0.25)",
                "accent_glow": "0 0 20px rgba(16, 185, 129, 0.35)",
                "text": "#ecfdf5", "text_secondary": "#a7f3d0",
                "button_bg": "rgba(16, 185, 129, 0.25)", "button_text": "#ecfdf5",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.3)"
            }
        },
        {
            "id": "midnight",
            "name": "Midnight",
            "primary": "#6366f1",
            "gradient": "linear-gradient(135deg, #1e1b4b, #312e81, #4c1d95)",
            "colors": {
                "background": "linear-gradient(135deg, #1e1b4b, #312e81, #4c1d95)",
                "card_bg": "rgba(255, 255, 255, 0.06)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(99, 102, 241, 0.2)",
                "accent_glow": "0 0 20px rgba(99, 102, 241, 0.35)",
                "text": "#ffffff", "text_secondary": "#c7d2fe",
                "button_bg": "rgba(99, 102, 241, 0.25)", "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.35)"
            }
        },
        {
            "id": "ocean",
            "name": "Ocean",
            "primary": "#0ea5e9",
            "gradient": "linear-gradient(135deg, #0c4a6e, #0369a1, #0284c7)",
            "colors": {
                "background": "linear-gradient(135deg, #0c4a6e, #0369a1, #0284c7)",
                "card_bg": "rgba(14, 165, 233, 0.08)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(14, 165, 233, 0.2)",
                "accent_glow": "0 0 20px rgba(14, 165, 233, 0.3)",
                "text": "#ffffff", "text_secondary": "#bae6fd",
                "button_bg": "rgba(14, 165, 233, 0.25)", "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.3)"
            }
        },
        {
            "id": "rose",
            "name": "Rose",
            "primary": "#ec4899",
            "gradient": "linear-gradient(135deg, #4c0519, #831843, #be185d)",
            "colors": {
                "background": "linear-gradient(135deg, #4c0519, #831843, #be185d)",
                "card_bg": "rgba(236, 72, 153, 0.08)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(236, 72, 153, 0.2)",
                "accent_glow": "0 0 20px rgba(236, 72, 153, 0.3)",
                "text": "#fce7f3", "text_secondary": "#fbcfe8",
                "button_bg": "rgba(236, 72, 153, 0.25)", "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.3)"
            }
        },
        {
            "id": "gold_premium",
            "name": "Gold Premium",
            "primary": "#d97706",
            "gradient": "linear-gradient(135deg, #0f172a, #1c1105, #451a03)",
            "colors": {
                "background": "linear-gradient(135deg, #0f172a, #1c1105, #451a03)",
                "card_bg": "rgba(217, 119, 6, 0.08)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(245, 158, 11, 0.3)",
                "accent_glow": "0 0 30px rgba(245, 158, 11, 0.4)",
                "text": "#fffbeb", "text_secondary": "#fde68a",
                "button_bg": "linear-gradient(135deg, #d97706, #f59e0b)", "button_text": "#1c1105",
                "card_shadow": "0 8px 32px rgba(0,0,0,0.4)"
            }
        },
        {
            "id": "ghost_white",
            "name": "Ghost White",
            "primary": "#ffffff",
            "gradient": "linear-gradient(135deg, #f8fafc, #e2e8f0, #cbd5e1)",
            "colors": {
                "background": "linear-gradient(135deg, #ffffff, #f8fafc, #f1f5f9)",
                "card_bg": "rgba(255, 255, 255, 0.7)",
                "backdrop_filter": "blur(12px)",
                "border": "1px solid rgba(0, 0, 0, 0.08)",
                "accent_glow": "0 0 20px rgba(99, 102, 241, 0.2)",
                "text": "#0f172a", "text_secondary": "#475569",
                "button_bg": "#0f172a", "button_text": "#ffffff",
                "card_shadow": "0 4px 16px rgba(0, 0, 0, 0.06)"
            }
        }
    ])))
}

pub async fn list_templates(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        // ==========================================
        // ARCHETYPE: DIGITAL BUSINESS CARD (5)
        // ==========================================
        {
            "id": "biz_executive",
            "name": "Executive",
            "type": "business_card",
            "category": "business_card",
            "niche": "C-Suite",
            "icon": "EX",
            "description": "Large centered avatar with dual-ring glow, prominent job title, 2×2 contact grid, glass bio snippet, LinkedIn+X row",
            "preview_colors": ["#0f172a", "#a855f7", "#ffffff"],
            "card_type": "business-card",
            "gradient": "radial-gradient(circle at 50% 25%, #1e1b4b 0%, #0f172a 60%, #311042 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type":"avatar","style":"circle_glow","size":"large","catchphrase":"Executive Profile"},
                {"type":"title_block","headline":"CEO & Founder","subtitle":"@ SwiftSoftware"}, 
                {"type":"action_grid_2x2","items":[
                    {"icon":"📞","label":"Direct Line","action":"tel:"},
                    {"icon":"📅","label":"Schedule","action":"cal:"},
                    {"icon":"📧","label":"InMail","action":"mailto:"},
                    {"icon":"💼","label":"Portfolio","action":"link:"}
                ]},
                {"type":"glass_bio","content":"Building the next generation of multi-tenant SaaS platforms."},
                {"type":"social_strip","platforms":["linkedin","twitter"]}
            ]
        },
        {
            "id": "biz_creative",
            "name": "Creative Pro",
            "type": "business_card",
            "category": "business_card",
            "niche": "Design/Art",
            "icon": "CP",
            "description": "Split layout: colorful avatar left, title + tagline right, 3-column skill badges, portfolio link grid, Instagram+Dribbble",
            "preview_colors": ["#831843", "#ec4899", "#fce7f3"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #4c0519 0%, #831843 50%, #a21caf 100%)",
            "theme_key": "rose",
            "layout_blocks": [
                {"type":"profile_split","avatar":"left","headline":"Creative Director","tagline":"Visual Storyteller"},
                {"type":"skill_badges","items":["Branding","UI/UX","Motion","3D","Illustration"]},
                {"type":"portfolio_grid","columns":3,"items":[
                    {"label":"Behance","icon":"🎨"},
                    {"label":"Dribbble","icon":"🏀"},
                    {"label":"Vimeo","icon":"🎬"}
                ]},
                {"type":"social_strip","platforms":["instagram","dribbble","behance"]}
            ]
        },
        {
            "id": "biz_realestate",
            "name": "Real Estate Agent",
            "type": "business_card",
            "category": "business_card",
            "niche": "Real Estate",
            "icon": "RE",
            "description": "Professional photo + agency badge at top, large 'Schedule a Tour' CTA button, property count stat, office location map link, Zillow+Realtor links",
            "preview_colors": ["#0c1929", "#0ea5e9", "#ffffff"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #0c4a6e 0%, #0369a1 50%, #0c1929 100%)",
            "theme_key": "ocean",
            "layout_blocks": [
                {"type":"avatar","style":"rounded_square","size":"large","catchphrase":"Your Neighborhood Expert"},
                {"type":"stat_badge","value":"150+","label":"Homes Sold"},
                {"type":"cta_button_large","label":"Schedule a Tour","icon":"🏠","action":"cal:"},
                {"type":"location_card","address":"Serving Greater Metro Area","google_maps":true},
                {"type":"action_row_3","items":[
                    {"icon":"📞","label":"Call"},
                    {"icon":"📱","label":"Text"},
                    {"icon":"📧","label":"Email"}
                ]}
            ]
        },
        {
            "id": "biz_medical",
            "name": "Medical Practice",
            "type": "business_card",
            "category": "business_card",
            "niche": "Healthcare",
            "icon": "MD",
            "description": "Credentials-first layout with certification badges, condition badges, 'Book Appointment' primary CTA, insurance accepted strip, office hours block",
            "preview_colors": ["#022c22", "#10b981", "#ecfdf5"],
            "card_type": "business-card",
            "gradient": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
            "theme_key": "emerald_glass",
            "layout_blocks": [
                {"type":"credentials_header","name":"Dr. Sarah Chen","title":"Board-Certified Dermatologist","badges":["MD","FAAD"]},
                {"type":"specialty_tags","items":["Cosmetic","Medical","Surgical","Pediatric"]},
                {"type":"cta_button_large","label":"Book Appointment","icon":"📅","action":"cal:"},
                {"type":"office_info","hours":"Mon-Fri 8am-6pm","phone":"(555) 123-4567"},
                {"type":"trust_strip","items":["🏥 Board Certified","💳 Most Insurance","⭐ 4.9 · 200+ Reviews"]}
            ]
        },
        {
            "id": "biz_startup",
            "name": "Tech Founder",
            "type": "business_card",
            "category": "business_card",
            "niche": "Startup",
            "icon": "TF",
            "description": "Minimalist: animated logo mark, 'Currently Building' tag, product link buttons (App Store, Web, GitHub), investor deck link, Twitter+LinkedIn+GitHub row",
            "preview_colors": ["#0f172a", "#6366f1", "#ffffff"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #312e81 50%, #4c1d95 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type":"logo_mark","shape":"hexagon","initial":"S","catchphrase":"Building in Stealth"},
                {"type":"title_block","headline":"Developer & Founder","subtitle":"SaaS · AI · Automation"},
                {"type":"project_links","items":[
                    {"label":"Product Hunt","icon":"🚀","url":"https://"},
                    {"label":"GitHub","icon":"💻","url":"https://"},
                    {"label":"Pitch Deck","icon":"📊","url":"https://"}
                ]},
                {"type":"stat_row","items":[
                    {"value":"$2.4M","label":"Raised"},
                    {"value":"15k+","label":"Users"},
                    {"value":"42","label":"Team"}
                ]},
                {"type":"social_strip","platforms":["twitter","linkedin","github"]}
            ]
        },

        // ==========================================
        // ARCHETYPE: BIO LINK PAGE (5)
        // ==========================================
        {
            "id": "bio_creator",
            "name": "Content Creator",
            "type": "bio_link",
            "category": "bio_link",
            "niche": "YouTuber/Streamer",
            "icon": "YT",
            "description": "Centered avatar with engagement ring, subscriber count pill, latest video embed slot, 5 stacked icon+label buttons, 'Join Community' CTA, all social platforms",
            "preview_colors": ["#1c1105", "#f59e0b", "#ffffff"],
            "card_type": "bio-link",
            "gradient": "linear-gradient(135deg, #451a03 0%, #78350f 50%, #1c1105 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"avatar","style":"circle_ring","size":"medium","catchphrase":"@YourChannel"},
                {"type":"follower_pill","count":"245K","platform":"YouTube"},
                {"type":"featured_media","media_type":"video","title":"🎬 Latest Upload","subtitle":"Why I Switched Tech Stacks"},
                {"type":"stacked_links","style":"icon_left","items":[
                    {"label":"Watch on YouTube","icon":"▶️","url":"#"},
                    {"label":"Join Discord","icon":"💬","url":"#"},
                    {"label":"Merch Store","icon":"👕","url":"#"},
                    {"label":"Sponsorships","icon":"🤝","url":"#"},
                    {"label":"Free Resources","icon":"📚","url":"#"}
                ]},
                {"type":"social_row","platforms":["youtube","twitter","instagram","tiktok","discord"]}
            ]
        },
        {
            "id": "bio_musician",
            "name": "Musician / Artist",
            "type": "bio_link",
            "category": "bio_link",
            "niche": "Music",
            "icon": "MU",
            "description": "Full-bleed album art background, artist name + genre tag, embedded music player slot, streaming platform grid (Spotify, Apple Music, SoundCloud), tour dates link, merch link",
            "preview_colors": ["#4c0519", "#ec4899", "#fce7f3"],
            "card_type": "bio-link",
            "gradient": "linear-gradient(135deg, #4c0519 0%, #831843 40%, #1a0510 100%)",
            "theme_key": "rose",
            "layout_blocks": [
                {"type":"hero_bg","image_slot":true,"overlay":"gradient"},
                {"type":"artist_header","name":"LUNA","genre":"Alt-Pop · Electronic"},
                {"type":"music_player","track":"New Release","album":"Midnight Sessions"},
                {"type":"streaming_grid","columns":4,"items":[
                    {"label":"Spotify","icon":"🟢"},
                    {"label":"Apple","icon":"🍎"},
                    {"label":"SoundCloud","icon":"☁️"},
                    {"label":"YouTube","icon":"▶️"}
                ]},
                {"type":"action_stack","items":[
                    {"label":"🎫 Tour Dates","url":"#"},
                    {"label":"👕 Official Merch","url":"#"},
                    {"label":"📸 Behind the Scenes","url":"#"}
                ]}
            ]
        },
        {
            "id": "bio_coach",
            "name": "Coach / Consultant",
            "type": "bio_link",
            "category": "bio_link",
            "niche": "Coaching",
            "icon": "CO",
            "description": "Trust-building header with credentials + photo, 'Book a Free Discovery Call' prominent CTA, testimonial rotator slot, 3 service tier link cards, Calendly embed link, newsletter signup",
            "preview_colors": ["#0f172a", "#6366f1", "#ffffff"],
            "card_type": "bio-link",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #312e81 50%, #1e1b4b 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type":"avatar","style":"circle_glow","size":"medium","catchphrase":"Transform Your Life"},
                {"type":"title_block","headline":"Executive Coach","subtitle":"ICF Certified · 10+ Years"},
                {"type":"cta_button_large","label":"Book Free Discovery Call","icon":"📅","action":"cal:"},
                {"type":"service_cards","items":[
                    {"title":"1:1 Coaching","desc":"Personalized sessions","price":"From $250"},
                    {"title":"Group Program","desc":"Cohort-based 8 weeks","price":"From $997"},
                    {"title":"Keynote Speaking","desc":"Events & workshops","price":"Custom"}
                ]},
                {"type":"testimonial_slot","quote":"\"Changed the trajectory of my career\" — Forbes 30U30"},
                {"type":"social_strip","platforms":["linkedin","twitter","instagram"]}
            ]
        },
        {
            "id": "bio_shop",
            "name": "Online Store",
            "type": "bio_link",
            "category": "bio_link",
            "niche": "eCommerce",
            "icon": "SH",
            "description": "Store logo at top, 'New Arrivals' featured product grid (3 products with images), discount code pill, category navigation links, cart/bookmark count badge, social proof strip",
            "preview_colors": ["#f8fafc", "#0f172a", "#ffffff"],
            "card_type": "bio-link",
            "gradient": "linear-gradient(135deg, #ffffff 0%, #f8fafc 50%, #f1f5f9 100%)",
            "theme_key": "ghost_white",
            "layout_blocks": [
                {"type":"store_header","logo_slot":true,"tagline":"Curated Lifestyle Goods"},
                {"type":"promo_pill","text":"🚚 Free Shipping Over $50"},
                {"type":"product_grid","columns":2,"items":[
                    {"label":"Ceramic Vase","price":"$38","badge":"NEW"},
                    {"label":"Linen Set","price":"$89","badge":"BEST"},
                    {"label":"Candle Trio","price":"$42","badge":"SALE"},
                    {"label":"Wall Art","price":"$64","badge":"ART"}
                ]},
                {"type":"stacked_links","style":"full_width","items":[
                    {"label":"🛍️ Shop All Products","url":"#"},
                    {"label":"⭐ Best Sellers","url":"#"},
                    {"label":"🎁 Gift Cards","url":"#"},
                    {"label":"📦 Track Order","url":"#"}
                ]},
                {"type":"trust_strip","items":["⭐ 4.8 · 2,500+ Reviews","🔒 Secure Checkout","📦 Easy Returns"]}
            ]
        },
        {
            "id": "bio_author",
            "name": "Author / Writer",
            "type": "bio_link",
            "category": "bio_link",
            "niche": "Writing",
            "icon": "AU",
            "description": "Book cover showcase (floating 3D tilt card), author bio excerpt, 'Latest Book' purchase link grid (Amazon, Barnes, local), newsletter subscribe form, podcast appearance links, socials",
            "preview_colors": ["#0f172a", "#d97706", "#fffbeb"],
            "card_type": "bio-link",
            "gradient": "linear-gradient(135deg, #1c1105 0%, #451a03 50%, #0f172a 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"book_showcase","cover_slot":true,"badge":"#1 Bestseller","title":"The Growth Playbook"},
                {"type":"bio_snippet","text":"Bestselling author of 3 books on product strategy and growth."},
                {"type":"purchase_grid","columns":3,"items":[
                    {"label":"Amazon","icon":"📦"},
                    {"label":"Barnes","icon":"📚"},
                    {"label":"Local","icon":"🏪"}
                ]},
                {"type":"email_capture","headline":"Get Chapter 1 Free","button":"Send It To Me"},
                {"type":"featured_links","items":[
                    {"label":"🎙️ Recent Podcast","url":"#"},
                    {"label":"✍️ Latest Article","url":"#"},
                    {"label":"📖 Reading List","url":"#"}
                ]},
                {"type":"social_strip","platforms":["twitter","linkedin","goodreads"]}
            ]
        },

        // ==========================================
        // ARCHETYPE: MINI PAGE (5)
        // ==========================================
        {
            "id": "page_saas",
            "name": "SaaS Product",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "SaaS",
            "icon": "SA",
            "description": "Product logo + tagline hero, animated feature highlight grid (4 cards), pricing table (3 tiers), enterprise CTA, integration logos strip, testimonial carousel slot",
            "preview_colors": ["#1e1b4b", "#6366f1", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #4c1d95 50%, #1e1b4b 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type":"hero","headline":"Automate Your Entire Workflow","subtitle":"The no-code platform that connects all your tools. Launch automations in minutes.","cta_primary":"Start Free Trial","cta_secondary":"See How It Works"},
                {"type":"feature_grid","columns":2,"items":[
                    {"icon":"⚡","title":"Instant Sync","desc":"Real-time data flow between apps"},
                    {"icon":"🧩","title":"500+ Integrations","desc":"Connect every tool you use"},
                    {"icon":"🤖","title":"AI Copilot","desc":"Smart suggestions as you build"},
                    {"icon":"📊","title":"Live Analytics","desc":"See what's working in real time"}
                ]},
                {"type":"pricing_table","tiers":[
                    {"name":"Starter","price":"$29","features":["1,000 actions","5 workflows","Email support"]},
                    {"name":"Pro","price":"$99","features":["10,000 actions","Unlimited workflows","Priority support","AI features"]},
                    {"name":"Enterprise","price":"Custom","features":["Unlimited everything","SSO","Dedicated support","SLA"]}
                ]},
                {"type":"integration_strip","logos":["slack","github","jira","stripe","hubspot","salesforce"]},
                {"type":"trust_bar","text":"Trusted by 15,000+ teams · SOC 2 Type II · 99.99% uptime"}
            ]
        },
        {
            "id": "page_event",
            "name": "Event / Conference",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Events",
            "icon": "EV",
            "description": "Date+location header bar with countdown, keynote speaker photo grid (3 columns), schedule timeline (collapsible days), sponsor logo strip, 'Get Tickets' sticky CTA, venue map link",
            "preview_colors": ["#451a03", "#f59e0b", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #451a03 0%, #78350f 50%, #1c1105 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"event_header","date":"October 15-17, 2026","location":"San Francisco, CA","countdown":true},
                {"type":"cta_button_large","label":"Get Your Tickets","icon":"🎟️","action":"link:"},
                {"type":"speaker_grid","columns":3,"headline":"Featured Speakers","items":[
                    {"name":"Jane Smith","title":"CEO, TechCorp"},
                    {"name":"Mark Rivera","title":"VP Product, CloudCo"},
                    {"name":"Priya Patel","title":"CTO, DataFlow"},
                    {"name":"James Chen","title":"Founder, AI Labs"},
                    {"name":"Lisa Park","title":"Head of Design, UXCo"},
                    {"name":"Tom Wright","title":"Partner, VC Fund"}
                ]},
                {"type":"schedule_timeline","days":[{"day":"Day 1","events":["10am Keynote","1pm Workshops","5pm Networking"]},{"day":"Day 2","events":["9am Panels","2pm Demos","6pm Afterparty"]}]},
                {"type":"sponsor_strip","tiers":["Platinum","Gold","Silver"]}
            ]
        },
        {
            "id": "page_restaurant",
            "name": "Restaurant",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Food",
            "icon": "RS",
            "description": "Hero food photo carousel, cuisine type tags, 'Reserve a Table' + 'Order Online' dual CTA, hours block (today highlighted), menu highlight grid with photos, location+map embed, review rating badge",
            "preview_colors": ["#1c1105", "#d97706", "#fffbeb"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #451a03 0%, #1c1105 50%, #0f172a 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"hero_media","media_type":"carousel","images":3},
                {"type":"restaurant_header","name":"The Ember Room","cuisine":["Modern American","Craft Cocktails"],"rating":"4.8 ★ (340 reviews)"},
                {"type":"cta_row_dual", "cta_primary":"Reserve a Table","cta_secondary":"Order Online"},
                {"type":"hours_block","today":"Today: 5pm-11pm","week":[{"day":"Mon-Thu","hours":"5pm-10pm"},{"day":"Fri-Sat","hours":"5pm-11pm"},{"day":"Sun","hours":"11am-3pm"}]},
                {"type":"menu_highlights","columns":2,"items":[
                    {"name":"Truffle Pasta","price":"$28","img":true},
                    {"name":"Wagyu Burger","price":"$34","img":true},
                    {"name":"Scallops","price":"$32","img":true},
                    {"name":"Tiramisu","price":"$16","img":true}
                ]},
                {"type":"location_block","address":"42 Main St, Downtown","map_slot":true}
            ]
        },
        {
            "id": "page_agency",
            "name": "Agency Portfolio",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Agency",
            "icon": "AG",
            "description": "Bold value prop hero, client logo wall (auto-scrolling), case study cards (3 featured), services list with icons, 'Start a Project' inquiry form, team photo strip, office locations",
            "preview_colors": ["#0f172a", "#a855f7", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "radial-gradient(circle at 50% 30%, #1e1b4b 0%, #0f172a 60%, #311042 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type":"hero","headline":"We Build Digital Products That Scale","subtitle":"A full-service design & development studio for ambitious startups and enterprises.","cta_primary":"Start a Project","cta_secondary":"View Our Work"},
                {"type":"client_logos","style":"scrolling","clients":["Nike","Spotify","Airbnb","Stripe","Coinbase","Notion"]},
                {"type":"case_studies","headline":"Selected Work","items":[
                    {"title":"FinTech App Redesign","tags":["UX","Mobile","FinTech"],"result":"+240% engagement"},
                    {"title":"E-Commerce Platform","tags":["Web","Full-Stack"],"result":"$12M in first year"},
                    {"title":"AI Dashboard","tags":["AI","Data Viz"],"result":"Used by 50k analysts"}
                ]},
                {"type":"services_grid","items":[
                    {"icon":"🎨","title":"Brand & Design"},
                    {"icon":"💻","title":"Web Development"},
                    {"icon":"📱","title":"Mobile Apps"},
                    {"icon":"🤖","title":"AI & Automation"},
                    {"icon":"📈","title":"Growth Marketing"},
                    {"icon":"🔧","title":"DevOps & Infra"}
                ]},
                {"type":"cta_button_large","label":"Let's Work Together","icon":"✉️","action":"mailto:"}
            ]
        },
        {
            "id": "page_course",
            "name": "Online Course",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Education",
            "icon": "ED",
            "description": "Course title + instructor badge hero, curriculum accordion (module list), student testimonial rotator, 'What You'll Learn' bullet grid, pricing card with payment plan, 'Enroll Now' CTA, student count badge",
            "preview_colors": ["#022c22", "#10b981", "#ecfdf5"],
            "card_type": "mini-page",
            "gradient": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
            "theme_key": "emerald_glass",
            "layout_blocks": [
                {"type":"hero","headline":"Master Product Strategy","subtitle":"A 6-week cohort-based course. Learn frameworks used at top tech companies.","badge":"🏆 4,200+ Students Enrolled"},
                {"type":"instructor_card","name":"David Park","title":"Ex-Google PM, 3x Founder","avatar":true},
                {"type":"learning_outcomes","headline":"What You'll Learn","items":[
                    "Craft compelling product visions",
                    "Run effective user research sprints",
                    "Build data-driven roadmaps",
                    "Lead cross-functional teams",
                    "Ace PM interviews at FAANG"
                ]},
                {"type":"curriculum","modules":[
                    {"title":"Module 1: Foundations","lessons":4},
                    {"title":"Module 2: Discovery","lessons":5},
                    {"title":"Module 3: Strategy","lessons":6},
                    {"title":"Module 4: Execution","lessons":5},
                    {"title":"Module 5: Growth","lessons":4},
                    {"title":"Module 6: Leadership","lessons":3}
                ]},
                {"type":"pricing_card","price":"$1,497","installments":"3 x $499","includes":["Live sessions","Slack community","1:1 coaching","Certificate"]},
                {"type":"cta_button_large","label":"Enroll Now — Seats Filling","icon":"🚀","action":"link:"},
                {"type":"testimonial_slot","quote":"\"Best career investment I've ever made. Landed a PM role at Stripe.\" — Sarah L."}
            ]
        },

        // ==========================================
        // ARCHETYPE: MINI FUNNEL (5)
        // ==========================================
        {
            "id": "funnel_lead_magnet",
            "name": "Freebie Lead Magnet",
            "type": "mini_funnel",
            "category": "mini_funnel",
            "niche": "Lead Gen",
            "icon": "LM",
            "description": "Mockup image of the freebie (eBook/checklist preview), value bullets with checkmarks, compact email+name form, 'Download Instantly' CTA, privacy reassurance line, 'What's Inside' preview row",
            "preview_colors": ["#0f172a", "#6366f1", "#ffffff"],
            "card_type": "mini-funnel",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #312e81 50%, #1e1b4b 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type":"media_preview","media_type":"image","label":"eBook Cover Mockup"},
                {"type":"value_props","headline":"The Ultimate Growth Playbook","bullets":[
                    "✅ 47-page actionable strategy guide",
                    "✅ 12 ready-to-use templates",
                    "✅ Case studies from $0 to $10M ARR",
                    "✅ Lifetime access + future updates"
                ]},
                {"type":"lead_form","fields":["name","email"],"button":"Send Me The Free Guide","button_style":"pulse"},
                {"type":"trust_line","text":"🔒 No spam. Unsubscribe anytime. Used by 35,000+ founders."}
            ]
        },
        {
            "id": "funnel_webinar",
            "name": "Webinar Registration",
            "type": "mini_funnel",
            "category": "mini_funnel",
            "niche": "Webinars",
            "icon": "WB",
            "description": "Live countdown timer to event, host photo + credentials, 3 key takeaways as icon cards, compact registration form (name+email), 'Save My Seat' CTA, calendar add link, 'Limited to 500 spots' urgency",
            "preview_colors": ["#451a03", "#f59e0b", "#ffffff"],
            "card_type": "mini-funnel",
            "gradient": "linear-gradient(135deg, #451a03 0%, #78350f 50%, #1c1105 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"countdown_timer","target_date":"2026-08-15T14:00:00Z","label":"Live Masterclass Starts In"},
                {"type":"host_card","name":"Maria Gonzalez","title":"Growth Lead @ Notion","avatar":true},
                {"type":"takeaway_cards","headline":"You'll Walk Away With","items":[
                    {"icon":"📊","text":"A repeatable growth framework"},
                    {"icon":"🛠️","text":"The exact tools we use daily"},
                    {"icon":"📋","text":"30-day action plan template"}
                ]},
                {"type":"lead_form","fields":["name","email","company"],"button":"Save My Spot — It's Free","button_style":"pulse"},
                {"type":"urgency_badge","text":"⚠️ Only 87 spots remaining · Live on Aug 15"},
                {"type":"calendar_link","platform":"google"}
            ]
        },
        {
            "id": "funnel_flash_sale",
            "name": "Flash Sale / Offer",
            "type": "mini_funnel",
            "category": "mini_funnel",
            "niche": "eCommerce",
            "icon": "FS",
            "description": "Strikethrough original price + bold sale price, animated countdown timer, product image gallery (swipeable), size/variant selector, 'Buy Now' big CTA, stock scarcity indicator, payment icons trust row",
            "preview_colors": ["#4c0519", "#ec4899", "#fce7f3"],
            "card_type": "mini-funnel",
            "gradient": "linear-gradient(135deg, #4c0519 0%, #831843 50%, #1a0510 100%)",
            "theme_key": "rose",
            "layout_blocks": [
                {"type":"countdown_timer","target_date":"2026-08-01T23:59:00Z","label":"Sale Ends In","style":"urgent"},
                {"type":"offer_header","original_price":"$199","sale_price":"$49","discount":"75% OFF","headline":"Premium Annual Plan"},
                {"type":"product_gallery","images":3,"swipeable":true},
                {"type":"variant_selector","options":["Monthly","Annual (Save 75%)","Lifetime"]},
                {"type":"cta_button_large","label":"Buy Now — $49","icon":"⚡","action":"checkout:","style":"pulse"},
                {"type":"scarcity_bar","text":"🔥 Only 124 left at this price · 47 people viewing"},
                {"type":"payment_strip","icons":["visa","mastercard","amex","paypal","applepay"]}
            ]
        },
        {
            "id": "funnel_waitlist",
            "name": "Product Waitlist",
            "type": "mini_funnel",
            "category": "mini_funnel",
            "niche": "Launch",
            "icon": "WL",
            "description": "Product teaser image/animation, 'Coming Q4 2026' badge, early access benefit bullets, email-only form (super low friction), 'Join the Waitlist' CTA, referral counter ('You're #X in line'), social share buttons, founding member perks list",
            "preview_colors": ["#0f172a", "#a855f7", "#ffffff"],
            "card_type": "mini-funnel",
            "gradient": "radial-gradient(circle at 50% 30%, #1e1b4b 0%, #0f172a 60%, #311042 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type":"teaser_image","label":"Product Preview","animated":true},
                {"type":"coming_soon_badge","text":"Launching Q4 2026 · Be First in Line"},
                {"type":"headline","text":"The Future of Team Collaboration","subtitle":"A workspace that thinks with you. Powered by AI."},
                {"type":"perk_list","headline":"Founding Member Perks","items":[
                    "🎖️ Lifetime discount (50% off forever)",
                    "🔮 Early access to beta features",
                    "💬 Private Slack with the founders",
                    "🎁 Swag pack shipped to your door"
                ]},
                {"type":"lead_form","fields":["email"],"button":"Join 8,400+ on the Waitlist","button_style":"pulse"},
                {"type":"referral_counter","text":"You'll be #8,421 in line"},
                {"type":"social_share","platforms":["twitter","linkedin","facebook"],"message":"I just joined the waitlist for @ProductName! 🚀"}
            ]
        },
        {
            "id": "funnel_survey",
            "name": "Quiz / Assessment",
            "type": "mini_funnel",
            "category": "mini_funnel",
            "niche": "Engagement",
            "icon": "QZ",
            "description": "Quiz title with curiosity hook, progress bar (Step 1 of 5), first question preview (multiple choice), 'Start the Quiz' big CTA, 'Takes 2 minutes' time badge, result preview teaser ('Get your personalized report'), trust badges",
            "preview_colors": ["#0c1929", "#0ea5e9", "#ffffff"],
            "card_type": "mini-funnel",
            "gradient": "linear-gradient(135deg, #0c4a6e, #0369a1, #0284c7)",
            "theme_key": "ocean",
            "layout_blocks": [
                {"type":"quiz_header","headline":"What's Your Leadership Style?","subtitle":"Discover your unique leadership archetype in 2 minutes.","difficulty":"5 questions · 2 min"},
                {"type":"progress_bar","current":0,"total":5},
                {"type":"question_preview","text":"When facing a team conflict, you typically:","options":["Mediate between parties","Make a quick decision","Gather more data","Delegate to a lead"]},
                {"type":"cta_button_large","label":"Start the Free Assessment","icon":"🧠","action":"start_quiz:","style":"pulse"},
                {"type":"result_teaser","headline":"Your Personalized Report Includes:","items":["Your leadership archetype","Strengths breakdown","Growth opportunities","Team communication guide"]},
                {"type":"trust_strip","items":["🏆 50,000+ assessments taken","📊 Research-backed","🔒 Results private"]}
            ]
        },

        // ==========================================
        // ARCHETYPE: HERO PAGE (5)
        // ==========================================
        {
            "id": "hero_brand",
            "name": "Brand Launch",
            "type": "hero",
            "category": "hero",
            "niche": "Brand",
            "icon": "BR",
            "description": "Full-screen gradient + animated logo reveal, brand tagline, 'Coming Soon' with launch date, email capture for early access, social media follow row, press kit download link",
            "preview_colors": ["#0f172a", "#a855f7", "#ffffff"],
            "card_type": "hero-page",
            "gradient": "radial-gradient(circle at 50% 30%, #1e1b4b 0%, #0f172a 60%, #311042 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type":"hero","headline":"A New Era of Productivity","subtitle":"We're reimagining how teams work together.","cta_primary":"Get Early Access","cta_secondary":"Learn More","logo_slot":true},
                {"type":"email_capture","headline":"Be the first to know","button":"Notify Me"},
                {"type":"countdown_timer","target_date":"2026-09-01T00:00:00Z","label":"Launching In"},
                {"type":"social_row","platforms":["twitter","linkedin","instagram","youtube"]}
            ]
        },
        {
            "id": "hero_app",
            "name": "App Download",
            "type": "hero",
            "category": "hero",
            "niche": "Mobile App",
            "icon": "AP",
            "description": "Phone mockup (center, angled), app name + rating stars, feature bullet points (left), 'Download on App Store' + 'Get on Google Play' dual store badges, QR code for instant download, '4.9 ★ · 2M+ Downloads' social proof",
            "preview_colors": ["#0f172a", "#6366f1", "#ffffff"],
            "card_type": "hero-page",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #4c1d95 50%, #1e1b4b 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type":"hero_split","media":"phone_mockup","headline":"Your Wallet, Smarter.","subtitle":"Track spending, save automatically, and invest your spare change. The app 2 million people trust.","rating":"4.9 ★"},
                {"type":"feature_bullets","items":[
                    {"icon":"🔔","text":"Smart notifications that actually help"},
                    {"icon":"📊","text":"Beautiful spending insights"},
                    {"icon":"🤖","text":"AI-powered savings goals"},
                    {"icon":"🔒","text":"Bank-level security, always"}
                ]},
                {"type":"store_badges","apps":["app_store","google_play"],"qr_code":true},
                {"type":"trust_bar","text":"⭐ 4.9 · 2M+ Downloads · Featured by Apple · SOC 2 Certified"}
            ]
        },
        {
            "id": "hero_community",
            "name": "Community Hub",
            "type": "hero",
            "category": "hero",
            "niche": "Community",
            "icon": "CM",
            "description": "Member avatar mosaic (overlapping circles), community name + member count, 'Join X,000+ members' headline, benefit tiles (networking, resources, events), 'Apply to Join' CTA, featured member quote, platform logos (Discord, Slack, Circle)",
            "preview_colors": ["#0c1929", "#0ea5e9", "#ffffff"],
            "card_type": "hero-page",
            "gradient": "linear-gradient(135deg, #0c4a6e, #0369a1, #0284c7)",
            "theme_key": "ocean",
            "layout_blocks": [
                {"type":"avatar_mosaic","count":12,"label":"+ 15,000 members"},
                {"type":"hero","headline":"Join the Largest Design Community","subtitle":"Connect with 15,000+ designers, get feedback on your work, and land your next role.","cta_primary":"Apply to Join","cta_secondary":"See What's Inside"},
                {"type":"benefit_grid","columns":3,"items":[
                    {"icon":"🤝","text":"Mentorship matching"},
                    {"icon":"📂","text":"Portfolio reviews"},
                    {"icon":"💼","text":"Job board access"},
                    {"icon":"🎓","text":"Weekly workshops"},
                    {"icon":"🌍","text":"Local meetups"},
                    {"icon":"📚","text":"Resource library"}
                ]},
                {"type":"testimonial_slot","quote":"\"Got my dream job at Figma through a connection in this community.\" — Alex K."},
                {"type":"platform_badges","platforms":["discord","slack","circle"]}
            ]
        },
        {
            "id": "hero_portfolio",
            "name": "Personal Portfolio",
            "type": "hero",
            "category": "hero",
            "niche": "Portfolio",
            "icon": "PF",
            "description": "Full-bleed hero image, name + role title large, 3-column project cards with hover previews, skill tag cloud, 'Available for hire' badge, contact form or Calendly link, resume download button",
            "preview_colors": ["#0f172a", "#d97706", "#fffbeb"],
            "card_type": "hero-page",
            "gradient": "linear-gradient(135deg, #1c1105 0%, #451a03 50%, #0f172a 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"hero","headline":"Hi, I'm Jordan.","subtitle":"Full-stack engineer & open source contributor. I build tools that developers love.","badge":"🟢 Available for new projects","cta_primary":"View My Work","cta_secondary":"Download Resume"},
                {"type":"project_grid","columns":3,"headline":"Featured Projects","items":[
                    {"title":"OpenAPI CLI","desc":"5.2k ★ on GitHub","tags":["Rust","CLI"]},
                    {"title":"Design System","desc":"Used by 3 startups","tags":["React","Storybook"]},
                    {"title":"AI Copilot","desc":"#1 Product of the Week","tags":["Python","LLM"]}
                ]},
                {"type":"skill_cloud","items":["Rust","TypeScript","React","Python","AWS","Docker","Kubernetes","GraphQL","Postgres","Redis","Terraform","Next.js"]},
                {"type":"cta_button_large","label":"Let's Build Something","icon":"💬","action":"cal:"},
                {"type":"social_strip","platforms":["github","twitter","linkedin","stackoverflow"]}
            ]
        },
        {
            "id": "hero_nonprofit",
            "name": "Nonprofit / Cause",
            "type": "hero",
            "category": "hero",
            "niche": "Nonprofit",
            "icon": "NP",
            "description": "Impact stat banner (lives changed, dollars raised), mission statement hero, 'Donate Now' primary CTA + 'Volunteer' secondary, progress bar for current campaign ($X of $Y raised), impact photo gallery, partner logo strip, newsletter signup",
            "preview_colors": ["#022c22", "#10b981", "#ecfdf5"],
            "card_type": "hero-page",
            "gradient": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
            "theme_key": "emerald_glass",
            "layout_blocks": [
                {"type":"impact_stats","items":[
                    {"value":"12,400+","label":"Lives Changed"},
                    {"value":"$3.2M","label":"Raised"},
                    {"value":"34","label":"Countries"}
                ]},
                {"type":"hero","headline":"Clean Water for Every Community","subtitle":"We're on a mission to bring sustainable water solutions to communities that need them most.","cta_primary":"Donate Now","cta_secondary":"Become a Volunteer"},
                {"type":"progress_bar","label":"Current Campaign","current":74250,"goal":100000,"prefix":"$"},
                {"type":"impact_gallery","columns":4,"images":4,"caption":"See your impact in action"},
                {"type":"partner_strip","logos":["unicef","redcross","who","gatesfoundation"]},
                {"type":"email_capture","headline":"Get Impact Updates","button":"Subscribe"}
            ]
        },

        // ==========================================
        // DAVID'S INDUSTRY TARGETS — REAL ESTATE, CREATORS, E-COMMERCE,
        // LOCAL SERVICES, B2B SAAS/AGENCY/FREELANCE
        // ==========================================

        // --- REAL ESTATE: Property Showcase + Luxury Agent ---
        {
            "id": "realestate_showcase",
            "name": "Property Showcase",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Real Estate",
            "icon": "PR",
            "description": "Swipeable property photo carousel, Beds/Baths/Price spec pills, 'Schedule Showing' booking widget, agent contact card, school/walk-score grid, 'Just Listed' urgency badge",
            "preview_colors": ["#0c1929", "#0ea5e9", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #0c4a6e 0%, #0369a1 50%, #0c1929 100%)",
            "theme_key": "ocean",
            "layout_blocks": [
                {"type":"urgency_badge","text":"🔥 Just Listed · 2 hours ago"},
                {"type":"hero_media","media_type":"carousel","images":4},
                {"type":"property_specs","items":[{"icon":"🛏️","value":"4","label":"Beds"},{"icon":"🚿","value":"3","label":"Baths"},{"icon":"📐","value":"2,400","label":"Sq Ft"},{"icon":"💰","value":"$875K","label":"Price"}]},
                {"type":"cta_row_dual","cta_primary":"📅 Schedule Showing","cta_secondary":"💬 Text Agent"},
                {"type":"agent_contact_card","name":"Marcus Chen","title":"Luxury Home Specialist","phone":"(555) 234-5678","avatar":true},
                {"type":"feature_grid","columns":2,"items":[{"icon":"🏫","title":"Top Schools","desc":"Rated 9/10 GreatSchools"},{"icon":"🚇","title":"Walk Score","desc":"91 · Walker's Paradise"},{"icon":"🛒","title":"Nearby","desc":"5 min to downtown"},{"icon":"📈","title":"Appreciation","desc":"+12% YoY in area"}]},
                {"type":"trust_bar","text":"🏆 #1 Agent in Metro Area · 150+ Homes Sold · 4.9 ★"}
            ]
        },
        {
            "id": "biz_realestate_luxury",
            "name": "Luxury Agent Card",
            "type": "business_card",
            "category": "business_card",
            "niche": "Luxury Real Estate",
            "icon": "LX",
            "description": "Frosted-glass overlay on headshot, 'Available Now' status dot, champagne gold borders, 2×2 marble-textured action grid, Zillow/Realtor profile badges, luxury sales stat counter",
            "preview_colors": ["#1c1105", "#d97706", "#fffbeb"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #1c1105 0%, #0f172a 50%, #1e1b4b 100%)",
            "theme_key": "gold_premium",
            "layout_blocks": [
                {"type":"avatar","style":"frosted_glass","size":"large","catchphrase":"Your Luxury Expert","status":"available"},
                {"type":"title_block","headline":"Victoria Sterling","subtitle":"Global Luxury Property Advisor"},
                {"type":"stat_row","items":[{"value":"$340M+","label":"Sold"},{"value":"12","label":"Years Exp"},{"value":"4.9★","label":"Rating"}]},
                {"type":"action_grid_2x2","items":[{"icon":"📞","label":"Private Line","action":"tel:"},{"icon":"📅","label":"Private Tour","action":"cal:"},{"icon":"🏠","label":"My Listings","action":"link:"},{"icon":"💎","label":"Off Market","action":"link:"}]},
                {"type":"profile_links","items":[{"label":"Zillow Profile","icon":"🏡"},{"label":"Realtor.com","icon":"🏘️"}]},
                {"type":"social_strip","platforms":["linkedin","instagram"]}
            ]
        },

        // --- CREATORS & COACHES: Creator Hub ---
        {
            "id": "creator_hub",
            "name": "Creator Hub",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Creator/Coach",
            "icon": "CH",
            "description": "Mesh-gradient hero, centered avatar with live pulsing ring + 'Live Now' indicator, high-visibility lead magnet card with glow animation, embedded video hook player, 1-click calendar, testimonial pill badges, bold gradient CTA",
            "preview_colors": ["#4c0519", "#ec4899", "#fce7f3"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #4c0519 0%, #831843 40%, #1a0510 100%)",
            "theme_key": "rose",
            "layout_blocks": [
                {"type":"avatar","style":"pulse_ring","size":"large","catchphrase":"@creatormaster · Live Now","status":"live"},
                {"type":"title_block","headline":"Master Your Craft","subtitle":"I help ambitious creators build 6-figure businesses."},
                {"type":"cta_button_large","label":"Download Free Strategy Guide","icon":"📘","action":"download:","style":"pulse"},
                {"type":"featured_media","media_type":"video","title":"▶️ Watch: My Exact Growth System","subtitle":"12-minute deep dive"},
                {"type":"stat_row","items":[{"value":"500K+","label":"Followers"},{"value":"3,200+","label":"Students"},{"value":"120","label":"Countries"}]},
                {"type":"testimonial_slot","quote":"\"Sarah's framework 5x'd my email list in 30 days.\" — Mark T."},
                {"type":"service_cards","items":[{"title":"The Accelerator","desc":"12-week group program"},{"title":"1:1 Coaching","desc":"Bi-weekly strategy calls"},{"title":"Template Vault","desc":"50+ done-for-you assets"}]},
                {"type":"cta_button_large","label":"Book Your 1-on-1 Strategy Call","icon":"📅","action":"cal:","style":"pulse"},
                {"type":"social_row","platforms":["youtube","twitter","instagram","tiktok","linkedin"]}
            ]
        },

        // --- E-COMMERCE: Boutique Storefront ---
        {
            "id": "ecom_boutique",
            "name": "Boutique Storefront",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Fashion/Beauty",
            "icon": "BT",
            "description": "Editorial hero banner with model photo, product tiles with Quick-Add, flash sale countdown ticker, multi-SKU variant selector, single-step checkout CTA, trust badges (free shipping, returns, secure checkout)",
            "preview_colors": ["#f8fafc", "#0f172a", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #ffffff 0%, #f8fafc 50%, #f1f5f9 100%)",
            "theme_key": "ghost_white",
            "layout_blocks": [
                {"type":"countdown_timer","target_date":"2026-08-02T23:59:00Z","label":"Summer Collection · Ends In","style":"urgent"},
                {"type":"hero","headline":"The Summer Edit","subtitle":"Effortless style for the season ahead. New arrivals dropping weekly.","cta_primary":"Shop New Arrivals","cta_secondary":"View Lookbook","image_slot":true},
                {"type":"product_grid","columns":2,"items":[{"label":"Linen Blazer","price":"$128","badge":"NEW","img":true},{"label":"Silk Cami","price":"$68","badge":"TREND","img":true},{"label":"Wide-Leg Pant","price":"$98","badge":"BEST","img":true},{"label":"Oversized Tee","price":"$45","badge":"SALE","img":true}]},
                {"type":"promo_pill","text":"✨ Buy 2+ items — save 15% with code SUMMER15"},
                {"type":"variant_selector","options":["XS","S","M","L","XL","XXL"]},
                {"type":"cta_button_large","label":"Shop All — Free Shipping","icon":"🛍️","action":"link:","style":"pulse"},
                {"type":"trust_strip","items":["📦 Free Shipping $50+","↩️ 30-Day Returns","🔒 Secure Checkout","⭐ 4.9 · 8,500+ Reviews"]}
            ]
        },

        // --- LOCAL SERVICES: Pro Card + Salon/Clinic ---
        {
            "id": "biz_local_service",
            "name": "Local Service Pro",
            "type": "business_card",
            "category": "business_card",
            "niche": "Local Services",
            "icon": "LS",
            "description": "Dark slate + cyan accent, instant one-tap action row (Call, Directions, Book), 'Open Now' live status badge, verified Google review stars, service menu cards with pricing, business hours block",
            "preview_colors": ["#0f172a", "#06b6d4", "#ffffff"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #0f172a 0%, #1e293b 50%, #0f172a 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type":"hours_badge","status":"open","text":"🟢 Open Now · Closes 8pm"},
                {"type":"avatar","style":"rounded_square","size":"medium","catchphrase":"Premium Auto Detailing"},
                {"type":"title_block","headline":"Elite Auto Spa","subtitle":"★★★★★ 5.0 · 340 reviews"},
                {"type":"action_row_3","items":[{"icon":"📞","label":"Tap to Call","action":"tel:"},{"icon":"🗺️","label":"Directions","action":"maps:"},{"icon":"📅","label":"Book Now","action":"cal:"}]},
                {"type":"service_cards","items":[{"title":"Full Detail","desc":"Interior + Exterior","price":"From $199"},{"title":"Express Wash","desc":"30-min turnaround","price":"From $39"},{"title":"Ceramic Coat","desc":"5-year protection","price":"From $899"},{"title":"Paint Correction","desc":"Showroom finish","price":"From $499"}]},
                {"type":"hours_block","today":"Today: 8am-8pm","week":[{"day":"Mon-Fri","hours":"8am-8pm"},{"day":"Sat","hours":"9am-6pm"},{"day":"Sun","hours":"10am-4pm"}]},
                {"type":"review_strip","rating":"5.0","count":340,"source":"Google","quotes":["\"Best detail in the city!\" — James R.","\"My car looks brand new\" — Lisa M."]},
                {"type":"cta_button_large","label":"Book Your Appointment","icon":"📅","action":"cal:"}
            ]
        },
        {
            "id": "biz_salon_clinic",
            "name": "Salon / Clinic Card",
            "type": "business_card",
            "category": "business_card",
            "niche": "Beauty/Health",
            "icon": "SC",
            "description": "Clean white design, practitioner photo + credentials, instant booking, service accordion (hair/nails/skin), 'Open Now' live status, verified Google review badge strip, location map pin link",
            "preview_colors": ["#f8fafc", "#8b5cf6", "#0f172a"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #ffffff 0%, #faf5ff 50%, #f3e8ff 100%)",
            "theme_key": "ghost_white",
            "layout_blocks": [
                {"type":"avatar","style":"rounded_square","size":"medium","catchphrase":"Board-Certified Specialist"},
                {"type":"title_block","headline":"Glow Aesthetics Studio","subtitle":"Advanced Skincare & Laser · Est. 2018"},
                {"type":"cta_button_large","label":"Book Appointment","icon":"📅","action":"cal:"},
                {"type":"hours_badge","status":"open","text":"🟢 Open Now · Until 7pm"},
                {"type":"service_cards","items":[{"title":"HydraFacial","desc":"60 min · Deep cleanse","price":"$185"},{"title":"Laser Treatment","desc":"45 min · Full face","price":"$350"},{"title":"Chemical Peel","desc":"30 min · 3 levels","price":"$150"},{"title":"Microneedling","desc":"90 min · With PRP","price":"$425"}]},
                {"type":"review_strip","rating":"4.9","count":520,"source":"Google","quotes":["\"Amazing results!\" — Sarah K.","\"My skin has never looked better\" — Rachel D."]},
                {"type":"action_row_3","items":[{"icon":"📞","label":"Call","action":"tel:"},{"icon":"🗺️","label":"Map","action":"maps:"},{"icon":"📸","label":"Gallery","action":"link:"}]},
                {"type":"social_strip","platforms":["instagram","facebook"]}
            ]
        },

        // --- B2B SAAS & AGENCY: Founder Card + Tech Agency + Freelancer ---
        {
            "id": "biz_saas_founder",
            "name": "SaaS Founder Card",
            "type": "business_card",
            "category": "business_card",
            "niche": "B2B SaaS",
            "icon": "SF",
            "description": "Dark mesh-gradient canvas, terminal-style typography, background grid lines, vCard download, calendar booking, live metric counters ($4.2M ARR, 50K+ Users, NPS 94), case study highlight, interactive lead capture form",
            "preview_colors": ["#0f172a", "#6366f1", "#c7d2fe"],
            "card_type": "business-card",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #312e81 50%, #0f172a 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type":"logo_mark","shape":"hexagon","initial":"S","catchphrase":"Scale. Automate. Grow."},
                {"type":"title_block","headline":"David Park","subtitle":"Founder & CEO · SwiftSoftware"},
                {"type":"stat_row","items":[{"value":"$4.2M","label":"ARR"},{"value":"50K+","label":"Users"},{"value":"94","label":"NPS"}]},
                {"type":"action_grid_2x2","items":[{"icon":"💾","label":"Save vCard","action":"download:"},{"icon":"📅","label":"Schedule Call","action":"cal:"},{"icon":"📊","label":"Pitch Deck","action":"link:"},{"icon":"💻","label":"Live Demo","action":"link:"}]},
                {"type":"case_studies","headline":"Case Study","items":[{"title":"Enterprise Client","result":"300% ROI in 6 months"}]},
                {"type":"lead_form","fields":["name","email","company"],"button":"Let's Talk","button_style":"pulse"},
                {"type":"social_strip","platforms":["linkedin","twitter","github"]}
            ]
        },
        {
            "id": "page_agency_tech",
            "name": "Tech Agency Page",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "Tech Agency",
            "icon": "TA",
            "description": "Terminal-style code block animation hero, live metric dashboard counters, case study cards with before/after metrics, ROI calculator placeholder, tech stack badge strip, 'Start a Project' CTA",
            "preview_colors": ["#0f172a", "#a855f7", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "radial-gradient(circle at 50% 30%, #1e1b4b 0%, #0f172a 60%, #311042 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type":"hero","headline":"We Build Software That Wins","subtitle":"Full-stack development agency. Rust, React, AI. Zero-fluff engineering for startups that ship.","cta_primary":"Start a Project","cta_secondary":"See Case Studies"},
                {"type":"stat_row","items":[{"value":"$120M+","label":"Client Revenue"},{"value":"47","label":"Products Shipped"},{"value":"14","label":"Years Building"},{"value":"98%","label":"Client Retention"}]},
                {"type":"case_studies","headline":"Recent Work","items":[{"title":"FinTech Platform","result":"$12M Series A, 45K users in 90d","tags":["Rust","React","AWS"]},{"title":"AI SaaS","result":"$3.2M ARR, acquired in 14mo","tags":["Python","LLM","GCP"]},{"title":"E-Commerce","result":"2.1M visits/mo, 99.9% uptime","tags":["Next.js","Postgres","K8s"]}]},
                {"type":"cta_button_large","label":"Book a Strategy Session","icon":"🚀","action":"cal:","style":"pulse"},
                {"type":"integration_strip","logos":["rust","typescript","react","python","aws","docker","kubernetes","postgres","redis","terraform"]},
                {"type":"lead_form","fields":["name","email","company","budget"],"button":"Get Free Tech Assessment","button_style":"pulse"},
                {"type":"trust_bar","text":"🔐 SOC 2 · ISO 27001 · 14-Year Track Record · US-Based Team"}
            ]
        },
        {
            "id": "biz_freelancer",
            "name": "Freelancer Card",
            "type": "business_card",
            "category": "business_card",
            "niche": "Freelance",
            "icon": "FL",
            "description": "Clean minimal card, availability toggle + hourly rate badge, skill tag cloud, portfolio preview links, downloadable CV, calendar booking, Upwork/Fiverr verification badges",
            "preview_colors": ["#0f172a", "#10b981", "#ecfdf5"],
            "card_type": "business-card",
            "gradient": "radial-gradient(circle at top left, #064e3b 0%, #0f172a 70%)",
            "theme_key": "emerald_glass",
            "layout_blocks": [
                {"type":"avatar","style":"circle_glow","size":"medium","status":"available","catchphrase":"🟢 Available for Projects"},
                {"type":"title_block","headline":"Alex Rivera","subtitle":"Senior Full-Stack Developer"},
                {"type":"stat_row","items":[{"value":"$125/hr","label":"Rate"},{"value":"8+","label":"Years Exp"},{"value":"32","label":"Projects"}]},
                {"type":"skill_cloud","items":["React","TypeScript","Node.js","Postgres","AWS","Docker","GraphQL","Next.js"]},
                {"type":"action_grid_2x2","items":[{"icon":"📄","label":"Download CV","action":"download:"},{"icon":"📅","label":"Book a Call","action":"cal:"},{"icon":"💼","label":"Portfolio","action":"link:"},{"icon":"✉️","label":"Send Email","action":"mailto:"}]},
                {"type":"profile_links","items":[{"label":"Upwork · Top Rated","icon":"🟢"},{"label":"GitHub · 1.2k ★","icon":"⭐"}]},
                {"type":"social_strip","platforms":["github","linkedin","twitter","stackoverflow"]}
            ]
        }
    ])))
}
