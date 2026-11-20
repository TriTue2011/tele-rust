# Ảnh DỰNG (không phải ảnh chạy): rust + thư viện C++/zlib để liên kết tĩnh NTgCalls.
FROM rust:1.90-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends g++ zlib1g-dev && rm -rf /var/lib/apt/lists/*
