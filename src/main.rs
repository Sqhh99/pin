#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    #[cfg(windows)]
    {
        pin::run()
    }
    #[cfg(not(windows))]
    {
        eprintln!("pin: this build target is Windows-only");
        Ok(())
    }
}
