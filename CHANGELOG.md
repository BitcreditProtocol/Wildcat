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
