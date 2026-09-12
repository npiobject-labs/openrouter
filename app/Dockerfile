# --- Etapa de compilacion ---
FROM rust:1-slim-bookworm AS builder

# rusqlite con "bundled" compila SQLite desde C: hace falta un compilador.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

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

RUN useradd --system --create-home --uid 10001 openrouter

# Punto de montaje del volumen de Fly, con dueño antes de dejar de ser root:
# el proceso no corre como root y tiene que poder escribir el SQLite.
# [SUPUESTO] Fly respeta la propiedad del directorio de la imagen al montar el
# volumen. Plan B si no: el backend avisa por el log y sigue en memoria, y
# /v1/estado lo dice con "almacen":"memoria", que es lo que verifica deploy.yml.
RUN mkdir -p /datos && chown openrouter:openrouter /datos
VOLUME ["/datos"]

USER openrouter
WORKDIR /app

COPY --from=builder /src/target/release/openrouter-backend /usr/local/bin/openrouter-backend

ENV PUERTO=8080
EXPOSE 8080

CMD ["/usr/local/bin/openrouter-backend"]
