CREATE INDEX IF NOT EXISTS mint_ops_bill_id_idx ON mint_ops ((blob->'data'->>'bill_id'));
