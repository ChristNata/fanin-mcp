//! benches/token_cost.rs — Phase 5 token-cost benchmark (harness=false).
//!
//! This is a plain `fn main()` binary (not the libtest harness) so it can
//! compute deterministic figures and write `target/token-figures.generated.md`.
//!
//! Token measure (deterministic, no heavy deps, anti-stack):
//!   bytes = exact UTF-8 length of the compact JSON-RPC payload (no pretty).
//!   tokens ≈ (bytes + 3) / 4   // stable 4-bytes-per-token ceiling
//!
//! This matches the documented "estimate" approach in the Phase 5 spec:
//! a lightweight, reproducible count over the exact wire bytes we emit.
//! It is stable across runs/machines (no timestamps, no platform values).
//!
//! Measured surfaces:
//!   1. Permanent meta-tools `tools/list` cost (the 3-tool array the server
//!      advertises for every `tools/list` call from the client).
//!   2. Representative session:
//!        - one `tools/list` response (the meta-tools)
//!        - discovery result from `list_tools` (3 rows)
//!        - schema result from `get_tool_schema` (compact schema JSON)
//!        - invoke result text from `invoke_tool` (representative rows JSON)
//!        - plus the tiny request argument payloads for the three calls.
//!
//! The benchmark always writes the generated snippet and exits 0 so that
//! `cargo bench --bench token_cost` and the integration gate can rely on it.
//!
//! Usage:
//!   cargo bench --bench token_cost
//!   cargo bench --bench token_cost -- --update-readme   # also patches README
//!
//! The generated block is inserted between the exact markers in README.md:
//!   <!-- fanin-token-figures:start -->
//!   ... exact content of target/token-figures.generated.md ...
//!   <!-- fanin-token-figures:end -->

use std::env;
use std::fs;
use std::path::Path;

const META_TOOLS_LIST_RESP: &str = r#"{"tools":[{"name":"list_tools","description":"Lists the tools available through this aggregator, grouped by server, with one-line descriptions. Call this once to see what's connected; pass server to fetch a single server's tools.","inputSchema":{"type":"object","properties":{"server":{"type":"string"}}}},{"name":"get_tool_schema","description":"Get the full input schema for a tool. Format: server__tool (e.g. postgres__query).","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}},{"name":"invoke_tool","description":"Call a tool by server__tool name with arguments.","inputSchema":{"type":"object","properties":{"name":{"type":"string"},"arguments":{"type":"object"}},"required":["name","arguments"]},"annotations":{"readOnlyHint":false,"destructiveHint":true,"openWorldHint":true}}]}"#;

fn token_estimate(bytes: usize) -> usize {
    // Deterministic ceiling: 4 bytes ≈ 1 token. Matches common LLM tokenizers
    // for JSON-ish English at a stable, platform-independent granularity.
    bytes.div_ceil(4)
}

fn main() {
    // Permanent cost of the meta-tools definition (what `tools/list` returns).
    let meta_bytes = META_TOOLS_LIST_RESP.len();
    let meta_tokens = token_estimate(meta_bytes);

    // Representative session payloads (compact JSON, exact bytes we would emit).
    // Discovery result (3 rows) — mirrors what `list_tools` returns in practice.
    let discovery_json = r#"[{"server":"postgres","tool":"query","name":"query","description":"Execute SQL."},{"server":"obsidian","tool":"search","name":"search","description":"Search notes."},{"server":"morph","tool":"complete","name":"complete","description":"Code morphing."}]"#;
    let discovery_bytes = discovery_json.len();
    let _discovery_tokens = token_estimate(discovery_bytes);

    // Schema result for a representative tool.
    let schema_json = r#"{"type":"object","properties":{"sql":{"type":"string","description":"The SQL to run"}},"required":["sql"]}"#;
    let schema_bytes = schema_json.len();
    let _schema_tokens = token_estimate(schema_bytes);

    // Invoke result text (the Content::text value inside the result).
    let invoke_text = r#"{"rows":42,"elapsed_ms":17,"sample":[{"id":1,"name":"foo"}]}"#;
    let invoke_bytes = invoke_text.len();
    let _invoke_tokens = token_estimate(invoke_bytes);

    // Request argument payloads sent by client (compact).
    let req_list_tools = "{}".to_string();
    let req_get_schema = r#"{"name":"postgres__query"}"#.to_string();
    let req_invoke = r#"{"name":"postgres__query","arguments":{"sql":"SELECT 1"}}"#.to_string();
    let req_bytes = req_list_tools.len() + req_get_schema.len() + req_invoke.len();
    let _req_tokens = token_estimate(req_bytes);

    let session_bytes = meta_bytes + discovery_bytes + schema_bytes + invoke_bytes + req_bytes;
    let session_tokens = token_estimate(session_bytes);

    let report = format!(
        "Meta-tools (`tools/list`): {meta_tokens} tokens (~{meta_bytes} bytes)\n\
         Representative session (list + schema + invoke + requests): {session_tokens} tokens (~{session_bytes} bytes)\n\
         \n\
         Token measure: exact compact-JSON UTF-8 byte length, then (bytes + 3) / 4.\n\
         Deterministic; no external tokenizer; stable across runs and platforms.\n\
         Generated by `cargo bench --bench token_cost` (see benches/token_cost.rs)."
    );

    // Always write the generated file (the contract expects it after bench run).
    let generated_path = Path::new("target/token-figures.generated.md");
    fs::write(generated_path, &report).expect("write target/token-figures.generated.md");

    // Optional: also patch README.md when invoked with --update-readme.
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--update-readme") {
        update_readme(&report);
    }

    // Print human-readable summary to stderr (stdout is reserved for harness in other benches).
    eprintln!("token_cost benchmark complete");
    eprintln!("{report}");
    // Exit 0 so `cargo bench --bench token_cost` succeeds for the gate.
}

const START: &str = "<!-- fanin-token-figures:start -->";
const END: &str = "<!-- fanin-token-figures:end -->";

fn update_readme(report: &str) {
    let readme_path = Path::new("README.md");
    let mut content = fs::read_to_string(readme_path).expect("README.md readable");

    if !content.contains(START) || !content.contains(END) {
        // Insert markers after the first paragraph that mentions the ~600 token claim
        // or at end of the "How the LLM uses it" section. Keep it minimal.
        // We look for a stable anchor line and insert a block after it.
        let anchor = "A session that runs one query costs roughly 2.5K tokens of tool-related context vs. 30–60K with direct configs. (Token figures are verified by an in-repo benchmark — see MVP Phase 5.)\n";
        if let Some(pos) = content.find(anchor) {
            let insert_at = pos + anchor.len();
            let block = format!("\n{START}\n{report}\n{END}\n");
            content.insert_str(insert_at, &block);
        } else {
            // Fallback: append a block at EOF with markers.
            content.push_str(&format!("\n\n{START}\n{report}\n{END}\n"));
        }
    } else {
        // Replace the region between markers (exclusive) with the fresh report.
        if let (Some(s), Some(e)) = (content.find(START), content.find(END)) {
            if s < e {
                let before = &content[..s + START.len()];
                let after = &content[e..];
                content = format!("{before}\n{report}\n{after}");
            }
        }
    }

    fs::write(readme_path, content).expect("write updated README.md");
}
