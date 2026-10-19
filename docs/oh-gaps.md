# What Teeny needs from OpenHuman

References are `upstream/main` at `6741ea4b41`, with
`$C = crates/openhuman-core/src`. None of these blocks the design in
[`architecture.md`](architecture.md). Each has a workaround in Teeny's bash,
and each upstream change would delete that workaround. They're ranked by how
much Teeny-side code they'd remove.

| # | Gap | Teeny's workaround today | Proposed upstream change |
|---|---|---|---|
| G1 ([#8118](https://github.com/tinyhumansai/openhuman/issues/8118)) | **Shell cron jobs are killed after 120s.** It's a hard constant (`$C/cron/scheduler/shell_job.rs:9`, `SHELL_JOB_TIMEOUT_SECS`). The morning post (several turns plus a video) takes minutes. | `bin/job` re-runs itself under `setsid`, returns at once, and records its own run in `hands.db`. Cron then reports "ok" for a job that may still fail later. | A per-job `timeout_secs` on `cron.add` for shell jobs, capped by a `[cron] max_shell_timeout_secs`. |
| G2 ([#8119](https://github.com/tinyhumansai/openhuman/issues/8119)) | **Cron jobs can't be declared in a file.** They're created only through `cron.add` (RPC or the agent tool), so the schedule isn't version-controlled. | `bin/seed-cron` reconciles `workspace/cron.toml` against `cron.list` at every boot. | Load `<workspace>/cron.toml` at boot as operator-declared jobs, owned by the file and pruned when they're removed from it. |
| G3 ([#8120](https://github.com/tinyhumansai/openhuman/issues/8120)) | **Channel replies have no outbound hook.** A Telegram turn's text goes to the chat with no step to filter, rewrite or block it. `afterAgentResponse` only observes (`vendor/tinyagents/crates/tinyagents-hooks/src/command/types.rs`). | Telegram runs through `hands/telegram` on a 1-minute shell cron instead of the channel. | A `beforeChannelSend` hook that can deny or rewrite, with the same contract as `preToolUse`. |
| G4 ([#8121](https://github.com/tinyhumansai/openhuman/issues/8121)) | **Read-only tools need MCP, and MCP is always `Execute`.** A no-shell agent can only reach a script through MCP, and every MCP call registers as `Execute` whatever the server's `readOnlyHint` says (`$C/tools/impl/network/mcp.rs:319`). So OpenHuman's gate can't tell a read verb from a write, and Teeny's safety rests only on the MCP catalog and the agents' allowlists. | `hands/mcp`, a stdio MCP server wrapping the read verbs. | Workspace-declared command tools (`tools/<name>.toml`: command, JSON schema, permission level, external effect) registered like built-ins. |
| G5 ([#8122](https://github.com/tinyhumansai/openhuman/issues/8122)) | **No direct Langfuse export.** Pushing traces needs the TinyHumans backend proxy. Locally there's only an NDJSON file (`[observability.agent_tracing]`, `$C/config/schema/observability.rs`). | Traces go to `workspace/.traces/`. A later hand could ship them. | `LANGFUSE_HOST`, `LANGFUSE_PUBLIC_KEY` and `LANGFUSE_SECRET_KEY` export from the core. |
| G6 ([#8123](https://github.com/tinyhumansai/openhuman/issues/8123)) | **`openhuman-core call` runs in-process.** It invokes the method in a new process against the same storage (`$C/core/cli.rs:419`) instead of reaching the running `serve`. | `bin/oh` posts JSON-RPC to `127.0.0.1:7788` with `OPENHUMAN_CORE_TOKEN`. | `call --url` / `OPENHUMAN_CORE_URL` to send the call to a running core. |
| G7 ([#7000](https://github.com/tinyhumansai/openhuman/issues/7000)) | **Browser control is unproven headless.** It's the TinyComputer module from `tinyhumansai/tinydesktop` (the submodule is still at `vendor/tinycomputer`; see #7000). The core image ships without Chromium. | Teeny's image adds Chromium and sets `chrome_path`. Browser use stays behind `BROWSER_RESEARCH` and is read-only. | Close #7000. Optionally ship a `-browser` image variant. |
| G8 ([#8102](https://github.com/tinyhumansai/openhuman/issues/8102)) | **The Telegram channel's allowlist isn't persisted after pairing** (#8102). | Not used (see G3). | Fix #8102. |
| G9 ([#8124](https://github.com/tinyhumansai/openhuman/issues/8124)) | **`beforeSubmitPrompt` doesn't fire on `agent_chat`.** It runs only on the web-chat path (`$C/web_chat/ops/start_chat.rs:313`), so a hook can't screen text that a script sends to an agent. | `bin/job` calls `hands/guard inbound` before every draft. | Fire `beforeSubmitPrompt` for every turn entry point (`agent_chat`, channels, cron). |
| G10 ([#8125](https://github.com/tinyhumansai/openhuman/issues/8125)) | **No proven way for a hand to file a labelled capture into memory.** The Rust bot stores mentions, replies and posts with labels, a key and a person, and recalls by label, through the embed-only `Memory::store`/`search` (#7486). From a script there's only RPC: `memory_learn` and `memory_brain_ingest`. | `hands/record` uses `memory_learn` and checks recall quality in the shadow run. | Expose labelled `memory.store` and `memory.search` over RPC (the #7486 facade). |

## Deliberately not asked for

- **Letting cron agent turns take external actions** (an `[cron]
  unattended_tools` allowlist). The rule at
  `$C/agent/tinyagents/host/security_gate.rs:532` is the right one for a
  public agent, and the design relies on it. Scripts act and models draft.
- **Secret injection for `http_request`.** Hands hold the secrets, so the
  model never needs a credentialed HTTP tool.
