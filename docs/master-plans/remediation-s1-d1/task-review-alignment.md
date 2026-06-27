## LENS: ALIGNMENT — write review-alignment.md

Verify the change honors the binding canon and the plan, and that it actually
closes the two findings it set out to. For each, give HONORED (file:line) or
DRIFTED (the divergence):

- **D-012 fully satisfied now?** Confirm EVERY upstream await that can block on
  remote behavior is inside the effective timeout: cold connect (spawn + serve
  handshake + initial list_all_tools) AND the dirty-refetch in ensure_fresh AND
  the existing call_tool. Is any blocking upstream await STILL unbounded? (This is
  the original S-1 finding — prove it is genuinely closed, not partially.)
- **D-007 / GOTCHA #16.** The new timeout wrapper must not introduce a registry
  map-lock or tools-lock held across an await. Verify the envelope sits where only
  the init guard + cloned Arc are in scope.
- **D-009 / GOTCHA #11/#14.** Containment semantics preserved on the timeout path.
- **D-005.** Public structured-error `code` strings unchanged; timeout reuses
  `upstream_timeout`; no new public wire code minted.
- **D-004.** Byte-faithful result passthrough untouched.
- **D-1 / GOTCHA #30 / ARCHITECTURE.md:97 / PRD Req 5.** The `cwd` field matches
  the documented spec: `Option`, `${VAR}` interpolation via the same resolver as
  env/headers, default = inherit aggregator CWD, ignored for HTTP, applied to the
  stdio child's working dir (not an arg rewrite). Does the implementation match
  the plan's stated decisions (reuse upstream_timeout; non-existent dir →
  upstream_connect_failed; HTTP accept-but-ignore)?
- **Scope.** Any change OUTSIDE S-1 + D-1? The plan's Scope-out list is binding
  (no O-*/D-2/H-* fixes, no rmcp bump, no new dep). Flag any creep.
- **Docs.** Should any binding doc (ARCHITECTURE/GOTCHA/DECISIONS/SECURITY) now be
  updated to reflect that S-1 is closed and cwd shipped? Note it for knowledge-sync
  (do not edit docs yourself).
