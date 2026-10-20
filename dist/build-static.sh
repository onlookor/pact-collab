#!/usr/bin/env bash
# PACT musl 静态二进制打包脚本（D26）。
#
# 用法（在任意目录）：
#   dist/build-static.sh
#
# 前置：
#   rustup target add x86_64-unknown-linux-musl
#   musl C 工具链（turso 的 C 依赖需要编译）：
#     Fedora/Aurora: dnf install musl-gcc
#     Debian/Ubuntu: apt install musl-tools
#     Alpine:        apk add musl-dev gcc
#
# 可选环境变量：
#   TARGET=…           目标三元组（缺省 x86_64-unknown-linux-musl）
#   MUSL_CC=…          musl C 编译器（缺省自动探测 musl-gcc / cc -dumpmachine=musl）
#   MUSL_AR=…          归档器（缺省自动探测 <target>-ar / ar）
#   NO_STRIP=1         不裁剪符号
#   SOURCE_DATE_EPOCH  tar 内文件时间戳（缺省 0，便于复现）
#
# 产物：dist/target/pact-<version>-<target>.tar.gz（内含 bin/、systemd unit、
#      配置示例、LICENSE 与 SHA256SUMS）及同名 .sha256。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
VERSION="$(awk -F'"' '/^version = /{print $2; exit}' "$ROOT/Cargo.toml")"
OUT="$ROOT/dist/target"
PKG="pact-${VERSION}-${TARGET}"
STAGE="$OUT/$PKG"
TARBALL="$OUT/$PKG.tar.gz"
BINS=(pact pactd pact-mcp pact-runtime)

die() {
    echo "build-static: $*" >&2
    exit 1
}

command -v cargo >/dev/null 2>&1 || die "找不到 cargo，请先安装 Rust 工具链"
[ -n "$VERSION" ] || die "无法从 Cargo.toml 读取版本号"

# —— 目标工具链检查 ——
if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed | grep -qx "$TARGET"; then
        die "缺少目标 $TARGET：请先执行 rustup target add $TARGET"
    fi
fi

# 目标名 → 环境变量后缀（x86_64-unknown-linux-musl → X86_64_UNKNOWN_LINUX_MUSL）
TARGET_ENV="$(printf '%s' "$TARGET" | tr '[:lower:]-' '[:upper:]_')"
LINKER_VAR="CARGO_TARGET_${TARGET_ENV}_LINKER"
CC_VAR="CC_${TARGET_ENV}"
AR_VAR="AR_${TARGET_ENV}"

MUSL_CC="${MUSL_CC:-}"
if [ -z "$MUSL_CC" ]; then
    if command -v musl-gcc >/dev/null 2>&1; then
        MUSL_CC="musl-gcc"
    elif cc -dumpmachine 2>/dev/null | grep -q musl; then
        MUSL_CC="cc"
    fi
fi

if [ -n "$MUSL_CC" ]; then
    export "${CC_VAR}=${MUSL_CC}"
    if [ -z "${!LINKER_VAR:-}" ]; then
        export "${LINKER_VAR}=${MUSL_CC}"
    fi
    if [ -z "${!AR_VAR:-}" ]; then
        if command -v "${TARGET}-ar" >/dev/null 2>&1; then
            export "${AR_VAR}=${TARGET}-ar"
        elif command -v ar >/dev/null 2>&1; then
            export "${AR_VAR}=ar"
        fi
    fi
else
    echo "build-static: 警告：未找到 musl C 工具链，C 依赖可能编译失败；" \
        "请安装 musl-gcc 或设置 MUSL_CC（见脚本头部前置说明）" >&2
fi

# —— 构建 ——
echo "build-static: 构建 $TARGET（release, lto, locked）"
(cd "$ROOT" && cargo build --release --locked --target "$TARGET" --bins)

# —— 暂存目录 ——
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/lib/systemd/system"
for bin in "${BINS[@]}"; do
    src="$ROOT/target/$TARGET/release/$bin"
    [ -x "$src" ] || die "缺少构建产物 $src"
    cp "$src" "$STAGE/bin/$bin"

    if [ -z "${NO_STRIP:-}" ]; then
        STRIP_BIN=""
        for candidate in "${TARGET}-strip" llvm-strip strip; do
            if command -v "$candidate" >/dev/null 2>&1; then
                STRIP_BIN="$candidate"
                break
            fi
        done
        [ -n "$STRIP_BIN" ] && "$STRIP_BIN" --strip-all "$STAGE/bin/$bin"
    fi

    # 静态链接核验：不得存在动态依赖（musl 可能产出 static-pie，故不查 file 文案）。
    if command -v readelf >/dev/null 2>&1; then
        if readelf -d "$STAGE/bin/$bin" 2>/dev/null | grep -q '(NEEDED)'; then
            die "$bin 不是静态链接（存在 NEEDED 动态依赖）"
        fi
    else
        echo "build-static: 警告：无 readelf，跳过 $bin 的静态链接核验" >&2
    fi
done

cp "$ROOT/dist/pact.toml.example" "$STAGE/pact.toml.example"
cp "$ROOT/dist/pactd.service" "$STAGE/lib/systemd/system/pactd.service"
cp "$ROOT/LICENSE" "$STAGE/LICENSE"

(cd "$STAGE" && sha256sum bin/* >SHA256SUMS)

# —— 打包（GNU tar 复现参数；不支持时退回普通打包） ——
rm -f "$TARBALL" "$TARBALL.sha256"
TAR_FLAGS=(--sort=name --owner=0 --group=0 --numeric-owner
    --mtime="@${SOURCE_DATE_EPOCH:-0}")
if ! tar "${TAR_FLAGS[@]}" -czf "$TARBALL" -C "$OUT" "$PKG"; then
    rm -f "$TARBALL"
    tar -czf "$TARBALL" -C "$OUT" "$PKG"
fi
(cd "$OUT" && sha256sum "$(basename "$TARBALL")" >"$(basename "$TARBALL").sha256")

echo "build-static: 完成"
echo "  目录：$STAGE"
echo "  归档：$TARBALL"
echo "  校验：$(cat "$TARBALL.sha256")"
