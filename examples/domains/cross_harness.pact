// 跨 harness 协调域：共享 Counter 作为协调基底
//
// 出处：docs/pactv1_rev3.md Part III Chapter 2 §2.1
//
// 展示范式的核心主张：多个自治 AI harness 之间不需要编排器、不需要直接通信。
// 每个 harness 只关心"当前 Counter 上有没有我能生产的前驱"，靠 Rule-P 可满足性
// 自驱（Rule-W 唤醒）。协调是 Counter 状态的涌现性质，不是控制流的产物。
//
// 论文草图中 Harness_A "produces plan_token" 未交代 plan 的来源。按 §1.4 源类型
// 约束，plan 若无前置规则即为源类型、不可 produce；故此处显式引入源类型
// task_brief 由 user 注入，plan 由 harness_a 从 task_brief 生产。这是把论文草图
// 落成良构域声明所必需的最小补充，不改变其协调结构。
//
// 结构：task_brief → plan → subtask → result → synthesis
//   harness_a 首尾两端（领任务、做综合），harness_b 拆解，harness_c 执行。
// 全部 token 都有消费者 ⇒ Counter 可归空 ⇒ Rule-T 可达
// （Rule-T 的实际触发过程见 examples/online/trace.txt 第 11 步，那里跑的是
//   se_lifecycle，机制完全相同）。

types { task_brief; plan; subtask; result; synthesis }

preconditions {
    task_brief → plan;
    plan → subtask;
    subtask → result;
    result → synthesis
}

gates {
    harness_a: reads task_brief, consumes task_brief, produces plan,
               reads result, consumes result, produces synthesis, consumes synthesis;
    harness_b: reads plan, consumes plan, produces subtask;
    harness_c: reads subtask, consumes subtask, produces result
}

seed_authority { user: seeds task_brief }
