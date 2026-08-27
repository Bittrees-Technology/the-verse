# SPDX-License-Identifier: AGPL-3.0-or-later
FROM rust:1.96-bookworm AS builder
RUN apt-get update \
    && apt-get install --yes --no-install-recommends build-essential clang cmake libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /source
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY apps/web-command-center ./apps/web-command-center
COPY content ./content
COPY crates ./crates
COPY services ./services
RUN cargo build --locked --release -p verse-simulation-worker

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 verse \
    && mkdir -p /home/verse/data \
    && chown verse:verse /home/verse/data
COPY --from=builder /source/target/release/verse-simulation-worker /usr/local/bin/
COPY --from=builder /source/apps/web-command-center/generated /usr/local/share/the-verse/browser-verifier
ENV VERSE_BROWSER_VERIFIER_ASSET_DIR=/usr/local/share/the-verse/browser-verifier
USER verse
WORKDIR /home/verse
VOLUME ["/home/verse/data"]
EXPOSE 7777
ENTRYPOINT ["verse-simulation-worker"]
CMD ["--bind", "0.0.0.0:7777", "--data-directory", "/home/verse/data"]
