const { spawn } = require("node:child_process");
const path = require("node:path");

const mode = process.argv[2];

if (mode !== "build" && mode !== "dev") {
  console.error("usage: node scripts/tauri-frontend.js <build|dev>");
  process.exit(1);
}

const repoRoot = path.resolve(__dirname, "..");
const frontendDir = path.join(repoRoot, "frontend");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const command = process.platform === "win32" ? "cmd.exe" : npmCommand;
const args =
  process.platform === "win32"
    ? ["/d", "/s", "/c", `${npmCommand} run ${mode}`]
    : ["run", mode];

const child = spawn(command, args, {
  cwd: frontendDir,
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
  console.error(`failed to start frontend ${mode} command:`, error);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }

  process.exit(code ?? 1);
});
