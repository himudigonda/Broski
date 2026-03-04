FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY backend/target/release/showcase-server /app/showcase-server
COPY frontend/dist /app/static

ENV STATIC_DIR=/app/static
ENV PORT=8080

EXPOSE 8080

ENTRYPOINT ["/app/showcase-server"]
