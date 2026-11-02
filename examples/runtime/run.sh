#!/usr/bin/env bash
# PACT runtime 黄金路径：一条命令验收「声明 → daemon → 多 agent 自驱 → Rule-T → 离线重放」。
#
# 验收内容（全部硬断言，任一失败即非零退出）：
#   1. `pact analyze`：exit 0，0 违规 / 0 警告，五项咨询（不可达、无 producer、
#      无 consumer、结构死锁、滞留风险）全 0；
#   2. `pact domain load` + `pact seed`（CLI 在线路径）成功，status 翻转为未终止；
#   3. `pact-runtime` exit 0：四个 agent 全部 saw_terminated / notified_terminated，
#      各角色 产/消/读 次数与角色表一致（seeder 0/0/0、worker 1/1/1、finisher 1/1/1、sink 0/1/0）；
#   4. 终止后 status：counter=0 archive=3 terminated=true；事件日志 8 行
#      （DomainLoaded + Seeded + 2×Produced + 3×Consumed + Terminated）；
#   5. `pact replay` 离线重放同一日志：counter/archive 与在线 status 一致。
#
# 只断言稳定量（计数、终止位、角色动作次数、日志行数）；ULID、时间戳、通知条数不断言。
#
# 用法：
#   examples/runtime/run.sh           # 跑验收
#   examples/runtime/run.sh --keep    # 保留临时目录（便于失败排查）
#
# 依赖：target/debug/pact 与 target/debug/pact-runtime
#       （cargo build -p pact-cli -p pact-runtime），curl（/ready 轮询），
#       jq 或 python3（JSON 断言，任选其一，优先 jq）。
#       可用 PACT=/path/to/pact、PACT_RUNTIME=/path/to/pact-runtime 覆盖。
# 清理：临时目录由 mktemp -d 创建；trap 覆盖一切退出路径，停 daemon 并删目录（--keep 除外）。

set -uo pipefail

KEEP=0
for a in "$@"; do
    case "$a" in
        --keep) KEEP=1 ;;
        -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "未知参数：$a（仅支持 --keep）" >&2; exit 2 ;;
    esac
done

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
PACT=${PACT:-$REPO/target/debug/pact}
PACT_RUNTIME=${PACT_RUNTIME:-$REPO/target/debug/pact-runtime}

[ -x "$PACT" ] || {
    echo "找不到可执行的 pact：$PACT" >&2
    echo "先构建：cargo build -p pact-cli，或用 PACT=/path/to/pact 指定。" >&2
    exit 1
}
[ -x "$PACT_RUNTIME" ] || {
    echo "找不到可执行的 pact-runtime：$PACT_RUNTIME" >&2
    echo "先构建：cargo build -p pact-runtime，或用 PACT_RUNTIME=/path/to/pact-runtime 指定。" >&2
    exit 1
}
command -v curl >/dev/null 2>&1 || {
    echo "缺少依赖 curl：daemon 就绪轮询（GET /ready over unix socket）需要它。" >&2
    exit 1
}
if command -v jq >/dev/null 2>&1; then
    JSON_TOOL=jq
elif command -v python3 >/dev/null 2>&1; then
    JSON_TOOL=py
else
    echo "缺少 JSON 工具：断言需要 jq 或 python3（任选其一），请安装后重跑。" >&2
    exit 1
fi

# --- JSON 取值原语：优先 jq，缺 jq 时用 python3 等价实现（输出口径一致） ---
PY_HELPER='
import json, os, sys
d = json.loads(os.environ["JSONQ_PAYLOAD"])
op, args = sys.argv[1], sys.argv[2:]
def dig(path):
    x = d
    for k in path.split("."):
        x = x[k]
    return x
def scalar(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    return str(v)
def agent(name):
    return next(x for x in d["agents"] if x["participant"] == name)
if op == "get":
    print(scalar(dig(args[0])))
elif op == "len":
    print(len(dig(args[0])))
elif op == "agent":
    print(scalar(agent(args[0])[args[1]]))
elif op == "counts":
    a = agent(args[0])
    print("%s/%s/%s" % (a["productions"], a["consumptions"], a["reads"]))
'
jsonq() { # <op> <json> <args...>
    local op=$1 json=$2
    shift 2
    JSONQ_PAYLOAD=$json python3 -c "$PY_HELPER" "$op" "$@"
}
jget() { # <dotted.path> <json>
    if [ "$JSON_TOOL" = jq ]; then jq -r ".$1" <<<"$2"; else jsonq get "$2" "$1"; fi
}
jlen() { # <dotted.path> <json>：数组/对象长度
    if [ "$JSON_TOOL" = jq ]; then jq -r ".$1 | length" <<<"$2"; else jsonq len "$2" "$1"; fi
}
jagent() { # <participant> <field> <json>
    if [ "$JSON_TOOL" = jq ]; then
        jq -r ".agents[] | select(.participant==\"$1\") | .$2" <<<"$3"
    else
        jsonq agent "$3" "$1" "$2"
    fi
}
jcounts() { # <participant> <json>：产/消/读
    if [ "$JSON_TOOL" = jq ]; then
        jq -r ".agents[] | select(.participant==\"$1\") | \"\(.productions)/\(.consumptions)/\(.reads)\"" <<<"$2"
    else
        jsonq counts "$2" "$1"
    fi
}

TMP=$(mktemp -d /tmp/pact-golden.XXXXXX)
SOCK=$TMP/pact.sock
DAEMON_PID=

cleanup() {
    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null
        wait "$DAEMON_PID" 2>/dev/null
    fi
    if [ "$KEEP" = 1 ]; then
        echo "现场保留：$TMP"
    else
        rm -rf "$TMP"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

PASS=0
FAIL=0
assert_eq() { # <描述> <期望> <实际>
    if [ "$2" = "$3" ]; then
        PASS=$((PASS + 1))
        printf 'PASS %02d | %s | 实际: %s\n' "$PASS" "$1" "$3"
    else
        FAIL=$((FAIL + 1))
        printf 'FAIL %02d | %s | 期望: %s | 实际: %s\n' "$FAIL" "$1" "$2" "$3"
    fi
}

START=$SECONDS
echo "# PACT runtime 黄金路径验收"
echo "# pact=$PACT"
echo "# pact-runtime=$PACT_RUNTIME"
echo "# JSON 工具：$JSON_TOOL"
echo "# 临时目录：$TMP（--keep 可保留）"

# ---------------------------------------------------------------------------
echo "== 0. 准备：复制资产并启动 daemon"
cp "$HERE/linear.pact" "$HERE/runtime.toml" "$TMP/"
cat >"$TMP/pact.toml" <<EOF
data_dir = "$TMP/data"
listen_unix = "$SOCK"
EOF
"$PACT" daemon start --config "$TMP/pact.toml" >"$TMP/daemon.log" 2>&1 &
DAEMON_PID=$!

READY=0
for _ in $(seq 1 100); do
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        echo "daemon 提前退出，日志如下：" >&2
        cat "$TMP/daemon.log" >&2
        exit 1
    fi
    if curl -sf --unix-socket "$SOCK" http://localhost/ready >/dev/null 2>&1; then
        READY=1
        break
    fi
    sleep 0.1
done
if [ "$READY" != 1 ]; then
    echo "daemon 10s 内未就绪（GET /ready 未返回 200），日志如下：" >&2
    cat "$TMP/daemon.log" >&2
    exit 1
fi
echo "daemon 就绪（pid=$DAEMON_PID，GET /ready = 200）"

PACT_ONLINE=("$PACT" --unix "$SOCK" --json)

# ---------------------------------------------------------------------------
echo "== 1. 离线分析：pact analyze 应 0 违规 / 0 咨询"
ANALYZE=$("$PACT" --json analyze "$TMP/linear.pact")
RC=$?
assert_eq "analyze 退出码" 0 "$RC"
assert_eq "analyze 违规数" 0 "$(jlen precondition_wf.violations "$ANALYZE")"
assert_eq "analyze 警告数" 0 "$(jlen precondition_wf.warnings "$ANALYZE")"
assert_eq "analyze 不可达类型数" 0 "$(jlen reachability.unreachable "$ANALYZE")"
assert_eq "analyze 无 producer 类型数" 0 "$(jlen gate_coverage "$ANALYZE")"
assert_eq "analyze 无 consumer 类型数" 0 "$(jlen consumption_reachability "$ANALYZE")"
assert_eq "analyze 结构死锁数" 0 "$(jlen structural_deadlock "$ANALYZE")"
assert_eq "analyze 滞留风险数" 0 "$(jlen stranding_risk "$ANALYZE")"

# ---------------------------------------------------------------------------
echo "== 2. CLI 在线路径：domain load + seed"
LOAD=$("${PACT_ONLINE[@]}" domain load "$TMP/linear.pact" --name linear)
assert_eq "domain load 域名" "linear" "$(jget name "$LOAD")"
assert_eq "domain load 类型数" 3 "$(jget types "$LOAD")"

SEED=$("${PACT_ONLINE[@]}" seed --domain linear --participant seeder --type request --thing-ref REQ-GOLDEN-1)
assert_eq "seed 产出 token 类型" "request" "$(jget token.type "$SEED")"

STATUS1=$("${PACT_ONLINE[@]}" status --domain linear)
assert_eq "seed 后 counter" 1 "$(jget counter_size "$STATUS1")"
assert_eq "seed 后 archive" 0 "$(jget archive_size "$STATUS1")"
assert_eq "seed 后 terminated" "false" "$(jget terminated "$STATUS1")"

# ---------------------------------------------------------------------------
echo "== 3. pact-runtime：四 agent 自驱至 Rule-T"
# runtime.toml 的相对路径（declaration / endpoint.unix）相对进程工作目录解析：
# 在临时目录内启动，保证不写仓库也不依赖调用者 cwd。
REPORT=$(cd "$TMP" && "$PACT_RUNTIME" --config runtime.toml 2>"$TMP/runtime.stderr")
RC=$?
assert_eq "pact-runtime 退出码" 0 "$RC"
if [ "$RC" != 0 ]; then
    echo "--- pact-runtime stderr ---"
    cat "$TMP/runtime.stderr"
else
    printf '%s\n' "$REPORT" >"$TMP/runtime-report.json"
    for p in seeder worker finisher sink; do
        assert_eq "$p saw_terminated" "true" "$(jagent "$p" saw_terminated "$REPORT")"
        assert_eq "$p notified_terminated" "true" "$(jagent "$p" notified_terminated "$REPORT")"
    done
    assert_eq "seeder 产/消/读" "0/0/0" "$(jcounts seeder "$REPORT")"
    assert_eq "worker 产/消/读" "1/1/1" "$(jcounts worker "$REPORT")"
    assert_eq "finisher 产/消/读" "1/1/1" "$(jcounts finisher "$REPORT")"
    assert_eq "sink 产/消/读" "0/1/0" "$(jcounts sink "$REPORT")"
fi

# ---------------------------------------------------------------------------
echo "== 4. 终止后在线 status"
STATUS2=$("${PACT_ONLINE[@]}" status --domain linear)
echo "status: $STATUS2"
assert_eq "终止后 counter" 0 "$(jget counter_size "$STATUS2")"
assert_eq "终止后 archive" 3 "$(jget archive_size "$STATUS2")"
assert_eq "终止后 terminated" "true" "$(jget terminated "$STATUS2")"

LOG=$TMP/data/linear/default.events.jsonl
LINES=$(wc -l <"$LOG")
assert_eq "事件日志行数" 8 "$LINES"

# ---------------------------------------------------------------------------
echo "== 5. 离线重放：pact replay 应与在线 status 一致"
REPLAY=$("$PACT" --json replay "$LOG")
assert_eq "replay 后 counter" 0 "$(jlen counter "$REPLAY")"
assert_eq "replay 后 archive" 3 "$(jlen archive "$REPLAY")"

# ---------------------------------------------------------------------------
echo
echo "===== 汇总：PASS=$PASS FAIL=$FAIL（耗时 $((SECONDS - START))s）====="
[ "$FAIL" -gt 0 ] && exit 1
echo "黄金路径验收通过。"
