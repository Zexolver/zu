use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, de::DeserializeOwned};

use crate::platform::{Os, Platform};

pub const CONFIG_FILE: &str = "config.toml";
pub const SOURCES_FILE: &str = "sources.toml";
pub const RULES_FILE: &str = "packages.toml";

/// `config.toml`: how zu behaves and how it is invoked.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub cli: CliConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub style: UiStyle,
    /// After `zu update`, list every package each backend reports as pending an upgrade.
    pub list_upgradable: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { style: UiStyle::Pretty, list_upgradable: true }
    }
}

/// How `zu update`/`zu upgrade` show backend activity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiStyle {
    /// zu's own spinner, progress bar and per-backend summary; each backend's own output is
    /// captured rather than shown.
    #[default]
    Pretty,
    /// Each backend's own output, verbatim, same as `install`/`remove`.
    Raw,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliConfig {
    pub style: Style,
}

/// Command-line dialect.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// `zu install`, `zu remove`, `zu purge`, ...
    #[default]
    Apt,
    /// `zu -S`, `zu -R`, `zu -Rns`, `zu -Syu`, ...
    Pacman,
}

impl Style {
    /// `ZU_STYLE` overrides the config file for one invocation.
    pub fn from_env() -> Option<Style> {
        match env::var("ZU_STYLE").ok()?.to_lowercase().as_str() {
            "apt" => Some(Style::Apt),
            "pacman" => Some(Style::Pacman),
            _ => None,
        }
    }
}

/// `sources.toml`: which package managers to use, and in what order.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sources {
    /// Highest priority first. Detected backends left out follow in default order.
    pub order: Vec<String>,
    pub disabled: Vec<String>,
    pub options: HashMap<String, BackendOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub mode: Mode,
    pub escalate: Escalate,
    pub fallback: bool,
    pub assume_yes: bool,
}

impl Default for General {
    fn default() -> Self {
        General { mode: Mode::Auto, escalate: Escalate::Auto, fallback: true, assume_yes: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Use whatever privileges the process has, escalating for backends that need root.
    Auto,
    /// User-only: skip every backend that needs root, use per-user scopes.
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Escalate {
    Auto,
    Sudo,
    Doas,
    Never,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackendOptions {
    /// Extra arguments for this backend's install command.
    pub install_args: Vec<String>,
}

impl Config {
    /// `dir` (user config) wins; falling back to the system-wide defaults in [`system_dir`].
    pub fn load(dir: &Path, os: Os) -> Result<Self> {
        read_toml(&dir.join(CONFIG_FILE), system_dir(os).as_deref().map(|d| d.join(CONFIG_FILE)))
    }
}

impl Sources {
    pub fn load(dir: &Path, os: Os) -> Result<Self> {
        read_toml(&dir.join(SOURCES_FILE), system_dir(os).as_deref().map(|d| d.join(SOURCES_FILE)))
    }

    pub fn install_args(&self, id: &str) -> &[String] {
        self.options.get(id).map_or(&[], |o| &o.install_args)
    }
}

/// Per-package rules from `packages.toml`: one top-level table per package.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct Rules(pub HashMap<String, PkgRule>);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PkgRule {
    /// Use nothing but these, in this order.
    pub only: Vec<String>,
    /// Try these first, in this order, then the normal order.
    pub prefer: Vec<String>,
    /// Never use these.
    pub skip: Vec<String>,
    /// Package name per backend id, for when it differs.
    pub names: HashMap<String, String>,
}

impl Rules {
    pub fn load(dir: &Path, os: Os) -> Result<Self> {
        read_toml(&dir.join(RULES_FILE), system_dir(os).as_deref().map(|d| d.join(RULES_FILE)))
    }

    pub fn get(&self, pkg: &str) -> Option<&PkgRule> {
        self.0.get(pkg)
    }

    /// The name `pkg` goes by on backend `id`.
    pub fn name<'a>(&'a self, pkg: &'a str, id: &str) -> &'a str {
        self.get(pkg).and_then(|r| r.names.get(id)).map_or(pkg, String::as_str)
    }
}

impl PkgRule {
    /// Apply skip/only/prefer to `items` (already in normal priority order).
    pub fn arrange<T>(&self, mut items: Vec<T>, id: impl Fn(&T) -> &str) -> Vec<T> {
        items.retain(|i| !self.skip.iter().any(|s| s == id(i)));
        if !self.only.is_empty() {
            items.retain(|i| self.only.iter().any(|s| s == id(i)));
            items.sort_by_key(|i| self.only.iter().position(|s| s == id(i)));
        }
        items.sort_by_key(|i| self.prefer.iter().position(|s| s == id(i)).unwrap_or(usize::MAX));
        items
    }
}

/// `user` wins if present; otherwise `system` (a package-provided default) is tried; otherwise
/// `T::default()`.
fn read_toml<T: DeserializeOwned + Default>(user: &Path, system: Option<PathBuf>) -> Result<T> {
    if let Some(v) = read_toml_at(user)? {
        return Ok(v);
    }
    if let Some(sys) = system
        && let Some(v) = read_toml_at(&sys)?
    {
        return Ok(v);
    }
    Ok(T::default())
}

fn read_toml_at<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map(Some).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Package-provided defaults, overridable per-user. `None` where there is no such convention
/// (Termux has no writable system-wide location worth using for this).
pub fn system_dir(os: Os) -> Option<PathBuf> {
    match os {
        Os::Windows => env::var_os("ProgramData").map(|p| PathBuf::from(p).join("zu")),
        Os::Android => None,
        _ => Some(PathBuf::from("/etc/zu")),
    }
}

/// `--config-dir`/`ZU_CONFIG_DIR`, else `%APPDATA%\zu` on Windows, else `$XDG_CONFIG_HOME/zu` or `~/.config/zu`.
pub fn dir(os: Os, over: Option<&Path>) -> PathBuf {
    if let Some(p) = over {
        return p.to_path_buf();
    }
    if os == Os::Windows
        && let Some(p) = env::var_os("APPDATA")
    {
        return PathBuf::from(p).join("zu");
    }
    if let Some(p) = env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        return PathBuf::from(p).join("zu");
    }
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join(".config").join("zu")
}

pub const CONFIG_TEMPLATE: &str = r#"# zu settings. Which package managers to use lives in sources.toml.
# This file overrides /etc/zu/config.toml (ProgramData\zu on Windows), if a package put one there.

[general]
# "auto": use the privileges you have, escalating for backends that need root.
# "user": user-only. Skip every backend that needs root and use per-user scopes
#         (e.g. flatpak --user).
mode = "auto"
# How to get root when a backend needs it and you are not root:
# "auto" (sudo, then doas), "sudo", "doas" or "never".
escalate = "auto"
# When a backend fails to install a package, try the next one.
fallback = true
# Always pass each backend's non-interactive flags (same as --yes).
assume_yes = false

[cli]
# Command-line dialect (ZU_STYLE=apt|pacman overrides it for one run):
#   "apt"    zu install / remove / purge / search / show / update / upgrade / list
#   "pacman" zu -S / -R / -Rns / -Ss / -Si / -Sy / -Syu / -Q
style = "apt"

[ui]
# How `zu update`/`zu upgrade` show backend activity:
#   "pretty" zu's own spinner, progress bar and per-backend summary (default)
#   "raw"    each backend's own output, verbatim, same as install/remove
style = "pretty"
# After `zu update`, list every package each backend reports as pending an upgrade.
list_upgradable = true
"#;

/// Starter sources listing only the backends that apply here and are installed.
pub fn sources_template(p: &Platform, detected: &[&str]) -> String {
    let order = detected.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", ");
    format!(
        r#"# zu sources, generated for {pretty}.
# Only package managers that exist on this platform and are installed are listed.

# First is tried first. Detected backends you leave out follow in default order.
order = [{order}]
# Never use these.
disabled = []

# Extra arguments for one backend's install command:
# [options.{first}]
# install_args = []
"#,
        pretty = p.pretty,
        first = detected.first().copied().unwrap_or("pacman"),
    )
}

pub const RULES_TEMPLATE: &str = r#"# Per-package rules for zu. One table per package; every key is optional.
#
# [hyprland]
# skip   = ["apt"]                   # never use these; the normal order applies to the rest
# prefer = ["pacstall"]              # try these first (in this order), then the normal order
# only   = ["pacstall", "flatpak"]   # use nothing but these, in this order
#
# [hyprland.names]                   # the package is called something else on some backends
# pacstall = "hyprland-git"
#
# [fd]
# names = { apt = "fd-find" }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn arrange(rule: &PkgRule, items: &[&'static str]) -> Vec<&'static str> {
        rule.arrange(items.to_vec(), |i| i)
    }

    #[test]
    fn skip_keeps_normal_order_for_the_rest() {
        let r: PkgRule = toml::from_str(r#"skip = ["apt"]"#).unwrap();
        assert_eq!(arrange(&r, &["apt", "pacstall", "flatpak"]), ["pacstall", "flatpak"]);
    }

    #[test]
    fn only_restricts_and_orders() {
        let r: PkgRule = toml::from_str(r#"only = ["flatpak", "pacstall"]"#).unwrap();
        assert_eq!(arrange(&r, &["apt", "pacstall", "flatpak"]), ["flatpak", "pacstall"]);
    }

    #[test]
    fn prefer_moves_to_front() {
        let r: PkgRule = toml::from_str(r#"prefer = ["pacstall"]"#).unwrap();
        assert_eq!(arrange(&r, &["apt", "flatpak", "pacstall"]), ["pacstall", "apt", "flatpak"]);
    }

    #[test]
    fn rules_file_and_names() {
        let rules: Rules = toml::from_str(
            "[hyprland]\nskip = [\"apt\"]\n[hyprland.names]\npacstall = \"hyprland-git\"\n[fd]\nnames = { apt = \"fd-find\" }\n",
        )
        .unwrap();
        assert_eq!(rules.name("hyprland", "pacstall"), "hyprland-git");
        assert_eq!(rules.name("hyprland", "flatpak"), "hyprland");
        assert_eq!(rules.name("fd", "apt"), "fd-find");
        assert_eq!(rules.name("unlisted", "apt"), "unlisted");
    }

    #[test]
    fn typos_are_rejected() {
        assert!(toml::from_str::<Config>("[general]\nmodee = \"user\"\n").is_err());
        assert!(toml::from_str::<PkgRule>("skp = [\"apt\"]").is_err());
    }

    #[test]
    fn generated_files_parse() {
        let p = Platform { os: Os::Linux, families: vec![], pretty: "Test".into() };
        let src: Sources = toml::from_str(&sources_template(&p, &["pacman", "flatpak"])).unwrap();
        assert_eq!(src.order, ["pacman", "flatpak"]);
        let cfg: Config = toml::from_str(CONFIG_TEMPLATE).unwrap();
        assert!(cfg.general.fallback);
        assert_eq!(cfg.cli.style, Style::Apt);
        assert_eq!(cfg.ui.style, UiStyle::Pretty);
        assert!(cfg.ui.list_upgradable);
        toml::from_str::<Rules>(RULES_TEMPLATE).unwrap();
    }

    #[test]
    fn ui_style_defaults_and_parses() {
        assert_eq!(Config::default().ui.style, UiStyle::Pretty);
        assert!(Config::default().ui.list_upgradable);
        let cfg: Config = toml::from_str("[ui]\nstyle = \"raw\"\nlist_upgradable = false\n").unwrap();
        assert_eq!(cfg.ui.style, UiStyle::Raw);
        assert!(!cfg.ui.list_upgradable);
    }

    #[test]
    fn style_defaults_to_apt_and_parses_pacman() {
        assert_eq!(Config::default().cli.style, Style::Apt);
        let cfg: Config = toml::from_str("[cli]\nstyle = \"pacman\"\n").unwrap();
        assert_eq!(cfg.cli.style, Style::Pacman);
        assert!(toml::from_str::<Config>("[cli]\nstyle = \"yum\"\n").is_err());
    }

    #[test]
    fn system_dir_is_a_fallback_a_user_file_overrides() {
        let tmp = std::env::temp_dir().join(format!("zu-cfg-test-{}", std::process::id()));
        let user = tmp.join("user");
        let system = tmp.join("system");
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join(SOURCES_FILE), "order = [\"apt\", \"pacstall\", \"flatpak\"]\n").unwrap();

        // No user file: falls back to the system one.
        let v: Sources = read_toml(&user.join(SOURCES_FILE), Some(system.join(SOURCES_FILE))).unwrap();
        assert_eq!(v.order, ["apt", "pacstall", "flatpak"]);

        // A user file wins outright, even an empty one.
        fs::write(user.join(SOURCES_FILE), "order = [\"flatpak\"]\n").unwrap();
        let v: Sources = read_toml(&user.join(SOURCES_FILE), Some(system.join(SOURCES_FILE))).unwrap();
        assert_eq!(v.order, ["flatpak"]);

        fs::remove_dir_all(&tmp).unwrap();
    }
}
