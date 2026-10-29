# 0.7.0
* SurrealDB → PostgreSQL migration: the core and treasury services now persist with sqlx, and the core service was consolidated into a single service.
* Quote expiry: quote offers now expire at the bill's maturity, late mints are honoured, and a bill can be quoted on its maturity day.
* Quote notifications: the quote service sends a notification for each new quote request.
* Foreign mint balances: the treasury has a new API for balances held at other mints.
* Admin aggregator endpoints: new endpoints for e-bill balance, onchain history, keyset balances, foreign balance, e-IOU deposits and e-IOU history.

# 0.6.0
* Consolidated key and swap services into core.
* Adopted the shared maturity boundary, spending-condition check and intermint HTLC helpers from bcr-common; onchain minting now fails closed without a debit keyset.
* Introduced redemption-date keysets.
* Added online and offline intermint exchanges.
* Expanded Clowder and multisig integration.
* Added mandatory swap commitments and attestations.
* Improved eBill minting and validation.
* Enhanced mint/melt fees and safeguards.
* Migrated persistence to PostgreSQL/SQLx.
* Added NUT-19 response caching.

# 0.5.0
* upgrade bcr-ebill-core to 0.4.5
* fixes in config for bcr-wdc-ebpp
* share full ebill detail in quote enquire
# 0.4.1

* Change `enquire` API in `bcr-wdc-quote-service` to accept a full shared bill, encrypted with the mint's pub key (breaking)
    * This needs to be coordinated with E-Bill 0.4.2 to make minting work again
    * The `quote-service` DB has a breaking change, so it needs to be cleared on deployment
* Up ebill dependencies

# 0.2.0
* MVP of the wildcat system
