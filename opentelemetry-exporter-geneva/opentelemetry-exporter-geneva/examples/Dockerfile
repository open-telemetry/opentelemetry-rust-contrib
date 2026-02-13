# Dockerfile for Geneva Uploader Workload Identity Test
  # 
  # This Dockerfile must be built from the repository root to access the workspace:
  #   cd /path/to/opentelemetry-rust-contrib
  #   docker build -f opentelemetry-exporter-geneva/opentelemetry-exporter-geneva/examples/Dockerfile -t geneva-uploader-test:latest .
  #
  # Or using ACR:
  #   az acr build --registry <registry-name> --image geneva-uploader-test:latest \
  #     --file opentelemetry-exporter-geneva/opentelemetry-exporter-geneva/examples/Dockerfile .

  FROM rust:1.85-slim AS builder

  # Install build dependencies
  RUN apt-get update && apt-get install -y \
      pkg-config \
      libssl-dev \
      && rm -rf /var/lib/apt/lists/*

  WORKDIR /app

  # Copy the entire workspace from repository root
  COPY . .

  # Build the example
  WORKDIR /app/opentelemetry-exporter-geneva/opentelemetry-exporter-geneva
  RUN cargo build --release --example basic_workload_identity_test

  # Runtime stage
  FROM debian:bookworm-slim

  # Install runtime dependencies
  RUN apt-get update && apt-get install -y \
      ca-certificates \
      libssl3 \
      && rm -rf /var/lib/apt/lists/*

  # Copy the binary
  COPY --from=builder /app/target/release/examples/basic_workload_identity_test /usr/local/bin/geneva-uploader-test

  # Run as non-root user
  RUN useradd -m -u 1000 appuser
  USER appuser

  ENTRYPOINT ["/usr/local/bin/geneva-uploader-test"]

