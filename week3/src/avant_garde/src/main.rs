use crate::agent::AgentStore;
use crate::comm::Comm;
use std::sync::Arc;
use tokio::sync::Mutex;

mod agent;
mod sensor;
mod comm;
mod ui;
mod logger;
mod localizer;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let agent_store = Arc::new(Mutex::new(AgentStore::new(None))); // create store with default sanitize duration
    const NAME: &str = "rust-client";
    const ADDRESS: &str = "10.178.199.211";
    const PORT: u16 = 1883;

    let comm = Comm::new(String::from(NAME), String::from(ADDRESS), PORT);
    
    let store_clone = agent_store.clone();
    tokio::spawn(async move {
        comm.run(store_clone).await;
    });

    ui::run(agent_store).await?;

    Ok(())
}
