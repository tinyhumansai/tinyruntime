//! Ad-hoc local repro: loads the freshly built cdylib the same way
//! `verify_github_release` does, without downloading a release asset.
//! Not part of the crate's public examples; deleted before commit.
use std::io;
use std::time::Duration;

use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinyruntime::{LanguagesResponse, names};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("path to .so");
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let module_host = ModuleHost::new(broker);
    let info = module_host.load_file_with_config(&path, serde_json::Value::default())?;
    println!("loaded {} ({:?})", info.name, info.state);

    let client = Connection::connect(bus.connect().await?).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let claimed = client.list_names().await?;
            if claimed.iter().any(|name| name.as_str() == names::INTERFACE) {
                return tinybus::Result::Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;

    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: LanguagesResponse = proxy.call(names::methods::LANGUAGES, ()).await?;
    println!("routed {} language(s)", reply.languages.len());
    broker_task.abort();
    Ok(())
}
