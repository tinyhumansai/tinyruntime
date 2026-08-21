# tinyruntime TinyBus Module

This package contains the native `tinyruntime` module for TinyBus module ABI
v1. Install only the archive matching the host operating system and
architecture.

The module claims `ai.tinyhumans.runtime.Runtime`, serves the object at
`/ai/tinyhumans/runtime/Runtime`, and provides `Languages`, `Resolve`,
`Execute`, and `PoolStats`. Every payload type, both interface names, both
object paths, and every member name are published as the `tinyruntime-bus`
crate, so a host names them from a library rather than by string literal.

## It needs provider modules

`tinyruntime` is a router and knows nothing about any language on its own. Load
at least one provider module alongside it:

- `tinyruntime-nodejs`, which claims `ai.tinyhumans.runtime.nodejs.Provider`
- `tinyruntime-python`, which claims `ai.tinyhumans.runtime.python.Provider`

A language whose provider is absent reports itself unavailable from `Languages`,
with a reason, rather than preventing the module from serving the others. Load
order does not matter: providers are contacted per call, not at setup.

## Configuration

The module accepts an optional JSON configuration at load time. Supplying
nothing routes the two first-party providers above, which is what almost every
host wants.

```json
{
  "providers": [
    { "language": "nodejs", "bus_name": "ai.tinyhumans.runtime.nodejs.Provider" },
    { "language": "python", "bus_name": "ai.tinyhumans.runtime.python.Provider" }
  ],
  "harness_dir": ""
}
```

`providers` is the routing table: any language identifier may be mapped to any
bus name, so a third-party provider needs no change to this module.
`harness_dir` is where worker scripts are written, defaulting to a directory
under the platform cache.

## What it writes to disk

Managed toolchains install under the cache directory each request names, or the
platform cache directory when a request names none. Worker harness scripts go
under `harness_dir`. Nothing else is written.

## Installing

The archive contains one `.so`, `.dylib`, or `.dll` plus `modules.toml`. Keep
those files together when copying them into a TinyBus module directory. The
allowlist binds the native library filename to its SHA-256 digest so TinyBus can
reject a missing, renamed, or modified artifact before initialization.

The GitHub release also publishes `checksum.toml` as a separate asset. TinyBus
checks that manifest before downloading and extracting the selected platform
archive. Install directly from a tagged release with:

```sh
tinybus modules load-github \
  https://github.com/tinyhumansai/tinyruntime/releases/tag/v0.1.0 \
  tinyruntime-0.1.0-ubuntu-24.04-x86_64.tar.gz \
  <archive-sha256>
```

TinyBus modules are trusted in-process code. Install release artifacts only
from a trusted source and restart the host after replacing a loaded module.
