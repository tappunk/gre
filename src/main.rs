use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr, eyre};
use git2::{BranchType, Repository, StatusOptions};
use rayon::prelude::*;
use serde::Deserialize;

const JSON_SCHEMA_VERSION: &str = "2";

#[derive(Parser, Debug)]
#[command(
    name = "gre",
    about = "super fast git recap for configured repositories"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(
        about = "create a boilerplate config file",
        after_help = "examples:\n  gre init\n  gre --config /tmp/gre.toml init\n  gre init --force"
    )]
    Init(InitArgs),
}

#[derive(Args, Debug)]
struct InitArgs {
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    repos: Option<Vec<RepoItem>>,
    repositories: Option<Vec<RepoItem>>,
    repo: Option<Vec<RepoItem>>,
    paths: Option<Vec<String>>,
    output: Option<OutputConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    default_json: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum RepoItem {
    Path(String),
    Named { name: Option<String>, path: String },
}

#[derive(Debug, Clone)]
struct RepoConfig {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct RepoReport {
    name: String,
    path: PathBuf,
    branch: String,
    ahead: u64,
    behind: u64,
    staged: usize,
    unstaged: usize,
    untracked: usize,
    conflicts: usize,
    last_hash: Option<String>,
    last_subject: Option<String>,
    last_relative: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct JsonRepoReport {
    name: String,
    path: String,
    branch: String,
    ahead: u64,
    behind: u64,
    staged: usize,
    unstaged: usize,
    untracked: usize,
    conflicts: usize,
    clean: bool,
    last_hash: Option<String>,
    last_subject: Option<String>,
    last_relative: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct JsonSummary {
    configured_total: usize,
    succeeded_total: usize,
    failed_total: usize,
    dirty: usize,
    behind: usize,
    ahead: usize,
    elapsed_ms: u64,
    avg_repo_ms: f64,
    p95_repo_ms: f64,
}

#[derive(Debug, serde::Serialize)]
struct JsonOutput {
    schema_version: &'static str,
    summary: JsonSummary,
    repos: Vec<JsonRepoReport>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let config_path = resolve_config_path(cli.config)?;

    if let Some(Commands::Init(args)) = cli.command {
        return run_init(args, &config_path);
    }

    let (repositories, output_config) = load_repositories(&config_path)?;
    let json_enabled = cli.json || default_json_enabled(output_config.as_ref());

    if repositories.is_empty() {
        eprintln!(
            "warning: no repositories configured in {}",
            config_path.display()
        );
        return Ok(());
    }

    let started = Instant::now();
    let reports: Vec<(Result<RepoReport>, std::time::Duration)> = repositories
        .par_iter()
        .map(|repository| {
            let repo_started = Instant::now();
            let result = inspect_repository(repository);
            (result, repo_started.elapsed())
        })
        .collect();

    let mut ok_reports = Vec::new();
    let mut ok_timings = Vec::new();
    let mut had_failure = false;
    let mut failed_total = 0usize;

    for (report, elapsed_repo) in reports {
        match report {
            Ok(value) => {
                ok_timings.push(elapsed_repo);
                ok_reports.push(value);
            }
            Err(error) => {
                had_failure = true;
                failed_total += 1;
                eprintln!("error: {error}");
            }
        }
    }

    let elapsed = started.elapsed();

    if json_enabled {
        ok_reports.sort_by(|a, b| a.name.cmp(&b.name));
        print_json(
            &ok_reports,
            elapsed,
            &ok_timings,
            repositories.len(),
            failed_total,
        )?;
    } else {
        print_human(&ok_reports, elapsed);
    }

    if had_failure {
        return Err(eyre!("one or more repositories failed to inspect"));
    }

    Ok(())
}

fn resolve_config_path(arg_path: Option<PathBuf>) -> Result<PathBuf> {
    match arg_path {
        Some(path) => Ok(expand_tilde(path)),
        None => {
            let home = dirs::home_dir().ok_or_else(|| eyre!("could not resolve home directory"))?;
            Ok(home.join(".config").join("gre").join("config.toml"))
        }
    }
}

fn run_init(args: InitArgs, config_path: &Path) -> Result<()> {
    if config_path.exists() && !args.force {
        return Err(eyre!(
            "config file '{}' already exists, rerun with --force to overwrite",
            config_path.display()
        ));
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }

    fs::write(config_path, boilerplate_config())
        .wrap_err_with(|| format!("failed to write {}", config_path.display()))?;

    println!("wrote config to '{}'", config_path.display());
    Ok(())
}

fn boilerplate_config() -> &'static str {
    "paths = [\n  \"~/src/gre\",\n  \"~/src/muthr\",\n]\n\n[output]\ndefault_json = false\n"
}

fn default_json_enabled(output: Option<&OutputConfig>) -> bool {
    output.and_then(|value| value.default_json).unwrap_or(false)
}

fn load_repositories(config_path: &Path) -> Result<(Vec<RepoConfig>, Option<OutputConfig>)> {
    let config_raw = fs::read_to_string(config_path)
        .wrap_err_with(|| format!("failed to read config at {}", config_path.display()))?;
    let parsed: Config = toml::from_str(&config_raw)
        .wrap_err_with(|| format!("invalid config format in {}", config_path.display()))?;

    let output = parsed.output.clone();

    let mut repositories = Vec::new();

    if let Some(entries) = parsed.repos {
        repositories.extend(entries.into_iter().map(repo_item_to_config));
    }
    if let Some(entries) = parsed.repositories {
        repositories.extend(entries.into_iter().map(repo_item_to_config));
    }
    if let Some(entries) = parsed.repo {
        repositories.extend(entries.into_iter().map(repo_item_to_config));
    }
    if let Some(paths) = parsed.paths {
        repositories.extend(paths.into_iter().map(|path| RepoConfig {
            name: infer_name_from_path(&path),
            path: expand_tilde(PathBuf::from(path)),
        }));
    }

    let mut kept: HashMap<PathBuf, String> = HashMap::new();
    repositories.retain(|repository| {
        if let Some(kept_name) = kept.insert(repository.path.clone(), repository.name.clone()) {
            eprintln!(
                "warning: duplicate path '{}' in config, keeping '{}' and dropping '{}'",
                repository.path.display(),
                kept_name,
                repository.name,
            );
            false
        } else {
            true
        }
    });
    Ok((repositories, output))
}

fn repo_item_to_config(item: RepoItem) -> RepoConfig {
    match item {
        RepoItem::Path(path) => RepoConfig {
            name: infer_name_from_path(&path),
            path: expand_tilde(PathBuf::from(path)),
        },
        RepoItem::Named { name, path } => RepoConfig {
            name: name.unwrap_or_else(|| infer_name_from_path(&path)),
            path: expand_tilde(PathBuf::from(path)),
        },
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let as_text = path.to_string_lossy();
    let resolved = if as_text == "~" {
        dirs::home_dir().unwrap_or(path)
    } else if let Some(stripped) = as_text.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        home.join(stripped)
    } else {
        path
    };

    // Canonicalize to normalize trailing slashes, ., .. etc.
    // Gracefully fall back to resolved path if it doesn't exist yet.
    resolved.canonicalize().unwrap_or(resolved)
}

fn infer_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.to_owned())
}

fn inspect_repository(repository: &RepoConfig) -> Result<RepoReport> {
    if !repository.path.exists() {
        return Err(eyre!(
            "repository '{}' path does not exist: {}",
            repository.name,
            repository.path.display()
        ));
    }

    if !repository.path.is_dir() {
        return Err(eyre!(
            "repository '{}' path is not a directory: {}",
            repository.name,
            repository.path.display()
        ));
    }

    let repo = Repository::open(&repository.path)
        .wrap_err_with(|| format!("failed to open git repo {}", repository.path.display()))?;

    if repo.is_bare() {
        return Err(eyre!(
            "repository '{}' is bare (bare repos are not supported): {}",
            repository.name,
            repository.path.display()
        ));
    }

    let head_ref = repo.head().wrap_err_with(|| {
        format!(
            "repository '{}' at {}: failed to read HEAD",
            repository.name,
            repository.path.display()
        )
    })?;

    let branch = if head_ref.is_branch() {
        head_ref.shorthand().unwrap_or("detached").to_string()
    } else {
        "detached".to_string()
    };

    let mut status_opts = StatusOptions::new();
    status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(false)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false)
        .include_ignored(false);

    let statuses = repo
        .statuses(Some(&mut status_opts))
        .wrap_err_with(|| format!("failed to read status in {}", repository.path.display()))?;

    let mut staged = 0usize;
    let mut unstaged = 0usize;
    let mut untracked = 0usize;
    let mut conflicts = 0usize;

    for entry in statuses.iter() {
        let status = entry.status();

        if status.is_conflicted() {
            conflicts += 1;
            continue;
        }

        if status.is_wt_new() {
            untracked += 1;
        }

        if status.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_TYPECHANGE
                | git2::Status::WT_RENAMED,
        ) {
            unstaged += 1;
        }

        if status.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_TYPECHANGE
                | git2::Status::INDEX_RENAMED,
        ) {
            staged += 1;
        }
    }

    let head_oid = head_ref.target();
    let (ahead, behind): (u64, u64) = if let (Some(oid), Ok(local_branch)) =
        (head_oid, repo.find_branch(&branch, BranchType::Local))
    {
        if let Ok(upstream_branch) = local_branch.upstream() {
            if let Some(upstream_oid) = upstream_branch.get().target() {
                if oid == upstream_oid {
                    (0, 0)
                } else {
                    {
                        let (a, b) =
                            repo.graph_ahead_behind(oid, upstream_oid)
                                .wrap_err_with(|| {
                                    format!(
                                        "failed to compute ahead/behind for '{}' at {}",
                                        repository.name,
                                        repository.path.display()
                                    )
                                })?;
                        (a as u64, b as u64)
                    }
                }
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let (last_hash, last_subject, last_relative) = if let Some(oid) = head_oid {
        if let Ok(commit) = repo.find_commit(oid) {
            let full = oid.to_string();
            let hash = full.get(0..7).unwrap_or(&full).to_string();
            (
                Some(hash),
                commit.summary().map(str::to_owned),
                Some(format_relative_time(commit.time().seconds())),
            )
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    Ok(RepoReport {
        name: repository.name.clone(),
        path: repository.path.clone(),
        branch,
        ahead,
        behind,
        staged,
        unstaged,
        untracked,
        conflicts,
        last_hash,
        last_subject,
        last_relative,
    })
}

fn format_relative_time(commit_unix_seconds: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let delta = now.saturating_sub(commit_unix_seconds).max(0);

    if delta < 60 {
        return "just now".to_string();
    }
    if delta < 3600 {
        let minutes = delta / 60;
        return format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        );
    }
    if delta < 86_400 {
        let hours = delta / 3600;
        return format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" });
    }
    if delta < 604_800 {
        let days = delta / 86_400;
        return format!("{} day{} ago", days, if days == 1 { "" } else { "s" });
    }
    if delta < 2_592_000 {
        let weeks = delta / 604_800;
        return format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" });
    }
    if delta < 31_536_000 {
        let months = delta / 2_592_000;
        return format!("{} month{} ago", months, if months == 1 { "" } else { "s" });
    }

    let years = delta / 31_536_000;
    format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
}

fn print_human(reports: &[RepoReport], elapsed: std::time::Duration) {
    const REPO_COL_MIN: usize = 18;
    const REPO_COL_MAX: usize = 28;
    const BRANCH_COL_MIN: usize = 14;
    const BRANCH_COL_MAX: usize = 32;
    const SYNC_COL: usize = 7;
    const STATE_COL: usize = 24;
    const NEXT_COL: usize = 17;
    const COMMIT_COL: usize = 56;

    let mut sorted = reports.to_vec();
    sorted.sort_by(|a, b| {
        sort_rank(a)
            .cmp(&sort_rank(b))
            .then_with(|| a.name.cmp(&b.name))
    });

    let total = sorted.len();
    let dirty = sorted.iter().filter(|report| !is_clean(report)).count();
    let behind = sorted.iter().filter(|report| report.behind > 0).count();
    let ahead = sorted.iter().filter(|report| report.ahead > 0).count();

    let focus = build_focus_line(&sorted);
    let elapsed_ms = elapsed.as_millis() as u64;
    let avg_repo_ms = average_repo_ms(total, elapsed);
    let repo_col = column_width(
        &sorted,
        "repo",
        |report| report.name.chars().count(),
        REPO_COL_MIN,
        REPO_COL_MAX,
    );
    let branch_col = column_width(
        &sorted,
        "branch",
        |report| report.branch.chars().count(),
        BRANCH_COL_MIN,
        BRANCH_COL_MAX,
    );

    println!(
        "repos:{}  dirty:{}  behind:{}  ahead:{}  time:{}ms  avg:{:.2}ms  focus:{}",
        total, dirty, behind, ahead, elapsed_ms, avg_repo_ms, focus
    );
    println!(
        "{:<repo_col$}  {:<branch_col$}  {:>SYNC_COL$}  {:<STATE_COL$}  {:<NEXT_COL$}  {:<COMMIT_COL$}  path",
        fit_cell("repo", repo_col),
        fit_cell("branch", branch_col),
        fit_cell("sync", SYNC_COL),
        fit_cell("state", STATE_COL),
        fit_cell("next", NEXT_COL),
        fit_cell("last_commit", COMMIT_COL)
    );

    for report in sorted {
        let status_text = if is_clean(&report) {
            String::from("clean")
        } else {
            format!(
                "dirty s:{} u:{} ?:{} c:{}",
                report.staged, report.unstaged, report.untracked, report.conflicts
            )
        };

        let branch_text = report.branch.clone();
        let sync_text = format!("+{} -{}", report.ahead, report.behind);
        let next_text = next_action(&report);

        let commit_summary = match (
            &report.last_hash,
            &report.last_subject,
            &report.last_relative,
        ) {
            (Some(hash), Some(subject), Some(relative)) => {
                format!("{} {} ({})", hash, subject, relative)
            }
            _ => String::from("no commits"),
        };

        println!(
            "{:<repo_col$}  {:<branch_col$}  {:>SYNC_COL$}  {:<STATE_COL$}  {:<NEXT_COL$}  {:<COMMIT_COL$}  {}",
            fit_cell(&report.name, repo_col),
            fit_cell(&branch_text, branch_col),
            fit_cell(&sync_text, SYNC_COL),
            fit_cell(&status_text, STATE_COL),
            fit_cell(next_text, NEXT_COL),
            fit_cell(&commit_summary, COMMIT_COL),
            format_path_for_display(&report.path)
        );
    }
}

fn column_width<F>(reports: &[RepoReport], label: &str, extract: F, min: usize, max: usize) -> usize
where
    F: Fn(&RepoReport) -> usize,
{
    let mut width = label.chars().count().max(min);
    for report in reports {
        width = width.max(extract(report));
    }
    width.min(max)
}

fn fit_cell(value: &str, width: usize) -> String {
    let len = value.chars().count();
    if len <= width {
        return value.to_string();
    }

    if width <= 3 {
        return value.chars().take(width).collect();
    }

    let mut text: String = value.chars().take(width.saturating_sub(3)).collect();
    text.push_str("...");
    text
}

fn is_clean(report: &RepoReport) -> bool {
    report.staged == 0 && report.unstaged == 0 && report.untracked == 0 && report.conflicts == 0
}

fn sort_rank(report: &RepoReport) -> usize {
    if report.conflicts > 0 {
        return 0;
    }
    if !is_clean(report) {
        return 1;
    }
    if report.behind > 0 {
        return 2;
    }
    if report.ahead > 0 {
        return 3;
    }
    4
}

fn next_action(report: &RepoReport) -> &'static str {
    if report.conflicts > 0 {
        return "resolve-conflicts";
    }
    if report.staged > 0 && report.unstaged == 0 && report.untracked == 0 {
        return "commit";
    }
    if !is_clean(report) {
        return "commit-or-stash";
    }
    if report.behind > 0 && report.ahead > 0 {
        return "sync";
    }
    if report.behind > 0 {
        return "pull";
    }
    if report.ahead > 0 {
        return "push";
    }
    "none"
}

fn build_focus_line(reports: &[RepoReport]) -> String {
    let mut items = Vec::new();

    for report in reports {
        let action = next_action(report);
        if action == "none" {
            continue;
        }
        items.push(format!("{}:{}", report.name, action));
    }

    if items.is_empty() {
        return "none".to_string();
    }

    let mut focus = items.iter().take(4).cloned().collect::<Vec<_>>().join(",");
    if items.len() > 4 {
        focus.push_str(&format!(",+{}", items.len() - 4));
    }
    focus
}

fn format_path_for_display(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home_text = home.display().to_string();
        if let Some(suffix) = text.strip_prefix(&home_text) {
            if suffix.is_empty() {
                return "~".to_string();
            }
            return format!("~{}", suffix);
        }
    }
    text
}

fn average_repo_ms(total: usize, elapsed: std::time::Duration) -> f64 {
    if total == 0 {
        return 0.0;
    }

    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    elapsed_ms / total as f64
}

fn percentile_repo_ms(samples: &[std::time::Duration], percentile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }

    let mut values: Vec<f64> = samples
        .iter()
        .map(|value| value.as_secs_f64() * 1000.0)
        .collect();
    values.sort_by(|a, b| a.total_cmp(b));

    let rank = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[rank]
}

fn print_json(
    reports: &[RepoReport],
    elapsed: std::time::Duration,
    ok_timings: &[std::time::Duration],
    configured_total: usize,
    failed_total: usize,
) -> Result<()> {
    let rows: Vec<JsonRepoReport> = reports
        .iter()
        .map(|report| JsonRepoReport {
            name: report.name.clone(),
            path: report.path.display().to_string(),
            branch: report.branch.clone(),
            ahead: report.ahead,
            behind: report.behind,
            staged: report.staged,
            unstaged: report.unstaged,
            untracked: report.untracked,
            conflicts: report.conflicts,
            clean: report.staged == 0
                && report.unstaged == 0
                && report.untracked == 0
                && report.conflicts == 0,
            last_hash: report.last_hash.clone(),
            last_subject: report.last_subject.clone(),
            last_relative: report.last_relative.clone(),
        })
        .collect();

    let summary = JsonSummary {
        configured_total,
        succeeded_total: reports.len(),
        failed_total,
        dirty: reports.iter().filter(|report| !is_clean(report)).count(),
        behind: reports.iter().filter(|report| report.behind > 0).count(),
        ahead: reports.iter().filter(|report| report.ahead > 0).count(),
        elapsed_ms: elapsed.as_millis() as u64,
        avg_repo_ms: average_repo_ms(reports.len(), elapsed),
        p95_repo_ms: percentile_repo_ms(ok_timings, 0.95),
    };

    let payload = JsonOutput {
        schema_version: JSON_SCHEMA_VERSION,
        summary,
        repos: rows,
    };

    let output =
        serde_json::to_string_pretty(&payload).wrap_err("failed to serialize json output")?;
    println!("{output}");
    Ok(())
}
