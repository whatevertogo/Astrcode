#!/usr/bin/env node

import { spawnSync } from "node:child_process";

const checks = [
  {
    name: "cli launcher attach",
    command: "cargo",
    args: ["test", "-p", "astrcode-cli", "attaches_to_existing_server_from_run_info", "--", "--nocapture"],
  },
  {
    name: "cli launcher managed-local",
    command: "cargo",
    args: ["test", "-p", "astrcode-cli", "spawns_local_server_when_run_info_is_missing", "--", "--nocapture"],
  },
  {
    name: "cli launcher repo-aware fallback",
    command: "cargo",
    args: ["test", "-p", "astrcode-cli", "uses_repo_aware_cargo_fallback_when_default_binary_is_unavailable", "--", "--nocapture"],
  },
  {
    name: "cli bootstrap fresh session",
    command: "cargo",
    args: ["test", "-p", "astrcode-cli", "bootstrap_creates_fresh_session_instead_of_restoring_existing_one", "--", "--nocapture"],
  },
  {
    name: "server conversation snapshot contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-server", "conversation_snapshot_contract_rejects_invalid_focus", "--", "--nocapture"],
  },
  {
    name: "server conversation compact contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-server", "compact_route_defers_when_session_is_busy", "--", "--nocapture"],
  },
  {
    name: "server conversation skill discovery contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-server", "composer_options_expose_session_scoped_skill_entries", "--", "--nocapture"],
  },
  {
    name: "client conversation snapshot contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-client", "fetch_conversation_snapshot_uses_cached_auth_and_decodes_payload", "--", "--nocapture"],
  },
  {
    name: "client conversation stream contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-client", "stream_conversation_surfaces_delta_rehydrate_and_disconnect_events", "--", "--nocapture"],
  },
  {
    name: "client conversation slash contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-client", "list_conversation_slash_candidates_uses_conversation_surface_contract", "--", "--nocapture"],
  },
  {
    name: "client conversation compact contract",
    command: "cargo",
    args: ["test", "-p", "astrcode-client", "request_compact_preserves_existing_compact_contract", "--", "--nocapture"],
  },
  {
    name: "cli conversation acceptance",
    command: "cargo",
    args: ["test", "-p", "astrcode-cli", "end_to_end_acceptance_covers_resume_compact_skill_and_single_active_stream_switch", "--", "--nocapture"],
  },
];

for (const check of checks) {
  console.log(`\n==> ${check.name}`);
  const result = spawnSync(check.command, check.args, {
    stdio: "inherit",
    shell: false,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

console.log("\nconversation surface acceptance passed");
