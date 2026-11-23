FROM rust:1.99-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations
RUN cargo build --release --bin cadence

FROM debian:bookworm-slim

RUN useradd --create-home --uid 10001 cadence
COPY --from=builder /app/target/release/cadence /usr/local/bin/cadence

USER cadence
WORKDIR /home/cadence
ENV CADENCE_DATABASE=/data/cadence.db
VOLUME ["/data"]
EXPOSE 3000

CMD ["cadence", "serve", "--bind", "0.0.0.0:3000"]
