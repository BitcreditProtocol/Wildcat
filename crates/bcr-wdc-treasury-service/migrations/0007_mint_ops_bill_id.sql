ALTER TABLE mint_ops ADD COLUMN bill_id TEXT;
UPDATE mint_ops SET bill_id = blob->'data'->>'bill_id';
ALTER TABLE mint_ops ALTER COLUMN bill_id SET NOT NULL;
CREATE UNIQUE INDEX mint_ops_bill_id_idx ON mint_ops (bill_id);
