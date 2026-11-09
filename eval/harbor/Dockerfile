# Builds the LiteCode release binary on Linux, for use as a Terminal-Bench agent.
#
# The build context is a staged source tree (Cargo.toml, Cargo.lock, build.rs,
# src/, sdk/generated/) assembled by build.sh. The image carries no credentials
# and no settings: a task container only ever receives the binary plus a
# litecode.db mounted in at run time.

FROM ubuntu:24.04 AS build

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

# rustup rather than the distro toolchain: the crate is edition 2024 and uses
# library features that the distro rustc does not have.
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
ENV PATH=/usr/local/cargo/bin:${PATH}

WORKDIR /src
# Only what the crate compiles: sdk/generated is include_str!'d by code_contract.
# rust-toolchain.toml pins the exact channel; `rustup show` installs it here.
COPY rust-toolchain.toml Cargo.toml Cargo.lock build.rs ./
RUN rustup show && rustc -V
COPY src ./src
COPY sdk/generated ./sdk/generated
RUN cargo build --release --locked

FROM ubuntu:24.04
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/litecode /usr/local/bin/litecode
ENTRYPOINT ["/usr/local/bin/litecode"]
