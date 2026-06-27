# Implementer task — Phase 5, plan **Phase 5: Token Benchmark & Generated README Figures**

Implement ONLY plan Phase 5: a `cargo bench --bench token_cost` target that
measures the proxy's token cost and OWNS the README token figures (generated,
never hand-edited — GOTCHA #26).

## Read first

- `master.md` §"Phase 5 — Token Benchmark And Generated README Figures".
- **`tests/integration/token_figures.rs`** — binding READ-ONLY contract (small;
  read it fully). It checks: (a) `Cargo.toml` has `[[bench]] name = "token_cost"`
  and `benches/token_cost.rs` exists; (b) `target/token-figures.generated.md`
  exists; (c) README between `<!-- fanin-token-figures:start -->` and
  `<!-- fanin-token-figures:end -->` (trimmed) EQUALS the generated file (trimmed).
- `src/server.rs` for the 3 static meta-tool definitions (`list_tools`,
  `get_tool_schema`, `invoke_tool`) whose `tools/list` JSON you measure.
- `README.md` (current token claims) + `ROADMAP.md` §"Release practice" (figures
  regenerated from the in-repo benchmark per release).
- Skills: `rust-general`.

## What to build

1. **`benches/token_cost.rs` + `[[bench]] name = "token_cost"` in `Cargo.toml`.**
   Use `harness = false` (a plain `fn main()` bench binary) so the bench can
   compute figures AND write a file — `cargo bench --bench token_cost` must run
   it and exit 0.
2. **Measure** two things (master P5.2): the permanent cost of the 3 meta-tool
   `tools/list` definitions, and a representative session (discovery via
   `list_tools` + a `get_tool_schema` lookup + an `invoke_tool` call). Define a
   deterministic token measure and document it inline. A heavy tokenizer dep
   (tiktoken etc.) conflicts with the anti-stack — prefer a lightweight,
   deterministic, documented approximation (e.g. a stable char/word-based
   estimate over the exact JSON-RPC payload bytes), clearly labeled as an
   estimate. The test only requires the README block to EQUAL the generated
   output — it does not validate the absolute number — but the figure must be a
   real, reproducible measurement of the actual payloads, not an invented
   constant (anti-gaming; GOTCHA #26).
3. **Write `target/token-figures.generated.md`** from the bench run (a small
   markdown snippet with the measured figures).
4. **Insert that exact snippet into `README.md`** between the
   `<!-- fanin-token-figures:start -->` / `end` markers (add the markers if the
   README has no such block yet), so the gate's exact-match passes. Provide a
   tiny updater path (a flag on the bench, e.g. `--update-readme`, or a small
   documented step) so the block is regenerated, never hand-edited.

## Constraints

- Scope: Phase 5 only. Do NOT change unrelated README prose beyond inserting/
  updating the marked token-figures block.
- Keep the dependency tree small (anti-stack / Phase 6 `cargo deny` + <10MB
  binary budget). Note any dep you add. rmcp stays `=1.8.0`.
- Tests read-only. End green on `cargo fmt`, `cargo clippy --all-targets
  -- -D warnings`, `cargo bench --bench token_cost` (exit 0, writes the
  generated file), and `cargo test --test integration token_figures`.
- Determinism: the generated figures must be STABLE across runs (no timestamps,
  no machine-specific values in the marked block) — otherwise the README block
  and a fresh bench run would diverge and the gate would flake.

## Return

`impl-p5-result.md`: the bench design + the token measure you chose (and why it
is deterministic + anti-stack-friendly), the files changed, any dep added, and
the exact figures generated.
