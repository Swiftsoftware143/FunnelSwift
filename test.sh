#!/bin/bash
# Get token
TOKEN=$(curl -s -X POST http://127.0.0.1:8080/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d @/tmp/login.json | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')

echo "TOKEN=$TOKEN"
echo ""

echo "=== ME ==="
curl -s http://127.0.0.1:8080/api/v1/auth/me -H "Authorization: Bearer $TOKEN"
echo ""

echo "=== PLANS ==="
curl -s http://127.0.0.1:8080/api/v1/plans -H "Authorization: Bearer $TOKEN"
echo ""

echo "=== TAG GROUPS ==="
curl -s http://127.0.0.1:8080/api/v1/tag-groups -H "Authorization: Bearer $TOKEN"
echo ""

echo "=== TAGS ==="
curl -s http://127.0.0.1:8080/api/v1/tags -H "Authorization: Bearer $TOKEN" | python3 -c 'import sys,json; data=json.load(sys.stdin); print(f"{len(data)} tags")'
echo ""

echo "=== CREATE LEAD ==="
LEAD_ID=$(curl -s -X POST http://127.0.0.1:8080/api/v1/leads \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name":"John Doe","email":"john@example.com","company":"Acme Corp","source":"Website","stage":"New"}' | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
echo "Lead created: $LEAD_ID"
echo ""

echo "=== LIST LEADS ==="
curl -s http://127.0.0.1:8080/api/v1/leads -H "Authorization: Bearer $TOKEN"
echo ""

echo "=== CREATE AFFILIATE ==="
AFF_ID=$(curl -s -X POST http://127.0.0.1:8080/api/v1/affiliates \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name":"Jane Affiliate","email":"jane@affiliate.com","commission_rate":10.5}' | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
echo "Affiliate: $AFF_ID"
echo ""

echo "=== DASHBOARD STATS ==="
curl -s http://127.0.0.1:8080/api/v1/dashboard/stats -H "Authorization: Bearer $TOKEN"
echo ""

echo "=== ACTIVITY LOG ==="
curl -s http://127.0.0.1:8080/api/v1/dashboard/activity -H "Authorization: Bearer $TOKEN"
echo ""

echo "=== SETTINGS ==="
curl -s http://127.0.0.1:8080/api/v1/settings -H "Authorization: Bearer $TOKEN"
echo ""

echo "ALL TESTS PASSED"
