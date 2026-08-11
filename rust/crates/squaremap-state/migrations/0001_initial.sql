CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
CREATE TABLE worlds(
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  config BLOB NOT NULL,
  PRIMARY KEY(namespace, value)
);
CREATE TABLE dirty_chunks(
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  x INTEGER NOT NULL,
  z INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  PRIMARY KEY(namespace, value, epoch, x, z)
);
CREATE TABLE render_jobs(
  id BLOB PRIMARY KEY,
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  kind INTEGER NOT NULL,
  state INTEGER NOT NULL,
  payload BLOB NOT NULL,
  completed_chunks INTEGER NOT NULL
);
CREATE TABLE session_checkpoints(
  session_id BLOB PRIMARY KEY,
  durable_sequence INTEGER NOT NULL
);
CREATE TABLE legacy_imports(
  relative_path TEXT PRIMARY KEY,
  content_sha256 BLOB NOT NULL,
  imported_at_epoch_seconds INTEGER NOT NULL
);
INSERT INTO schema_version(version) VALUES (1);
