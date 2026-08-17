-- Schema 4: bridge-owned dirty rows with owner/lease/replay state.
-- The fresh bootstrap chain (0001 + 0002) already creates all four owner
-- columns; this incremental file upgrades only the legacy schema-3 lineage
-- (whose dirty_chunks table lacks the owner columns).
ALTER TABLE dirty_chunks ADD COLUMN owner_bridge_id BLOB NOT NULL;
ALTER TABLE dirty_chunks ADD COLUMN owner_session_id BLOB NOT NULL;
ALTER TABLE dirty_chunks ADD COLUMN lease_expires_epoch_seconds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE dirty_chunks ADD COLUMN replay_pending INTEGER NOT NULL DEFAULT 0;
-- Any legacy row that was not explicitly assigned fails the upgrade atomically:
-- the ADD COLUMNs above roll back with the whole transaction and no partial
-- schema mutation is left behind.
CREATE TABLE _must_abort_if_unassigned(x INTEGER PRIMARY KEY);
INSERT INTO _must_abort_if_unassigned SELECT 1 FROM dirty_chunks
  WHERE owner_bridge_id IS NULL
     OR length(owner_bridge_id) != 16
     OR owner_bridge_id = zeroblob(16);
DROP TABLE _must_abort_if_unassigned;
UPDATE schema_version SET version=4;
