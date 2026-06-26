CREATE TABLE IF NOT EXISTS mint_ops (
    uid     UUID PRIMARY KEY,
    kid     TEXT NOT NULL,
    minted  BIGINT NOT NULL CHECK (minted >= 0),
    blob    JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS mint_ops_kid_idx ON mint_ops (kid);
