//! Phase 5 P5 — README token figures are generated, not hand-maintained.

use std::path::Path;

const START: &str = "<!-- fanin-token-figures:start -->";
const END: &str = "<!-- fanin-token-figures:end -->";

#[test]
fn readme_token_figure_markers_match_benchmark_generated_output_exactly() {
    let readme = std::fs::read_to_string("README.md").expect("README.md must be readable");
    let start = readme
        .find(START)
        .unwrap_or_else(|| panic!("README.md must contain token figure start marker {START}"));
    let end = readme
        .find(END)
        .unwrap_or_else(|| panic!("README.md must contain token figure end marker {END}"));
    assert!(start < end, "README token markers must be ordered");
    let marked = &readme[start + START.len()..end];

    let generated_path = Path::new("target/token-figures.generated.md");
    assert!(
        generated_path.exists(),
        "benchmark helper must write target/token-figures.generated.md before this gate; run cargo bench --bench token_cost"
    );
    let generated =
        std::fs::read_to_string(generated_path).expect("generated token figures must be readable");
    fn normalize(s: &str) -> String {
        s.replace("\r\n", "\n").replace('\r', "\n")
    }
    assert_eq!(
        normalize(marked).trim(),
        normalize(&generated).trim(),
        "README token figures must match benchmark-generated output exactly; run the token figure updater, never hand-edit the block"
    );
}

#[test]
fn cargo_bench_token_cost_target_is_declared() {
    let cargo = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml readable");
    assert!(
        cargo.contains("[[bench]]") && cargo.contains("name = \"token_cost\""),
        "Cargo.toml must declare [[bench]] name = \"token_cost\" so cargo bench --bench token_cost is available"
    );
    assert!(
        Path::new("benches/token_cost.rs").exists(),
        "benches/token_cost.rs must exist for the gate target"
    );
}
