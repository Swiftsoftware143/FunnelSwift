import re

with open('/etc/nginx/nginx.conf', 'r') as f:
    content = f.read()

block = '''  server {
    listen 80;
    server_name app.funnelswift.com funnelswift.swiftsoftware.vc;

    location / {
      proxy_pass http://127.0.0.1:8080;
      proxy_set_header Host $host;
      proxy_set_header X-Real-IP $remote_addr;
      proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
      proxy_set_header X-Forwarded-Proto $scheme;
    }
  }

'''

# Insert before 'listen 80 default_server;' line
content = content.replace('  listen 80 default_server;', block + '  listen 80 default_server;')

with open('/etc/nginx/nginx.conf', 'w') as f:
    f.write(content)

print('Config updated')
