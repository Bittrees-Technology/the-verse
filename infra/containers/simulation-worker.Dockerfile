# SPDX-License-Identifier: AGPL-3.0-or-later
FROM rust:1.96-bookworm AS builder
WORKDIR /source
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY apps/web-command-center ./apps/web-command-center
COPY content ./content
COPY crates ./crates
COPY services ./services
RUN cargo build --locked --release -p verse-simulation-worker

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 verse \
    && mkdir -p /home/verse/data \
    && chown verse:verse /home/verse/data
COPY --from=builder /source/target/release/verse-simulation-worker /usr/local/bin/
USER verse
WORKDIR /home/verse
VOLUME ["/home/verse/data"]
EXPOSE 7777
ENTRYPOINT ["verse-simulation-worker"]
CMD ["--bind", "0.0.0.0:7777", "--data-directory", "/home/verse/data"]
