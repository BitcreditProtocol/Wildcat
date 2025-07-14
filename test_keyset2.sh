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

echo "Keycloak response: $KEYCLOAK_RESPONSE"

TOKEN=$(echo "$KEYCLOAK_RESPONSE" | jq -r '.access_token')

echo "Got token: $TOKEN"

curl -v \
  -H "Authorization: Bearer $TOKEN" \
  http://localhost:4343/v1/keysets/008373479282b552
