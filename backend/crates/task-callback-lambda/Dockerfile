FROM rust:1.96-bookworm AS build

RUN rustup target add x86_64-unknown-linux-musl \
    && apt-get update \
    && apt-get install --yes --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*

COPY backend ./backend
COPY proto ./proto
COPY migrations ./migrations
WORKDIR /app/backend
RUN cargo build --release --target x86_64-unknown-linux-musl --package task-callback-lambda

FROM public.ecr.aws/lambda/provided:al2023

COPY --from=build /app/backend/target/x86_64-unknown-linux-musl/release/task-callback-lambda ${LAMBDA_RUNTIME_DIR}/bootstrap
CMD ["bootstrap"]
