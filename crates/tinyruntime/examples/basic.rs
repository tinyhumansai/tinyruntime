//! Resolving a runtime through the engine, without a bus.
//!
//! The engine is ordinary Rust: a routing table, a client, and a place to write
//! harnesses. Running it here rather than through the module is the point — it
//! shows that the router's behaviour is testable and reusable without anything
//! being loaded into anyone's process.
//!
//! Nothing is routed in this example, so the resolution fails with the reason a
//! host would see when it asked for a language whose provider module is not
//! loaded. Load `tinyruntime-nodejs` alongside the module and the same call
//! returns a toolchain.

use tinyruntime::{Engine, Language, Registry, ResolveRequest, RuntimeSettings};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let harness_root = std::env::temp_dir().join("tinyruntime-example");
    let engine = Engine::new(Registry::new(), reqwest::Client::new(), harness_root);

    println!("routing {} language(s)", engine.registry().len());

    let request = ResolveRequest::probe(Language::nodejs(), RuntimeSettings::new("v22.11.0"));
    match engine.resolve(&request).await {
        Ok(Some(runtime)) => println!("resolved {} {}", runtime.language, runtime.version),
        Ok(None) => println!("nothing is provisioned yet"),
        Err(error) => println!("could not resolve: {error}"),
    }

    Ok(())
}
