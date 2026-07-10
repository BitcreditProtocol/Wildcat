CREATE TABLE keys (
    kid TEXT PRIMARY KEY,
    unit TEXT NOT NULL,
    active BOOLEAN NOT NULL,
    final_expiry BIGINT,
    blob JSONB NOT NULL
);

CREATE INDEX keys_unit_idx ON keys (unit);
CREATE INDEX keys_final_expiry_idx ON keys (final_expiry);
