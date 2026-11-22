# syntax=docker/dockerfile:1

FROM rust:1.90-slim-bookworm AS builder

WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        cmake \
        make \
        pkg-config \
        g++ \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY web ./web

RUN if [ -f Cargo.lock ]; then \
        cargo build --release --locked --bin tggw; \
    else \
        cargo build --release --bin tggw; \
    fi \
    && strip /src/target/release/tggw

FROM python:3.12-slim-bookworm AS tdjson

ARG TDJSON_VERSION=1.8.69
ARG TDJSON_SHA256=""

WORKDIR /tmp/tdjson-dl

RUN python3 -m pip download \
        --no-cache-dir \
        --no-deps \
        --only-binary=:all: \
        "tdjson==${TDJSON_VERSION}" \
    && WHL="$(ls -1 tdjson-*.whl | head -n 1)" \
    && test -n "$WHL" \
    && ACTUAL_SHA256="$(sha256sum "$WHL" | awk '{print $1}')" \
    && echo "TDJSON_WHEEL=${WHL} SHA256=${ACTUAL_SHA256}" \
    && if [ -n "$TDJSON_SHA256" ] && [ "$ACTUAL_SHA256" != "$TDJSON_SHA256" ]; then \
        echo "SHA256 không khớp: mong đợi $TDJSON_SHA256, nhận $ACTUAL_SHA256" >&2; \
        exit 1; \
    fi \
    && mkdir -p /opt/tdjson \
    && python3 -m zipfile -e "$WHL" /opt/tdjson \
    && rm -rf /opt/tdjson/*.dist-info /tmp/tdjson-dl \
    && find /opt/tdjson -type d -name '__pycache__' -exec rm -rf {} + \
    && LIB_PATH="$(find /opt/tdjson -type f -name 'libtdjson*.so*' | sort | head -n 1)" \
    && test -n "$LIB_PATH" \
    && ln -sf "$LIB_PATH" /opt/tdjson/libtdjson.so

FROM debian:bookworm-slim AS runtime

ARG APP_VERSION=""
ARG WITH_TTS=1
ARG EDGE_TTS_SPEC="edge-tts>=6.1,<8"

LABEL org.opencontainers.image.source="https://github.com/TriTue2011/tele-rust" \
      org.opencontainers.image.title="tele-rust" \
      org.opencontainers.image.description="Gateway Telegram bằng Rust (nhắn tin + gọi thoại/video P2P) cho Home Assistant và giao diện web" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${APP_VERSION}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        ffmpeg \
        jq \
        libstdc++6 \
        zlib1g \
    && if [ "$WITH_TTS" = "1" ]; then \
        apt-get install -y --no-install-recommends python3 python3-venv \
        && python3 -m venv /opt/tts-venv \
        && /opt/tts-venv/bin/pip install --no-cache-dir "${EDGE_TTS_SPEC}" \
        && ln -sf /opt/tts-venv/bin/edge-tts /usr/local/bin/edge-tts; \
    fi \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system --gid 10001 tggw \
    && useradd --system --uid 10001 --gid 10001 --home-dir /data --shell /usr/sbin/nologin tggw \
    && mkdir -p /data /licenses \
    && chown 10001:10001 /data \
    && chmod 0700 /data

COPY --from=builder /src/target/release/tggw /usr/local/bin/tggw
COPY --from=tdjson /opt/tdjson /opt/tdjson
COPY docker/entrypoint.sh /usr/local/bin/tggw-entrypoint
COPY LICENSE NOTICE.md web/QRCODE-LICENSE.txt /licenses/

RUN chmod 0755 /usr/local/bin/tggw /usr/local/bin/tggw-entrypoint \
    && find /opt/tdjson -type f -name '*.so*' -exec dirname {} \; | sort -u > /etc/ld.so.conf.d/tdjson.conf \
    && ldconfig

ENV GATEWAY_HOST=0.0.0.0 \
    REST_PORT=3334 \
    TGGW_DATA=/data \
    TDJSON_LIB=/opt/tdjson/libtdjson.so

USER tggw:tggw
WORKDIR /data

VOLUME ["/data"]
EXPOSE 3334

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD sh -c 'H="${GATEWAY_HOST:-127.0.0.1}"; [ "$H" = "0.0.0.0" ] && H=127.0.0.1; curl -fsS "http://${H}:${REST_PORT:-3334}/health" >/dev/null' || exit 1

ENTRYPOINT ["/usr/local/bin/tggw-entrypoint"]
CMD ["serve"]
