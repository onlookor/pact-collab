# syntax=docker/dockerfile:1
#
# PACT daemon 容器镜像（D26）。
#
# 构建（在仓库根目录执行；BuildKit 会读取 dist/Dockerfile.dockerignore）：
#   docker build -f dist/Dockerfile -t pact:0.1.0 .
#
# 运行（务必只绑定回环；v1 无 TLS）：
#   docker run --rm -p 127.0.0.1:8787:8787 -v pact-data:/var/lib/pact pact:0.1.0
#
# 自定义配置：
#   docker run --rm -p 127.0.0.1:8787:8787 \
#     -v "$PWD/pact.toml:/etc/pact/pact.toml:ro" -v pact-data:/var/lib/pact pact:0.1.0

ARG RUST_VERSION=1.98

# —— 构建阶段：musl 静态链接（x86_64-unknown-linux-musl） ——
FROM rust:${RUST_VERSION}-alpine AS build
# turso 的 C 依赖（simsimd）需要 C 工具链；musl 目标默认静态链接。
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked --target x86_64-unknown-linux-musl --bins

# —— 运行阶段：非 root、只读根文件系统可用 ——
FROM alpine:3.22
RUN addgroup -S -g 10001 pact \
    && adduser -S -u 10001 -G pact -h /var/lib/pact -s /sbin/nologin pact \
    && mkdir -p /var/lib/pact /run/pact /etc/pact \
    && chown -R pact:pact /var/lib/pact /run/pact

COPY --from=build /src/target/x86_64-unknown-linux-musl/release/pact /usr/local/bin/pact
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/pactd /usr/local/bin/pactd
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/pact-mcp /usr/local/bin/pact-mcp
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/pact-runtime /usr/local/bin/pact-runtime
COPY dist/pact.container.toml /etc/pact/pact.toml

USER pact
WORKDIR /var/lib/pact
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8787/health >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/pactd"]
CMD ["--config", "/etc/pact/pact.toml"]
