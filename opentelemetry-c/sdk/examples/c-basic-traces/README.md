# c-basic-traces (split API + SDK example)

A C program that links against **both** `libopentelemetry_c_api` and
`libopentelemetry_c_sdk`. It plays the **application** role (build + install + flush +
shutdown the SDK) while emitting spans through the **API only** — exactly as an
instrumentation library would — demonstrating that API-only calls export through the
installed SDK.

```sh
make run    # builds both Rust libs (release), links the example, runs it
```

By default it exports to `http://localhost:4318/v1/traces`; override with
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`. Point it at an OpenTelemetry Collector (or any
OTLP/HTTP endpoint) to see the spans. If nothing is listening, export errors are logged but
the program still exits cleanly.
