const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const cargoCommand = process.platform === "win32" ? "cargo.exe" : "cargo";
const repoRoot = fs.realpathSync.native(path.resolve(__dirname, ".."));
const tauriArgs = process.argv.slice(2);

if (tauriArgs.length === 0) {
  console.error("usage: node scripts/tauri-cli.js <tauri-subcommand> [...args]");
  process.exit(1);
}

// Why: tauri-cli 2.10.x 在 Windows 上会把 `tauri_dir/Cargo.toml`
// 与 `cargo metadata` 返回的 manifest_path 做字面相等比较。
// 当前目录若是 `d:\\repo` 而 metadata 返回 `D:\\repo`，就会误报
// “tauri project package doesn't exist in cargo metadata output `packages`”。
// 这里先把仓库路径规范化为系统真实大小写，再启动 cargo tauri。
const child = spawn(cargoCommand, ["tauri", ...tauriArgs], {
  cwd: repoRoot,
  stdio: "inherit",
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}

child.on("error", (error) => {
  console.error(`failed to start cargo tauri: ${error.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }

  process.exit(code ?? 1);
});
