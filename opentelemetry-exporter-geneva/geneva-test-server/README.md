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
- `GENEVA_TEST_SERVER_MONITORING_ENDPOINT`
- `GENEVA_TEST_SERVER_PRIMARY_MONIKER`
- `GENEVA_TEST_SERVER_ACCOUNT_GROUP`

## Debug APIs

- `GET /healthz`
- `GET /api/v1/debug/requests`
- `GET /api/v1/debug/requests/{request_id}`
- `GET /api/v1/debug/records`

## Notes

- The server validates the issued bearer token, expected monitoring endpoint, moniker, format, and body length.
- Upload bodies are stored compressed and, on successful decode, also stored as decoded rows in SQLite.
- The decoder currently targets the Bond schema and row shapes emitted by the current `geneva-uploader` encoder.
