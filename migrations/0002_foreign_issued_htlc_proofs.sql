CREATE TABLE IF NOT EXISTS treasury_foreign_issued_htlc_proofs (
    y        TEXT   PRIMARY KEY,
    hash     TEXT   NOT NULL,
    locktime BIGINT NOT NULL,
    blob     JSONB  NOT NULL
);

CREATE INDEX IF NOT EXISTS treasury_foreign_issued_htlc_proofs_locktime_idx
    ON treasury_foreign_issued_htlc_proofs (locktime);

CREATE INDEX IF NOT EXISTS treasury_foreign_issued_htlc_proofs_hash_idx
    ON treasury_foreign_issued_htlc_proofs (hash);
