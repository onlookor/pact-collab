// compose 演示片段 B：安全门禁（与 se_lifecycle.pact 组合被拒）
//
// 用途：演示 C2（共享类型前置一致性）如何机械地拦住一次看似合理的扩展。
//
// 意图：把安全评审从"建议"升级为"门禁"——deployment 必须同时有测试报告与
// 安全报告才能产出。写法是给 deployment 换一条前置规则：
//   { test_report + security_report } → deployment
// 但宿主片段已声明 test_report → deployment。同一共享类型出现两条不同的前置规则，
// C2 判定冲突，compose 拒绝合并并指明类型。
//
// 这不是工具的缺陷，而是范式的设计意图（§1.8 C2 + design.md D18）：
// 前置图是生产合法性的唯一来源，两个片段对同一类型给出不同的生产前提，
// 意味着它们对"什么算完成"的理解不一致，必须显式解决而不是静默合并。
// 正确的升级路径是修改宿主片段本身（把 deployment 的规则改成合取），
// 而不是叠加一个矛盾的片段。
//
// 与片段 A 同理：compose 逐片段做良构校验，所以本片段必须自足——显式声明所引用的
// implementation / test_report / deployment，并为 security_report 补上前置规则
// （否则它在片段内是源类型，而 security_scanner 又 produces 它，直接违反
// §1.4 Source-Production Constraint，连 C2 都走不到）。
//
// 预期：pact compose domains/se_lifecycle.pact fragments/security_blocking.pact
//       以非 0 退出并报 C2 冲突。见 examples/offline/run.sh 第 4.2 步。

types { implementation; test_report; deployment; security_report }

preconditions {
    implementation → security_report;
    { test_report + security_report } → deployment
}

gates {
    security_scanner: reads implementation, produces security_report
}

seed_authority { }
