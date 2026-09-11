# --- Etapa de compilacion ---
FROM rust:1-slim-bookworm AS builder

WORKDIR /src

# Primero solo el manifiesto, para cachear la compilacion de dependencias.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
# Forzar la recompilacion del binario propio tras restaurar las fuentes reales.
RUN touch src/main.rs && cargo build --release

# --- Etapa de ejecucion ---
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 desdemovil
USER desdemovil
WORKDIR /app

# PLANTILLA: "desdemovil-backend" es el "name" del paquete en app/Cargo.toml.
COPY --from=builder /src/target/release/desdemovil-backend /usr/local/bin/desdemovil-backend

ENV PUERTO=8080
EXPOSE 8080

CMD ["/usr/local/bin/desdemovil-backend"]
