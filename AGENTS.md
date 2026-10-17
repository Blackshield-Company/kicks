# Kicks — Agent Brief

Guitar workstation (Tauri 2 + Svelte + Rust): amp/effects toolchain companion for a working guitarist. Real Tauri IPC and Firefox E2E already landed — the bones are proven. Repo has existing code: READ IT before writing anything. Match existing conventions.

## Your mission: next milestone
- [ ] Run the existing test suite + build first; report actual state before changing anything
- [ ] Preset system: save/load named effect-chain presets to disk (JSON), list/load in UI
- [ ] Tuner: keep it simple — chromatic tuner display wired to the existing audio input path (if audio path isn't ready, build the UI against a mock and mark the wiring TODO explicitly)
- [ ] Fix anything the test/build pass surfaces

## Stack
Existing Tauri 2 + Svelte + Rust codebase. Do not introduce new frameworks.

## House Rules (non-negotiable)

- **Theme:** Blackshield (steel+blood: bg #101014, surface #16161C, text #D8D3C8, accent #C1121F) is the DEFAULT everywhere. Other palettes (incl. Synthwave '84) stay opt-in/selectable. See the blackshield-theme skill for the full token set.
- **CLI naming:** the binary is the bare project name. Never a `-cli` suffix.
- **Commits:** conventional commits (`feat:`, `fix:`, `chore:`...). Local commits are fine.
- **NEVER:** push to a remote, create GitHub remotes, force-push, rewrite/delete tags, or touch `.env`/credential files.
- **No support/donation links** (BuyMeACoffee etc.) — the user removed those deliberately.
- **Done means verified:** build it AND run it before claiming completion. No stubs-as-deliverables.
