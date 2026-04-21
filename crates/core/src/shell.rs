//! Shell 共享数据结构。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellFamily {
    PowerShell,
    Cmd,
    Posix,
    Wsl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedShell {
    pub program: String,
    pub family: ShellFamily,
    pub label: String,
}
