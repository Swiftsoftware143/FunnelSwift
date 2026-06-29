use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};

pub async fn security_headers(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
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

    // Permissions policy
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        header::HeaderValue::from_static("camera=(), microphone=(), geolocation=(), interest-cohort=()"),
    );

    // Remove server headers that leak info
    headers.remove(header::SERVER);

    Ok(response)
}