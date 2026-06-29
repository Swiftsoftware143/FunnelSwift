import re

with open('src/routes.rs') as f:
    content = f.read()

# Fix the layer ordering - move with_state() before layer() calls
content = content.replace(
    '        .layer(cors)\n        .layer(TraceLayer::new_for_http())\n}',
    '        .with_state(state)\n        .layer(cors)\n        .layer(TraceLayer::new_for_http())\n}'
)

with open('src/routes.rs', 'w') as f:
    f.write(content)

print('Layer ordering fixed')
