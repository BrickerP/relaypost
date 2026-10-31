# RelayPost engineering rules

- Do not run cargo, rustc, local builds, or local tests. Push to the implementation branch and use GitHub Actions for compilation, formatting and tests.
- The project is independent of Miaoda; never copy company code, credentials or data into it.
- Implement the approved plan in docs/PROJECT_PLAN.zh-CN.md and the shared API in docs/IMPLEMENTATION_CONTRACT.md.
- Use one Rust core for CLI and native UI. No NNCP runtime, Go sidecar, network service, or remote command execution.
- Secrets must never be logged or accepted as plaintext command-line arguments. Tests use clearly labelled synthetic fixtures.
- Never claim Kiro participation unless an actual Kiro session performed the recorded work. .kiro steering/specification inputs are not usage evidence by themselves.
- Do not create or update a CodeGraph index. Use it only if the user has already indexed the repository.
- Other contributors are working in parallel. Modify only your owned files and preserve their changes.

