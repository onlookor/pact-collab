// 任务即制品（用法二）：任务位置/产出作为类型，任务记录作为物引
//
// 对应专题：docs/task-as-artifact.md §3.2、§3.3
//
// 场景：一条交付流水线。每个类型是一个任务的产出（output-of-task），
// 前置图就是任务 DAG —— 这正是母本 §4.1 的构造编码：
//   A_i → output-of-A_i，(A_i, A_j) → T_i → T_j（限 SOAD：单输出、AND 结构）。
//
// 与用法一（task_board.pact）的区别：
//   task_board 的类型是"通用状态列"，所有任务共享同一组类型；
//   task_dag 的类型是"任务位置"，任务种类在声明期就固定。
//   前者适合任务种类不确定的任务板，后者适合拓扑固定的交付流水线。
//
// 任务身份仍然在物引里：produce 时用 --thing-ref task://DELIVERY-7/triage 等。
// 范式不读 ThingRef，所以"具体是哪个交付"由参与者自己识别；
// 类型只决定"它在流程的哪一格"。
//
// 这是任务 DAG 的拓扑编码，不是任务图执行器：
//   Rule-P 不消费前驱、没有自动点火/重试/超时（专题 §6）。
//   需要恰好一次执行时，由参与者或外部执行器保证。

types { intake; triage_report; build_artifact; test_report; release_record }

preconditions {
    intake → triage_report;
    triage_report → build_artifact;
    build_artifact → test_report;
    test_report → release_record
}

gates {
    triager: reads intake, consumes intake, produces triage_report;

    builder: reads triage_report, consumes triage_report, produces build_artifact;

    tester: reads build_artifact, consumes build_artifact, produces test_report;

    release_manager: reads test_report, consumes test_report, produces release_record;

    stakeholder: consumes release_record
}

seed_authority { intake: seeds intake }
