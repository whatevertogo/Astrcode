## 1. 判定 discovery 与 Skill Tool 的保留范围

- [x] 1.1 盘点旧项目中的工具发现、模糊搜索、Skill Tool、相关 server 合同与调用方。
- [x] 1.2 判断哪些能力仍具备产品价值，哪些应明确废弃。
- [x] 1.3 将”保留/废弃”的结论同步回 spec，禁止留下空壳接口。

## 2. 以现有事实源重建保留能力

- [x] 2.1 若保留工具发现，则以当前 capability surface 为唯一事实源实现该能力。
- [x] 2.2 若保留技能发现或 Skill Tool，则以当前 skill catalog / materializer 为唯一事实源实现该能力。
- [x] 2.3 若 discovery 需要额外语义字段，则扩展 capability semantic model，而不是新增平行注册表。

## 3. 验证不会重新长出旧 runtime registry

- [x] 3.1 为保留的 discovery / skill 能力编写测试，验证其事实源来自当前 surface 或 catalog。
- [x] 3.2 删除或拒绝继续传播旧 runtime 风格的 discovery cache / registry / skeleton。
- [x] 3.3 运行 `cargo fmt --all --check`。
- [x] 3.4 运行 `cargo clippy --all-targets --all-features -- -D warnings`。
- [x] 3.5 运行 `cargo test`。
