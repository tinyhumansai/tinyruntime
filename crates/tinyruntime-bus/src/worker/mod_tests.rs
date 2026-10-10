//! Existing persistent worker wire defaults and status representation.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures assert failures by panicking, matching existing suites"
)]
use super::*;

#[test]
fn legacy_worker_wire_defaults_and_status_are_preserved() {
    let ready: ReadyLine = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(!ready.ready);
    assert_eq!(ready.protocol, None);
    assert!(ready.backends.is_empty());
    let request: ServerRequest =
        serde_json::from_value(serde_json::json!({"id":"7","method":"alpha.run"})).unwrap();
    assert_eq!(request.params, serde_json::Value::Null);
    let response: ServerResponse = serde_json::from_value(serde_json::json!({"id":null})).unwrap();
    assert!(!response.ok);
    assert_eq!(response.result, None);
    assert_eq!(
        serde_json::to_value(ServerStatus::disabled("idle")).unwrap(),
        serde_json::json!({"enabled":false,"running":false,"backends":[],"message":"idle"})
    );
}

#[test]
fn declarative_cache_recipe_keeps_exact_bytes_and_entry_kinds_on_wire() {
    let wire = serde_json::json!({"scope":"cache","artifacts":[{"path":"source","bytes":[0,255]}],"steps":[],"required":[{"path":"bin","kind":"directory"},{"path":"link","kind":"entry"}],"marker":{"path":"ready","bytes":[118,49]},"adoption":"strict","timeout_ms":1234});
    let recipe: CacheRecipe = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(recipe.artifacts[0].bytes, vec![0, 255]);
    assert_eq!(serde_json::to_value(recipe).unwrap(), wire);
    assert_eq!(crate::CONTRACT_VERSION, (1, 2));
    assert!(crate::version::provider_is_compatible((1, 0)));
    let legacy: CacheRecipe = serde_json::from_value(serde_json::json!({
        "scope":"cache","artifacts":[],"steps":[],"required":[],
        "marker":{"path":"ready","bytes":[]},"timeout_ms":1
    }))
    .unwrap();
    assert_eq!(legacy.adoption, CacheAdoptionPolicy::Strict);
    assert_eq!(
        serde_json::to_value(CacheAdoptionPolicy::AdoptVerifiedLegacy).unwrap(),
        serde_json::json!("adopt_verified_legacy")
    );
}
