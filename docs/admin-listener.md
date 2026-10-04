# Admin listener

Several services bind their admin-only routes (signing, burning, keyset creation,
treasury operations, ...) to their own listener, separate from the public one, so a
public-facing ingress can never reach them by construction.

## Setting

Each service that has one reads an `admin_bind_address` next to its existing
`bind_address`. There is no default: a service refuses to start if it is unset,
rather than silently falling back to loopback, since a container's loopback is not
reachable from its own internal callers either, and a silent fallback would break
them without closing any exposure a deployer didn't already control.

| Service           | Config key            | Env var                                  |
|-------------------|------------------------|-------------------------------------------|
| core-service      | `admin_bind_address`   | `CORE_SERVICE__ADMIN_BIND_ADDRESS`         |
| mint-service      | `admin_bind_address`   | `MINT_SERVICE__ADMIN_BIND_ADDRESS`         |
| quote-service     | `admin_bind_address`   | `QUOTE_SERVICE__ADMIN_BIND_ADDRESS`        |
| treasury-service  | `admin_bind_address`   | `TREASURY_SERVICE__ADMIN_BIND_ADDRESS`     |

## core-service's and wallet-aggregator's own outbound admin clients

core-service calls treasury-service's admin endpoint `fees_store_proofs` after
every swap, so its single `treasury_url` setting is renamed `treasury_admin_url`
(env `CORE_SERVICE__APPCFG__TREASURY_ADMIN_URL`) and must point at treasury's
admin listener; core-service has no public call into treasury left.

wallet-aggregator calls treasury-service's admin endpoint `try_htlc` on every HTLC
swap, so its `treasury_client_url` setting is renamed `treasury_admin_client_url`
(env `WALLET_AGGREGATOR__APPCFG__TREASURY_ADMIN_CLIENT_URL`) and must point at
treasury's admin listener.

## quote-service's own outbound admin clients

quote-service calls core-service's and treasury-service's admin endpoints
internally (`appcfg.core_admin_url`, `appcfg.treasury_admin_url`, both read next to
the existing `core_url` and no default either, so a missing setting fails startup
rather than silently pointing at the public listener):

- `core_admin_url` (env `QUOTE_SERVICE__CORE_ADMIN_URL`): `new_keyset`, `sign`.
  `core_url` (public, unchanged) still serves `list_keyset_info`, `keys`.
- `treasury_admin_url` (env `QUOTE_SERVICE__TREASURY_ADMIN_URL`): every call quote
  makes to treasury (`new_ebill_mint_operation`, `ebill_mint_operation_status`,
  `fees_store_proofs`) is an admin endpoint, so quote's whole treasury client now
  points at the admin listener; there is no public treasury client left in
  quote-service.

## admin-aggregator's outbound clients

admin-aggregator calls core-, quote- and treasury-service's admin endpoints, so its
`appcfg` names each admin listener explicitly, none defaulted. Nested keys are set
from the environment with `__` between them (top-level keys keep the single `_`
after the prefix, e.g. `ADMIN_AGGREGATOR_BIND_ADDRESS`):

| Config key                  | Env var                                    | Calls                         |
|-----------------------------|--------------------------------------------|-------------------------------|
| `appcfg.core_admin_url`     | `ADMIN_AGGREGATOR_APPCFG__CORE_ADMIN_URL`     | `new_keyset`                  |
| `appcfg.quotes_admin_url`   | `ADMIN_AGGREGATOR_APPCFG__QUOTES_ADMIN_URL`   | every quote call (replaces `quotes_url`) |
| `appcfg.treasury_admin_url` | `ADMIN_AGGREGATOR_APPCFG__TREASURY_ADMIN_URL` | every treasury call (replaces `treasury_url`) |

`appcfg.core_url` (`ADMIN_AGGREGATOR_APPCFG__CORE_URL`, public, unchanged) still
serves `list_keyset_info` and `keyset_info`.

## admin-aggregator's own auth

admin-aggregator has no public portion to split off, so its whole API (everything
but `/health`) requires `Authorization: Bearer <appcfg.admin_api_key>`
(env `ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY`), checked by
`bcr_wdc_utils::auth::require_api_key`. The key must be a non-empty, printable
ASCII string with no leading or trailing whitespace, since an HTTP header value
can never carry such a key once it crosses a transport. Its swagger UI
(`/swagger-ui`, `/api-docs/openapi.json`) is deliberately left undecorated, since
it discloses only the API shape, not data.

## Internal callers of core-service's admin endpoints

core-service's admin routes (`/admin/keys`, `/admin/keys/sign`,
`/admin/keys/verify/proof`, `/admin/keys/verify/fingerprint`, `/admin/burn`,
`/admin/swap/recover`, `/admin/reserve`) are called internally through
`bcr_common::client::core::Client` (re-exported from `client::admin::core::Client`),
the same client type used for its public (`web_ep`) endpoints. Today every caller
below points that one client at core-service's public address; once each caller is
updated it must point at the admin address instead for the methods marked admin:

- `bcr-wdc-treasury-service/src/ebill/client.rs`: `sign`, `burn`, `recover` (admin);
  `keyset_info` (public)
- `bcr-wdc-treasury-service/src/onchain/clients.rs`: `sign`, `burn`, `recover`,
  `reserve`, `verify_fingerprint`, `verify_proof` (admin); `keyset_info`, `keys`,
  `list_keyset_info`, `check_state` (public)
- `bcr-wdc-treasury-service/src/foreign/clients.rs`: `sign`, `burn` (admin); `keys`,
  `check_state` (public)
- `bcr-wdc-quote-service/src/client.rs`: `sign`, `new_keyset` (admin, now pointed at
  `core_admin_url`); `list_keyset_info`, `keys` (public, unchanged) — done
- `bcr-wdc-admin-aggregator/src/lib.rs`: `new_keyset` (admin, now pointed at
  `core_admin_url`) — done

`bcr-wdc-mint-service/src/vault/clients.rs` and `bcr-wdc-wallet-aggregator` only call
core-service's public endpoints (`check_state`, `currency_unit`) and do not need to
move.

Today's exposure: every one of core-service's admin paths is under `/admin/...`,
and the ingress in front of it only rewrites `/v1/*`, so none of them is reachable
through a public route even before this split; the split closes the remaining gap of
the admin router sharing a socket with the public one.
