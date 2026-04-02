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
	cargo test --workspace --exclude astrcode --lib
	cd frontend && npm run typecheck

check-ci:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --workspace --exclude astrcode
	cd frontend && npm run typecheck
	cd frontend && npm run lint
	cd frontend && npm run format:check
