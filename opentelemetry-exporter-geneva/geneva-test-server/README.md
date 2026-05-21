# geneva-test-server

Internal Geneva-like test server for `geneva-uploader` and `opentelemetry-exporter-geneva`.

It provides:

- mock GCS endpoints under `/api/agent/v3/.../MonitoringStorageKeys`
- a Geneva-like ingest endpoint at `/api/v1/ingestion/ingest`
- expiring bearer tokens issued by the mock GCS flow
- asynchronous decode of `centralbond/lz4hc` uploads into SQLite
- debug read APIs for accepted requests and decoded rows

## Run

```sh
cargo run -p geneva-test-server
```

Defaults:

- listen address: `127.0.0.1:18080`
- base URL: `http://127.0.0.1:18080`
- SQLite DB: `target/geneva-test-server.sqlite3`

## Environment

- `GENEVA_TEST_SERVER_ADDR`
- `GENEVA_TEST_SERVER_BASE_URL`
- `GENEVA_TEST_SERVER_DB`
- `GENEVA_TEST_SERVER_TOKEN_TTL_SECS`
- `GENEVA_TEST_SERVER_MAX_BODY_BYTES`
- `GENEVA_TEST_SERVER_MONITORING_ENDPOINT`
- `GENEVA_TEST_SERVER_PRIMARY_MONIKER`
- `GENEVA_TEST_SERVER_ACCOUNT_GROUP`

## Debug APIs

- `GET /healthz`
- `GET /api/v1/debug/requests`
- `GET /api/v1/debug/requests/{request_id}`
- `GET /api/v1/debug/requests/{request_id}/wait?timeout_ms=5000`
- `GET /api/v1/debug/records`

The wait endpoint is intended for automated tests. It returns when the
asynchronous decode worker has marked the request as `decoded` or
`decode_failed`, or when the timeout expires.

## Validation

Run the happy-path integration test with:

```sh
cargo test -p geneva-test-server --features geneva-uploader/mock_auth
```

The test starts the server on an ephemeral local port, sends a real
`geneva-uploader` batch through the mock GCS and ingest endpoints, waits for
decode, and asserts the decoded row payload.

## Notes

- The server validates the issued bearer token, token namespace, expected monitoring endpoint, moniker, format, and body length.
- Upload bodies are stored compressed and, on successful decode, also stored as decoded rows in SQLite.
- The decoder currently targets the Bond schema and row shapes emitted by the current `geneva-uploader` encoder.

## Future Ideas

- Add controlled GCS and ingest failures for retry testing, such as HTTP 429
  with `Retry-After`, HTTP 503, malformed JSON, and delayed responses.
- Add debug summaries for batch shape, including schema count, row count, and
  distinct role or event values per upload.
- Add replay/download endpoints for compressed bodies and decoded central blobs
  to simplify bug reproduction.
- Add negative decode fixtures for truncated LZ4, invalid central blob
  terminators, and unknown schema IDs.
