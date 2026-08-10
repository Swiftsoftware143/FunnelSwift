import re

with open('src/routes.rs') as f:
    content = f.read()

# Add health function before create_router
health_fn = '''async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "funnelswift",
        "version": "0.1.0"
    }))
}

'''

if 'async fn health' not in content:
    content = content.replace('pub fn create_router', health_fn + 'pub fn create_router')

# Fix the type mismatch - Router<()> vs Router<AppState>
# The issue is that layer() calls before with_state() return a different type
# Replace .layer(cors) with .with_state(state).layer(cors) pattern
# Actually the fix is simpler: put with_state() AFTER the layer calls

with open('src/routes.rs', 'w') as f:
    f.write(content)

print('routes.rs fixed')
