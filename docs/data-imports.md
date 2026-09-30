# SurrealDB → PostgreSQL data imports

The `core-migrate` and `treasury-migrate` binaries use the existing `config.toml` and
`CORE_SERVICE__…` / `TREASURY_SERVICE__…` environment settings. Run each service's
binary with `--dry-run` to read and report source data. Dry runs never connect to
PostgreSQL, run schema migrations, or read/write import state, even when an
import has already completed. Treasury dry runs also report duplicate source
ebill `uid`/`bill_id` and vault `y` values; reconcile these before importing. Dry
runs do not inspect PostgreSQL for existing destination conflicts.

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

IDs identify one-time imports and must not change with image tags. They are not
a way to trigger schema upgrades or reload an existing destination.
No imports have been added for foreign repositories, denied onchain operations,
or other data not read by the original binaries.

Each destination database has its own `wdc_data_imports` table, separate from
SQLx's `_sqlx_migrations`. Each target acquires a session advisory lock on a
dedicated PostgreSQL connection before checking this table. Concurrent attempts
wait; after the first completes, the waiting attempt skips successfully. Targets
sharing a database also serialize and retain separate completion rows.

Completed targets log that the import was already applied and return successfully
without connecting to their SurrealDB source. Every non-dry execution applies
pending SQLx schema migrations under the advisory lock before checking import
completion, so subsequent deployments still receive schema updates. A schema
migration failure fails the job even for a completed import. All imported rows,
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
existing spent proofs cause failure and are never overwritten. A commitment
with a repeated input or output y is rejected as malformed source data. Treasury
imports reject duplicate destination records instead of skipping or overwriting
them, and preserve onchain source statuses
without runtime expiry updates. These rules are limited to the migration helpers;
normal application persistence is unchanged.

Preparation should use dry runs or disposable rehearsal destinations while source
writers remain active. A completed real import does not synchronize newer source
data. Completion also does not change application backend selection. The cutover
and retirement plan, including runtime changes and consumer inventory, is tracked
in [issue #670](https://github.com/BitcreditProtocol/Wildcat/issues/670#issuecomment-5911794646).

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
SQLx gives each test an isolated database. CI runs these PostgreSQL tests with
`--include-ignored` as part of `cargo test --workspace`. For example:

```sh
SQLX_OFFLINE=true cargo test -p bcr-wdc-utils --test data_import -- --include-ignored
SQLX_OFFLINE=true cargo test -p bcr-wdc-core-service --test data_import --bin core-migrate -- --include-ignored
SQLX_OFFLINE=true cargo test -p bcr-wdc-treasury-service --test data_import --bin treasury-migrate -- --include-ignored
```

The tests cover first/repeat imports, real advisory-lock contention, interruption,
pending schema updates on completed imports, write/validation rollback, Core
conflict precedence, separate Treasury markers, dry runs, and completed targets
with unavailable SurrealDB sources. Binary tests
use in-memory SurrealDB; production source connectivity and production-scale
transaction sizes need validation in the deployment environment.
