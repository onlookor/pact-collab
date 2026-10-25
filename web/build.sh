#!/usr/bin/env bash
# 构建单文件 PACT 浏览器页面：pact-wasm + 共享配方 + 各页模板 → index.html
#
#   web/authoring/index.html  域作者工作台（内嵌作者契约）
#   web/sandbox/index.html    域运行沙盒（纯内存运行，不落盘）
#
# 两页共用同一份 wasm 与 web/shared/recipes.js，重新生成请跑本脚本。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HERE="$ROOT/web"

export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.local/share/rustup}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.local/share/cargo-home}"
TOOLCHAIN="$RUSTUP_HOME/toolchains/stable-x86_64-unknown-linux-gnu/bin"
if [ -d "$TOOLCHAIN" ]; then
  export PATH="$TOOLCHAIN:$CARGO_HOME/bin:$PATH"
fi
CARGO="${CARGO:-cargo}"

BINDGEN="${WASM_BINDGEN:-}"
if [ -z "$BINDGEN" ]; then
  for c in wasm-bindgen "$HOME/.local/bin/wasm-bindgen" "$CARGO_HOME/bin/wasm-bindgen"; do
    if command -v "$c" >/dev/null 2>&1; then BINDGEN="$(command -v "$c")"; break; fi
    if [ -x "$c" ]; then BINDGEN="$c"; break; fi
  done
fi
if [ -z "$BINDGEN" ]; then
  echo "wasm-bindgen 未找到。安装方式：cargo install wasm-bindgen-cli --version 0.2.127" >&2
  exit 1
fi

echo "==> cargo build -p pact-wasm --target wasm32-unknown-unknown --release"
"$CARGO" build -p pact-wasm --target wasm32-unknown-unknown --release --manifest-path "$ROOT/Cargo.toml"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> wasm-bindgen --target no-modules"
"$BINDGEN" "$ROOT/target/wasm32-unknown-unknown/release/pact_wasm.wasm" \
  --target no-modules --no-typescript --out-dir "$TMP" --out-name pact_wasm

echo "==> assemble single-file pages"
python3 - "$TMP/pact_wasm.js" "$TMP/pact_wasm_bg.wasm" "$HERE" <<'PY'
import base64, pathlib, re, sys

glue_p, wasm_p, web = sys.argv[1], sys.argv[2], pathlib.Path(sys.argv[3])
glue = pathlib.Path(glue_p).read_text(encoding="utf-8")
wasm_b64 = base64.b64encode(pathlib.Path(wasm_p).read_bytes()).decode("ascii")
recipes = (web / "shared" / "recipes.js").read_text(encoding="utf-8")
contract = (web.parent / "docs" / "authoring-contract.md").read_text(encoding="utf-8")

for name, text in (("wasm glue", glue), ("recipes", recipes), ("authoring contract", contract)):
    if "</script" in text.lower():
        sys.exit(f"error: {name} contains a closing script tag; cannot inline safely")

pages = {
    "authoring": {
        "__RECIPES_JS__": recipes,
        "__WASM_GLUE__": glue,
        "__WASM_BASE64__": wasm_b64,
        "__CONTRACT_TEXT__": contract,
    },
    "sandbox": {
        "__RECIPES_JS__": recipes,
        "__WASM_GLUE__": glue,
        "__WASM_BASE64__": wasm_b64,
    },
    # 接入向导是纯静态生成器：不需要 wasm 与契约，占位符为空表。
    "connect": {},
}

for page, values in pages.items():
    tpl_p = web / page / "template.html"
    out_p = web / page / "index.html"
    tpl = tpl_p.read_text(encoding="utf-8")
    for ph in values:
        if ph not in tpl:
            sys.exit(f"error: {tpl_p} is missing placeholder {ph}")
    out = tpl
    for ph, text in values.items():
        out = out.replace(ph, text)
    leftover = re.findall(r"__[A-Z][A-Z0-9_]*__", out)
    if leftover:
        sys.exit(f"error: {tpl_p} has unresolved placeholders: {sorted(set(leftover))}")
    out_p.write_text(out, encoding="utf-8")
    print(f"    wrote {out_p} ({out_p.stat().st_size / 1024:.0f} KiB)")
PY

echo "==> done: $HERE/authoring/index.html, $HERE/sandbox/index.html, $HERE/connect/index.html"
