//! Config model and startup validation — Phase 1 wire-level tests.
//!
//! Covers master Success Criteria 1 (valid config starts), 2 (invalid server
//! names rejected), 3 (unknown namespace rejected), and 19 (no stdout
//! diagnostics), plus Phase 1 sub-phase Success Criteria 1–5.
//!
//! Every test spawns the built `fanin-mcp` binary over stdio with a `--config`
//! pointing at a temp TOML file built by `common::fixtures::ConfigBuilder`.
//! The TOML schema encoded in the fixtures is the binding contract the
//! implementer must parse (see `tests.md` §Config schema).
//!
//! Startup failures must happen BEFORE `serve(stdio())` begins: the process
//! exits non-zero and writes NOTHING to stdout (GOTCHA #1). A config error
//! that leaks onto stdout corrupts the JSON-RPC stream the client is about to
//! open. We assert on the observable effect — empty stdout + non-zero exit —
//! not on a return value.

use std::time::Duration;

use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Ceiling for asserting a startup-failure does NOT serve. A correct
/// implementation exits before serving, so `initialize` gets no response
/// (EOF / connection close) well within this window. The Phase 0 stub, by
/// contrast, IGNORES `--config` and serves `initialize` happily — so this
/// assertion fails RED against the stub (the stub returns a valid
/// initialize result) and passes GREEN once the implementer rejects the
/// config before serving. This is the load-bearing distinction that makes
/// the negative tests meaningful.
const NO_SERVE_DEADLINE: Duration = Duration::from_secs(2);

/// Assert that the spawned child does NOT come up as an MCP server — i.e.
/// `initialize` does NOT return a valid result within [`NO_SERVE_DEADLINE`].
/// A correct startup-failure path exits before `serve(stdio())` begins, so
/// the initialize request either gets no response (the child has exited) or
/// the connection is closed. A stub that ignores the invalid config and
/// serves anyway returns a valid initialize result — failing this assertion.
///
/// Also asserts no bytes are written to stdout (GOTCHA #1): a config error
/// that leaks onto stdout corrupts the would-be JSON-RPC stream.
async fn assert_does_not_serve(child: &mut common::JsonRpcChild) {
    // Send initialize. A correct impl has already exited; the write may fail
    // or succeed (buffered). Either way, we do not expect a valid response.
    let _ = child
        .send_request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "phase1-test", "version": "0.0.0" },
            }),
        )
        .await;

    // Read raw stdout within the deadline. A correct impl produces NO bytes
    // (it exited before serving). A stub that serves produces an initialize
    // RESPONSE (valid JSON with a result field) — that is the failure signal.
    let stdout = child.drain_stdout_raw(NO_SERVE_DEADLINE).await;
    assert!(
        stdout.is_empty(),
        "startup failure must not write to stdout (GOTCHA #1) and must not \
         serve initialize; got {} bytes: {:?}",
        stdout.len(),
        String::from_utf8_lossy(&stdout)
    );
}

/// Master criterion 1 / P1.SC1: a valid Phase 1 TOML config with one stdio
/// server and a default namespace starts the aggregator successfully. The
/// observable effect is that `initialize` returns a well-formed result and
/// `tools/list` returns exactly the three static meta-tools — proving the
/// config loaded and the server came up without writing to stdout.
#[tokio::test]
async fn valid_phase1_config_starts_aggregator() {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;

    let init = common::initialize(&mut child).await;
    assert!(
        init.get("serverInfo").is_some(),
        "valid config must let initialize return serverInfo"
    );
    let name = init
        .get("serverInfo")
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or_default();
    assert_eq!(name, "fanin-mcp", "server must still name itself fanin-mcp");

    // tools/list must still return exactly the three static meta-tools — the
    // config load path must not have added or removed any downstream tools.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list after valid config");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    crate::common::expectations::assert_exact_meta_tools(tools);

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 2 / P1.SC2: a server name outside `[a-z0-9-]+` fails
/// startup BEFORE serving. The observable effect is: `initialize` does NOT
/// return a valid result (the process exits before serving) and NO bytes are
/// written to stdout (the error routed to stderr/tracing only).
///
/// `UPPERCASE` is the canonical invalid-char case (uppercase letters are
/// outside the allowed set). The Phase 0 stub ignores `--config` and serves
/// `initialize` — so this test fails RED against the stub (the stub returns
/// a valid initialize response) and passes GREEN once the implementer
/// validates the server name before serving.
#[tokio::test]
async fn invalid_server_name_uppercase_fails_startup_before_serving() {
    let cfg = fx::ConfigBuilder::new()
        .server_name("UPPERCASE")
        .namespace_servers(["UPPERCASE"])
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    assert_does_not_serve(&mut child).await;
    child.into_guard().shutdown().await.ok();
}

/// Master criterion 2 / P1.SC2: a server name containing `__` fails startup
/// before serving. `__` in a server name makes `server__tool` parsing
/// ambiguous (GOTCHA #15); config load rejects it. Same observable contract:
/// no initialize response + empty stdout.
#[tokio::test]
async fn server_name_with_double_underscore_fails_startup() {
    let cfg = fx::ConfigBuilder::new()
        .server_name("my__db")
        .namespace_servers(["my__db"])
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    assert_does_not_serve(&mut child).await;
    child.into_guard().shutdown().await.ok();
}

/// Master criterion 2 edge: a server name with an underscore (single `_`,
/// outside `[a-z0-9-]+`) fails startup. Underscores are not in the allowed
/// character set; only hyphens are. This is distinct from the `__` case.
#[tokio::test]
async fn server_name_with_single_underscore_fails_startup() {
    let cfg = fx::ConfigBuilder::new()
        .server_name("my_db")
        .namespace_servers(["my_db"])
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    assert_does_not_serve(&mut child).await;
    child.into_guard().shutdown().await.ok();
}

/// Master criterion 3 / P1.SC4: an unknown `--namespace` fails startup
/// before serving. The config defines `default` only; requesting `nope`
/// must exit before serving. Same observable contract: no initialize
/// response + empty stdout.
#[tokio::test]
async fn unknown_namespace_fails_startup_before_serving() {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), Some("nope")).await;
    assert_does_not_serve(&mut child).await;
    child.into_guard().shutdown().await.ok();
}

/// Master criterion 3 / P1.SC3: a config with NO `[namespaces.default]`
/// table still starts when `--namespace` is omitted — the default namespace
/// is preserved. The implementer may synthesize it or require it; this test
/// pins the behavior: omitting `--namespace` must work against a config that
/// declares a non-`default` namespace only if that non-default namespace is
/// selected. Here we declare only `default` and omit the flag.
#[tokio::test]
async fn default_namespace_preserved_when_flag_omitted() {
    let cfg = fx::ConfigBuilder::new()
        .namespace_name("default")
        .namespace_servers(["probe"])
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;

    let init = timeout(Duration::from_secs(10), common::initialize(&mut child))
        .await
        .expect(
            "initialize must return within 10s when the default namespace is selected by omission",
        );
    assert!(
        init.get("serverInfo").is_some(),
        "default namespace must start the aggregator"
    );

    child.into_guard().shutdown().await.ok();
}

/// P1.SC5 / master criterion 19: a config-validation failure never emits to
/// stdout. This is the explicit side-effect assertion for GOTCHA #1 on the
/// startup path — `assert_does_not_serve` already enforces empty stdout per
/// case; this test re-asserts it as a standalone contract using a different
/// invalid input (a name with spaces) so a regression that moves the leak to
/// a different failure path still fails here.
#[tokio::test]
async fn no_config_failure_writes_to_stdout() {
    let cfg = fx::ConfigBuilder::new()
        .server_name("Bad Name With Space")
        .namespace_servers(["Bad Name With Space"])
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    assert_does_not_serve(&mut child).await;
    child.into_guard().shutdown().await.ok();
}

/// Master criterion 2 edge: a config with a missing `command` field (a
/// stdio server declared without the spawn command) fails startup before
/// serving. This is a malformed-config path, distinct from the invalid-name
/// path. Same observable contract: no initialize response + empty stdout.
#[tokio::test]
async fn stdio_server_without_command_fails_startup() {
    // Build a config with no command by appending a raw server table that
    // lacks the `command` key.
    let cfg = fx::ConfigBuilder::new()
        .extra_raw(
            "[servers.nocmd]\ntransport = \"stdio\"\nargs = []\n[namespaces.default]\nservers = [\"nocmd\"]\n",
        )
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    assert_does_not_serve(&mut child).await;
    child.into_guard().shutdown().await.ok();
}

/// Master criterion 1 edge: a config whose single server points at a
/// non-existent command still STARTS the aggregator — the upstream spawn is
/// lazy (P1.SC1 / criterion 11) and happens only on the first targeting
/// meta-tool call. `initialize` + `tools/list` must succeed without ever
/// trying to spawn the missing binary.
///
/// This is the load-bearing laziness assertion on the config path: a config
/// load that eagerly validated the command's existence would fail here.
#[tokio::test]
async fn config_with_nonexistent_command_still_starts_due_to_lazy_spawn() {
    let cfg = fx::ConfigBuilder::new()
        .command("/this/binary/does/not/exist/anywhere")
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;

    let init = common::initialize(&mut child).await;
    assert!(
        init.get("serverInfo").is_some(),
        "lazy spawn: initialize must succeed even if the upstream command is missing"
    );

    // tools/list must succeed and stay static — no upstream contact on the
    // discovery path (D-003, GOTCHA #7).
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list with missing upstream command");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    crate::common::expectations::assert_exact_meta_tools(tools);

    child.into_guard().shutdown().await.ok();
}

/// A1.SC1 / CA-004 data-model half: a configured description is retained on
/// the deserialized production `ServerConfig` rather than merely tolerated as
/// an unknown TOML key.
#[test]
fn server_description_deserializes_when_present() {
    let fixture = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("probe").with_description("Code navigation"))
        .namespace(fx::NamespaceEntry::new("default", ["probe"]))
        .write();

    let config = crate::config_model::load_and_validate(&fixture.path, "")
        .expect("description-bearing config must deserialize and validate");
    let server = config
        .servers
        .get("probe")
        .expect("fixture must contain the probe server");

    assert_eq!(server.description.as_deref(), Some("Code navigation"));
}

/// A1.SC2: omitting the additive key preserves the old fixture shape and
/// defaults the production field to `None`.
#[test]
fn server_description_defaults_to_none_when_omitted() {
    let fixture = fx::ConfigBuilder::new().write();

    let config = crate::config_model::load_and_validate(&fixture.path, "")
        .expect("existing description-free fixture must deserialize and validate");
    let server = config
        .servers
        .get("probe")
        .expect("fixture must contain the probe server");

    assert_eq!(server.description, None);
}

/// A1.SC3: startup validation is description-neutral. The same valid config
/// passes `load_and_validate` both with the key present and with it omitted.
#[test]
fn server_description_is_optional_for_validation() {
    let with_description = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("probe").with_description("Code navigation"))
        .namespace(fx::NamespaceEntry::new("default", ["probe"]))
        .write();
    let without_description = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("probe"))
        .namespace(fx::NamespaceEntry::new("default", ["probe"]))
        .write();

    let with_description = crate::config_model::load_and_validate(&with_description.path, "")
        .expect("validation must accept description");
    let without_description = crate::config_model::load_and_validate(&without_description.path, "")
        .expect("validation must not require description");

    assert_eq!(
        (
            with_description.servers["probe"].description.as_deref(),
            without_description.servers["probe"].description.as_deref(),
        ),
        (Some("Code navigation"), None)
    );
}
