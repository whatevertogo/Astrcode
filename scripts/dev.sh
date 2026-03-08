#!/usr/bin/env bash

set -euo pipefail

FRONTEND_PORT=5173
FRONTEND_URL="http://localhost:${FRONTEND_PORT}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
FRONTEND_DIR="${REPO_ROOT}/frontend"
FRONTEND_PID=""
REUSE_FRONTEND=0

stop_frontend() {
  if [[ -n "${FRONTEND_PID}" ]] && kill -0 "${FRONTEND_PID}" 2>/dev/null; then
    echo
    echo "[stop] 终止前端进程..."
    kill "${FRONTEND_PID}" 2>/dev/null || true
    wait "${FRONTEND_PID}" 2>/dev/null || true
  fi
}

trap stop_frontend EXIT INT TERM

port_in_use() {
  if command -v lsof >/dev/null 2>&1; then
    lsof -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
  elif command -v ss >/dev/null 2>&1; then
    ss -ltn "( sport = :$1 )" | tail -n +2 | grep -q .
  else
    netstat -an 2>/dev/null | grep -E "[\.\:]$1 .*LISTEN" >/dev/null 2>&1
  fi
}

wait_frontend_ready() {
  local url="$1"
  local timeout_seconds="$2"

  for ((attempt = 1; attempt <= timeout_seconds; attempt++)); do
    if curl -sSf "$url" >/dev/null 2>&1; then
      echo "✓ 前端已就绪: $url"
      return 0
    fi

    echo "[wait] 等待前端启动... (${attempt}/${timeout_seconds})"
    sleep 1
  done

  echo "[error] 前端在 ${timeout_seconds} 秒内未就绪：${url}" >&2
  return 1
}

echo "[check] 检查端口 ${FRONTEND_PORT} ..."
if port_in_use "${FRONTEND_PORT}"; then
  echo "[warn] 端口 ${FRONTEND_PORT} 已被占用。"
  read -r -p "继续使用现有服务？输入 y 继续，其他任意键退出: " answer
  if [[ "${answer}" != "y" && "${answer}" != "Y" ]]; then
    echo "[error] 用户取消启动。" >&2
    exit 1
  fi
  REUSE_FRONTEND=1
  echo "[info] 复用现有前端服务。"
else
  echo "[start] 在后台启动前端开发服务器..."
  (
    cd "${FRONTEND_DIR}"
    npm run dev
  ) &
  FRONTEND_PID=$!
fi

wait_frontend_ready "${FRONTEND_URL}" 60

echo "[start] 启动 Tauri 开发环境..."
cd "${REPO_ROOT}"
cargo tauri dev
