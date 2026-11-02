// PACT runtime 黄金路径域：最小线性流水线（request → work → result）。
//
// 定位：examples/runtime/run.sh 的验收域——无 stranding、无任何咨询项，
// `pact analyze` 判定 0 违规 / 0 警告，五项咨询（可达性、生产覆盖、
// 消费可达、结构死锁、滞留风险）全部为 0。
// 需要演示「分析器能报出确定性风险」的教学场景见
// examples/domains/se_lifecycle.pact（教学反例，勿用于本路径）。
//
// 角色与规则覆盖：
//   seeder   —— seed_authority（Rule-S 唯一种子入口）+ reads request（只读位）
//   worker   —— Rule-R/P/E：读 request → 产 work → 消费 request
//   finisher —— Rule-R/P/E：读 work → 产 result → 消费 work
//   sink     —— Rule-E：终点消费 result；Counter 清空后 Rule-T 成立
// 全部 agent 经 SSE 推送（Rule-W）被唤醒并收到 Terminated 通知。

types { request; work; result }

preconditions {
    request → work;
    work → result
}

gates {
    seeder:   reads request;
    worker:   reads request, consumes request, produces work;
    finisher: reads work, consumes work, produces result;
    sink:     consumes result
}

seed_authority { seeder: seeds request }
