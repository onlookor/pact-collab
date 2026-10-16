// 软件工程通用域：生命周期（中等粒度）
//
// 出处：docs/pactv1_rev2.md Part II Section 1
//   Chapter 1 骨架（5 类型 / 4 规则 / 生产链）
//   Chapter 2 权限细化（reviewer、qa_lead 为只读参与者，从不生产也不消费）
//   Chapter 3 审批关联（deployer 的 via 溯源见证）
//
// 与任何具体方法论、技能、组织无关：类型名取通用工程产物，角色位取通用工程职能。
// 换领域只需替换四个注入点，范式语义不变（§2.2）。
//
// 关键结构事实：
//   requirement 是源类型（无前置规则）⇒ 按 §3.1 约束 6 不可被任何参与者 produce，
//   只能由 seed_authority 注入。这是"入口唯一性"，也是 Rule-T 可终止的前提。
//   deployment 是终态产物，由 stakeholder 消费归档。

types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}

gates {
    // 主生产链（Chapter 1）
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester:    reads implementation, produces test_report, consumes implementation;

    // 只读参与者（Chapter 2）：持 reads gate，既不 produces 也不 consumes
    reviewer:  reads requirement, reads design;
    qa_lead:   reads implementation, reads test_report;

    // 审批关联（Chapter 3）：处置 test_report 时要求其来源 implementation 仍在 Counter，
    // 且该 implementation 的 token id 出现在 test_report.depends_on 中（§1.4.0）。
    // 效果：无法对"来源实现已不在场"的测试报告动手。
    deployer:  reads test_report via implementation,
               consumes test_report via implementation,
               produces deployment;

    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
