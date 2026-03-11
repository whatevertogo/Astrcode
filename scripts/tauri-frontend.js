const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const mode = process.argv[2];

if (mode !== "build" && mode !== "dev") {
  console.error("usage: node scripts/tauri-frontend.js <build|dev>");
  process.exit(1);
}

const repoRoot = path.resolve(__dirname, "..");
const frontendDir = path.join(repoRoot, "frontend");
const sidecarDir = path.join(repoRoot, "src-tauri", "binaries");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const command = process.platform === "win32" ? "cmd.exe" : npmCommand;
const args =
  process.platform === "win32"
    ? ["/d", "/s", "/c", `${npmCommand} run ${mode}`]
    : ["run", mode];
const cargoCommand = process.platform === "win32" ? "cargo.exe" : "cargo";

function resolveTargetTriple() {
  const envTriple = process.env.TAURI_ENV_TARGET_TRIPLE?.trim();
  if (envTriple) {
    return envTriple;
  }

  const result = spawnSync("rustc", ["-vV"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    console.error("failed to resolve rust target triple");
    process.exit(result.status ?? 1);
  }

  const hostLine = result.stdout
    .split(/\r?\n/)
    .find((line) => line.startsWith("host: "));
  if (!hostLine) {
    console.error("rustc -vV did not return a host triple");
    process.exit(1);
  }

  return hostLine.slice("host: ".length).trim();
}

function serverBinaryName() {
  return process.platform === "win32" ? "astrcode-server.exe" : "astrcode-server";
}

function bundledSidecarName(targetTriple) {
  return process.platform === "win32"
    ? `astrcode-server-${targetTriple}.exe`
    : `astrcode-server-${targetTriple}`;
}

function prepareSidecar(currentMode) {
  const targetTriple = resolveTargetTriple();
  const release = currentMode === "build";
  const cargoArgs = ["build", "-p", "astrcode-server", "--target", targetTriple];
  if (release) {
    cargoArgs.push("--release");
  }

  const result = spawnSync(cargoCommand, cargoArgs, {
    cwd: repoRoot,
    stdio: "inherit",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }

  const profileDir = release ? "release" : "debug";
  const sourceBinary = path.join(
    repoRoot,
    "target",
    targetTriple,
    profileDir,
    serverBinaryName(),
  );
  const targetBinary = path.join(sidecarDir, bundledSidecarName(targetTriple));

  fs.mkdirSync(sidecarDir, { recursive: true });
  fs.copyFileSync(sourceBinary, targetBinary);
}

prepareSidecar(mode);

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
