# Wildcat
Wildcat Credit Mint Service

## Quote-service scaling constraint

The current local/testnet deployment must run exactly one quote-service replica. Transitions out of
`Pending` and their exposure updates use one in-process mutex plus SurrealDB transactions because
SurrealDB 2.6's embedded engine does not provide the cross-replica compare-and-set semantics these
transitions need. Do not horizontally scale quote-service until they use a DB-backed cross-replica
CAS or lease.

# Docker Images

Login with your Github user account and PAT token with:

```
docker login ghcr.io
```

Then

```
just build-docker-images
```
