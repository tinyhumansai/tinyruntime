# Persistent cache recipe implementation

Specification: ../specs/persistent-cache-recipes.md.

1. Add canonical serde-only recipe/scope/entry/artifact/request vocabulary and
   WorkerPrepareCached, bump router wire revision to 1.2. Pin serialization and
   unchanged provider compatibility in sibling fixtures.
2. Add handle-relative scoped filesystem operations, bounded no-follow reads,
   owned staging/promotion/cleanup and nonblocking cross-process cache locking.
   Pin credential/traversal/symlink swap refusals and cleanup isolation first.
3. Integrate the recipe session into the existing detached worker supervisor;
   retain the session until native preparation reaping and staging cleanup finish.
   Add cache hit/fault/parallel/restart/cancellation/public retry fixtures.
4. Wire host scope configuration and real bus dispatch. Document trusted recipe
   limits and the remaining describe-only provider/model ownership.
5. Run owner default/all format, clippy, build, test, docs, pure-contract closure,
   per-file coverage and actual native member verification. Freeze exact commit,
   full and scoped diff/evidence for independent review before upstream PR update.
