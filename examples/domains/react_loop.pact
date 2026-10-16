// 单 agent ReAct 循环域
//
// 出处：docs/pactv1_rev2.md Part III Chapter 1（逐字对应 §1.1 编码）
//
// 展示范式的一个反直觉性质：前置图可以有环，且仍然自然终止。
//   observation → thought → action → tool_result → observation（回到起点）
// 环不阻碍终止，因为 Rule-P 可满足性取决于 Counter 当前内容，而非图的环状性
// （§1.2 终止性论证）。agent 决定收工时消费掉最后一个 tool_result，Counter 归空，
// Rule-T 触发。
//
// 本域没有 §1.4 意义上的源类型：四个类型各有唯一前驱（Precond(T) ≠ ∅）。
// observation 的首个实例由 user 以 Rule-S 注入（seed 豁免前置条件，§1.4.3），
// 为循环提供启动推力；此后 observation 由 agent 从 tool_result 生产。
//
// 读 CLI 输出时的一个坑：`reachability: 1 源类型` 打印的是 seed_types 的数量
// （持有 SeedAuthority 的类型），不是 Precond(T) = ∅ 的类型数。JSON 键名
// seed_types 是准确的，纯循环域上以 JSON 为准（详见 examples/README.md §5）。
//
// Counter 卫生（§1.3）：稳态下 Counter 从不超过 2 个 token——每轮生产新 token 后
// 立即消费其前驱。长时间运行的 agent 不会累积状态。

types { observation; thought; action; tool_result }

preconditions {
    observation → thought;
    thought → action;
    action → tool_result;
    tool_result → observation
}

gates {
    agent: reads observation, consumes observation, produces thought,
           reads thought, consumes thought, produces action,
           reads action, consumes action, produces tool_result,
           reads tool_result, consumes tool_result, produces observation
}

seed_authority { user: seeds observation }
