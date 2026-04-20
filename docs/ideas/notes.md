待办事项
1. 关闭对话框可以更好的看llm的排版
2. 语音选项左下角
3. 增加agent可选tools 
 ---
 name: coordinator
 description: Coordinates work across specialized agents
 tools: Agent(worker, researcher), Read, Bash
 ---
4. 我想到了个设计agent company，一个部门审查其他部门的内容，其他部门自己干自己的事情，每个部门都是一个agent team.每个部门的leader会将自己队员做了的事情发在leaders session里面，由leaders自行编排逻辑，只有所有leaders都同意才能完成plan编排部门teammates工作，这样工作流基本就被废弃了，全依靠agent的自己的能力

5. 终端工具的输入输出功能
6. fork agent
7. pending messages(完成部分)
8. 更好的compact功能
9. 多agent共享任务列表
10. 更安全更自由的权限，让agent能操控工作区以外的文件
    - TODO: v1 先默认全局放开文件工具对工作区的围栏，后续再补 Claude Code 风格的目录白名单、危险模式、审批规则与受保护路径。
