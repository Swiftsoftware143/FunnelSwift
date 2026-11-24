use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};

pub async fn security_headers(request: Request, next: Next) -> Result<Response, StatusCode> {
    let mut response = next.run(request).await;

    let headers = response.headers_mut();

    // Prevent MIME type sniffing
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );

    // Prevent clickjacking
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        header::HeaderValue::from_static("DENY"),
    );

    // Enable XSS filter in older browsers
    headers.insert(
        header::HeaderName::from_static("x-xss-protection"),
        header::HeaderValue::from_static("1; mode=block"),
    );

    // HSTS - force HTTPS
    headers.insert(
        header::HeaderName::from_static("strict-transport-security"),
        header::HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );

    // Referrer policy
    headers.insert(
        header::HeaderName::from_static("referrer-policy"),
        header::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // CSP — Content Security Policy
    //
    // `static.cloudflareinsights.com` is allowed because Cloudflare injects its RUM beacon into the
    // HTML of every zone with Web Analytics on, and until 2026-11-24 the CSP blocked it: measured in
    // a real browser on a served funnel page, the console carried
    // `Loading the script 'https://static.cloudflareinsights.com/beacon.min.js/…' violates the
    // following CSP directive: "script-src 'self' 'unsafe-inline' …"` and the request never left the
    // browser — so every visitor to a public funnel or card page was invisible to Web Analytics.
    // The policy already allows the other analytics vendors these pages render themselves (Google
    // Tag Manager, the Meta pixel), so refusing Cloudflare's own beacon was an oversight, not a
    // decision. The beacon's POST target is covered by `connect-src 'self' https:`.
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        header::HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval' https://cdn.tailwindcss.com https://www.googletagmanager.com https://connect.facebook.net https://static.cloudflareinsights.com; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://cdn.tailwindcss.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data: https:; connect-src 'self' https:; frame-src 'self' https://www.youtube.com https://player.vimeo.com https://www.facebook.com;"
        ),
    );

    // Permissions policy
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        header::HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=(), interest-cohort=()",
        ),
    );

    // Remove server headers that leak info
    headers.remove(header::SERVER);

    Ok(response)
}
