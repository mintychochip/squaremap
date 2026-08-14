CREATE TABLE dirty_retries(
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  x INTEGER NOT NULL,
  z INTEGER NOT NULL,
  attempt INTEGER NOT NULL,
  next_attempt INTEGER NOT NULL,
  PRIMARY KEY(namespace, value, epoch, x, z)
);
UPDATE schema_version SET version=2;
