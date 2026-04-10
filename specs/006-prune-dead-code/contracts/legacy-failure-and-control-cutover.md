# Contract: Legacy Failure 与 Control Cutover

本合同定义两件事：

1. 不再支持的旧输入如何失败
2. 当前仍在使用的 legacy control 如何切到正式入口

## 1. Legacy Failure

对于旧共享历史、descriptor 缺失 legacy subrun 或其他已决定不再支持的旧输入：

- 系统必须明确失败
- 不再返回 downgrade status
- 不再构建 legacy tree
- 不再伪造 lineage、child 结构或“部分可用”视图

如需要稳定错误信息，可保留清晰错误码；但错误码不应再伴随一整套 downgrade 公开模型存在。

## 2. Cancel Control Cutover

### 当前正式入口

清理完成后，“取消子会话/关闭子 agent”只允许通过 `closeAgent` 协作能力完成。

### 删除入口

以下入口在迁移完成后必须删除：

- `cancelSubRun` 前端包装
- `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`

### 约束

- 不允许长期同时保留两条主线入口
- 不允许新增一个新的临时兼容 route
- 当前 UI 按钮行为必须保持可用

## 3. Validation

- legacy 输入进入当前主线时表现为明确失败
- 当前 UI 的取消按钮仍然可用
- 删除入口在代码、测试、文档中不再出现
