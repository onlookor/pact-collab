#!/usr/bin/env bash
# 任务即制品示例：离线静态分析，不需要 daemon。
#
# 运行方式（任意工作目录均可）：
#   examples/task-as-artifact/run.sh                         # 打印
#   examples/task-as-artifact/run.sh > trace.txt 2>&1        # 录制
#
# 前置：已构建二进制（cargo build -p pact-cli），或用 PACT=/path/to/pact 指定。
# 说明：这里只跑 analyze；seed/produce/read/consume 需要 daemon，
#       命令示例见 README.md「在线动作序列」。

set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
EXAMPLES=$(cd "$HERE/.." && pwd)
REPO=$(cd "$EXAMPLES/.." && pwd)
PACT=${PACT:-$REPO/target/debug/pact}

if [ ! -x "$PACT" ]; then
    echo "找不到可执行的 pact 二进制：$PACT" >&2
    echo "先构建：cargo build -p pact-cli，或用 PACT=/path/to/pact 重跑。" >&2
    exit 1
fi

printf '%s\n' \
    '# ===== 任务即制品示例：离线 analyze =====' \
    '# 轨迹中 `+ pact ...` 是实际执行的命令，`#` 开头是讲解，其余是命令输出。' \
    '# 分析数字与退出码应当完全一致；本示例不涉及 token id / 时间戳。'

run() {
    printf '\n+ pact analyze %s\n' "$1"
    "$PACT" analyze "$HERE/$1"
    printf '  -> exit=%d\n' "$?"
}

printf '\n# 用法一：任务状态作为类型，任务记录作为物引\n'
run task_board.pact
printf '%s\n' \
    '# stranding_risk=1 是刻意的：archivist 可以消费 task_doing，' \
    '# 而 releaser 的 via 见证要求同一枚 task_doing 仍在 Counter 上。' \
    '# 这是真实的顺序约束，不是缺陷（专题 §3.1、§7）。'

printf '\n# 用法二：任务产出作为类型（母本 §4.1 的构造编码）\n'
run task_dag.pact
printf '%s\n' \
    '# 六项全 0：拓扑固定、每个类型都有 producer 与 consumer。' \
    '# 但「六项全 0」不等于「任务图执行器」——恰好一次与同任务配对仍不在范式内（专题 §6）。'
