CREATE TABLE IF NOT EXISTS core_swaps (
    signature TEXT PRIMARY KEY,
    blob JSONB NOT NULL
);
