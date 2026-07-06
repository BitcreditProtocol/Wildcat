CREATE TABLE IF NOT EXISTS onchain_denied_melt_ops (
    qid UUID PRIMARY KEY,
    blob JSONB NOT NULL
);
