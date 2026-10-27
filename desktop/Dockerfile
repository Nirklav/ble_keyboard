# syntax=docker/dockerfile:1
# Browser build of kb-gui (WebHID), served as static files by nginx on port 80.
# WebHID needs a secure context, so put it behind an HTTPS proxy.
#
# Build from the workspace root:
#   docker build -f desktop/Dockerfile -t kb-gui .

# The output is wasm, so the build stage always runs natively.
FROM --platform=$BUILDPLATFORM rust:1-slim-bookworm AS build

ARG TRUNK_VERSION=0.22.1
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-x86_64-unknown-linux-gnu.tar.gz" \
  | tar -xz -C /usr/local/bin trunk

WORKDIR /src
# rust-toolchain.toml pulls in the wasm32 target, .cargo/config.toml the web_sys_unstable_apis cfg.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY .cargo .cargo
RUN rustup show active-toolchain
COPY protocol protocol
COPY desktop desktop

# trunk fetches the matching wasm-bindgen and wasm-opt itself.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/root/.cache/trunk \
    cd desktop && trunk build --release --locked --dist /dist

FROM nginx:stable-alpine
COPY <<'EOF' /etc/nginx/conf.d/default.conf
server {
  listen 80;
  root /usr/share/nginx/html;

  gzip on;
  gzip_types application/wasm application/javascript text/css;

  # trunk hashes the wasm/js names, index.html must always be fresh.
  location = /index.html {
    add_header Cache-Control "no-cache";
  }
  location / {
    try_files $uri =404;
    add_header Cache-Control "public, max-age=31536000, immutable";
  }
}
EOF
COPY --from=build /dist /usr/share/nginx/html
