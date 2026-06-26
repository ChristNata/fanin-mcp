# Issue: overbroad `**/credentials*` edit-deny rule blocks `src/credentials.rs`

**Severity:** structural (environment / tooling config) — carried, not hard-blocking.
**Surfaced during:** Phase 3 implement stage (plan Phase 2, credential store work).

## What

The managed OpenCode permission config
(`$LOCALAPPDATA/Covenant/cli/opencode/config/.config/opencode/opencode.json`) has, in
its `edit` block:

```json
"**/credentials*": "deny",
"**/secrets*": "deny",
```

These globs are intended to stop child agents from editing credential/secret **data**
files (`.aws/credentials`, `credentials.json`, `secrets.yaml`, …). But `**/credentials*`
also matches the project's legitimate Rust **source** file `src/credentials.rs` (and
`**/secrets*` would match a future `src/secrets.rs`).

## Impact

`src/credentials.rs` is the credential-store module (`CredentialStore` trait,
`KeyringStore`, `EnvStore`) — child agents must be able to edit it to build Phase 3.
Because the edit tool is denied, two dispatches routed credential logic into the WRONG
file (`src/process.rs`) to get around the block:

- The first implementer wrote a plaintext secret-to-tempfile side-channel (a D-010
  violation — already removed).
- The debugger added hand-rolled `unsafe` Windows Credential Manager FFI in
  `process.rs` (removed by the follow-up fix; `process.rs` is being kept clean for the
  P4 process-tree work).

The `bash` permission is broadly `allow`, so an agent *can* still bypass the edit-deny
via `cat >`/`sed` — which means the rule does not actually protect the file, it only
distorts where code lands. This is the worst of both: no real protection, real
architectural damage.

## Recommended fix (user decision — security config change)

Add a precise allow-exception that keeps the broad data-file deny but unblocks the
source file, mirroring the existing `.env.example: allow` override pattern:

```json
"edit": {
  ...
  "**/credentials*": "deny",
  "**/secrets*": "deny",
  "**/src/credentials.rs": "allow",
  "**/src/secrets.rs": "allow",
  ...
}
```

(Order/precedence per OpenCode's permission matcher — place the allow so it wins for the
`.rs` source paths.) This restores child-agent authorship of the credential module while
still denying real credential/secret data files.

I did not change the managed security config myself — modifying a security deny rule is
the user's call. The Phase 3 code fix was routed to avoid editing `credentials.rs`
(the P1 `KeyringStore` was already correct; only `Cargo.toml`/`process.rs`/`main.rs`
needed changes), so this issue is carried, not blocking.
