use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};

use crate::config::Style;

/// Options shared by both command-line styles.
pub struct Opts {
    pub yes: bool,
    pub dry_run: bool,
    pub backend: Option<String>,
    pub skip: Vec<String>,
    pub user: bool,
    pub no_fallback: bool,
    pub config_dir: Option<PathBuf>,
}

pub enum Action {
    Install(Vec<String>),
    Remove(Vec<String>),
    Purge(Vec<String>),
    Search(Vec<String>),
    Info(Vec<String>),
    Update,
    Upgrade,
    List,
    Backends,
    Doctor,
    Config(ConfigCmd),
}

pub struct Parsed {
    pub opts: Opts,
    /// Run in order; the first failure stops the rest (`-Sy foo` is refresh, then install).
    pub actions: Vec<Action>,
}

pub fn parse(style: Style) -> Parsed {
    match style {
        Style::Apt => AptCli::parse().into(),
        Style::Pacman => {
            let cli = PacCli::parse();
            cli.resolve().unwrap_or_else(|msg| PacCli::command().error(ErrorKind::MissingRequiredArgument, msg).exit())
        }
    }
}

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Print where the config files live
    Path,
    /// Write starter config.toml, sources.toml (only detected backends) and packages.toml
    Init {
        /// Overwrite existing files
        #[arg(long)]
        force: bool,
    },
}

// ---- apt style: `zu install foo` ----

#[derive(Parser)]
#[command(
    name = "zu",
    version,
    about = "Zex's Universal: one front-end for every package manager",
    long_about = "Zex's Universal: one front-end for every package manager.\n\n\
        Tries the platform's own package manager first, then falls back through the rest in the \
        order set in sources.toml. Per-package overrides live in packages.toml. Set `style = \
        \"pacman\"` under [cli] in config.toml for `zu -S` / `zu -R` style flags."
)]
struct AptCli {
    /// Pass each backend's non-interactive flags
    #[arg(short, long, global = true)]
    yes: bool,

    /// Print the commands instead of running them (read-only probes still run)
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,

    /// Use only this backend, ignoring package rules
    #[arg(short, long, global = true, value_name = "ID")]
    backend: Option<String>,

    /// Never use this backend (repeatable)
    #[arg(long, global = true, value_name = "ID")]
    skip: Vec<String>,

    /// User-only mode: skip backends that need root, use per-user scopes
    #[arg(long, global = true)]
    user: bool,

    /// Stop at the first backend that fails instead of trying the next
    #[arg(long, global = true)]
    no_fallback: bool,

    /// Directory holding config.toml, sources.toml and packages.toml
    #[arg(long, global = true, env = "ZU_CONFIG_DIR", value_name = "DIR")]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: AptCmd,
}

#[derive(Subcommand)]
enum AptCmd {
    /// Install packages, falling back through backends in priority order
    #[command(visible_aliases = ["i", "add"])]
    Install {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Remove packages from whichever backend has them installed
    #[command(visible_aliases = ["rm", "r", "uninstall"])]
    Remove {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Remove packages along with their configuration and data
    Purge {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Search every backend
    #[command(visible_alias = "s")]
    Search {
        #[arg(required = true)]
        query: Vec<String>,
    },
    /// Show package details from the first backend that knows it
    #[command(visible_alias = "show")]
    Info {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Refresh package indexes
    Update,
    /// Upgrade installed packages on every backend
    #[command(visible_alias = "up")]
    Upgrade,
    /// List installed packages per backend
    #[command(visible_alias = "ls")]
    List,
    /// Show the detected backends in priority order
    Backends,
    /// Show platform, privileges, config and backend status
    Doctor,
    /// Manage configuration files
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
}

impl From<AptCli> for Parsed {
    fn from(c: AptCli) -> Parsed {
        let action = match c.cmd {
            AptCmd::Install { packages } => Action::Install(packages),
            AptCmd::Remove { packages } => Action::Remove(packages),
            AptCmd::Purge { packages } => Action::Purge(packages),
            AptCmd::Search { query } => Action::Search(query),
            AptCmd::Info { packages } => Action::Info(packages),
            AptCmd::Update => Action::Update,
            AptCmd::Upgrade => Action::Upgrade,
            AptCmd::List => Action::List,
            AptCmd::Backends => Action::Backends,
            AptCmd::Doctor => Action::Doctor,
            AptCmd::Config { action } => Action::Config(action),
        };
        Parsed {
            opts: Opts {
                yes: c.yes,
                dry_run: c.dry_run,
                backend: c.backend,
                skip: c.skip,
                user: c.user,
                no_fallback: c.no_fallback,
                config_dir: c.config_dir,
            },
            actions: vec![action],
        }
    }
}

// ---- pacman style: `zu -S foo` ----

#[derive(Parser)]
#[command(
    name = "zu",
    version,
    about = "Zex's Universal (pacman-style flags)",
    long_about = "Zex's Universal, pacman-style flags.\n\n\
        zu -S <pkg>      install          zu -Ss <query>   search\n\
        zu -R <pkg>      remove           zu -Si <pkg>     info\n\
        zu -Rn <pkg>     purge            zu -Sy           refresh indexes\n\
        zu -Q            list installed   zu -Syu / -Su    upgrade\n\n\
        Also: zu backends | doctor | config path | config init",
    override_usage = "zu <-S|-R|-Q>[modifiers] [packages...]\n       zu <backends|doctor|config path|config init>"
)]
struct PacCli {
    /// Sync: install packages (-Ss search, -Si info, -Sy refresh, -Su upgrade)
    #[arg(short = 'S', long)]
    sync: bool,

    /// Remove packages (-Rn purges configuration and data too)
    #[arg(short = 'R', long)]
    remove: bool,

    /// Query: list installed packages
    #[arg(short = 'Q', long)]
    query: bool,

    /// With -S: search. With -R: accepted for habit (dependencies are handled by each backend)
    #[arg(short = 's', long)]
    search: bool,

    /// With -S: show package details
    #[arg(short = 'i', long)]
    info: bool,

    /// With -S: refresh package indexes
    #[arg(short = 'y', long)]
    refresh: bool,

    /// With -S: upgrade installed packages
    #[arg(short = 'u', long)]
    sysupgrade: bool,

    /// With -R: purge configuration and data too
    #[arg(short = 'n', long)]
    nosave: bool,

    /// Pass each backend's non-interactive flags
    #[arg(long)]
    noconfirm: bool,

    /// Print the commands instead of running them (read-only probes still run)
    #[arg(short = 'p', long)]
    print: bool,

    /// Use only this backend, ignoring package rules
    #[arg(short = 'b', long, value_name = "ID")]
    backend: Option<String>,

    /// Never use this backend (repeatable)
    #[arg(long, value_name = "ID")]
    skip: Vec<String>,

    /// User-only mode: skip backends that need root, use per-user scopes
    #[arg(long)]
    user: bool,

    /// Stop at the first backend that fails instead of trying the next
    #[arg(long)]
    no_fallback: bool,

    /// Directory holding config.toml, sources.toml and packages.toml
    #[arg(long, env = "ZU_CONFIG_DIR", value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// With `config init`: overwrite existing files
    #[arg(long)]
    force: bool,

    /// Packages, or (without -S/-R/-Q) one of: backends, doctor, config path, config init
    packages: Vec<String>,
}

const NEED_OP: &str = "pick exactly one operation: -S (install, search, info, refresh, upgrade), -R (remove), \
    -Q (list), or one of: backends, doctor, config path, config init";

impl PacCli {
    fn resolve(self) -> Result<Parsed, String> {
        let opts = Opts {
            yes: self.noconfirm,
            dry_run: self.print,
            backend: self.backend,
            skip: self.skip,
            user: self.user,
            no_fallback: self.no_fallback,
            config_dir: self.config_dir,
        };
        let pkgs = self.packages;
        let need_pkgs = |what: &str, pkgs: &[String]| {
            if pkgs.is_empty() { Err(format!("{what} needs at least one package")) } else { Ok(()) }
        };
        let mut actions = Vec::new();

        let ops = [self.sync, self.remove, self.query].iter().filter(|b| **b).count();
        // Without an operation flag the words are zu's own commands. They are not clap
        // subcommands so that `zu -S config` still installs a package called "config".
        if ops == 0 {
            let words: Vec<&str> = pkgs.iter().map(String::as_str).collect();
            let action = match words[..] {
                ["backends"] => Action::Backends,
                ["doctor"] => Action::Doctor,
                ["config", "path"] => Action::Config(ConfigCmd::Path),
                ["config", "init"] => Action::Config(ConfigCmd::Init { force: self.force }),
                _ => return Err(NEED_OP.into()),
            };
            return Ok(Parsed { opts, actions: vec![action] });
        }
        if ops != 1 {
            return Err(NEED_OP.into());
        }
        if self.sync {
            if self.search {
                need_pkgs("-Ss", &pkgs)?;
                actions.push(Action::Search(pkgs));
            } else if self.info {
                need_pkgs("-Si", &pkgs)?;
                actions.push(Action::Info(pkgs));
            } else {
                // Upgrade already refreshes wherever a backend needs it.
                if self.sysupgrade {
                    actions.push(Action::Upgrade);
                } else if self.refresh {
                    actions.push(Action::Update);
                }
                if !pkgs.is_empty() {
                    actions.push(Action::Install(pkgs));
                } else if actions.is_empty() {
                    return Err("-S needs at least one package (or -y / -u)".into());
                }
            }
        } else if self.remove {
            need_pkgs("-R", &pkgs)?;
            actions.push(if self.nosave { Action::Purge(pkgs) } else { Action::Remove(pkgs) });
        } else {
            if !pkgs.is_empty() || self.search || self.info {
                return Err("-Q lists everything installed; filtering is not supported".into());
            }
            actions.push(Action::List);
        }
        Ok(Parsed { opts, actions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pac(args: &[&str]) -> Result<Vec<String>, String> {
        let cli = PacCli::try_parse_from(std::iter::once("zu").chain(args.iter().copied())).map_err(|e| e.to_string())?;
        let p = cli.resolve()?;
        Ok(p.actions.iter().map(describe).collect())
    }

    fn describe(a: &Action) -> String {
        let j = |v: &Vec<String>| v.join(",");
        match a {
            Action::Install(v) => format!("install {}", j(v)),
            Action::Remove(v) => format!("remove {}", j(v)),
            Action::Purge(v) => format!("purge {}", j(v)),
            Action::Search(v) => format!("search {}", j(v)),
            Action::Info(v) => format!("info {}", j(v)),
            Action::Update => "update".into(),
            Action::Upgrade => "upgrade".into(),
            Action::List => "list".into(),
            Action::Backends => "backends".into(),
            Action::Doctor => "doctor".into(),
            Action::Config(_) => "config".into(),
        }
    }

    #[test]
    fn pacman_operations() {
        assert_eq!(pac(&["-S", "vim", "htop"]).unwrap(), ["install vim,htop"]);
        assert_eq!(pac(&["-Ss", "vim"]).unwrap(), ["search vim"]);
        assert_eq!(pac(&["-Si", "vim"]).unwrap(), ["info vim"]);
        assert_eq!(pac(&["-R", "vim"]).unwrap(), ["remove vim"]);
        assert_eq!(pac(&["-Rs", "vim"]).unwrap(), ["remove vim"]);
        assert_eq!(pac(&["-Rns", "vim"]).unwrap(), ["purge vim"]);
        assert_eq!(pac(&["-Sy"]).unwrap(), ["update"]);
        assert_eq!(pac(&["-Syu"]).unwrap(), ["upgrade"]);
        assert_eq!(pac(&["-Su"]).unwrap(), ["upgrade"]);
        assert_eq!(pac(&["-Sy", "vim"]).unwrap(), ["update", "install vim"]);
        assert_eq!(pac(&["-Q"]).unwrap(), ["list"]);
        assert_eq!(pac(&["doctor"]).unwrap(), ["doctor"]);
    }

    #[test]
    fn pacman_rejects_nonsense() {
        assert!(pac(&[]).is_err());
        assert!(pac(&["-S"]).is_err());
        assert!(pac(&["-R"]).is_err());
        assert!(pac(&["-Ss"]).is_err());
        assert!(pac(&["-S", "-R", "vim"]).is_err());
        assert!(pac(&["-Q", "vim"]).is_err());
    }

    #[test]
    fn pacman_shared_options() {
        let cli = PacCli::try_parse_from(["zu", "-S", "vim", "--noconfirm", "-p", "-b", "flatpak", "--skip", "apt"]).unwrap();
        let p = cli.resolve().unwrap();
        assert!(p.opts.yes && p.opts.dry_run);
        assert_eq!(p.opts.backend.as_deref(), Some("flatpak"));
        assert_eq!(p.opts.skip, ["apt"]);
    }

    #[test]
    fn pacman_own_commands_take_options_and_dont_shadow_packages() {
        assert_eq!(pac(&["--user", "doctor"]).unwrap(), ["doctor"]);
        assert_eq!(pac(&["backends"]).unwrap(), ["backends"]);
        assert_eq!(pac(&["config", "init", "--force"]).unwrap(), ["config"]);
        assert_eq!(pac(&["-S", "config"]).unwrap(), ["install config"]);
        assert!(pac(&["config"]).is_err());
        assert!(pac(&["vim"]).is_err());
    }

    #[test]
    fn apt_purge() {
        let p: Parsed = AptCli::try_parse_from(["zu", "purge", "vim", "-y"]).unwrap().into();
        assert!(p.opts.yes);
        assert!(matches!(&p.actions[..], [Action::Purge(v)] if v == &["vim"]));
    }
}
