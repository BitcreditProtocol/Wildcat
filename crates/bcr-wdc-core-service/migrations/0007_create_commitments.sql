CREATE TABLE commitments (
    signature TEXT PRIMARY KEY,
    expiration TIMESTAMPTZ NOT NULL,
    blob JSONB NOT NULL
);

CREATE TABLE commitment_inputs (
    y TEXT PRIMARY KEY,
    signature TEXT NOT NULL REFERENCES commitments(signature) ON DELETE CASCADE
);

CREATE TABLE commitment_outputs (
    y TEXT PRIMARY KEY,
    signature TEXT NOT NULL REFERENCES commitments(signature) ON DELETE CASCADE
);

CREATE INDEX commitments_expiration_idx ON commitments (expiration);
CREATE INDEX commitment_inputs_signature_idx ON commitment_inputs (signature);
CREATE INDEX commitment_outputs_signature_idx ON commitment_outputs (signature);
