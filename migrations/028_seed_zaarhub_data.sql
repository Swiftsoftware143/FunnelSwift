-- Seed ZaarHub city pages and business listings from multidirectory database
-- Requires: dblink extension and multidirectory in same cluster

DO $$
DECLARE
    sys_tenant_id UUID := '00000000-0000-0000-0000-000000000001';
    dir_record RECORD;
    biz_record RECORD;
    v_city_page_id UUID;
BEGIN
    -- Seed city_pages from multidirectory directories
    FOR dir_record IN
        SELECT d.id AS dir_id, d.slug, d.name, d.city, d.description,
               d.zaarhub_config,
               (d.zaarhub_config->>'homepage_hero_title') AS hero_title,
               (d.zaarhub_config->>'homepage_hero_subtitle') AS hero_subtitle,
               (d.zaarhub_config->>'featured_image_url') AS featured_image
        FROM dblink('host=localhost dbname=multidirectory user=swift password=SwiftSecure2026!', 
            'SELECT id, slug, name, city, description, zaarhub_config FROM directories WHERE status = ''published''')
        AS d(id UUID, slug VARCHAR, name VARCHAR, city TEXT, description TEXT, zaarhub_config JSONB)
        WHERE NOT EXISTS (SELECT 1 FROM city_pages WHERE city_slug = d.slug)
    LOOP
        INSERT INTO city_pages (tenant_id, city_slug, city_name, state, description, 
                                hero_image_url, meta_title, meta_description, is_active, display_order)
        VALUES (sys_tenant_id, dir_record.slug, COALESCE(dir_record.city, dir_record.name),
                'FL', dir_record.description, dir_record.featured_image,
                'Best Businesses in ' || COALESCE(dir_record.city, dir_record.name) || ' | ZaarHub',
                'Find top-rated local businesses in ' || COALESCE(dir_record.city, dir_record.name) || ', FL. Browse reviews, deals, and more.',
                true, 0)
        ON CONFLICT (tenant_id, city_slug) DO NOTHING
        RETURNING id INTO v_city_page_id;
    END LOOP;

    -- Seed business_listings from multidirectory businesses
    FOR biz_record IN
        SELECT b.id AS biz_id, b.name, b.description, b.address, b.city, b.state,
               b.phone, b.website, b.rating, b.review_count,
               b.images, b.latitude, b.longitude,
               d.slug AS dir_slug, d.id AS dir_id
        FROM dblink('host=localhost dbname=multidirectory user=swift password=SwiftSecure2026!', 
            'SELECT b.id, b.name, b.description, b.address, b.city, b.state, b.phone, b.website, b.rating, b.review_count, b.images, b.latitude, b.longitude, d.slug, d.id FROM businesses b JOIN directories d ON b.directory_id = d.id WHERE b.is_active = true AND d.status = ''published''')
        AS b(id UUID, name VARCHAR, description TEXT, address VARCHAR, city VARCHAR, state VARCHAR, phone VARCHAR, website VARCHAR, rating DOUBLE PRECISION, review_count INTEGER, images JSONB, latitude DOUBLE PRECISION, longitude DOUBLE PRECISION, dir_slug VARCHAR, dir_id UUID)
    LOOP
        -- Find matching city_page
        SELECT id INTO v_city_page_id FROM city_pages WHERE city_slug = biz_record.dir_slug;
        IF v_city_page_id IS NULL THEN
            CONTINUE; -- Skip if no city page was created
        END IF;

        -- Get primary category for this business
        INSERT INTO business_listings (
            city_page_id, business_name, category, subcategory,
            description, address, phone, website,
            logo_url, cover_image_url,
            rating, review_count, is_featured, is_claimed,
            coordinates_lat, coordinates_lng, display_order
        )
        SELECT
            v_city_page_id,
            biz_record.name,
            dc.name,
            NULL,
            biz_record.description,
            biz_record.address,
            biz_record.phone,
            biz_record.website,
            biz_record.images->>0,
            biz_record.images->>1,
            biz_record.rating,
            COALESCE(biz_record.review_count, 0),
            false, false,
            biz_record.latitude,
            biz_record.longitude,
            0
        FROM dblink('host=localhost dbname=multidirectory user=swift password=SwiftSecure2026!',
            format('SELECT dc.name FROM business_categories bc JOIN directory_categories dc ON bc.category_id = dc.id WHERE bc.business_id = ''%s'' AND bc.is_primary = true LIMIT 1', biz_record.biz_id))
        AS cat(cat_name VARCHAR)
        ON CONFLICT DO NOTHING;
    END LOOP;

    RAISE NOTICE 'Seeded city_pages and business_listings successfully';
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'Seed error (dblink may not be available): % — run manual seed instead', SQLERRM;
END;
$$;
