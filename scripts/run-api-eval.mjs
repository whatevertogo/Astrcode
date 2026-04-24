#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createWriteStream, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { homedir } from "node:os";
import process from "node:process";
import readline from "node:readline";

const READY_PREFIX = "ASTRCODE_SERVER_READY ";
const cargoCommand = process.platform === "win32" ? "cargo.exe" : "cargo";

const helpText = `run-api-eval

启动真实 astrcode-server，并通过 Astrcode agent 框架运行 eval task-set。

Usage:
  node scripts/run-api-eval.mjs [options]

Options:
  --task-set <path>       评测任务集，默认 eval-tasks/task-set.yaml
  --output <path>         报告输出路径，默认 eval-reports/api-eval-report.json
  --concurrency <n>       并发数，默认 1
  --baseline <path>       可选 baseline report
  --keep-workspace        保留 eval 隔离工作区
  --home <path>           覆盖 ASTRCODE_HOME_DIR，用于隔离配置和 session 存储
  --server-log <path>     server 日志路径，默认 eval-reports/api-eval-server.log
  --keep-server           eval 结束后不自动停止 server
  --help                  显示帮助

Environment:
  DEEPSEEK_API_KEY / OPENAI_API_KEY 或你在 ~/.astrcode/config.json 中引用的 API key 必须可用。

Examples:
  node scripts/run-api-eval.mjs
  node scripts/run-api-eval.mjs --task-set eval-tasks/task-set.yaml --concurrency 1
  node scripts/run-api-eval.mjs --home .tmp/eval-home --output eval-reports/report.json
`;

function parseArgs(argv) {
  const args = {
    taskSet: "eval-tasks/task-set.yaml",
    output: "eval-reports/api-eval-report.json",
    concurrency: "1",
    baseline: null,
    keepWorkspace: false,
    home: null,
    serverLog: "eval-reports/api-eval-server.log",
    keepServer: false,
    help: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case "--task-set":
        args.taskSet = nextValue(argv, ++index, arg);
        break;
      case "--output":
        args.output = nextValue(argv, ++index, arg);
        break;
      case "--concurrency":
        args.concurrency = nextValue(argv, ++index, arg);
        break;
      case "--baseline":
        args.baseline = nextValue(argv, ++index, arg);
        break;
      case "--home":
        args.home = nextValue(argv, ++index, arg);
        break;
      case "--server-log":
        args.serverLog = nextValue(argv, ++index, arg);
        break;
      case "--keep-workspace":
        args.keepWorkspace = true;
        break;
      case "--keep-server":
        args.keepServer = true;
        break;
      case "--help":
      case "-h":
        args.help = true;
        break;
      default:
        throw new Error(`未知参数: ${arg}\n\n${helpText}`);
    }
  }

  if (!/^[1-9]\d*$/.test(args.concurrency)) {
    throw new Error(`--concurrency 必须是正整数，收到 ${args.concurrency}`);
  }

  return args;
}

function nextValue(argv, index, flag) {
  const value = argv[index];
  if (!value || value.startsWith("--")) {
    throw new Error(`${flag} 需要一个值`);
  }
  return value;
}

function resolveAstrcodeHome(args, env) {
  if (args.home) {
    return resolve(args.home);
  }
  if (env.ASTRCODE_TEST_HOME) {
    return resolve(env.ASTRCODE_TEST_HOME);
  }
  if (env.ASTRCODE_HOME_DIR) {
    return resolve(env.ASTRCODE_HOME_DIR);
  }
  return homedir();
}

function spawnLogged(command, args, options) {
  return spawn(command, args, {
    cwd: options.cwd,
    env: options.env,
    stdio: ["pipe", "pipe", "pipe"],
  });
}

async function waitForReady(child, logPath) {
  return new Promise((resolveReady, rejectReady) => {
    const timeout = setTimeout(() => {
      cleanup();
      rejectReady(new Error(`等待 astrcode-server ready 超时，详见 ${logPath}`));
    }, 120_000);

    const onExit = (code, signal) => {
      cleanup();
      rejectReady(
        new Error(`astrcode-server 启动前退出 code=${code} signal=${signal}，详见 ${logPath}`),
      );
    };
    const onError = (error) => {
      cleanup();
      rejectReady(new Error(`启动 astrcode-server 失败: ${error.message}`));
    };

    const onLine = (line) => {
      const index = line.indexOf(READY_PREFIX);
      if (index < 0) {
        return;
      }
      const payload = line.slice(index + READY_PREFIX.length).trim();
      try {
        const ready = JSON.parse(payload);
        cleanup();
        resolveReady(ready);
      } catch (error) {
        cleanup();
        rejectReady(new Error(`解析 server ready 载荷失败: ${error.message}`));
      }
    };

    const stdout = readline.createInterface({ input: child.stdout });
    const stderr = readline.createInterface({ input: child.stderr });
    stdout.on("line", onLine);
    stderr.on("line", onLine);
    child.once("exit", onExit);
    child.once("error", onError);

    function cleanup() {
      clearTimeout(timeout);
      child.off("exit", onExit);
      child.off("error", onError);
      stdout.off("line", onLine);
      stderr.off("line", onLine);
      stdout.close();
      stderr.close();
    }
  });
}

async function exchangeApiToken(serverUrl, bootstrapToken) {
  const response = await fetch(`${serverUrl}/api/auth/exchange`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token: bootstrapToken }),
  });
  if (!response.ok) {
    throw new Error(`交换 API token 失败，HTTP ${response.status}: ${await response.text()}`);
  }
  const payload = await response.json();
  if (!payload.token) {
    throw new Error("交换 API token 响应缺少 token");
  }
  return payload.token;
}

async function runCommand(command, args, options) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      stdio: "inherit",
    });
    child.on("error", rejectRun);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolveRun();
      } else {
        rejectRun(new Error(`${command} 退出 code=${code} signal=${signal}`));
      }
    });
  });
}

async function stopServer(child) {
  if (!child || child.exitCode !== null) {
    return;
  }
  if (child.stdin.writable) {
    child.stdin.end();
  }
  await new Promise((resolveStop) => {
    const timer = setTimeout(() => {
      child.kill();
      resolveStop();
    }, 5_000);
    child.once("exit", () => {
      clearTimeout(timer);
      resolveStop();
    });
  });
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    process.stdout.write(helpText);
    return;
  }

  const root = process.cwd();
  const outputPath = resolve(args.output);
  const serverLogPath = resolve(args.serverLog);
  mkdirSync(dirname(outputPath), { recursive: true });
  mkdirSync(dirname(serverLogPath), { recursive: true });

  const env = { ...process.env };
  const astrcodeHome = resolveAstrcodeHome(args, env);
  if (args.home) {
    env.ASTRCODE_HOME_DIR = astrcodeHome;
  }

  const serverLog = createWriteStream(serverLogPath, { flags: "w" });
  const server = spawnLogged(cargoCommand, ["run", "-p", "astrcode-server"], {
    cwd: root,
    env,
  });
  server.stdout.pipe(serverLog);
  server.stderr.pipe(serverLog);

  try {
    const ready = await waitForReady(server, serverLogPath);
    const serverUrl = `http://127.0.0.1:${ready.port}`;
    const apiToken = await exchangeApiToken(serverUrl, ready.token);
    const sessionStorageRoot = resolve(astrcodeHome, ".astrcode", "projects");

    const evalArgs = [
      "run",
      "-p",
      "astrcode-eval",
      "--",
      "--server-url",
      serverUrl,
      "--session-storage-root",
      sessionStorageRoot,
      "--task-set",
      resolve(args.taskSet),
      "--concurrency",
      args.concurrency,
      "--output",
      outputPath,
    ];
    if (args.baseline) {
      evalArgs.push("--baseline", resolve(args.baseline));
    }
    if (args.keepWorkspace) {
      evalArgs.push("--keep-workspace");
    }

    await runCommand(cargoCommand, evalArgs, {
      cwd: root,
      env: {
        ...env,
        ASTRCODE_EVAL_TOKEN: apiToken,
      },
    });

    process.stderr.write(`eval report: ${outputPath}\n`);
    process.stderr.write(`server log: ${serverLogPath}\n`);
  } finally {
    if (!args.keepServer) {
      await stopServer(server);
    }
    serverLog.end();
  }
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
