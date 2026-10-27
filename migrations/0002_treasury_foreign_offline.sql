-- treasury-service: foreign offline repository
CREATE TABLE IF NOT EXISTS treasury_foreign_offline_fps (
    hash    TEXT  PRIMARY KEY,
    y       TEXT  NOT NULL,
    mint_id TEXT  NOT NULL,
    blob    JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS treasury_foreign_offline_fps_y_idx
    ON treasury_foreign_offline_fps (y);

CREATE TABLE IF NOT EXISTS treasury_foreign_offline_proofs (
    y       TEXT  PRIMARY KEY,
    mint_id TEXT  NOT NULL,
    blob    JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS treasury_foreign_offline_proofs_mint_id_idx
    ON treasury_foreign_offline_proofs (mint_id);

CREATE TABLE IF NOT EXISTS treasury_foreign_offline_redemptions (
    digest BYTEA PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS treasury_foreign_offline_exchanges (
    exchange_digest BYTEA  PRIMARY KEY,
    alpha_id        TEXT   NOT NULL,
    expires_at      BIGINT NOT NULL,
    blob            JSONB  NOT NULL
);

-- the input ys an exchange holds: the primary key on y is the reservation lock
CREATE TABLE IF NOT EXISTS treasury_foreign_offline_exchange_ys (
    y               TEXT  PRIMARY KEY,
    exchange_digest BYTEA NOT NULL
        REFERENCES treasury_foreign_offline_exchanges (exchange_digest) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS treasury_foreign_offline_exchange_ys_digest_idx
    ON treasury_foreign_offline_exchange_ys (exchange_digest);
