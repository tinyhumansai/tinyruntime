# Persistent worker implementation

1. Pin existing JSONL/status serde forms in contract regressions; move identical
   vocabulary into the bus and reexport it through legacy pyserver paths.
2. Add reserve/prepare/start/request/status/stop/shutdown DTOs and names. Preserve
   existing member arities and distinguish router/provider version requirements.
3. Add bounded framing and supervised native process primitives with fault,
   malformed-frame, total-deadline and descendant-cleanup regressions.
4. Add detached per-resource actors and manager reservation/retirement state.
   Pin abandoned startup/request/stop futures, repeated workflow recovery,
   queued stop, terminal shutdown and no late publication with barrier fixtures.
5. Wire the real module service, assert manifest/names and mock bus dispatch.
6. Verify fmt/clippy/build/tests/docs default/all, independent bus purity,
   changed source coverage and the actual native module. Freeze for independent
   review before push/PR. Provider/model recipes and host switches stay separate.
