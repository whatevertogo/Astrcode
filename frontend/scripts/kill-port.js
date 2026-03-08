import { exec } from 'child_process';
import { platform } from 'os';

const PORT = process.env.PORT || 5173;

function killPort(port) {
  return new Promise((resolve) => {
    const isWindows = platform() === 'win32';

    if (isWindows) {
      // Windows: 查找并关闭占用端口的进程
      exec(`netstat -ano | findstr :${port}`, (error, stdout) => {
        if (error || !stdout) {
          console.log(`Port ${port} is available`);
          resolve();
          return;
        }

        const lines = stdout.trim().split('\n');
        const pids = new Set();

        lines.forEach(line => {
          const parts = line.trim().split(/\s+/);
          const pid = parts[parts.length - 1];
          if (pid && !isNaN(pid) && pid !== '0') {
            pids.add(pid);
          }
        });

        if (pids.size === 0) {
          console.log(`Port ${port} is available`);
          resolve();
          return;
        }

        let killed = 0;
        pids.forEach(pid => {
          exec(`taskkill /F /PID ${pid}`, (err) => {
            if (!err) {
              console.log(`Killed process ${pid} on port ${port}`);
            }
            killed++;
            if (killed === pids.size) {
              setTimeout(resolve, 500); // 等待端口释放
            }
          });
        });
      });
    } else {
      // Unix/macOS: 使用 lsof 和 kill
      exec(`lsof -ti:${port}`, (error, stdout) => {
        if (error || !stdout) {
          console.log(`Port ${port} is available`);
          resolve();
          return;
        }

        const pids = stdout.trim().split('\n').filter(p => p);

        if (pids.length === 0) {
          resolve();
          return;
        }

        exec(`kill -9 ${pids.join(' ')}`, (err) => {
          if (!err) {
            console.log(`Killed processes on port ${port}: ${pids.join(', ')}`);
          }
          setTimeout(resolve, 500);
        });
      });
    }
  });
}

killPort(PORT).then(() => {
  process.exit(0);
}).catch((err) => {
  console.error('Error killing port:', err.message);
  process.exit(0); // 不阻止启动
});
