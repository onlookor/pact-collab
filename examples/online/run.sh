#!/usr/bin/env bash
# PACT 在线轨迹演示：用 se_lifecycle 域把 daemon 从空 Counter 一路驱动到 Rule-T。
#
# 演示什么
#   1. daemon 启动与配置（pact.toml → data_dir / listen_unix）
#   2. 域装载生命周期（validate / load / list / info / status / unload）
#   3. 四类动作（seed / produce / read / consume）与 Gate 的实际拦截效果
#   4. `via` 溯源见证如何把"顺序"变成可机械检查的硬约束（stranding_risk 实点）
#   5. Rule-T 终止判定与 Archive 留痕
#   6. 事件溯源：JSONL 是真相源，`pact replay` 可离线重建同一状态
#
# 运行方式（任意工作目录均可）：
#   examples/online/run.sh                          # 打印轨迹
#   examples/online/run.sh > my-trace.txt 2>&1      # 录制
#
# 前置：已构建二进制（cargo build -p pact-cli），或用 PACT=/path/to/pact 指定。
# 副作用：只在 examples/online/run-data/ 下写文件，脚本开头会清空该目录。
# 退出码：全部演示按预期完成时为 0；被演示的"拒绝"是脚本内容而非失败。

set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
EXAMPLES=$(cd "$HERE/.." && pwd)
REPO=$(cd "$EXAMPLES/.." && pwd)
PACT=${PACT:-$REPO/target/debug/pact}
DOMAIN_FILE=$EXAMPLES/domains/se_lifecycle.pact
RUN_DIR=$HERE/run-data
SOCK=$RUN_DIR/pact.sock
CONF=$RUN_DIR/pact.toml
CONN=(--unix "$SOCK")
D=se_lifecycle

if [ ! -x "$PACT" ]; then
    echo "找不到可执行的 pact 二进制：$PACT" >&2
    echo "先构建：cargo build -p pact-cli，或用 PACT=/path/to/pact 重跑。" >&2
    exit 1
fi

printf '%s\n' \
    '# ===== PACT 在线全生命周期演示 =====' \
    '# 轨迹中 `+ pact ...` 是实际执行的命令，`#` 开头是讲解，其余是命令输出。' \
    '# token id（ULID）、时间戳、daemon pid、绝对路径每次运行都不同；' \
    '# 步骤结构、错误码与退出码应当完全一致。'

step() { printf '\n===== %s =====\n' "$*"; }
note() { printf '# %s\n' "$*"; }
# 执行并回显（结果留在 $REPLY，便于提取 token id）
emit() {
    printf '+ pact %s\n' "${CONN[*]} $*"
    REPLY=$("$PACT" "${CONN[@]}" "$@")
    local rc=$?
    [ -n "$REPLY" ] && printf '%s\n' "$REPLY"
    return $rc
}
# 执行并回显，预期被引擎拒绝
deny() {
    printf '+ pact %s\n' "${CONN[*]} $*"
    local out rc
    out=$("$PACT" "${CONN[@]}" "$@" 2>&1)
    rc=$?
    [ -n "$out" ] && printf '%s\n' "$out"
    printf '  -> exit=%d（预期非 0：引擎拒绝）\n' "$rc"
}
tok() { printf '%s\n' "$1" | awk '/^token /{print $2; exit}'; }

DAEMON_PID=
cleanup() {
    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null
        wait "$DAEMON_PID" 2>/dev/null
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
step "0. 准备运行目录与配置（pact.toml）"
rm -rf "$RUN_DIR"
mkdir -p "$RUN_DIR"
cat >"$CONF" <<EOF
# 由 examples/online/run.sh 生成；缺省值为 data_dir="./data"、listen_unix="./pact.sock"
data_dir = "$RUN_DIR"
listen_unix = "$SOCK"
EOF
printf '+ cat examples/online/run-data/pact.toml\n'
cat "$CONF"

step "1. 启动 daemon"
printf '+ pact daemon start --config examples/online/run-data/pact.toml &\n'
"$PACT" daemon start --config "$CONF" >"$RUN_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 1 50); do
    [ -S "$SOCK" ] && break
    sleep 0.1
done
if [ ! -S "$SOCK" ]; then
    echo "daemon 未在 5s 内创建 socket，日志如下：" >&2
    cat "$RUN_DIR/daemon.log" >&2
    exit 1
fi
note "daemon pid=$DAEMON_PID socket=$SOCK"
note "daemon 是 Counter 的唯一权威；CLI / REST / MCP 三接口最终都汇聚到它"

# ---------------------------------------------------------------------------
step "2. 装载前先离线分析（analyze 不经 daemon）"
printf '+ pact analyze domains/se_lifecycle.pact\n'
"$PACT" analyze "$DOMAIN_FILE"
note "六项分析全过；唯一非零项是 stranding_risk，第 12 步会把它跑成真实拒绝"
printf "+ pact --json analyze domains/se_lifecycle.pact | grep -A8 '\"stranding_risk\"'\n"
"$PACT" --json analyze "$DOMAIN_FILE" | grep -A8 '"stranding_risk"'

step "3. 域装载生命周期"
emit domain validate "$DOMAIN_FILE"
emit domain load "$DOMAIN_FILE"
emit domain list
emit domain info --domain "$D"
emit status --domain "$D"
note "注意 terminated=true：Rule-T 的条件是 Counter=∅ 且无参与者 Rule-P-enabled，"
note "刚装载的域两条都满足，所以空域本身就是终止态。注入 seed 后会翻回 false。"

# ---------------------------------------------------------------------------
step "4. 查看 Gate 与当前可用生产"
emit gates show --domain "$D" --participant architect
emit gates show --domain "$D" --participant deployer
note "deployer 两条子句带 when via implementation：读/消费 test_report 时，"
note "必须有一个 implementation token 仍在 Counter，且它是该 test_report 的直接前驱"
emit enabled --domain "$D" --participant architect
note "Counter 为空 ⇒ 没有任何生产可用（前置找不到见证）"

step "5. 空 Counter 上的拒绝"
deny produce --domain "$D" --participant architect --type design
note "design 的前置是 requirement，Counter 里没有 requirement ⇒ Rule-P 不可用。"
note "顺带说明：produce 必须用 --predecessors 显式给出见证，引擎不会替你猜"
note "（所以上面的报错里 token id 是空的）。多个前驱用逗号分隔。"
deny seed --domain "$D" --participant architect --type requirement
note "SeedAuthority 只授予 external_stakeholder ⇒ Rule-S 拒绝越权注入"

# ---------------------------------------------------------------------------
step "6. 注入 seed（Rule-S，豁免前置但受 SeedAuthority 约束）"
emit seed --domain "$D" --participant external_stakeholder --type requirement --thing-ref REQ-001
REQ=$(tok "$REPLY")
emit counter ls --domain "$D"
emit status --domain "$D"
note "seed 之后 terminated 翻回 false —— Counter 非空"
emit enabled --domain "$D" --participant architect
note "witnesses=[$REQ] —— 引擎直接告诉 architect 该用哪个前驱"

step "7. 身份与动作权限的拒绝"
deny consume --domain "$D" --participant reviewer --token "$REQ"
note "reviewer 只有 reads 子句，没有 consumes requirement ⇒ Rule-E 拒绝"
deny produce --domain "$D" --participant ghost --type design --predecessors "$REQ"
note "ghost 不在域的参与者集合里"

# ---------------------------------------------------------------------------
step "8. 主链路：requirement → design → implementation → test_report → deployment"
note "关键顺序：Rule-P 要求前驱**仍在 Counter 上**，所以必须 produce 之后再 consume。"
note "反过来做会得到 PredecessorNotOnCounter —— 消费是不可逆的（token 入 Archive）。"
emit read --domain "$D" --participant architect --token "$REQ"
emit produce --domain "$D" --participant architect --type design --predecessors "$REQ" --thing-ref DES-001
DES=$(tok "$REPLY")
note "Rule-P 不消费前驱：$REQ 此刻仍在 Counter 上，design 只是记录了 depends_on=[$REQ]"
emit consume --domain "$D" --participant architect --token "$REQ"
emit archive ls --domain "$D"
note "入 Archive 是显式 consume 的结果，不是 produce 的副作用（无级联消费）"

emit read --domain "$D" --participant developer --token "$DES"
emit produce --domain "$D" --participant developer --type implementation --predecessors "$DES" --thing-ref IMP-001
IMP=$(tok "$REPLY")
emit consume --domain "$D" --participant developer --token "$DES"

emit read --domain "$D" --participant tester --token "$IMP"
emit produce --domain "$D" --participant tester --type test_report --predecessors "$IMP" --thing-ref TR-001
TR=$(tok "$REPLY")
note "tester 暂不 consume implementation —— 它还要充当 deployer 的 via 见证"
emit counter ls --domain "$D"

step "9. 只读参与者与带 via 的消费"
deny read --domain "$D" --participant reviewer --token "$DES"
note "预期失败：design 已被 developer 消费入 Archive，Rule-R 只对 Counter 上的 token 成立"
emit read --domain "$D" --participant qa_lead --token "$IMP"
emit read --domain "$D" --participant qa_lead --token "$TR"
emit read --domain "$D" --participant deployer --token "$TR"
note "via implementation 成立：$IMP 在 Counter 里，且 $TR.depends_on 含 $IMP"
emit produce --domain "$D" --participant deployer --type deployment --predecessors "$TR" --thing-ref DEP-001
DEP=$(tok "$REPLY")
note "同样先 produce 后 consume：deployment 的前驱是 $TR，$TR 一旦被消费就不能再当见证"
emit consume --domain "$D" --participant deployer --token "$TR"

step "10. 清空 Counter"
emit consume --domain "$D" --participant tester --token "$IMP"
emit consume --domain "$D" --participant stakeholder --token "$DEP"
emit counter ls --domain "$D"

# ---------------------------------------------------------------------------
step "11. Rule-T：终止判定"
emit status --domain "$D"
note "terminated=true —— Counter=∅ 且没有任何参与者 Rule-P-enabled"
emit archive ls --domain "$D"
note "Archive 保留每个 token 的 exhausted_by / exhausted_at，这是审计线索"
deny produce --domain "$D" --participant architect --type design
note "拒绝理由是 PredecessorNotOnCounter：Counter 已空，找不到 design 的前驱见证。"
note "引擎里没有一条独立的「已终止」规则——终止态就是 Counter=∅ 且无 Rule-P-enabled"
note "的推论，所以终止后自然做不出任何生产。"

# ---------------------------------------------------------------------------
step "12. 搁浅陷阱：把第 8 步的顺序换一下会怎样"
D2=se_lifecycle_trap
emit domain load "$DOMAIN_FILE" --name "$D2"
emit seed --domain "$D2" --participant external_stakeholder --type requirement --thing-ref REQ-901
R=$(tok "$REPLY")
emit produce --domain "$D2" --participant architect --type design --predecessors "$R" --thing-ref DES-901
DD=$(tok "$REPLY")
emit consume --domain "$D2" --participant architect --token "$R"
emit produce --domain "$D2" --participant developer --type implementation --predecessors "$DD" --thing-ref IMP-901
II=$(tok "$REPLY")
emit consume --domain "$D2" --participant developer --token "$DD"
emit produce --domain "$D2" --participant tester --type test_report --predecessors "$II" --thing-ref TR-901
TT=$(tok "$REPLY")
note "这一次 tester 先消费 implementation（第 8 步刻意没这么做）"
emit consume --domain "$D2" --participant tester --token "$II"
deny read --domain "$D2" --participant deployer --token "$TT"
note "via implementation 的见证已消失 ⇒ deployer 连读 test_report 都不行"
note "这正是 analyze 报的那条 stranding_risk："
note "  ty_b=test_report ty_a=implementation via_holder=deployer consumer=tester potential=false"
note "potential=false 表示无环拓扑下的确定性报告（有环时为保守的 POTENTIAL_STRANDING）"
emit status --domain "$D2"
note "域卡在 terminated=false，Counter 里剩一个谁也消费不掉的 test_report"
note "修法（任选其一，都是改域声明而非改引擎）："
note "  a) tester 不 consume implementation（让见证活得比 test_report 久）"
note "  b) deployer 的 via 换成生命周期更长的见证类型"
note "  c) 去掉 via，接受更弱的溯源保证"

# ---------------------------------------------------------------------------
step "13. 事件溯源：JSONL 是真相源，replay 可离线重建"
printf '+ ls -1 examples/online/run-data\n'
ls -1 "$RUN_DIR"
note "重放主链路域的完整事件日志（离线命令，不需要 daemon）："
printf '+ pact replay examples/online/run-data/%s.events.jsonl\n' "$D"
"$PACT" replay "$RUN_DIR/$D.events.jsonl"
note "counter=0 archive=5 与第 11 步的 status 一致 —— 状态可从事件日志完全重建"
note "崩溃恢复走的是同一条路径：快照 + 其后的事件重放"

step "14. 卸载域并停止 daemon"
emit domain unload --domain "$D2"
emit domain unload --domain "$D"
emit domain list
kill "$DAEMON_PID" 2>/dev/null
wait "$DAEMON_PID" 2>/dev/null
DAEMON_PID=
note "daemon 已停止；run-data/ 下的事件日志与快照仍在，可用于事后审计"

printf '\n演示结束。离线三件套见 examples/offline/run.sh。\n'
