// compose 演示片段 A：安全评审（与 se_lifecycle.pact 干净组合）
//
// 用途：演示 §1.8 片段并行组合。
//
// 实测约束：`pact compose` 对每个输入片段**单独**执行良构校验（含 Type Closure），
// 因此片段必须自己声明所引用的全部类型——这里要显式写出 implementation，即使它
// 由宿主片段负责生产。片段内 implementation 没有前置规则，按 §1.4 暂为源类型；
// 合并后宿主给出 design → implementation，源类型身份消失，不产生矛盾
// （C1 只约束 RHS 唯一性与 LHS 唯一性，允许一侧未声明）。
//
// 组合后效果（四元组按集合并）：
//   types       + security_report
//   preconditions + implementation → security_report
//   gates       + security_scanner 全部子句；deployer 增 reads/consumes security_report
//   seed_authority 不变
// C3 不触发：deployer 在本片段的子句是 (reads, security_report) 与
// (consumes, security_report)，与宿主的 (reads/consumes, test_report) 是不同的
// (参与者, 动作, 类型) 三元组，不存在需要语法同一的条件冲突。

types { implementation; security_report }

preconditions {
    implementation → security_report
}

gates {
    security_scanner: reads implementation, produces security_report;
    deployer: reads security_report, consumes security_report
}

seed_authority { }
