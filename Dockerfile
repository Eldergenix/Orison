# syntax=docker/dockerfile:1.7
#
# Orison CLI container image.
#
# Two-stage build:
#   1. `builder` compiles the `ori` binary against the pinned Rust toolchain.
#   2. `runtime` is a minimal Debian slim image that copies just the binary.
#
# The final image is < 200 MB and runs `ori` as its entrypoint, so:
#   docker run --rm orison/ori --help
# behaves identically to a locally installed binary.

# ---- Stage 1: build ---------------------------------------------------------
FROM rust:1.92 AS builder

WORKDIR /src

# Copy the full workspace. Cargo will only rebuild changed crates.
COPY . .

# Reproducible, single-target build of the `ori` package.
RUN cargo build --release -p ori \
    && install -D -m 0755 target/release/ori /out/ori \
    && strip /out/ori || true

# ---- Stage 2: runtime -------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Minimal runtime deps: just CA certs for HTTPS-using subcommands.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/ori /usr/local/bin/ori

# Create an unprivileged user so containers do not run as root by default.
RUN useradd --create-home --shell /bin/bash ori
USER ori
WORKDIR /home/ori

ENTRYPOINT ["ori"]
CMD ["--help"]
