# AGENTS.md

Agent-specific context for changing Wildcat, the Bitcredit credit-mint services;
[README.md](README.md) says what it is. These are good defaults, not hard rules:
the developer's instructions win, and if a rule fights the task at hand, say so
and get sign-off before breaking it.

## Project Map

One Cargo workspace of axum services; they talk to each other, and to the
external Clowder and E-Bill nodes, through `bcr-common` (see Key Patterns).

    crates/
      bcr-wdc-core-service/       # the Cashu-style mint: keysets, blind signing, swap, proof state
      bcr-wdc-quote-service/      # e-bill credit quotes: pending → offered → accepted → minting enabled
      bcr-wdc-treasury-service/   # mint operations, on-chain payment, vault and inter-mint proofs
      bcr-wdc-admin-aggregator/   # /v1/admin/* facade over the others; its gen_api bin emits openapi.json
      bcr-wdc-wallet-aggregator/  # the facade wallets talk to (info, swap)
      bcr-wdc-utils/              # shared config, migration runner, keys, NUT-19 cache, routines
    migrations/                   # one SQL migration set for every service (core_/treasury_/quote_ tables)

## Getting Started

There is no scripted local setup (README covers only Docker images).

- Clone with `--recursive` or run `git submodule update --init`: Cargo reaches
  `bcr-common` through the `[patch]` in `Cargo.toml`, so an empty `bcr-common/`
  builds nothing. Git dependencies are fetched with the git CLI
  (`CARGO_NET_GIT_FETCH_WITH_CLI` is set in every workflow and Dockerfile), so
  your git credentials need read access to the BitcreditProtocol repositories.
- Each binary reads `config.toml` from its working directory plus
  service-prefixed environment variables (the `config::Environment` call in its
  `main.rs`). `config.toml` is not gitignored; keep local ones out of commits.

## Quality Gates

CI is the ground truth (`.github/workflows/rust.yml` and `test.yml`, run on
every branch push). The `justfile` covers only Docker images and OpenAPI, so
the gates are plain cargo; run the full set before opening a PR:

    cargo fmt -- --check                          # hard failure in CI
    cargo check                                   # hard failure; CI has no database, compiles from .sqlx/
    cargo test --workspace                        # in-memory and SurrealDB backends, needs no services
    cargo test --workspace -- --include-ignored   # adds the Postgres tests; this is what CI runs
    cargo clippy --all-features && cargo deny check  # CI records both but does not fail on them

- The `#[ignore]`d tests are the sqlx ones: they need `DATABASE_URL` for a
  Postgres role with CREATEDB (`#[sqlx::test]` creates a database per test);
  CI runs `cargo sqlx database reset -y --source migrations` first.
- After changing a `sqlx::query!`/`query_as!` call or the SQL it touches,
  regenerate `.sqlx/` with sqlx-cli (`cargo sqlx prepare --workspace`, needs
  `DATABASE_URL`) and commit it; CI's `cargo check` compiles from that metadata.

## Key Patterns

- **`bcr-common` owns the wire types and clients, and the submodule commit is
  what compiles, not the tag.** A contract change lands in bcr-common first,
  then the submodule is bumped and each crate adapted (the "adapt to new
  bcr-common key types" commits ending at bc16136). `Cargo.toml` names
  `bcr-common` by tag, but `[patch]` redirects it to `./bcr-common` (the comment
  cites rust-lang/cargo#5478) and `Cargo.lock` records the submodule's version.
  `bcr-ebill-core` is a real git-tag pin.
- **One `Repository` trait, three backends per service.** Each `persistence/`
  has in-memory, SurrealDB and sqlx implementations and the tests run the same
  functions against all of them, so a trait change touches all three. Postgres
  is the production store (CHANGELOG 0.6.0; the `migrate` bins move SurrealDB
  data over); `migrations/` is shared and compiled into `bcr-wdc-utils` via the
  `crates/bcr-wdc-utils/migrations` symlink.
- **Quote status, mint operation and ecash proofs are separate state machines
  in separate services.** `quotes::Status` ends at `MintingEnabled`; whether
  ecash was issued lives in the treasury's `MintOperation` (`target` vs
  `minted`) and core's proof tables. Never infer one machine's state from
  another's: `Accepted` or `MintingEnabled` does not mean value exists.
- **Side effects run before the status is persisted, so every step must be
  re-fireable.** quote-service `Service::enable_minting` fetches keys, mints
  fees and registers the mint operation with the treasury before the conditional
  `UPDATE … WHERE status = expected` commits. There is no cross-service
  transaction; recovery is re-running the step, which is why treasury
  `new_minting_operation` is idempotent for the same operation and rejects a
  different one for the same bill (`crates/bcr-wdc-treasury-service/src/ebill/service.rs`).
- **No router authenticates requests.** core-service keeps `admin` and `web` in
  separate `Router`s because admin "will likely have different auth
  requirements", yet they share one listener and the aggregators add no auth
  either; access control is the deployment's job. Keep that split, and never
  expose admin-strength operations (signing, burn, recover, new keyset) on a
  `web` or wallet-aggregator route.

## Common Gotchas

None recorded yet. Add an entry when something in this repo costs real time:
the trap, the symptom, the fix, and the commit or issue it came from.

## Glossary

- **`quotes::Status`** — the quote's workflow stage; it says nothing about issued ecash.
- **`Accepted`** — the wallet accepted an unexpired offer; minting is not yet enabled.
- **`MintingEnabled`** — the enablement side effects completed; issuance is not proven.
- **`MintOperation`** — the treasury's target and minted amounts; not the core proof state.
- **proof state** — core's spent/reserved record; it does not imply quote progress.
- **`admin` / `web` routers** — intended privilege, not authentication (Key Patterns).

## Hit every surface

Before calling a service change done, say which of these applied:

- **All three persistence backends** (in-memory, SurrealDB, sqlx): tests run the same
  functions against each, so one implementation drifting fails late.
- **`.sqlx/`** regenerated after any SQL change (Quality Gates); CI compiles from the cache.
- **OpenAPI**: `just openapi-generate-docs` after a route change; the dashboard's client is
  generated from that spec.
- **`bcr-common/`**: the submodule commit, not the manifest tag, is what compiles.

## Plans and work artifacts

- Plans, research notes and scratch files stay outside the worktree or gitignored; the
  merged PR is the implementation record. Do not add a second checklist or PR summary to
  the repo.

## Working Agreements

Organisation-wide rules (branch protection, reviews, labels, Dependabot) live in the
[contributing guide](https://github.com/BitcreditProtocol/.github/blob/master/CONTRIBUTING.md).
This section is the per-task delta.

- Open pull requests against `master`. Branch from it too: basing work on another branch
  conflicts in exactly the files other people are changing.
- Never open, mark ready or merge a PR, and never push a tag, unless the developer
  explicitly asks. Each is visible to the whole team, and a tag also starts a release.
- Commit small and often. Each commit is self-contained, passes the gate above and is
  reviewable on its own; the subject says why, not just what. Reviewers only catch
  mistakes in changes they can hold in their head.
- Titles: the repo's `[crate-name] plain-language summary` form, e.g.
  `[bcr-wdc-quote-service] reject duplicate mint requests`. Mark breaking changes with the
  `breaking` label, not a `!` in the title; release notes are built from labels (see
  [`.github/release.yml`](.github/release.yml)), so also label `bug`, `enhancement`,
  `documentation` or `dependencies`.
- Body: the problem in a sentence or two, then how it was fixed, then how it was verified.
  The [organisation PR
  template](https://github.com/BitcreditProtocol/.github/blob/master/.github/PULL_REQUEST_TEMPLATE.md)
  asks exactly that. End with the model and harness that did the work.
- Evidence: the test that failed before and passes now, a log excerpt, or the OpenAPI diff
  when routes change. Upload evidence to the PR on GitHub; never commit PR-only
  screenshots or assets.
- One concern per PR. If the description needs an "also", split it.
- Babysitting a PR: poll checks and comments newer than the last push; verify each bot
  finding against the source, fix the real ones, dismiss false positives with a written
  reason. No status check is required to merge, so a red check may predate your change:
  confirm that before blaming it, and say so in the PR. Stay quiet when nothing is new;
  stop when checks are green on the latest commit.
- Squash-merge keeps `master` linear, so the PR title becomes the commit subject; `ci:`
  and `chore:` prefixes stay for infrastructure. A `bcr-common` contract change is its own
  `[bcr-common] update ...` commit, followed by one adaptation commit per crate.
- [CHANGELOG.md](CHANGELOG.md) is written at release time, not per PR. Releases are
  `vX.Y.Z` tags: `build.yml` publishes the service images from a tag and `nightly.yml`
  from `master`, so leave tagging to maintainers.

## See Also

- [README.md](README.md) — what Wildcat is; building the Docker images
- [CHANGELOG.md](CHANGELOG.md) — release-level history
