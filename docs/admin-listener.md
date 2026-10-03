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

(Treasury and quote-service gain the same setting and env-var pattern, prefixed with
their own service name, as they are split.)

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
- `bcr-wdc-quote-service/src/client.rs`: `sign` (admin); `keys` (public)
- `bcr-wdc-admin-aggregator/src/lib.rs`: `new_keyset` (admin)

`bcr-wdc-mint-service/src/vault/clients.rs` and `bcr-wdc-wallet-aggregator` only call
core-service's public endpoints (`check_state`, `currency_unit`) and do not need to
move.

Today's exposure: every one of core-service's admin paths is under `/admin/...`,
and the ingress in front of it only rewrites `/v1/*`, so none of them is reachable
through a public route even before this split; the split closes the remaining gap of
the admin router sharing a socket with the public one.
