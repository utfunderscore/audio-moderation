FROM rust:1.96-bookworm AS build

RUN rustup target add x86_64-unknown-linux-musl \
    && apt-get update \
    && apt-get install --yes --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl --package audio-processing-lambda

FROM public.ecr.aws/lambda/provided:al2023

COPY --from=build /app/target/x86_64-unknown-linux-musl/release/audio-processing-lambda ${LAMBDA_RUNTIME_DIR}/bootstrap
CMD ["bootstrap"]
