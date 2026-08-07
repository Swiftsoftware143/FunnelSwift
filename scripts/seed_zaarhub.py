"""Seed ZaarHub city pages and business listings from multidirectory."""
import psycopg2
import json

SRC = "postgresql://swift:SwiftSecure2026!@localhost:5432/multidirectory"
DST = "postgresql://swift:SwiftSecure2026!@localhost:5432/funnelswift"
SYS_TENANT = "00000000-0000-0000-0000-000000000001"

def main():
    src = psycopg2.connect(SRC)
    dst = psycopg2.connect(DST)
    
    # Check if already seeded
    cur = dst.cursor()
    cur.execute("SELECT COUNT(*) FROM city_pages")
    existing = cur.fetchone()[0]
    if existing > 0:
        print(f"Already {existing} city pages; skipping seed")
        src.close()
        dst.close()
        return
    
    # Fetch directories
    cur_src = src.cursor()
    cur_src.execute("""
        SELECT id, slug, name, city, description, zaarhub_config
        FROM directories WHERE status = 'active'
    """)
    dirs = cur_src.fetchall()
    print(f"Found {len(dirs)} directories")
    
    slug_map = {}
    count_cities = 0
    for row in dirs:
        dir_id, slug, name, city, desc, zaarhub_cfg = row
        cfg = zaarhub_cfg or {}
        city_name = city or name
        featured_img = cfg.get("featured_image_url")
        cur.execute("""
            INSERT INTO city_pages (tenant_id, city_slug, city_name, state, description,
                hero_image_url, meta_title, meta_description, is_active, display_order)
            VALUES (%s, %s, %s, 'FL', %s, %s, %s, %s, true, 0)
            ON CONFLICT (tenant_id, city_slug) DO UPDATE SET
                city_name = EXCLUDED.city_name,
                description = EXCLUDED.description,
                updated_at = now()
            RETURNING id
        """, (
            SYS_TENANT, slug, city_name, desc, featured_img,
            f'Best Businesses in {city_name} | ZaarHub',
            f'Find top-rated local businesses in {city_name}, FL. Browse reviews, deals, and more.'
        ))
        page_id = cur.fetchone()[0]
        slug_map[dir_id] = page_id
        count_cities += 1
    print(f"Seeded {count_cities} city pages")
    
    # Fetch category map
    cur_src.execute("SELECT id, name FROM directory_categories")
    cat_map = {row[0]: row[1] for row in cur_src.fetchall()}
    
    # Fetch businesses with primary category
    cur_src.execute("""
        SELECT b.id, b.name, b.description, b.address, b.phone, b.website,
               b.rating, b.review_count, b.images, b.latitude, b.longitude,
               b.directory_id
        FROM businesses b
        JOIN directories d ON b.directory_id = d.id
        WHERE b.is_active = true AND d.status = 'active'
    """)
    bizs = cur_src.fetchall()
    print(f"Found {len(bizs)} businesses")
    
    count_listings = 0
    for row in bizs:
        biz_id, name, desc, addr, phone, website, rating, rc, imgs, lat, lng, dir_id = row
        cp_id = slug_map.get(dir_id)
        if cp_id is None:
            continue
        
        # Get primary category
        cat_name = None
        cur_src.execute("""
            SELECT bc.category_id FROM business_categories bc
            WHERE bc.business_id = %s AND bc.is_primary = true
            LIMIT 1
        """, (biz_id,))
        cat_row = cur_src.fetchone()
        if cat_row:
            cat_name = cat_map.get(cat_row[0])
        
        logos = json.loads(imgs) if isinstance(imgs, str) else (imgs or [])
        logo = logos[0] if len(logos) > 0 else None
        cover = logos[1] if len(logos) > 1 else None
        
        cur.execute("""
            INSERT INTO business_listings (city_page_id, business_name, category,
                description, address, phone, website,
                logo_url, cover_image_url,
                rating, review_count, is_featured, is_claimed,
                coordinates_lat, coordinates_lng, display_order)
            VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, false, false, %s, %s, 0)
            ON CONFLICT DO NOTHING
        """, (cp_id, name, cat_name, desc, addr, phone, website, logo, cover, rating, rc or 0, lat, lng))
        count_listings += 1
    
    dst.commit()
    print(f"Seeded {count_listings} business listings")
    src.close()
    dst.close()

if __name__ == "__main__":
    main()
