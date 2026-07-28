import os, shutil

handler_dir = 'src/handlers'
originals = [
    'admin_handler.rs', 'affiliate_handler.rs', 'api_key_handler.rs',
    'dashboard_handler.rs', 'integration_target_handler.rs', 'lead_handler.rs',
    'mintbird_handler.rs', 'plan_handler.rs', 'plan_tag_handler.rs',
    'portfolio_handler.rs', 'routing_handler.rs', 'settings_handler.rs',
    'tag_group_handler.rs', 'tag_handler.rs', 'webhook_handler.rs'
]

handler_real = os.path.realpath(handler_dir)
to_remove = [f for f in os.listdir(handler_dir) if f.endswith('.rs') and f not in originals and f != 'mod.rs']
print(f'Removing {len(to_remove)} ADASwift handlers: {to_remove}')

for f in to_remove:
    target = os.path.realpath(os.path.join(handler_dir, f))
    if not target.startswith(handler_real + os.sep):
        raise ValueError(f"path traversal blocked: {f}")
    os.remove(target)

mod_path = os.path.realpath(os.path.join(handler_dir, 'mod.rs'))
if not mod_path.startswith(handler_real + os.sep):
    raise ValueError("path traversal blocked")
with open(mod_path, 'w') as f:
    for h in originals:
        mod_name = h.replace('.rs', '')
        f.write(f'pub mod {mod_name};\n')

print('mod.rs rebuilt')
print(f'Remaining handlers: {len(os.listdir(handler_dir))}')
