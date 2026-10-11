//! Verify that the current core can read an isolated restored DevDesk state.
//! Store::open may migrate the database, so never point this at live state.
use devdesk_core::{config, store::Store};
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), String> {
    let root = PathBuf::from(env::args().nth(1).ok_or(
        "Usage: cargo run -p devdesk-core --example verify_restored_state -- RESTORED_DIRECTORY",
    )?);
    let content =
        fs::read_to_string(root.join("config.toml")).map_err(|error| error.to_string())?;
    let home = PathBuf::from(env::var_os("HOME").ok_or("HOME is not set")?);
    let projects = config::parse(&content, &home)?.projects.len();
    let store = Store::open(&root.join("state.db"))?;
    let agents = store.agents()?.len();
    let meetings = store.meetings()?.len();
    let _layout = store.layout()?;
    let mut prompts = 0;
    loop {
        let page = store.list_prompts("", 100, prompts)?;
        if page.is_empty() {
            break;
        }
        prompts += page.len();
        if prompts > 100_000 {
            return Err("More than 100,000 prompts; verifier pagination limit reached".into());
        }
    }
    println!(
        "Current core read restored state: {projects} projects, {agents} agents, {prompts} prompts, {meetings} meetings"
    );
    Ok(())
}
