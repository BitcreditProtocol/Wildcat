CREATE TABLE IF NOT EXISTS onchain_melt_ops (
    qid UUID PRIMARY KEY,
    expiry TIMESTAMPTZ NOT NULL,
    input_ys TEXT[] NOT NULL,
    status TEXT NOT NULL,
    blob JSONB NOT NULL
);
