use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::{
    backends::{self, Op, Probe},
    cli::Opts,
    config::{
        self, CONFIG_FILE, CONFIG_TEMPLATE, Config, Escalate, Mode, RULES_FILE, RULES_TEMPLATE, Rules, SOURCES_FILE, Sources,
        UiStyle,
    },
    plan::{self, Ctx, Detected},
    platform::{self, Platform, which},
};

pub struct App {
    platform: Platform,
    dir: PathBuf,
    cfg: Config,
    sources: Sources,
    rules: Rules,
    ctx: Ctx,
    /// Usable backends in priority order, after `--backend`/`--skip`.
    backends: Vec<Detected>,
    /// `--backend` was given: package rules no longer apply.
    forced: bool,
    dry_run: bool,
}

struct Group<'a> {
    cands: Vec<&'a Detected>,
    pkgs: Vec<&'a str>,
}

impl App {
    pub fn new(cli: &Opts, mut cfg: Config, platform: Platform, dir: PathBuf) -> Result<Self> {
        let sources = Sources::load(&dir, platform.os)?;
        let rules = Rules::load(&dir, platform.os)?;
        if cli.user {
            cfg.general.mode = Mode::User;
        }
        if cli.no_fallback {
            cfg.general.fallback = false;
        }
        let ctx = Ctx {
            is_root: platform::is_root(platform.os),
            user_mode: cfg.general.mode == Mode::User,
            escalator: escalator(cfg.general.escalate),
            yes: cli.yes || cfg.general.assume_yes,
        };
        let mut backends = plan::detect(&platform, &sources);
        if let Some(id) = &cli.backend {
            backends.retain(|d| d.backend.id == id);
            if backends.is_empty() {
                let have: Vec<_> = plan::detect(&platform, &sources).iter().map(|d| d.backend.id).collect();
                bail!("backend '{id}' is not available here (detected: {})", have.join(", "));
            }
        }
        backends.retain(|d| !cli.skip.iter().any(|s| s == d.backend.id));
        Ok(App { platform, dir, cfg, sources, rules, ctx, backends, forced: cli.backend.is_some(), dry_run: cli.dry_run })
    }

    // ---- planning ----

    /// Backends to try for `pkg` on `op`, in order, plus why the others were left out.
    fn plan(&self, pkg: &str, op: Op) -> (Vec<&Detected>, Vec<String>) {
        let mut v: Vec<&Detected> = self.backends.iter().collect();
        if !self.forced
            && matches!(op, Op::Install | Op::Info)
            && let Some(rule) = self.rules.get(pkg)
        {
            v = rule.arrange(v, |d| d.backend.id);
        }
        if op == Op::Install {
            // dpkg-style backends take local files only; everything else takes names.
            let ext = Path::new(pkg)
                .is_file()
                .then(|| Path::new(pkg).extension().and_then(|e| e.to_str()))
                .flatten();
            v.retain(|d| d.backend.local_ext.is_none() || d.backend.local_ext == ext);
            v.sort_by_key(|d| d.backend.local_ext.is_none());
        }
        let mut skipped = Vec::new();
        v.retain(|d| {
            if d.backend.cmd(op).is_none() {
                return false;
            }
            match self.ctx.access(d.backend, op) {
                Ok(_) => true,
                Err(why) => {
                    skipped.push(format!("{}: {why}", d.backend.id));
                    false
                }
            }
        });
        (v, skipped)
    }

    /// Every backend that can do `op` here, in priority order. Reports skipped ones on stderr.
    fn usable(&self, op: Op) -> Vec<&Detected> {
        self.backends
            .iter()
            .filter(|d| d.backend.cmd(op).is_some())
            .filter(|d| match self.ctx.access(d.backend, op) {
                Ok(_) => true,
                Err(why) => {
                    eprintln!(":: skipping {}: {why}", d.backend.id);
                    false
                }
            })
            .collect()
    }

    fn run(&self, d: &Detected, op: Op, names: &[String]) -> bool {
        let extra = self.sources.install_args(d.backend.id);
        match self.ctx.argv(d, op, names, extra) {
            Ok(argv) => run_argv(&argv, self.dry_run),
            Err(why) => {
                eprintln!(":: {}: {why}", d.backend.id);
                false
            }
        }
    }

    fn names(&self, pkgs: &[&str], id: &str) -> Vec<String> {
        pkgs.iter().map(|p| self.rules.name(p, id).to_string()).collect()
    }

    /// Like `run`, but captures output instead of showing it (for the `[ui] style = "pretty"`
    /// spinner); the captured text is only meant to be shown on failure.
    fn run_captured(&self, d: &Detected, op: Op) -> (bool, String) {
        let extra = self.sources.install_args(d.backend.id);
        let argv = match self.ctx.argv(d, op, &[], extra) {
            Ok(argv) => argv,
            Err(why) => return (false, why),
        };
        match Command::new(&argv[0]).args(&argv[1..]).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                (out.status.success(), text)
            }
            Err(e) => (false, format!("cannot run {}: {e}", argv[0])),
        }
    }

    /// Packages `p` reports as pending an upgrade, for `update`'s per-backend count/summary.
    fn list_pending(&self, d: &Detected, p: &backends::Pending) -> Option<Vec<String>> {
        let argv = plan::expand(p.cmd, d, false, false);
        let out = Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| p.format.parse(&String::from_utf8_lossy(&out.stdout)))
    }

    /// `Some(true)` installed, `Some(false)` not, `None` unknown (no probe, or its tool is missing).
    fn installed(&self, d: &Detected, pkg: &str) -> Option<bool> {
        let probe = d.backend.probe.as_ref()?;
        let (argv, takes_pkg) = match probe {
            Probe::Exit(a) | Probe::Marker(a, _) => (a, true),
            Probe::Listing(a) => (a, false),
        };
        let mut cmd = plan::expand(argv, d, false, false);
        if takes_pkg {
            cmd.push(pkg.to_string());
        }
        let out = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        match probe {
            Probe::Exit(_) => Some(out.status.success()),
            Probe::Marker(_, m) => Some(out.status.success() && stdout.contains(m)),
            Probe::Listing(_) => out.status.success().then(|| mentions(&stdout, pkg)),
        }
    }

    // ---- operations ----

    pub fn install(&self, pkgs: &[String]) -> bool {
        let mut groups: Vec<Group> = Vec::new();
        let mut failed: Vec<&str> = Vec::new();
        for p in pkgs {
            let (cands, skipped) = self.plan(p, Op::Install);
            if cands.is_empty() {
                let why = if skipped.is_empty() { String::new() } else { format!(" ({})", skipped.join("; ")) };
                eprintln!("error: no usable backend for '{p}'{why}");
                failed.push(p);
                continue;
            }
            let same = |g: &Group| g.cands.iter().map(|d| d.backend.id).eq(cands.iter().map(|d| d.backend.id));
            match groups.iter_mut().find(|g| same(g)) {
                Some(g) => g.pkgs.push(p),
                None => groups.push(Group { cands, pkgs: vec![p] }),
            }
        }
        for g in groups {
            let order: Vec<_> = g.cands.iter().map(|d| d.backend.id).collect();
            eprintln!(":: {}: {}", g.pkgs.join(", "), order.join(" > "));
            let mut pending = g.pkgs;
            for (i, d) in g.cands.iter().enumerate() {
                if pending.is_empty() {
                    break;
                }
                if i > 0 {
                    eprintln!(":: falling back to {} for {}", d.backend.id, pending.join(", "));
                }
                if self.run(d, Op::Install, &self.names(&pending, d.backend.id)) {
                    pending.clear();
                    break;
                }
                if pending.len() > 1 {
                    // One bad package must not sink the rest of the batch.
                    pending.retain(|p| !self.run(d, Op::Install, &self.names(&[p], d.backend.id)));
                }
                if !self.cfg.general.fallback {
                    break;
                }
            }
            failed.extend(pending);
        }
        if !failed.is_empty() {
            eprintln!("error: failed to install: {}", failed.join(", "));
        }
        failed.is_empty()
    }

    /// `op` is `Op::Remove` or `Op::Purge`.
    pub fn remove(&self, pkgs: &[String], op: Op) -> bool {
        let verb = if op == Op::Purge { "purge" } else { "remove" };
        let mut ok = true;
        for p in pkgs {
            let (cands, skipped) = self.plan(p, op);
            let mut known = Vec::new();
            let mut unknown = Vec::new();
            for d in cands {
                match self.installed(d, self.rules.name(p, d.backend.id)) {
                    Some(true) => known.push(d),
                    None => unknown.push(d),
                    Some(false) => {}
                }
            }
            known.extend(unknown);
            if known.is_empty() {
                let why = if skipped.is_empty() { String::new() } else { format!(" (skipped: {})", skipped.join("; ")) };
                eprintln!("error: '{p}' is not installed by any usable backend{why}");
                ok = false;
                continue;
            }
            let done = known.iter().enumerate().any(|(i, d)| {
                if i > 0 {
                    eprintln!(":: falling back to {} for {p}", d.backend.id);
                }
                eprintln!(":: {verb} {p} via {}", d.backend.id);
                self.run(d, op, &self.names(&[p], d.backend.id))
            });
            if !done {
                eprintln!("error: failed to {verb} {p}");
                ok = false;
            }
        }
        ok
    }

    pub fn search(&self, query: &[String]) -> bool {
        let mut any = false;
        for d in self.usable(Op::Search) {
            eprintln!(":: {}", d.backend.id);
            any |= self.run(d, Op::Search, query);
        }
        any
    }

    pub fn info(&self, pkgs: &[String]) -> bool {
        let mut ok = true;
        for p in pkgs {
            let (cands, _) = self.plan(p, Op::Info);
            let found = cands.iter().any(|d| {
                eprintln!(":: {}", d.backend.id);
                self.run(d, Op::Info, &self.names(&[p], d.backend.id))
            });
            if !found {
                eprintln!("error: no backend has info for '{p}'");
                ok = false;
            }
        }
        ok
    }

    pub fn update(&self) -> bool {
        // Unlike `usable()`, this keeps backends with no `refresh` command: they get a one-line
        // explanation instead of silently vanishing (that looked like only the first backend was
        // ever being updated).
        let candidates: Vec<&Detected> = self
            .backends
            .iter()
            .filter(|d| match self.ctx.access(d.backend, Op::Refresh) {
                Ok(_) => true,
                Err(why) => {
                    eprintln!(":: skipping {}: {why}", d.backend.id);
                    false
                }
            })
            .collect();
        if candidates.is_empty() {
            eprintln!("no usable backend to update");
            return true;
        }

        let pretty = self.cfg.ui.style == UiStyle::Pretty && !self.dry_run;
        let total = candidates.len();
        let mp = pretty.then(MultiProgress::new);
        let overall = mp.as_ref().map(|mp| {
            let pb = mp.add(ProgressBar::new(total as u64));
            pb.set_style(ProgressStyle::with_template("{bar:28.cyan/blue} {pos}/{len}").unwrap());
            pb
        });

        let mut all_ok = true;
        let mut upgradable: Vec<(&str, String)> = Vec::new();
        for (i, d) in candidates.iter().enumerate() {
            let b = d.backend;
            let success = if b.refresh.is_none() {
                println!(":: {}: {}", b.id, b.refresh_note);
                true
            } else if let Some(mp) = &mp {
                let spinner = mp.add(ProgressBar::new_spinner());
                spinner.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").unwrap());
                spinner.set_message(format!("Updating {}...", b.name));
                spinner.enable_steady_tick(Duration::from_millis(80));
                let (success, captured) = self.run_captured(d, Op::Refresh);
                spinner.finish_and_clear();
                if success {
                    println!(":: {} updated", b.name);
                } else {
                    eprintln!(":: {} failed:\n{}", b.id, captured.trim_end());
                }
                success
            } else {
                eprintln!(":: updating {}", b.name);
                self.run(d, Op::Refresh, &[])
            };

            if success
                && let Some(p) = &b.pending
                && let Some(names) = self.list_pending(d, p)
            {
                println!("   {} package(s) can be updated", names.len());
                upgradable.extend(names.into_iter().map(|n| (b.id, n)));
            }
            match &overall {
                Some(pb) => pb.inc(1),
                None => eprintln!(":: [{}/{total}] {} done", i + 1, b.id),
            }
            all_ok &= success;
        }
        // Not `ProgressBar::finish_with_message`: indicatif hides its own output entirely when
        // stderr isn't a terminal (piped/logged), which would otherwise drop "Done!" with it.
        if let Some(pb) = overall {
            pb.finish_and_clear();
        }
        println!("Done!");

        if self.cfg.ui.list_upgradable && !upgradable.is_empty() {
            println!("\n{} package(s) can be upgraded:", upgradable.len());
            for (id, name) in &upgradable {
                println!("  {name}  ({id})");
            }
        }
        all_ok
    }

    pub fn upgrade(&self) -> bool {
        let mut ok = true;
        let mut groups = HashSet::new();
        for d in self.usable(Op::Upgrade) {
            let b = d.backend;
            if !b.group.is_empty() && !groups.insert(b.group) {
                continue;
            }
            eprintln!(":: upgrading {}", b.id);
            if b.refresh_before_upgrade && b.refresh.is_some() && !self.run(d, Op::Refresh, &[]) {
                ok = false;
                continue;
            }
            ok &= self.run(d, Op::Upgrade, &[]);
        }
        ok
    }

    pub fn list(&self) -> bool {
        let mut any = false;
        for d in self.usable(Op::List) {
            eprintln!(":: {}", d.backend.id);
            any |= self.run(d, Op::List, &[]);
        }
        any
    }

    // ---- reporting ----

    fn who(&self) -> String {
        let user = if self.ctx.is_root { "root" } else { "user" };
        let mode = if self.ctx.user_mode { ", user-only mode" } else { "" };
        match (&self.ctx.escalator, self.ctx.is_root) {
            (Some(e), false) => format!("{user} (can escalate with {e}){mode}"),
            _ => format!("{user}{mode}"),
        }
    }

    fn status(&self, d: &Detected) -> String {
        match self.ctx.access(d.backend, Op::Install) {
            Ok(_) => "ok".into(),
            Err(why) => format!("skipped: {why}"),
        }
    }

    pub fn backends(&self) {
        println!("{} ({}), running as {}", self.platform.pretty, self.platform.os.name(), self.who());
        if self.backends.is_empty() {
            println!("no package managers detected");
        }
        for (i, d) in self.backends.iter().enumerate() {
            let b = d.backend;
            let note = match self.status(d).as_str() {
                "ok" => String::new(),
                s => format!("  [{s}]"),
            };
            println!("{:>2}. {:<8} {:<12} {:<10} {}{note}", i + 1, b.id, b.name, b.kind.label(), d.path.display());
        }
    }

    pub fn doctor(&self) {
        let sysdir = config::system_dir(self.platform.os);
        let file = |name: &str| {
            let user = self.dir.join(name);
            if user.exists() {
                return format!("{} (found)", user.display());
            }
            match sysdir.as_ref().map(|d| d.join(name)).filter(|p| p.exists()) {
                Some(sys) => format!("{} (none; using {})", user.display(), sys.display()),
                None => format!("{} (not found, built-in defaults in use)", user.display()),
            }
        };
        println!("zu {}", env!("CARGO_PKG_VERSION"));
        println!("platform   {} [{}]", self.platform.pretty, self.platform.os.name());
        if !self.platform.families.is_empty() {
            println!("families   {}", self.platform.families.join(" "));
        }
        println!("running as {}", self.who());
        println!("config     {}", file(CONFIG_FILE));
        println!("sources    {}", file(SOURCES_FILE));
        println!("rules      {}", file(RULES_FILE));
        println!("cli style  {}", match self.cfg.cli.style { config::Style::Apt => "apt (zu install ...)", config::Style::Pacman => "pacman (zu -S ...)" });
        println!("fallback   {}", if self.cfg.general.fallback { "on" } else { "off" });
        println!();
        self.backends();

        let applicable = backends::applicable(&self.platform);
        let missing: Vec<_> = applicable
            .iter()
            .filter(|b| !self.backends.iter().any(|d| std::ptr::eq(d.backend, **b)))
            .collect();
        if !missing.is_empty() {
            println!("\nnot in use:");
            for b in missing {
                let why = if self.sources.disabled.iter().any(|d| d == b.id) {
                    "disabled in config".to_string()
                } else if which(b.bin).is_none() {
                    format!("not installed{}", if b.hint.is_empty() { String::new() } else { format!(" ({})", b.hint) })
                } else {
                    "filtered by --backend/--skip".to_string()
                };
                println!("    {:<8} {why}", b.id);
            }
        }

        let known: HashSet<&str> = applicable.iter().map(|b| b.id).collect();
        let mut issues = Vec::new();
        let check = |what: &str, ids: &mut dyn Iterator<Item = &String>, issues: &mut Vec<String>| {
            for id in ids {
                if !known.contains(id.as_str()) {
                    issues.push(format!("{what}: '{id}' is not a backend on this platform (ignored)"));
                }
            }
        };
        check("sources.toml order", &mut self.sources.order.iter(), &mut issues);
        check("sources.toml disabled", &mut self.sources.disabled.iter(), &mut issues);
        for id in self.sources.options.keys().filter(|k| !known.contains(k.as_str())) {
            issues.push(format!("sources.toml options: '{id}' is not a backend on this platform (ignored)"));
        }
        let mut pkgs: Vec<_> = self.rules.0.iter().collect();
        pkgs.sort_by_key(|(name, _)| *name);
        for (name, r) in pkgs {
            let what = format!("packages.toml [{name}]");
            check(&what, &mut r.only.iter().chain(&r.prefer).chain(&r.skip).chain(r.names.keys()), &mut issues);
        }
        if !issues.is_empty() {
            println!("\nconfig notes:");
            for i in issues {
                println!("    {i}");
            }
        }
    }
}

pub fn config_path(platform: &Platform, dir: &Path) {
    println!("{}", dir.display());
    println!("{}", dir.join(CONFIG_FILE).display());
    println!("{}", dir.join(SOURCES_FILE).display());
    println!("{}", dir.join(RULES_FILE).display());
    if let Some(sys) = config::system_dir(platform.os) {
        println!("\nsystem-wide defaults (used where the files above don't exist):");
        println!("{}", sys.display());
    }
}

/// Write starter files. Existing files are left alone unless `force`.
pub fn config_init(platform: &Platform, dir: &Path, force: bool) -> Result<()> {
    let detected = plan::detect(platform, &Sources::default());
    let ids: Vec<&str> = detected.iter().map(|d| d.backend.id).collect();
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let files = [
        (CONFIG_FILE, CONFIG_TEMPLATE.to_string()),
        (SOURCES_FILE, config::sources_template(platform, &ids)),
        (RULES_FILE, RULES_TEMPLATE.to_string()),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        if path.exists() && !force {
            eprintln!("{} exists, leaving it alone (--force to overwrite)", path.display());
            continue;
        }
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn escalator(mode: Escalate) -> Option<String> {
    let candidates: &[&str] = match mode {
        Escalate::Auto => &["sudo", "doas"],
        Escalate::Sudo => &["sudo"],
        Escalate::Doas => &["doas"],
        Escalate::Never => &[],
    };
    candidates.iter().find(|c| which(c).is_some()).map(|c| c.to_string())
}

fn run_argv(argv: &[String], dry_run: bool) -> bool {
    let shown: Vec<_> = argv.iter().map(|a| quote(a)).collect();
    eprintln!("+ {}", shown.join(" "));
    if dry_run {
        return true;
    }
    match Command::new(&argv[0]).args(&argv[1..]).status() {
        Ok(s) => s.success(),
        Err(e) => {
            eprintln!("error: cannot run {}: {e}", argv[0]);
            false
        }
    }
}

fn quote(a: &str) -> String {
    let plain = !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if plain { a.to_string() } else { format!("'{}'", a.replace('\'', r"'\''")) }
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || "-_+".contains(c)))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether `text` mentions `pkg` as whole words (so "git" doesn't match "github-cli").
fn mentions(text: &str, pkg: &str) -> bool {
    let want = words(pkg);
    !want.is_empty() && words(text).windows(want.len()).any(|w| w == want.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_matching() {
        assert!(mentions("Installed apps:\ngit 2.4\n7zip 1.0", "git"));
        assert!(!mentions("github-cli 2.0", "git"));
        assert!(mentions("legacyPackages.x86_64-linux.hello", "hello"));
        assert!(mentions("python3.11 3.11.2", "python3.11"));
        assert!(!mentions("python3.12", "python3.11"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("nix-command flakes"), "'nix-command flakes'");
        assert_eq!(quote("nixpkgs#x"), "'nixpkgs#x'");
        assert_eq!(quote("-Syu"), "-Syu");
    }
}
