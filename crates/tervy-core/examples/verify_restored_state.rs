//! Verify that the current core can read an isolated restored Tervy state.
//! Store::open may migrate the database, so never point this at live state.
use std::{env, fs, path::PathBuf};
use tervy_core::{config, store::Store};

fn main() -> Result<(), String> {
    let root = PathBuf::from(env::args().nth(1).ok_or(
        "Usage: cargo run -p tervy-core --example verify_restored_state -- RESTORED_DIRECTORY",
    )?);
    let content =
        fs::read_to_string(root.join("config.toml")).map_err(|error| error.to_string())?;
    let home = PathBuf::from(env::var_os("HOME").ok_or("HOME is not set")?);
    let projects = config::parse(&content, &home)?.projects.len();
    let store = Store::open(&root.join("state.db"))?;
    let agents = store.agents()?.len();
    let meetings = store.meetings()?.len();
    let _layout = store.layout()?;
    let mut notes = 0;
    loop {
        let page = store.list_notes("", 100, notes)?;
        if page.is_empty() {
            break;
        }
        notes += page.len();
        if notes > 100_000 {
            return Err("More than 100,000 notes; verifier pagination limit reached".into());
        }
    }
    println!(
        "Current core read restored state: {projects} projects, {agents} agents, {notes} notes, {meetings} meetings"
    );
    Ok(())
}
