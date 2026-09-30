# SurrealDB → PostgreSQL data imports

The `core-migrate` and `treasury-migrate` binaries use the existing `config.toml` and
`CORE_SERVICE__…` / `TREASURY_SERVICE__…` environment settings. Run each service's
binary with `--dry-run` to read and report source data. Dry runs never connect to
PostgreSQL, run schema migrations, or read/write import state, even when an
import has already completed.

Before importing, **stop all source writers** and keep them stopped until all
required targets finish. The separate SurrealDB reads do not form a snapshot.
Stop destination application writers as well, and back up the destination
before the first import. The advisory lock coordinates migration processes; it
does not stop application writers.
After verifying completion, switch writers to PostgreSQL; completed imports do
not synchronize later source changes.

| Target | Destination setting | Stable import ID |
| --- | --- | --- |
| Core | `appcfg.repository_new` | `surreal-to-postgres/core/v1` |
| Treasury ebill | `appcfg.ebill.new` | `surreal-to-postgres/treasury-ebill/v1` |
| Treasury vault | `appcfg.vault.new` | `surreal-to-postgres/treasury-vault/v1` |
| Treasury onchain | `appcfg.onchain.new` | `surreal-to-postgres/treasury-onchain/v1` |

IDs describe the data conversion version and must not change with image tags.
No imports have been added for foreign repositories, denied onchain operations,
or other data not read by the original binaries.

Each destination database has its own `wdc_data_imports` table, separate from
SQLx's `_sqlx_migrations`. Each target acquires a session advisory lock on a
dedicated PostgreSQL connection before checking this table. Concurrent attempts
wait; after the first completes, the waiting attempt skips successfully. Targets
sharing a database also serialize and retain separate completion rows.

Completed targets log that the import was already applied and return successfully
without connecting to their SurrealDB source or running schema migrations.
For a new import, existing SQLx schema migrations run first. All imported rows,
write validation, and the completion marker then share one transaction and the
locked connection. Any read, conversion, write, or validation error fails the
process and rolls back that target's data transaction. Schema initialization
and the empty bookkeeping table can remain after failure.

Core imports keysets, signatures, commitments, reserved ys and spent proofs.
It keeps existing keysets and signatures on their specific primary-key
conflicts. Commitments import before reservations; a conflicting commitment
signature, input or output skips that whole commitment, with no partial rows.
Reservations keep an existing proof state. Imported spent proofs replace
committed/reserved placeholders by clearing their signature and deadline;
existing spent proofs cause failure and
are never overwritten. Treasury imports reject duplicate destination records
instead of skipping or overwriting them, and preserve onchain source statuses
without runtime expiry updates. These rules are limited to the migration helpers;
normal application persistence is unchanged.

## Preparation, cutover and retirement

During preparation, keep the deployed backend selection unchanged. Use dry runs
or disposable rehearsal destinations while applications continue writing to
SurrealDB. A successful real import is permanent for that destination and ID;
rerunning it at cutover will skip, not import newer source data. If the final
destination was imported prematurely, use the backup/audit recovery procedure
below rather than deleting its marker or changing its ID.

At cutover, stop source and destination writers, including background routines,
back up the databases, import all required targets, and reconcile the frozen
source with PostgreSQL. Check record contents and business totals as well as
counts, accounting for the intentional Core conflict rules. Only then switch
application repositories and verify their reads and writes against PostgreSQL.
The completion marker confirms the configured import transaction succeeded;
it does not switch any application's backend or prove all SurrealDB consumers
have migrated.

Runtime changes still needed:

- Core selects SurrealDB when `appcfg.repository_new.max_connections` is zero.
  A positive value selects its SQLx repository; configure the imported destination
  before starting writers. Import completion alone never changes this setting.
- Treasury constructs SurrealDB repositories for ebill, vault, onchain, foreign
  online and foreign offline at startup. The first three must be wired to their
  existing SQLx implementations and `*.new` settings. Foreign online has a SQLx
  implementation but no import here; foreign offline has no SQLx implementation.
  Both require a separate transition plan before Treasury can start without
  SurrealDB. Denied onchain operations also remain outside this import's scope.
- The new `bcr-wdc-mint-service` is upstream's first step towards merging Core and
  Treasury. This task does not select that runtime automatically. If deployed,
  include its repositories in backend and consumer checks too.

In Wildcat-deployment, later remove the SurrealDB health dependencies from the
application services and any retained migration jobs in `base/docker-compose.yml`
and applicable environment overrides. The `clowder-dev` migration jobs currently
use `--dry-run`, which always needs the source, and applications wait for those
jobs to succeed. Move such reporting jobs out of the startup path, or retain
normal completed-import checks without a SurrealDB health dependency. Keep
PostgreSQL provisioning/health dependencies. Source configuration fields remain
required by current configuration parsing even when completed imports skip
their source connection.

Before retiring the shared SurrealDB instance, inventory the actual deployed
images and remaining consumers. Current code/configuration identifies Quote,
Treasury's foreign repositories, the separate eBill service database, EIC and
ENS. Treasury's ebill target is not the eBill service's own database. Older local
Compose files also reference key/swap services; confirm whether they still run.
Wallet deployment files retain a legacy SurrealDB stanza, but current wallet
runtime code has no database connection and its SurrealDB dependency is for
development tests. Source access from other deployments or external tools must
also be checked before removing the source service and its data.

After PostgreSQL starts accepting writes, the frozen SurrealDB is no longer a
current rollback destination. Define and verify PostgreSQL backup/restore or an
explicit reconciliation procedure before cutover; simply switching back would
discard newer changes. Retire SurrealDB dependencies and the source database
only after operation, recovery and all remaining consumer transitions are verified.

## Recovery after a failure or interruption

1. Keep writers stopped. Inspect the error and query **each configured destination**:

   ```sql
   SELECT id, completed_at FROM wdc_data_imports ORDER BY id;
   ```

   A missing table means import state was never initialized there. An absent
   target ID means its data transaction did not commit. Previously completed
   Treasury targets can remain committed when a later target fails.
2. Fix the reported source, conversion, constraint, permission, or connectivity
   problem, then rerun the same binary with the same destination and import IDs.
   Failed/interrupted transactions leave no partial imported rows. PostgreSQL
   releases the lock and rolls back when the dedicated connection closes. If a
   migration process is still alive or its broken connection has not yet been
   detected, wait for it to exit or have the database operator terminate that
   specific migration session before retrying.
3. If the process died during commit or the commit acknowledgement was lost,
   rerun normally: PostgreSQL atomically committed both data and marker, or
   neither. The completion check resolves the outcome without needing the source
   when the commit succeeded.

Do not delete a completion marker or change its ID to force a retry: that can
reimport into live data. If an older migration binary already wrote partial or
complete data without a marker, restore the affected destination from its
pre-import backup before rerunning, or perform an explicit operator audit and
reconciliation first. Never mark an import completed solely to bypass errors.

## Focused verification

Use a disposable PostgreSQL server and a `DATABASE_URL` with `CREATEDB` permission;
SQLx gives each test an isolated database. For example:

```sh
SQLX_OFFLINE=true cargo test -p bcr-wdc-utils --test data_import -- --include-ignored
SQLX_OFFLINE=true cargo test -p bcr-wdc-core-service --test data_import --bin core-migrate -- --include-ignored
SQLX_OFFLINE=true cargo test -p bcr-wdc-treasury-service --test data_import --bin treasury-migrate -- --include-ignored
```

The tests cover first/repeat imports, real advisory-lock contention, interruption,
write/validation rollback, Core conflict precedence, separate Treasury markers,
dry runs, and completed targets with unavailable SurrealDB sources. Binary tests
use in-memory SurrealDB; production source connectivity and production-scale
transaction sizes need validation in the deployment environment.
