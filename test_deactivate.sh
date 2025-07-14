#! /bin/bash

KEYCLOAK_RESPONSE=$(
curl -X POST 'http://localhost:8080/realms/dev/protocol/openid-connect/token' \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  -d 'grant_type=password' \
  -d 'client_id=bff-local-dev' \
  -d 'client_secret=cute-kitties' \
  -d 'username=democat' \
  -d 'password=bitcredit'
)

TOKEN=$(echo "$KEYCLOAK_RESPONSE" | jq -r '.access_token')
echo "Got token: $TOKEN"

DATA='{"kid": "0011d780bb595e09"}'

curl -v \
  -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "$DATA" \
  http://localhost:4242/v1/admin/keys/deactivate
