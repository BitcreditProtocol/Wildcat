CREATE TABLE IF NOT EXISTS quotes (
    qid              UUID PRIMARY KEY,
    status           TEXT NOT NULL,
    submitted        TIMESTAMPTZ NOT NULL,
    maturity_date    DATE NOT NULL,
    bill_id          TEXT NOT NULL,
    bill_sum         BIGINT NOT NULL CHECK (bill_sum > 0),
    bill_drawee_id   TEXT NOT NULL,
    bill_drawer_id   TEXT NOT NULL,
    bill_payer_id    TEXT NOT NULL,
    bill_holder_id   TEXT NOT NULL,
    blob             JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS quotes_status_submitted_idx
     ON quotes (status, submitted DESC);

 CREATE INDEX IF NOT EXISTS quotes_bill_id_holder_submitted_idx
     ON quotes (bill_id, bill_holder_id, submitted DESC);

 CREATE INDEX IF NOT EXISTS quotes_maturity_date_idx
     ON quotes (maturity_date);
