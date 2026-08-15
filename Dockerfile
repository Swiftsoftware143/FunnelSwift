# ============================================================
# FunnelSwift — PRODUCTION Dockerfile (canonical deploy path)
# ============================================================
# FunnelSwift deploys via BIND-MOUNT (see /opt/swift/docker/funnelswift/docker-compose.yml):
# the container mounts target/release/funnelswift + migrations directly,
# so a fresh build is picked up by `docker restart` (no image rebuild needed).
#
# This Dockerfile exists only to document the minimal runtime image.
#   1. /root/.cargo/bin/cargo build --release   # produces target/release/funnelswift
# ============================================================
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*
RUN groupadd -r funnelswift && useradd -r -g funnelswift funnelswift
WORKDIR /app
COPY funnelswift /app/funnelswift
COPY migrations /app/migrations
RUN chmod +x /app/funnelswift && chown -R funnelswift:funnelswift /app
USER funnelswift
EXPOSE 8080
CMD ["/app/funnelswift"]
