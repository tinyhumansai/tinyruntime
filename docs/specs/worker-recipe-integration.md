# Follow-up: Python preparation and TinyJuice ML recipes

This generic slice accepts `WorkerPrepare { handle, plan }`. The plan contains
bounded script bytes, an explicit absolute executable, arguments before the
installed script, explicit environment, ordered backend ids, bounded native
preparation commands and startup/request/idle settings. The router executes
preparation in its owned per-handle directory and appends the installed script
path to the final worker command. There is no implicit executable fallback,
provider recipe member, package resolver, persistent provisioning marker or
cached-model interpretation in this slice.

The root still owns the following implementation until later adapter switches:

| Existing source | Decision/mechanism | Required owner |
| --- | --- | --- |
| `crates/openhuman-core/src/runtime/python_server/server.rs` | Select enabled backends and map product configuration/idle policy | OpenHuman host adapter |
| Same file | Resolve interpreter, build launch environment, install script, process-wide `ServerSlot` composition | Python provider describes language setup; generic router installs/spawns; host caches opaque worker handles |
| `crates/openhuman-core/src/runtime/python_server/kompress.rs` | Venv location/layout and `-m venv`/pip invocation conventions | Python provider describes declarative setup; router executes it |
| Same file | Torch CPU index, transformers/tokenizers dependencies, model pre-download script, model-specific readiness marker | TinyJuice supplies ML recipe/data; router performs generic bounded provisioning |
| Same file and server environment helper | Model/device/ratio/input limit, HF cache/offline/telemetry environment | TinyJuice recipe semantics; host supplies approved config values |
| `vendor/tinyruntime/crates/tinyruntime-pyserver/src/server.py` | Backend dispatch, tokenizer/model loading and Kompress algorithm | TinyJuice owns ML worker recipe/source |

`tinyruntime-python` remains describe-only: no installation, archive downloads,
cache writes or worker spawning. A follow-up contract must define declarative
preparation vocabulary in router-owned `tinyruntime-bus`, publish it canonically,
then update the provider's pinned contract separately. Do not duplicate payload
structs or edit the provider's vendored runtime checkout to bypass that ordering.

Preserve existing cache locations and readiness marker contents, installation
locking, stable backend order, cached provisioning and offline model-load flags.
The generic manager's transient script directory is not yet the persistent
venv/model cache abstraction; design a bounded, owned provisioning/cache recipe
before switching hosts. Python-specific path and package conventions belong to
the provider, while model readiness and compressor knowledge belong to TinyJuice.

The two protocols stay distinct. Warm code jobs use authenticated loopback
`JobRequest { code, cwd, timeout_ms }`. Persistent backend workers use stdio
`ReadyLine`, `ServerRequest { id, method, params }` and `ServerResponse`; do not
translate a method request into an Execute source string. The legacy pyserver
library still executes its old mechanisms; this slice only moves its wire/status
vocabulary to one shared definition. Its removal is a later independently
reviewed host/module transition.

TinyJuice's turn-bound ML callback remains host registered, but its eventual
executor must call worker bus operations. No linked PythonServer or root pip/model
fallback may survive that switch. Publish provider/TinyJuice owning PRs before
host adapters; require compatible released native artifacts before gitlink and
contract-only dependency admission. This document records remaining work, not
permission to expand the frozen generic manager slice.
