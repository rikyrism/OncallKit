# syntax=docker/dockerfile:1.7
# OncallKit API + worker. Multi-stage: build with the Rust toolchain, run on a slim base as a
# non-root user.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY migrations migrations
COPY docs/openapi.yaml docs/openapi.yaml
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin oncallkit && cp target/release/oncallkit /oncallkit

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home oncallkit
COPY --from=build /oncallkit /usr/local/bin/oncallkit
USER 10001
EXPOSE 8080
ENV BIND_ADDR=0.0.0.0:8080
ENTRYPOINT ["oncallkit"]
CMD ["serve"]
