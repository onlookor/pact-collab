// 需求工程通用域：需求获取到基线变更
//
// 与任何具体需求方法论、技能、模板无关：只编码需求工程普遍成立的两条结构命题
//   (1) 什么必须先于什么存在（前置图）
//   (2) 谁有权读 / 消费 / 生产什么（gate 矩阵）
//
// 两个源类型（无前置规则 ⇒ 只能 seed，不可 produce）：
//   stakeholder_need  干系人原始诉求，来自系统外部
//   change_request    变更请求，同样来自系统外部
// 这是 PACT 对"外部输入"的统一处理：真实世界的工作确实存在，但在范式内它以
// 注入（Rule-S）而非内部生产（Rule-P）进场。
//
// 持久资产：baseline 与 revised_baseline 刻意不被任何参与者消费。
// 基线是常驻配置资产，消费即归档会使其退出 Counter，后续变更将失去前驱。
// 代价：pact analyze 的 consumption_reachability 会把这两项列为"可能永不被消费"。
// 该分析是咨询性的，此处属预期结果，分类理由见 examples/README.md §4。

types { stakeholder_need; specification; validation_report; baseline;
        change_request; impact_assessment; revised_baseline }

preconditions {
    stakeholder_need → specification;
    specification → validation_report;
    { specification + validation_report } → baseline;
    { change_request + baseline } → impact_assessment;
    { baseline + impact_assessment } → revised_baseline
}

gates {
    analyst: reads stakeholder_need, consumes stakeholder_need, produces specification;

    validator: reads specification, produces validation_report, consumes validation_report;

    // 基线建立需要两个前驱同时在场：规格 + 其验证结论（AND 合取，§1.4.4）
    baseline_owner: reads specification, reads validation_report,
                    consumes specification, consumes validation_report,
                    produces baseline;

    // 变更影响评估需要变更请求与现行基线同时在场
    change_analyst: reads change_request, reads baseline,
                    consumes change_request, produces impact_assessment;

    configuration_manager: reads baseline, reads impact_assessment,
                           consumes impact_assessment, produces revised_baseline
}

seed_authority {
    external_stakeholder: seeds stakeholder_need;
    change_control_board: seeds change_request
}
