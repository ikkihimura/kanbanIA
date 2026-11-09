-- Recibos de idempotencia: un request_id por actor, ligado a operación + hash del payload.
-- Las migraciones se aplican en transacción; no usar `-- no-transaction`.
CREATE TABLE mutation_receipts (
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (actor, request_id)
) STRICT, WITHOUT ROWID;
