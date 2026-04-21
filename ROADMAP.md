  活跃 Change（8个）:                                                                                                                        
                                                                                                                                             
    不动:                                                                                                                                    
    A  linearize-session-runtime-application-boundaries    ← 你在进行的                                                                      
                                               
    边界清理线（串行）:                                                                                                                      
    B  session-runtime-state-turn-boundary                 ← 依赖 A
    C  server-session-runtime-isolation                    ← 依赖 A
    D  core-slimming                                       ← 依赖 B

    内部重组（可与 C 并行）:
    E  application-decomposition                           ← 依赖 A

    新功能（可独立推进）:
    F  hooks-platform                                      ← 依赖 A+B，已吸收 G+H
    I  async-shell-terminal-sessions                       ← 独立

    治理演进（建议 D 之后）:
    J  unify-declarative-dsl-compiler-architecture

  已归档:
    G  extract-governance-prompt-hooks          → 已合并入 F
    H  introduce-hooks-platform-crate           → 已合并入 F