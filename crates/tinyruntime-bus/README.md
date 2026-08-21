# tinyruntime-bus

Every type that crosses the tinyruntime boundary, and the names of the members
that carry them.

`tinyruntime` ships as a loadable module so a host does not compile the
implementation: `crates/tinyruntime` is built as a `cdylib` and exports one
object. A host can load that binary but cannot `use` anything out of it, so the
payload vocabulary has to be published as an ordinary library. This is it.

Two dependencies, both pure Rust: `serde` and `serde_json`.

## Two interfaces, one contract

This crate defines both halves of the system, because they are two views of one
agreement and splitting them would let the halves drift.

`INTERFACE` is what a **host** calls: resolve a language, run something on it,
ask what is available. `PROVIDER_INTERFACE` is what the **router** calls: the
five questions only a language module can answer. The router is a consumer of
the second exactly as a host is a consumer of the first.

Every provider implements `PROVIDER_INTERFACE` — that is what makes them
interchangeable — and claims its own well-known bus name, because two peers
cannot hold the same one. Each therefore serves at its own object path, derived
from that bus name by `object_path_for`: `tinybus_module!` builds a module's
manifest path the same way, so a provider serving anywhere else would ship a
manifest that disagreed with the object it exports.

| module      | what it holds                                                 |
| ----------- | ------------------------------------------------------------- |
| `names`     | both interfaces, both object paths, one constant per member    |
| `language`  | the routing key that selects a provider                        |
| `settings`  | what a host asks for, carried on every request                 |
| `resolve`   | asking for a runtime, and being told which one you got         |
| `provision` | how a provider describes a toolchain it does not install       |
| `harness`   | the worker script a provider ships and the router runs         |
| `exec`      | running code, and what came back                               |
| `pool`      | warm-worker tuning and counters                                |
| `version`   | `CONTRACT_VERSION` and the bind rule both sides apply          |

## What is deliberately not here

**No behaviour.** No process is spawned, no byte is downloaded, and no path is
touched by anything in this crate. A payload type describes what a frame
carries, not what a module does with it.

**No transport.** This crate does not depend on `tinybus` and holds no
connection, client, or codec. A host already owns its connection — its reconnect
policy, its timeouts, its tracing — and the useful part is the vocabulary, not
another wrapper around it.

That is also structural rather than only a preference: `tinybus` is vendored as
a submodule whose manifest inherits from its own nested `[workspace.package]`.
A crate that every workspace member — and every provider repository — can depend
on has to stay transport-free.

## This crate sits underneath the implementations, not beside them

`tinyruntime` depends on this crate and re-exports all of it, and so does each
provider crate. `tinyruntime::ExecRequest` and `tinyruntime_bus::ExecRequest` are
the *same type*, not structural twins.

Defining a parallel set of payload types for hosts would mean a conversion at
every call site that nothing checks. One definition, here, at the bottom.

So: a module author depends on `tinyruntime` and gets behaviour and vocabulary.
A host, or a provider, depends on `tinyruntime-bus` and gets vocabulary alone.
