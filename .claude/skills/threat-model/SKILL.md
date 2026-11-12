---
name: "threat-model"
description: "Repository-grounded threat modeling that enumerates trust boundaries, assets, attacker capabilities, abuse paths, and mitigations, and writes a concise Markdown threat model. Trigger only when the user explicitly asks to threat model a codebase or path, enumerate threats/abuse paths, or perform AppSec threat modeling. Do not trigger for general architecture summaries, code review, or non-security design work."
---

# Threat Model Source Code Repo

Deliver an actionable AppSec-grade threat model that is specific to the repository or a project path, not a generic checklist. Anchor every architectural claim to evidence in the repo and keep assumptions explicit. Prioritizing realistic attacker goals and concrete impacts over generic checklists.

## Quick start

1) Collect (or infer) inputs:
- Repo root path and any in-scope paths.
- Intended usage, deployment model, internet exposure, and auth expectations (if known).
- Any existing repository summary or architecture spec.
- Use prompts in `references/prompt-template.md` to generate a repository summary.
- Follow the required output contract in `references/prompt-template.md`. Use it verbatim when possible.

## Workflow

### 1) Scope and extract the system model
- Identify primary components, data stores, and external integrations from the repo summary.
- Identify how the system runs (server, CLI, library, worker) and its entrypoints.
- Separate runtime behavior from CI/build/dev tooling and from tests/examples.
- Map the in-scope locations to those components and exclude out-of-scope items explicitly.
- Do not claim components, flows, or controls without evidence.

### 2) Derive boundaries, assets, and entry points
- Enumerate trust boundaries as concrete edges between components, noting protocol, auth, encryption, validation, and rate limiting.
- List assets that drive risk (data, credentials, models, config, compute resources, audit logs).
- Identify entry points (endpoints, upload surfaces, parsers/decoders, job triggers, admin tooling, logging/error sinks).

### 3) Calibrate assets and attacker capabilities
- List the assets that drive risk (credentials, PII, integrity-critical state, availability-critical components, build artifacts).
- Describe realistic attacker capabilities based on exposure and intended usage.
- Explicitly note non-capabilities to avoid inflated severity.


### 4) Enumerate threats as abuse paths
- Prefer attacker goals that map to assets and boundaries (exfiltration, privilege escalation, integrity compromise, denial of service).
- Classify each threat and tie it to impacted assets.
- Keep the number of threats small but high quality.

### 5) Prioritize with explicit likelihood and impact reasoning
- Use qualitative likelihood and impact (low/medium/high) with short justifications.
- Set overall priority (critical/high/medium/low) using likelihood x impact, adjusted for existing controls.
- State which assumptions most influence the ranking.

### 6) Validate service context and assumptions with the user
- Summarize key assumptions that materially affect threat ranking or scope, then ask the user to confirm or correct them.
- Ask 1–3 targeted questions to resolve missing context (service owner and environment, scale/users, deployment model, authn/authz, internet exposure, data sensitivity, multi-tenancy).
- Pause and wait for user feedback before producing the final report.
- If the user declines or can’t answer, state which assumptions remain and how they influence priority.

### 7) Recommend mitigations and focus paths
- Distinguish existing mitigations (with evidence) from recommended mitigations.
- Tie mitigations to concrete locations (component, boundary, or entry point) and control types (authZ checks, input validation, schema enforcement, sandboxing, rate limits, secrets isolation, audit logging).
- Prefer specific implementation hints over generic advice (e.g., "enforce schema at gateway for upload payloads" vs "validate inputs").
- Base recommendations on validated user context; if assumptions remain unresolved, mark recommendations as conditional.

### 8) Run a quality check before finalizing
- Confirm all discovered entrypoints are covered.
- Confirm each trust boundary is represented in threats.
- Confirm runtime vs CI/dev separation.
- Confirm user clarifications (or explicit non-responses) are reflected.
- Confirm assumptions and open questions are explicit.
- Confirm that the format of the report matches closely the required output format defined in prompt template: `references/prompt-template.md`
- Write the final Markdown to `docs/security/threat-model.md`, or `docs/security/<dir-name>-threat-model.md` when you were asked to model a subpath. Never write it to the repo root.


## printCAD seed

Start the system model from `CLAUDE.md` and the `printcad-*` skills (crate map detail in `printcad-workbench-api`, `printcad-document-model`, `printcad-scripting-agents`), `docs/PLUGINS.md`, `docs/AI.md` and `docs/SCRIPTING.md`, then confirm each item in code. printCAD is a desktop application: the attacker is a file, a package, an agent, a script or a network answer the user brings in, not a remote caller. Treat these as candidate trust boundaries, not as findings:

- Workbench packages: wasm components run by wasmtime (`crates/workbenches/wb_wasm/src/{guest,host,jobs}.rs`): the package's `data/` preopened as `/data`, the network only when granted, a memory cap, an epoch deadline, `Access::None/Read/Write` per call, `doc.*` calls on the package's own kinds only, native helpers under the `helper` grant, a trap replacing the instance; the compiled component cached beside `bench.wasm`.
- Package installs: `.pcbench` tar archives (`package.rs`), GitHub releases checked against the digest (`remote.rs`), store indexes read from registries the user keeps (`store.rs`, `PRINTCAD_STORE_INDEX`, `install_listed` checking sha256), installs and updates on threads (`app/packages.rs`).
- AI agents: external agent processes over the Agent Client Protocol (`crates/agents/acp`), the MCP relay (`printcad --mcp`) on `$XDG_RUNTIME_DIR/printcad/mcp-<pid>.sock` (`app/mcp.rs`), `CommandSpec::read_only` and `AgentAccess`, approvals, `agent_check`, the rules and context an agent is told (`app/agent_context.rs`), attachments, `chats.json`.
- Scripts: the Lua engine (`crates/scripting`), files in `settings::scripts_dir()` bound to keys, menus and the palette, `printcad --script`, file commands taking a `path`, the slicer command the user configures (`slicer_command` runs a program on a written file), `open_with_system`.
- The document server: `printcad-serverd` on a unix socket under `$XDG_RUNTIME_DIR/printcad`, single client, the op log beside the file, the `DirectFiles` fallback, `PRINTCAD_SERVERD` overriding the binary, the app spawning it from its own directory.
- File readers on untrusted bytes: STEP, IGES, STL, OBJ, 3MF, PLY, glTF, VRML, DXF, PNG, JPEG, the `.prtcad` tar (and its `brep/` blobs and assets), `.pcbench`, linked parts read from other files (`app/links.rs`), `Replace shape` assets, recovery copies.
- Network: the update check (`app/updates.rs`, informs only), store fetches, package downloads; nothing else talks to the network.
- The user's data: the settings and data folder (`crates/settings`), recovery copies, `hole_cuts.json`, documents and the files beside them (`.oplog.jsonl`), export and slicer output under `$TMPDIR/printcad/`.
- CI and release: `.github/workflows/release.yml` publishes with `contents: write` (see gha-security-review); the Linux archive's `install.sh`.

## Risk prioritization guidance (illustrative, not exhaustive)
- High: pre-auth RCE, auth bypass, cross-tenant access, sensitive data exfiltration, key or token theft, model or config integrity compromise, sandbox escape.
- Medium: targeted DoS of critical components, partial data exposure, rate-limit bypass with measurable impact, log/metrics poisoning that affects detection.
- Low: low-sensitivity info leaks, noisy DoS with easy mitigation, issues requiring unlikely preconditions.

## References

- Output contract and full prompt template: `references/prompt-template.md`
- Optional controls/asset list: `references/security-controls-and-assets.md`

Only load the reference files you need. Keep the final result concise, grounded, and reviewable.
