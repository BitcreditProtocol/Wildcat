CREATE TABLE IF NOT EXISTS onchain_mint_ops (
    qid UUID PRIMARY KEY,
    expiry TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL,
    blob JSONB NOT NULL
);
