#!/bin/bash


curl -X POST "http://127.0.0.1:3338/v1/admin/identity/create" \
  -H "Content-Type: application/json" \
  -d '{
    "t": 0,
    "name": "BitcreditWildcat - dev",
    "email": "info@bitcr.org",
    "postal_address": {
      "country": "AT",
      "city": "Vienna",
      "zip": "1020",
      "address": "Smithstreet 15, Top 22"
    },
    "date_of_birth": "2020-01-01",
    "country_of_birth": "AT",
    "city_of_birth": null,
    "identification_number": null,
    "profile_picture_file_upload_id": null,
    "identity_document_file_upload_id": null
  }'

