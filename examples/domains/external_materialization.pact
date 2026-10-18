// 外部物化确认模式（witness-token）
//
// 场景：一段工作成果在被消费/归档之前，必须先由外部系统确认其真实产物已经就位。
// 外部系统的状态无法直接写进 Gate 条件（条件只读 Counter），于是把「确认就绪」
// 表示为一枚 token（readiness），由外部系统经 seed_authority 注入。
//
// 关键点：coordinator 消费 work_item 的准入条件是 exists readiness。
// 外部尚未确认时 readiness 不在 Counter 上，强制消费会被 GateConditionFailed 拦下；
// 确认就位后，exists readiness 与消费 work_item 落在同一次 GateHolds 判定里，
// 于是「确认」与「消耗」共享同一个原子步（见 docs/design.md D23）。
//
// 对照：若让 coordinator 无条件 consumes work_item，就可能出现「已消费但产物未就位」。

types { work_item; readiness; completion }

preconditions {
    work_item → completion
}

gates {
    coordinator: consumes work_item when exists readiness, consumes readiness, produces completion;
    reviewer:    consumes completion
}

seed_authority {
    requester:       seeds work_item;
    external_system: seeds readiness
}
