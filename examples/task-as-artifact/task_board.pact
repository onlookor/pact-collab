// 任务即制品（用法一）：任务状态作为类型，任务记录作为物引
//
// 对应专题：docs/task-as-artifact.md §3.1、§3.2、§7
//
// 场景：一块通用的任务板。类型是看板列（任务状态），不是任务身份；
// 具体是哪个任务由运行时的 `--thing-ref`（如 task://BOARD-42）承载。
// 范式不读 ThingRef，因此 gate 只按"状态"准入，不按任务 id 路由。
//
// 关键结构事实：
//   1. task_open 是源类型（无前置规则）⇒ 只能由 planner 经 seed_authority 注入。
//   2. 每个类型都必须表示"已经成立的事实"：task_doing 表示任务已领取，
//      不是"待办任务"本身；否则 task_in_review 会在领取前就被使能（专题 §2.3）。
//   3. reviewer 只读 task_in_review 与 task_doing ⇒ 同一枚 token 可被多个
//      只读参与者同时观察（Part II §4.1）。
//   4. releaser 用 `via task_doing` 守护 task_in_review 的读/消费：只有"产生
//      task_in_review 时用的那枚 task_doing"仍在 Counter 上，动作才成立。
//      archivist 可以消费 task_doing ⇒ analyze 会报 1 处 stranding_risk，
//      这是真实的顺序约束，不是缺陷。
//
// 这是"任务板"，不是"任务图执行器"：
//   Rule-P 不消费前驱，所以同一枚 task_doing 可以产出多枚 task_in_review；
//   类型级 AND 不能保证两枚下游 token 来自同一个任务（专题 §6）。
//   需要恰好一次或同任务配对时，应外置执行器，或在域侧用"一次一个任务"纪律约束。

types { task_open; task_doing; task_in_review; task_done }

preconditions {
    task_open → task_doing;
    task_doing → task_in_review;
    task_in_review → task_done
}

gates {
    planner: reads task_open, produces task_doing, consumes task_open;

    worker: reads task_doing, produces task_in_review;

    // 只读参与者：可以同时看到"进行中"和"待评审"两枚 token，不改变 Counter
    reviewer: reads task_in_review, reads task_doing;

    // 溯源见证：读/消费 task_in_review 时，要求它生产时用的 task_doing 仍在场
    releaser: reads task_in_review via task_doing,
              consumes task_in_review via task_doing,
              produces task_done;

    archivist: consumes task_doing;

    stakeholder: consumes task_done
}

seed_authority { planner: seeds task_open }
