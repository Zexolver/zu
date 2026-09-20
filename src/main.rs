mod app;
mod backends;
mod cli;
mod config;
mod plan;
mod platform;

use std::{env, path::PathBuf, process::ExitCode};

use anyhow::Result;

use backends::Op;
use cli::{Action, ConfigCmd};
use config::{Config, Style};

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let platform = platform::Platform::detect()?;

    // The command-line dialect lives in config.toml, so the config dir is needed before clap runs.
    let dir = config::dir(platform.os, config_dir_hint().as_deref());
    let cfg = Config::load(&dir);
    // A broken config must not stop `zu config init --force` from fixing it, so fall back to the
    // default dialect here; the error is reported below for every command that needs the config.
    let style = Style::from_env().or_else(|| cfg.as_ref().ok().map(|c| c.cli.style)).unwrap_or_default();

    let parsed = cli::parse(style);
    let dir = config::dir(platform.os, parsed.opts.config_dir.as_deref());

    if let [Action::Config(action)] = &parsed.actions[..] {
        match action {
            ConfigCmd::Path => app::config_path(&dir),
            ConfigCmd::Init { force } => app::config_init(&platform, &dir, *force)?,
        }
        return Ok(true);
    }

    let mut cfg = cfg?;
    cfg.cli.style = style; // so `doctor` reports the dialect actually in use, ZU_STYLE included
    let app = app::App::new(&parsed.opts, cfg, platform, dir)?;
    for action in &parsed.actions {
        let ok = match action {
            Action::Install(p) => app.install(p),
            Action::Remove(p) => app.remove(p, Op::Remove),
            Action::Purge(p) => app.remove(p, Op::Purge),
            Action::Search(q) => app.search(q),
            Action::Info(p) => app.info(p),
            Action::Update => app.update(),
            Action::Upgrade => app.upgrade(),
            Action::List => app.list(),
            Action::Backends => {
                app.backends();
                true
            }
            Action::Doctor => {
                app.doctor();
                true
            }
            Action::Config(_) => unreachable!("handled above"),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `--config-dir` has to be known before clap runs, so scan for it by hand (`ZU_CONFIG_DIR` also counts).
fn config_dir_hint() -> Option<PathBuf> {
    let mut args = env::args_os().skip(1);
    while let Some(a) = args.next() {
        if a == "--config-dir" {
            return args.next().map(PathBuf::from);
        }
        if let Some(v) = a.to_str().and_then(|s| s.strip_prefix("--config-dir=")) {
            return Some(PathBuf::from(v));
        }
    }
    env::var_os("ZU_CONFIG_DIR").map(PathBuf::from)
}
