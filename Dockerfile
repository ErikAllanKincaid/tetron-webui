# Multi-stage build. The frontend (static/*.html/css/js) is embedded into
# the binary at compile time via include_str! -- the runtime image only
# ever needs the compiled binary itself, nothing else copied in.

FROM rust:1.91-slim AS build
WORKDIR /build
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/tetron-web /usr/local/bin/tetron-web

# tetron-web talks to the daemon over its Unix socket -- mount that socket
# into the container at the same path tetron itself uses, e.g.:
#   docker run -p 127.0.0.1:7870:7870 -v /var/run/tetron:/var/run/tetron tetron-web
EXPOSE 7870
ENTRYPOINT ["/usr/local/bin/tetron-web"]
