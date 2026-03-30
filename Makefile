dev:
	powershell -ExecutionPolicy Bypass -File scripts/dev.ps1

dev-unix:
	bash scripts/dev.sh

build:
	cargo tauri build

frontend:
	cd frontend && npm run dev

tauri:
	cargo tauri dev

check:
	cargo check --workspace
	cargo test --workspace
	cd frontend && npm run typecheck
