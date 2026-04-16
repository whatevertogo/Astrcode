1. 关闭对话框可以更好的看llm的排版
2. 语音选项左下角
3.增加agent可选tools 
 ---
 name: coordinator
 description: Coordinates work across specialized agents
 tools: Agent(worker, researcher), Read, Bash
 ---
4. 我想到了个设计agent company，一个部门审查其他部门的内容，其他部门自己干自己的事情，每个部门都是一个agent team.每个部门的leader会将自己队员做了的事情发在leaders session里面，由leaders自行编排逻辑，只有所有leaders都同意才能完成plan编排部门teammates工作，这样工作流基本就被废弃了，全依靠agent的自己的能力

5. 终端的输入输出功能
6. fork agent
7. pending messages
8. 更好的compact功能