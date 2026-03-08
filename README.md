# AstrCode

桌面开发启动：

```bash
cargo tauri dev
```

也可以在仓库根目录使用：

```bash
npm run dev
```

也可以直接打开 `http://127.0.0.1:5173/` 进行浏览器调试。
此时前端会走 Vite 本地代理提供的 Web Chat 通道，可以真实请求模型并验证流式渲染。
需要 Tauri IPC、工作目录、原生窗口能力时，再使用 `cargo tauri dev`。
