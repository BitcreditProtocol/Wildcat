CREATE TABLE IF NOT EXISTS reserved_ys (
    y TEXT PRIMARY KEY,
    deadline TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS reserved_ys_deadline_idx ON reserved_ys (deadline);
