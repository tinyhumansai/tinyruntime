//! Loads a built module through the real `TinyBus` dynamic loader.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinyruntime::{LanguagesResponse, names};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = module_argument()?;
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let module_host = ModuleHost::new(broker);
    let info = module_host.load_file(&module)?;

    if info.name != env!("CARGO_PKG_NAME") {
        return Err(io::Error::other(format!(
            "loaded module `{}` instead of `{}`",
            info.name,
            env!("CARGO_PKG_NAME")
        ))
        .into());
    }

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

    // `Languages` is the right probe for a router: it exercises the whole
    // dispatch path and needs neither a provider module nor a network, so it
    // verifies the artifact rather than the environment it happens to run in.
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: LanguagesResponse = proxy.call(names::methods::LANGUAGES, ()).await?;
    if reply.languages.is_empty() {
        return Err(io::Error::other("module routed no languages at all").into());
    }

    println!(
        "verified {} as TinyBus module `{}`, routing {} language(s):",
        module.display(),
        info.name,
        reply.languages.len()
    );
    for status in &reply.languages {
        let state = if status.available {
            "available".to_string()
        } else {
            format!(
                "unavailable ({})",
                status.detail.as_deref().unwrap_or("no reason given")
            )
        };
        println!("  {} -> {} [{state}]", status.language, status.bus_name);
    }
    broker_task.abort();
    Ok(())
}

fn module_argument() -> Result<PathBuf, io::Error> {
    std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: cargo run --example verify_module -- <module-path>",
            )
        })
}
