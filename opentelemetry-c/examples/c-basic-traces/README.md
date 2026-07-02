# c-basic-traces

A minimal C program that emits two spans (a parent and a child) through the
`opentelemetry-c` SDK using the OTLP HTTP/protobuf exporter.

## What it shows

- Building an SDK with a resource (`service.name` + a custom attribute), an OTLP
  endpoint, and batch span processor options.
- Installing the SDK as the process-global tracer provider.
- Getting a tracer from the global provider.
- Starting a span, setting string/bool/int64/double attributes, adding an event, and
  setting the span status.
- Starting a child span via the `parent` start option.
- Force-flushing and shutting down cleanly.

## Prerequisites

- A Rust toolchain (to build the `opentelemetry-c` library).
- A C compiler (`cc`, `gcc`, or `clang`).
- Optional: an OpenTelemetry Collector (or any OTLP/HTTP endpoint) listening on
  `http://localhost:4318`. Without one, the program still runs and exits cleanly; the
  SDK just logs export errors.

## Build and run

```sh
# From this directory:
make          # builds the Rust library (release) and the example binary
make run      # builds, then runs the example
```

Point the exporter at a different collector with the standard environment variable
(the value must be the full traces URL, used as-is):

```sh
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://localhost:4318/v1/traces make run
```

To only sanity-check that the headers compile (no Rust build, no linking):

```sh
make verify
```

## Running a local collector (optional)

The quickest way to see the spans is the Collector's debug exporter:

```sh
# collector-config.yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: 0.0.0.0:4318
exporters:
  debug:
    verbosity: detailed
service:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [debug]
```

```sh
docker run --rm -p 4318:4318 \
  -v "$(pwd)/collector-config.yaml:/etc/otelcol/config.yaml" \
  otel/opentelemetry-collector:latest
```

Then run `make run` in another terminal and watch the two spans arrive in the
collector's log.

## Linking notes

The Makefile links against the shared library built at
`../../../target/release/libopentelemetry_c.{so,dylib}` and sets an rpath so the binary
finds it at runtime. To link the static library (`libopentelemetry_c.a`) instead,
you must also link the platform's system libraries that the Rust runtime and TLS stack
depend on (e.g. `-lpthread -ldl -lm` on Linux; the `Security` and `CoreFoundation`
frameworks on macOS). See the top-level crate `README.md` for details.
