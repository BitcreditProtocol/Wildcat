#!/bin/bash

curl -v -X POST \
  -H "Content-Type: application/json" \
  -d '{}' \
  "http://localhost:3339/v1/admin/credit/quote/enable_mint/${ID}"

