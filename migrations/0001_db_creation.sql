-- core-service
CREATE TABLE IF NOT EXISTS core_signatures (
    y TEXT PRIMARY KEY,
    blob JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS core_proofs (
    y TEXT PRIMARY KEY,
    blob JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS core_keys (
    kid TEXT PRIMARY KEY,
    unit TEXT NOT NULL,
    active BOOLEAN NOT NULL,
    final_expiry BIGINT,
    blob JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS core_keys_unit_idx ON core_keys (unit);
CREATE INDEX IF NOT EXISTS core_keys_final_expiry_idx ON core_keys (final_expiry);

CREATE TABLE core_commitments (
    signature TEXT PRIMARY KEY,
    expiration TIMESTAMPTZ NOT NULL,
    blob JSONB NOT NULL
);

CREATE TABLE core_commitment_inputs (
    y TEXT PRIMARY KEY,
    signature TEXT NOT NULL REFERENCES core_commitments(signature) ON DELETE CASCADE
);

CREATE TABLE core_commitment_outputs (
    y TEXT PRIMARY KEY,
    signature TEXT NOT NULL REFERENCES core_commitments(signature) ON DELETE CASCADE
);

CREATE INDEX core_commitments_expiration_idx ON core_commitments (expiration);
CREATE INDEX core_commitment_inputs_signature_idx ON core_commitment_inputs (signature);
CREATE INDEX core_commitment_outputs_signature_idx ON core_commitment_outputs (signature);

CREATE TABLE IF NOT EXISTS core_reserved_ys (
    y TEXT PRIMARY KEY,
    deadline TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS core_reserved_ys_deadline_idx ON core_reserved_ys (deadline);

-- treasury-service
CREATE TABLE IF NOT EXISTS treasury_ebill_mint_ops (
    uid     UUID PRIMARY KEY,
    kid     TEXT NOT NULL,
    bill_id TEXT NOT NULL,
    minted  BIGINT NOT NULL CHECK (minted >= 0),
    blob    JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS treasury_ebill_mint_ops_kid_idx ON treasury_ebill_mint_ops (kid);
CREATE UNIQUE INDEX treasury_ebill_mint_ops_bill_id_idx ON treasury_ebill_mint_ops (bill_id);

CREATE TABLE IF NOT EXISTS treasury_onchain_mint_ops (
    qid UUID PRIMARY KEY,
    expiry TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL,
    blob JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS treasury_onchain_melt_ops (
    qid UUID PRIMARY KEY,
    expiry TIMESTAMPTZ NOT NULL,
    input_ys TEXT[] NOT NULL,
    status TEXT NOT NULL,
    blob JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS treasury_onchain_denied_melt_ops (
    qid UUID PRIMARY KEY,
    blob JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS treasury_vault_proofs (
    y    TEXT PRIMARY KEY,
    blob JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS treasury_foreign_proofs (
    mint_id TEXT PRIMARY KEY,
    blobs   TEXT[] NOT NULL
);

CREATE TABLE IF NOT EXISTS treasury_foreign_online_htlc_proofs (
    y       TEXT PRIMARY KEY,
    hash    TEXT NOT NULL,
    mint_id TEXT NOT NULL,
    blob    JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS treasury_foreign_online_htlc_proofs_hash_idx
    ON treasury_foreign_online_htlc_proofs (hash);

-- quote-service
CREATE TABLE IF NOT EXISTS quote_quotes (
    qid              UUID PRIMARY KEY,
    status           TEXT NOT NULL,
    submitted        TIMESTAMPTZ NOT NULL,
    maturity_date    DATE NOT NULL,
    bill_id          TEXT NOT NULL,
    bill_sum         BIGINT NOT NULL CHECK (bill_sum > 0),
    bill_drawee_id   TEXT NOT NULL,
    bill_drawer_id   TEXT NOT NULL,
    bill_payer_id    TEXT NOT NULL,
    bill_holder_id   TEXT NOT NULL,
    blob             JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS quote_quotes_status_submitted_idx
     ON quote_quotes (status, submitted DESC);
 CREATE INDEX IF NOT EXISTS quote_quotes_bill_id_holder_submitted_idx
     ON quote_quotes (bill_id, bill_holder_id, submitted DESC);
 CREATE INDEX IF NOT EXISTS quote_quotes_maturity_date_idx
     ON quote_quotes (maturity_date);
