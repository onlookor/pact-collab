// 六个形状配方（作者工作台与运行沙盒共用；改这里会同时影响两个页面）。
const TEMPLATES = {
  "R1 线性流水线": `types { request; answer }

preconditions { request → answer }

gates {
    responder: reads request, consumes request, produces answer;
    client:    consumes answer
}

seed_authority { client: seeds request }
`,
  "R2 只读评审支线": `types { draft; review; final }

preconditions { draft → review; review → final }

gates {
    author:      reads draft, consumes draft, produces review;
    reviewer:    reads review;
    owner:       reads review, consumes review, produces final;
    stakeholder: consumes final
}

seed_authority { intake: seeds draft }
`,
  "R3 审批关联（via）": `types { design; test_report; deployment }

preconditions { design → test_report; test_report → deployment }

gates {
    engineer:    reads design, produces test_report;
    reviewer:    reads test_report via design;
    deployer:    reads test_report via design,
                 consumes test_report via design,
                 produces deployment;
    stakeholder: consumes deployment
}

seed_authority { intake: seeds design }
`,
  "R4 外部物化见证": `types { work_item; readiness; completion }

preconditions { work_item → completion }

gates {
    coordinator: consumes work_item when exists readiness,
                 consumes readiness,
                 produces completion;
    reviewer:    consumes completion
}

seed_authority {
    requester:       seeds work_item;
    external_system: seeds readiness
}
`,
  "R5 扇出 / ack 汇聚": `types { shared; ack_a; ack_b; merged }

preconditions {
    shared → ack_a;
    shared → ack_b;
    { ack_a + ack_b } → merged
}

gates {
    worker_a:    reads shared, produces ack_a;
    worker_b:    reads shared, produces ack_b;
    coordinator: consumes shared when count ack_a >= 1 and count ack_b >= 1,
                 produces merged;
    cleaner:     consumes ack_a when count merged >= 1,
                 consumes ack_b when count merged >= 1;
    stakeholder: consumes merged
}

seed_authority { intake: seeds shared }
`,
  "R6 循环点火": `types { observation; thought; action; tool_result }

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
`
};
