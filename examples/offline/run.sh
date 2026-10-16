#!/usr/bin/env bash
# PACT 离线三件套演示：analyze / compose / replay，全程不需要 daemon。
#
# 演示什么
#   1. analyze：四个通用域的六项静态分析，以及三项非零咨询输出的定性
#   2. analyze：三类典型良构违例长什么样（脚本临时生成，不入库）
#   3. compose：片段组合成功（§1.8 C1–C4）与 C2 冲突被机械拦下
#   4. replay：从事件日志离线重建 Counter/Archive
#
# 运行方式（任意工作目录均可）：
#   examples/offline/run.sh                          # 打印
#   examples/offline/run.sh > my-offline.txt 2>&1    # 录制
#
# 前置：已构建二进制（cargo build -p pact-cli），或用 PACT=/path/to/pact 指定。
# 副作用：只在 examples/offline/out/ 下写临时文件，脚本开头会清空该目录。

set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
EXAMPLES=$(cd "$HERE/.." && pwd)
REPO=$(cd "$EXAMPLES/.." && pwd)
PACT=${PACT:-$REPO/target/debug/pact}
OUT=$HERE/out

if [ ! -x "$PACT" ]; then
    echo "找不到可执行的 pact 二进制：$PACT" >&2
    echo "先构建：cargo build -p pact-cli，或用 PACT=/path/to/pact 重跑。" >&2
    exit 1
fi

printf '%s\n' \
    '# ===== PACT 离线三件套演示：analyze / compose / replay =====' \
    '# 轨迹中 `+ pact ...` 是实际执行的命令，`#` 开头是讲解，其余是命令输出。' \
    '# 绝对路径每次运行都不同；分析数字、违例名与退出码应当完全一致。'

step() { printf '\n===== %s =====\n' "$*"; }
note() { printf '# %s\n' "$*"; }
show() { printf '+ pact %s\n' "$*"; "$PACT" "$@"; printf '  -> exit=%d\n' "$?"; }
# --json 输出自带换行，所以这里不再补空行
showj() { printf '+ pact --json analyze %s\n' "$(basename "$1")"; "$PACT" --json analyze "$1"; printf '  -> exit=%d\n' "$?"; }

rm -rf "$OUT"
mkdir -p "$OUT"

# ===========================================================================
step "1. 四个通用域：pact analyze"
note "analyze 是纯函数（pact-analysis），不起进程、不碰存储，退出码 0 = 良构。"

step "1.1 domains/se_lifecycle.pact —— 软件工程通用域（中等粒度）"
show analyze "$EXAMPLES/domains/se_lifecycle.pact"
note "唯一非零项 stranding_risk：deployer 用 via implementation 守护 test_report，"
note "而 tester 有权消费 implementation。这不是错误，是一条真实的顺序约束"
note "（examples/online/run.sh 第 12 步会把它跑成一次真实的 GateConditionFailed）。"

step "1.2 domains/re_pipeline.pact —— 需求工程通用域"
show analyze "$EXAMPLES/domains/re_pipeline.pact"
note "consumption_reachability 列出 baseline / revised_baseline：这两个类型没人消费。"
note "这是刻意设计——基线是常驻配置资产，被反复读取而不被耗尽；"
note "分析器如实报告「无 consumer」，是否接受由建模者裁决，引擎不替你决定。"
note "两个源类型 stakeholder_need / change_request 只能由 seed_authority 注入。"

step "1.3 domains/react_loop.pact —— 前置图带环仍自然终止"
show analyze "$EXAMPLES/domains/react_loop.pact"
note "structural_deadlock 报 0：前置图有环（observation→thought→action→tool_result→"
note "observation）不构成死锁，因为每一步都显式消费前驱，Counter 稳态 ≤ 2 个 token，"
note "user 停止注入 observation 后循环自然排空 ⇒ Rule-T 可达。"
note "唯一的 SEED_PRODUCE_OVERLAP 警告：user 可 seed observation，agent 又能 produce"
note "observation。跨参与者重叠也告警（实现比论文 §1.4.3 的参与者级定义更宽，"
note "取假阳性方向不漏报；见 docs/design.md D21）。"

step "1.4 domains/cross_harness.pact —— 多智能体协作，无编排器"
show analyze "$EXAMPLES/domains/cross_harness.pact"
note "六项分析全 0。三个 harness 互不通信，全靠 Counter 上的 token 状态自驱："
note "harness_a 放下 plan 就撒手，harness_b 看到 plan 才动，谁先谁后由引擎的"
note "Rule-P 可用性决定，不需要中心调度。"

# ===========================================================================
step "2. 三项非零咨询输出的 JSON 详情"
note "非 JSON 输出给人看，--json 输出给机器看（同一份数据，键序固定为附录 A 顺序）。"
printf "+ pact --json analyze domains/se_lifecycle.pact | grep -A8 '\"stranding_risk\"'\n"
"$PACT" --json analyze "$EXAMPLES/domains/se_lifecycle.pact" | grep -A8 '"stranding_risk"'
printf "+ pact --json analyze domains/re_pipeline.pact | grep -A3 '\"consumption_reachability\"'\n"
"$PACT" --json analyze "$EXAMPLES/domains/re_pipeline.pact" | grep -A3 '"consumption_reachability"'
printf "+ pact --json analyze domains/react_loop.pact | grep -A7 '\"warnings\"'\n"
"$PACT" --json analyze "$EXAMPLES/domains/react_loop.pact" | grep -A7 '"warnings"'

# ===========================================================================
step "3. 良构违例长什么样（临时生成，不入库）"
note "analyze 退出码非 0 即表示域声明不可用；--json 逐条给出违例类别。"
note "以下三例都用 --json，为的是看清违例名而不只是计数。"

cat >"$OUT/bad_source_production.pact" <<'EOF'
types { a; b }
preconditions { a → b }
gates { p: produces a, reads b }
seed_authority { q: seeds a }
EOF
step "3.1 源类型被 produce（违反 §3.1 约束 6 Source-Production Constraint）"
showj "$OUT/bad_source_production.pact"

cat >"$OUT/bad_rhs_uniqueness.pact" <<'EOF'
types { a; b; c }
preconditions { a → c; b → c }
gates { p: reads a, reads b, produces c; q: consumes c }
seed_authority { s: seeds a; s: seeds b }
EOF
step "3.2 同一类型有两条前置规则（违反 RHS Uniqueness）"
showj "$OUT/bad_rhs_uniqueness.pact"

cat >"$OUT/bad_type_closure.pact" <<'EOF'
types { a }
preconditions { a → b }
gates { p: reads a, produces b; q: consumes b }
seed_authority { s: seeds a }
EOF
step "3.3 引用未声明的类型（违反 Type Closure）"
showj "$OUT/bad_type_closure.pact"
note "同一个未声明类型 b 被引用了 3 次（preconditions 1 处 + gates 2 处），逐处报告。"

# ===========================================================================
step "4. compose：片段组合（§1.8 C1–C4）"
note "compose 先对每个片段单独做良构校验（含 Type Closure），所以片段必须自足声明"
note "它引用到的全部类型；再做 C1–C4 合并检查，最后对合并结果跑 validate_domain 终检。"

step "4.1 干净组合：se_lifecycle + security_review"
printf '+ pact compose domains/se_lifecycle.pact fragments/security_review.pact > out/se_with_security.pact\n'
"$PACT" compose "$EXAMPLES/domains/se_lifecycle.pact" "$EXAMPLES/fragments/security_review.pact" >"$OUT/se_with_security.pact"
printf '  -> exit=%d\n' "$?"
cat "$OUT/se_with_security.pact"
note "四元组按集合并：types/preconditions/gates/seed_authority 各自取并，"
note "deployer 的宿主条款与片段条款合成同一个参与者的完整 gate 集。"
note "注意序列化把源文件里的 via implementation 规范地打印成 when via implementation。"
step "4.1b 合并结果本身仍是良构域"
show analyze "$OUT/se_with_security.pact"
note "多出 implementation → security_report 一条规则；security_report 有 producer"
note "也有 consumer（deployer），所以六项分析没有新增非零项。"

step "4.2 被拒的组合：se_lifecycle + security_blocking（C2 冲突）"
show compose "$EXAMPLES/domains/se_lifecycle.pact" "$EXAMPLES/fragments/security_blocking.pact"
printf '+ pact --json compose ...（同一命令，取结构化错误）\n'
"$PACT" --json compose "$EXAMPLES/domains/se_lifecycle.pact" "$EXAMPLES/fragments/security_blocking.pact"
note "PreconditionMismatch：宿主说 deployment 的前置是 {test_report}，"
note "片段说是 {test_report, security_report}。共享类型出现两套不同前置 ⇒ C2 拒绝。"
note "这不是工具缺陷而是范式意图：前置图是生产合法性的唯一来源，两个片段对"
note "「什么算完成」理解不一致时必须显式解决，不能静默合并。"
note "正确做法是改宿主声明本身（把 deployment 的规则写成合取），而不是叠加矛盾片段。"

step "4.3 组合后的域可以直接喂给在线流程"
note "compose 的输出就是合法的域声明文本，可直接 domain load（见 online/run.sh 第 3 步）。"

# ===========================================================================
step "5. replay：从事件日志离线重建状态"
FIX=$HERE/fixtures/se_lifecycle.events.jsonl
if [ -f "$FIX" ]; then
    note "素材：fixtures/se_lifecycle.events.jsonl，由 examples/online/run.sh 录制。"
    note "该文件共 12 行事件：1 条 DomainLoaded（重置状态）、1 条 Terminated（信息性）、"
    note "10 条会改变状态的动作事件（1 seed + 4 produce + 5 consume）。"
    show replay "$FIX"
    note "counter=0 archive=5 与在线轨迹第 11 步的 status 完全一致 ——"
    note "JSONL 是真相源，SQLite 索引与快照都只是派生物，删了也能重建。"
    note "崩溃恢复走同一条路径：读最近快照，再重放其后的事件。"
else
    note "缺少 fixtures/se_lifecycle.events.jsonl。先跑 examples/online/run.sh 生成，"
    note "或直接把 replay 指向任意 <data_dir>/<domain>.events.jsonl。"
fi
note "想看新鲜日志：pact replay examples/online/run-data/se_lifecycle.events.jsonl"

printf '\n演示结束。在线全生命周期见 examples/online/run.sh。\n'
