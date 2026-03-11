import { execFile } from 'child_process';
import { platform } from 'os';
import { promisify } from 'util';

const execFileAsync = promisify(execFile);
const DEFAULT_PORT = 5173;

function parsePort(value) {
  const raw = String(value).trim();
  if (!/^\d+$/.test(raw)) {
    return null;
  }

  const port = Number(raw);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    return null;
  }

  return port;
}

function isValidPid(value) {
  return /^\d+$/.test(value) && value !== '0';
}

async function runCommand(file, args) {
  const result = await execFileAsync(file, args, {
    windowsHide: true,
    maxBuffer: 1024 * 1024,
  });
  return result.stdout;
}

function collectWindowsPids(stdout, port) {
  const lines = stdout.split(/\r?\n/);
  const pids = new Set();

  lines.forEach((line) => {
    const parts = line.trim().split(/\s+/);
    if (parts.length < 4) {
      return;
    }

    const localAddress = parts[1];
    const pid = parts[parts.length - 1];
    if (localAddress?.endsWith(`:${port}`) && isValidPid(pid)) {
      pids.add(pid);
    }
  });

  return [...pids];
}

function collectUnixPids(stdout) {
  return stdout
    .split(/\r?\n/)
    .map((pid) => pid.trim())
    .filter(isValidPid);
}

async function killWindowsPort(port) {
  try {
    const stdout = await runCommand('netstat', ['-ano']);
    const pids = collectWindowsPids(stdout, port);
    if (pids.length === 0) {
      console.log(`Port ${port} is available`);
      return;
    }

    await runCommand('taskkill', ['/F', ...pids.flatMap((pid) => ['/PID', pid])]);
    pids.forEach((pid) => {
      console.log(`Killed process ${pid} on port ${port}`);
    });
    await new Promise((resolve) => setTimeout(resolve, 500));
  } catch (error) {
    if (!error.stdout) {
      console.log(`Port ${port} is available`);
      return;
    }

    throw error;
  }
}

async function killUnixPort(port) {
  try {
    const stdout = await runCommand('lsof', ['-t', '-i', `:${port}`]);
    const pids = collectUnixPids(stdout);
    if (pids.length === 0) {
      console.log(`Port ${port} is available`);
      return;
    }

    await runCommand('kill', ['-9', ...pids]);
    console.log(`Killed processes on port ${port}: ${pids.join(', ')}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  } catch (error) {
    if (!error.stdout) {
      console.log(`Port ${port} is available`);
      return;
    }

    throw error;
  }
}

async function killPort(port) {
  if (platform() === 'win32') {
    await killWindowsPort(port);
    return;
  }

  await killUnixPort(port);
}

const port = parsePort(process.env.PORT ?? DEFAULT_PORT);
if (port === null) {
  console.warn(`Invalid PORT value: ${process.env.PORT}`);
  process.exit(0);
}

killPort(port)
  .then(() => {
    process.exit(0);
  })
  .catch((err) => {
    console.error('Error killing port:', err.message);
    process.exit(0); // 不阻止启动
  });
