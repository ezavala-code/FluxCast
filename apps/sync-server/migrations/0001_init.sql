CREATE TABLE IF NOT EXISTS sync_devices (
    device_id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    device_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sync_devices_user ON sync_devices (user_id);

CREATE TABLE IF NOT EXISTS pairing_tokens (
    token TEXT PRIMARY KEY,
    user_id UUID NOT NULL,
    expires_at BIGINT NOT NULL,
    consumed BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS sync_blobs (
    seq BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL,
    entity_id UUID NOT NULL,
    entity_kind SMALLINT NOT NULL,
    device_id UUID NOT NULL,
    hlc_timestamp TEXT NOT NULL,
    dedup_hmac BYTEA,
    nonce BYTEA NOT NULL,
    ciphertext BYTEA NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_sync_blobs_user_seq ON sync_blobs (user_id, seq);
