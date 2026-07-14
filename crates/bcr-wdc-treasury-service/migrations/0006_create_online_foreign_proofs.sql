CREATE TABLE IF NOT EXISTS foreign_proofs (
    mint_id TEXT PRIMARY KEY,
    blobs   TEXT[] NOT NULL
);

CREATE TABLE IF NOT EXISTS foreign_online_htlc_proofs (
    y       TEXT PRIMARY KEY,
    hash    TEXT NOT NULL,
    mint_id TEXT NOT NULL,
    blob    JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS foreign_online_htlc_proofs_hash_idx
    ON foreign_online_htlc_proofs (hash);
