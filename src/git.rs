//! Git access: scopes, changed files, and diffs.
//!
//! The only writes are private refs under `refs/worktree/diff-reckoner/`. Nothing here
//! commits, stages, or mutates the worktree, the index, or any branch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::model::{ChangeKind, ChangedFile, Scope};

/// Run `git -C <repo> <args>` and return stdout. Errors on non-zero exit.
fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = crate::proc::command("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !out.status.success() {
        bail!("git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like [`git`], but returns stdout even on non-zero exit (e.g. `diff --no-index`).
fn git_lenient(repo: &Path, args: &[&str]) -> String {
    crate::proc::command("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Run `git -C <repo> <args>` and return its trimmed stdout, or `None` if the command fails to
/// spawn, exits non-zero, or prints nothing. The one-line query workhorse for `rev-parse`/`merge-base`.
fn git_line(repo: &Path, args: &[&str]) -> Option<String> {
    let out = crate::proc::command("git").arg("-C").arg(repo).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// Whether `git -C <repo> <args>` spawns and exits zero. The predicate workhorse for existence checks.
fn git_ok(repo: &Path, args: &[&str]) -> bool {
    crate::proc::command("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether `path` is inside a git work tree.
pub fn is_repo(path: &Path) -> bool {
    git_ok(path, &["rev-parse", "--is-inside-work-tree"])
}

/// The git top-level of `path`, or `None` if it is not a repo. Collapses "git ran and said no"
/// and "git could not run" — use [`worktree_of`] when that difference matters.
pub fn toplevel(path: &Path) -> Option<PathBuf> {
    match worktree_of(path) {
        Worktree::Root(root) => Some(root),
        Worktree::Outside | Worktree::Unknown => None,
    }
}

/// A directory's git top level, keeping "git ran and it is outside any worktree" (`Outside`, a
/// determination) apart from "git could not be run at all" (`Unknown`, the absence of one — a
/// spawn error under load). A caller deciding membership must hold on `Unknown` rather than read
/// it as `Outside`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Worktree {
    Root(PathBuf),
    Outside,
    Unknown,
}

/// Resolve `path` to its worktree, distinguishing the two ways resolution yields no root.
pub fn worktree_of(path: &Path) -> Worktree {
    match crate::proc::command("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Err(_) => Worktree::Unknown,
        Ok(out) if !out.status.success() => Worktree::Outside,
        Ok(out) => match String::from_utf8_lossy(&out.stdout).trim() {
            "" => Worktree::Outside,
            root => Worktree::Root(PathBuf::from(root)),
        },
    }
}

// --- git command helpers -----------------------------------------------------
//
// One failure contract: a git command that *fails* is a transient [`GitFail`], never read
// as absence.

/// A git command that failed (spawn error or unexpected non-zero exit) — a transient
/// failure, never absence.
#[derive(Debug)]
pub struct GitFail(pub String);

/// Spawn one PR-fetch git read. `LC_ALL=C` pins Git's messages to English — remote discovery
/// classifies a missing remote by stderr text, which Git otherwise localizes.
fn run_git(repo: &Path, args: &[&str]) -> Result<std::process::Output, GitFail> {
    crate::proc::command("git")
        .arg("-C")
        .arg(repo)
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .map_err(|e| GitFail(format!("git {args:?}: {e}")))
}

/// Run git where exit 0 is a value, exit 1 is a designated clean absence (`--verify
/// --quiet`, `symbolic-ref --quiet`, `cat-file -e`), and anything else is a failure.
fn git_tristate(repo: &Path, args: &[&str]) -> Result<Option<String>, GitFail> {
    let out = run_git(repo, args)?;
    if out.status.success() {
        return Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()));
    }
    if out.status.code() == Some(1) {
        return Ok(None);
    }
    Err(GitFail(format!("git {args:?}: {}", String::from_utf8_lossy(&out.stderr).trim())))
}

/// Run git where any non-zero exit is a failure. Exit 0 with empty output is a clean
/// "found nothing" (e.g. `for-each-ref` matching no refs).
fn git_strict(repo: &Path, args: &[&str]) -> Result<String, GitFail> {
    let out = run_git(repo, args)?;
    if !out.status.success() {
        return Err(GitFail(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The winning base: a branch (origin then local) or any other spelling
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedBase {
    Branch { name: String, oid: String },
    Rev { spelling: String, oid: String },
}

impl ResolvedBase {
    fn branch(name: String, oid: String) -> Self {
        Self::Branch { name, oid }
    }

    fn rev(spelling: String, oid: String) -> Self {
        Self::Rev { spelling, oid }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Branch { name, .. } => name,
            Self::Rev { spelling, .. } => spelling,
        }
    }

    #[must_use]
    pub fn oid(&self) -> &str {
        match self {
            Self::Branch { oid, .. } | Self::Rev { oid, .. } => oid,
        }
    }
}

/// The chain outcome the header paints: the winner and the first recorded choice the
/// chain skipped because it no longer resolves. The skip rides beside the winner, not
/// inside it, so it survives a chain where nothing resolves at all — a dormant pick
/// never reads as never-chosen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseStatus {
    pub winner: Option<ResolvedBase>,
    pub skipped: Option<String>,
}

/// One pass over the base chain. `candidates` keeps every source that resolved, in
/// precedence order and deduped by OID — the PR frontier walk needs all of them, not just
/// the winner (`pr_local`). `recorded` keeps every source name the chain considered —
/// every candidate's name and every dormant one's, since a pick that fails to resolve
/// still shields its name from the PR name lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseResolution {
    pub status: BaseStatus,
    /// The default branch the chain ran against ([`default_branch_name`]), so the picker
    /// marks its row from the same pass that resolved the winner.
    pub default: Option<String>,
    candidates: Vec<ResolvedBase>,
    recorded: Vec<String>,
}

impl BaseResolution {}

/// Resolve the base chain: the `--base` flag, then this worktree's pick, then the default
/// branch ([`default_branch_name`]). A source that does not
/// resolve to a commit is skipped, never an error; a skipped flag or pick that would have
/// outranked the winner is recorded for the header.
///
/// A pick spelling the default branch (one an earlier release wrote, or one the repo
/// re-defaulted onto) resolves to the same base the default step would, so it needs no
/// special case here; [`write_base_pick`] keeps such a ref from being written.
pub fn resolve_base(repo: &Path, base_flag: Option<&str>) -> Result<BaseResolution, GitFail> {
    let mut candidates: Vec<ResolvedBase> = Vec::new();
    let mut recorded: Vec<String> = Vec::new();
    let mut skipped: Option<String> = None;
    let default = default_branch_name(repo)?;
    let push = |c: ResolvedBase, list: &mut Vec<ResolvedBase>| {
        if !list.iter().any(|x| x.oid() == c.oid()) {
            list.push(c);
        }
    };
    let record = |name: String, r: &mut Vec<String>| {
        if !name.is_empty() && !r.contains(&name) {
            r.push(name);
        }
    };
    if let Some(flag) = base_flag.filter(|b| !b.is_empty()) {
        let (hit, skip) = classify_flag(repo, flag)?;
        if let Some(c) = hit {
            record(c.name().to_string(), &mut recorded);
            push(c, &mut candidates);
        }
        if let Some(s) = skip {
            record(s.clone(), &mut recorded);
            skipped = Some(s);
        }
    }
    if let Some(pick) = read_base_pick(repo)? {
        record(pick.clone(), &mut recorded);
        match resolve_spelling(repo, &pick)? {
            Some(c) => push(c, &mut candidates),
            None if candidates.is_empty() => skipped = skipped.or(Some(pick)),
            None => {}
        }
    }
    if let Some(name) = &default {
        record(name.clone(), &mut recorded);
        if let Some(oid) = resolve_base_entry(repo, name)? {
            push(ResolvedBase::branch(name.clone(), oid), &mut candidates);
        }
    }
    let winner = candidates.first().cloned();
    Ok(BaseResolution { status: BaseStatus { winner, skipped }, default, candidates, recorded })
}

/// The repo's default branch: what `origin/HEAD` names, else `init.defaultBranch`, else
/// `main`, else `master` — the last three only when a branch of exactly that name exists,
/// on origin or locally. `origin/HEAD` is the best evidence of the trunk, not its
/// definition: a clone with no remote still has one, and without this fallback such a
/// repo has no base at all.
///
/// Existence is read back from the ref list, never probed with `rev-parse`: a loose-ref
/// lookup on a case-insensitive filesystem resolves `refs/heads/main` to a branch named
/// `Main`, and a name no ref spells would then paint the header and match no row.
pub fn default_branch_name(repo: &Path) -> Result<Option<String>, GitFail> {
    if let Some(name) = origin_default_branch(repo)? {
        return Ok(Some(name));
    }
    let configured = git_tristate(repo, &["config", "--get", "init.defaultBranch"])?
        .filter(|name| is_branch_label(name));
    let names: Vec<&str> =
        configured.iter().map(String::as_str).chain(["main", "master"]).collect();
    // One listing for every candidate: `for-each-ref` takes several patterns, and it
    // matches them case-sensitively and by whole path, so the output is checked for the
    // exact ref (the pattern alone would also match a branch `main/foo`).
    let patterns: Vec<String> = names
        .iter()
        .flat_map(|name| BRANCH_REF_PREFIXES.iter().map(move |prefix| format!("{prefix}{name}")))
        .collect();
    let mut args = vec!["for-each-ref", "--format=%(refname)"];
    args.extend(patterns.iter().map(String::as_str));
    let out = git_strict(repo, &args)?;
    let listed: std::collections::HashSet<&str> = out.lines().collect();
    Ok(names
        .into_iter()
        .find(|name| {
            BRANCH_REF_PREFIXES.iter().any(|p| listed.contains(format!("{p}{name}").as_str()))
        })
        .map(str::to_string))
}

/// The branch name `origin/HEAD` points at. Some
/// clones carry `origin/HEAD` as a plain ref instead of a symref — then the name is the
/// origin tip whose commit matches it.
fn origin_default_branch(repo: &Path) -> Result<Option<String>, GitFail> {
    let target = git_tristate(repo, &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"])?;
    if let Some(name) =
        target.and_then(|t| t.strip_prefix("refs/remotes/origin/").map(str::to_string))
    {
        // `fetch --prune` can delete the target and leave the symref dangling: a name
        // that resolves to nothing is no default, or the picker could never mark the
        // default row and the name shield would carry a phantom.
        let probe = format!("refs/remotes/origin/{name}^{{commit}}");
        let resolves = git_tristate(repo, &["rev-parse", "--verify", "--quiet", &probe])?;
        return Ok(resolves.map(|_| name));
    }
    let Some(oid) = git_tristate(
        repo,
        &["rev-parse", "--verify", "--quiet", "refs/remotes/origin/HEAD^{commit}"],
    )?
    else {
        return Ok(None);
    };
    Ok(origin_tips(repo)?.into_iter().find_map(|(tip, name)| (tip == oid).then_some(name)))
}

/// Strip the ref prefixes a `--base` branch name may carry.
pub(crate) fn strip_base_prefix(entry: &str) -> String {
    ["refs/remotes/origin/", "refs/heads/", "origin/"]
        .iter()
        .find_map(|p| entry.strip_prefix(p))
        .unwrap_or(entry)
        .to_string()
}

/// One base picker row: a bare branch name and the unix time of its tip commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRow {
    pub name: String,
    pub tip_secs: u64,
}

/// Every branch for the base picker: `refs/heads` and `refs/remotes/origin` merged by
/// bare name, newest tip first, `origin/HEAD` excluded. A name on both sides keeps
/// origin's tip, the one the chain resolves it to. The checked-out branch is listed: it is
/// a legitimate base (the diff is then the uncommitted one), and excluding it is what left
/// a one-branch repo with no rows.
pub fn list_branches(repo: &Path) -> Result<Vec<BranchRow>, GitFail> {
    let out = git_strict(
        repo,
        &[
            "for-each-ref",
            "refs/remotes/origin",
            "refs/heads",
            "--sort=-committerdate",
            "--format=%(refname)%00%(committerdate:unix)",
        ],
    )?;
    // The sort interleaves origin and local refs by date, so origin's rows are taken in a
    // first pass and local ones fill in after: the merge keeps origin's tip by rule
    // (`BRANCH_REF_PREFIXES`), not by whichever side happens to be newer. A tip whose
    // date does not parse (a ref at a non-commit) keeps `0`, which paints as no age.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut rows: Vec<BranchRow> = Vec::new();
    for prefix in BRANCH_REF_PREFIXES {
        for line in out.lines() {
            let Some((refname, secs)) = line.split_once('\0') else { continue };
            let Some(name) = refname.strip_prefix(prefix) else { continue };
            if name == "HEAD" || !seen.insert(name) {
                continue;
            }
            rows.push(BranchRow { name: name.to_string(), tip_secs: secs.parse().unwrap_or(0) });
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.tip_secs));
    Ok(rows)
}

/// The checked-out branch's bare name, `None` when `HEAD` is detached.
pub fn checked_out_branch(repo: &Path) -> Result<Option<String>, GitFail> {
    git_tristate(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

/// The `origin` remote-tracking tips as `(OID, bare name)`, `origin/HEAD` excluded — one
/// listing per pass serves the frontier names and the published-at-all short-circuit.
fn origin_tips(repo: &Path) -> Result<Vec<(String, String)>, GitFail> {
    let out = git_strict(
        repo,
        &["for-each-ref", "refs/remotes/origin", "--format=%(objectname) %(refname)"],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let (oid, refname) = line.split_once(' ')?;
            let name = refname.strip_prefix("refs/remotes/origin/")?;
            (name != "HEAD").then(|| (oid.to_string(), name.to_string()))
        })
        .collect())
}

/// Whether `commit` is an ancestor of (or equal to) `of`.
fn is_ancestor(repo: &Path, commit: &str, of: &str) -> Result<bool, GitFail> {
    Ok(git_tristate(repo, &["merge-base", "--is-ancestor", commit, of])?.is_some())
}

/// Whether the pinned `HEAD` contains `commit` — the merged/closed admission guard: a
/// reused branch name never resurrects a PR whose commits this branch does not hold
/// A commit absent from the object database is not
/// contained; an unfetched head proves nothing.
pub fn contains_commit(repo: &Path, head: &str, commit: &str) -> Result<bool, GitFail> {
    if git_tristate(repo, &["cat-file", "-e", commit])?.is_none() {
        return Ok(false);
    }
    is_ancestor(repo, commit, head)
}

/// Peel `rev` to a commit object id. A leading `-` is not a
/// rev. An ambiguous abbreviated SHA is a miss, not an error.
pub fn resolve_commit(repo: &Path, rev: &str) -> Result<Option<String>, GitFail> {
    if rev.is_empty() || rev.starts_with('-') {
        return Ok(None);
    }
    let probe = format!("{rev}^{{commit}}");
    git_tristate(repo, &["rev-parse", "--verify", "--quiet", &probe])
}

/// The abbreviated object id the header and the picker paint.
#[must_use]
pub fn abbreviate_oid(oid: &str) -> String {
    const N: usize = 7;
    if oid.len() <= N { oid.to_string() } else { oid[..N].to_string() }
}

/// Whether `spelling` is a hex prefix of `oid`.
#[must_use]
pub fn spelling_is_sha_prefix(spelling: &str, oid: &str) -> bool {
    let s = spelling.to_ascii_lowercase();
    !s.is_empty()
        && s.bytes().all(|b| b.is_ascii_hexdigit())
        && oid.to_ascii_lowercase().starts_with(&s)
}

/// Shown name and optional abbreviated SHA for a non-branch spelling. A SHA prefix paints once; anything else keeps the spelling and
/// carries the mark.
#[must_use]
pub fn rev_paint(spelling: &str, oid: &str) -> (String, Option<String>) {
    let abbrev = abbreviate_oid(oid);
    if spelling_is_sha_prefix(spelling, oid) {
        (abbrev, None)
    } else {
        (spelling.to_string(), Some(abbrev))
    }
}

/// Complete a unique SHA prefix to the abbreviated object id. A spelling that is
/// already that abbrev, or a longer hex prefix of the oid (a pasted 40-hex), is kept
#[must_use]
pub fn complete_sha_prefix(spelling: &str, oid: &str) -> String {
    let abbrev = abbreviate_oid(oid);
    if spelling_is_sha_prefix(spelling, oid) && spelling.len() < abbrev.len() {
        abbrev
    } else {
        spelling.to_string()
    }
}

/// A branch name the picker would list, not `HEAD` and not a rev-walk.
#[must_use]
pub fn is_branch_label(value: &str) -> bool {
    branch_name_shaped(value) && !value.eq_ignore_ascii_case("HEAD")
}

/// Origin then local, else a verbatim commit.
pub(crate) fn resolve_spelling(
    repo: &Path,
    spelling: &str,
) -> Result<Option<ResolvedBase>, GitFail> {
    if let Some(oid) = resolve_base_entry(repo, spelling)? {
        return Ok(Some(ResolvedBase::branch(spelling.to_string(), oid)));
    }
    Ok(resolve_commit(repo, spelling)?.map(|oid| ResolvedBase::rev(spelling.to_string(), oid)))
}

/// `--base`: verbatim first, else prefix-stripped as a branch. A miss keeps the flag
/// spelling unless the stripped form is a branch name.
fn classify_flag(
    repo: &Path,
    flag: &str,
) -> Result<(Option<ResolvedBase>, Option<String>), GitFail> {
    let entry = strip_base_prefix(flag);
    let verbatim = resolve_commit(repo, flag)?;
    let via_branch = resolve_base_entry(repo, &entry)?;
    Ok(match (verbatim, via_branch) {
        (Some(oid), None) => (Some(ResolvedBase::rev(flag.to_string(), oid)), None),
        (Some(oid), Some(_)) | (None, Some(oid)) => (Some(ResolvedBase::branch(entry, oid)), None),
        (None, None) => {
            let skip = if is_branch_label(&entry) { entry } else { flag.to_string() };
            (None, Some(skip))
        }
    })
}

/// Where a bare branch name is looked up, in the order that decides a name on both
/// sides: origin's tip is what the PR sees, so it wins. One list serves the resolve, the
/// default fallback, and the picker's merge.
const BRANCH_REF_PREFIXES: [&str; 2] = ["refs/remotes/origin/", "refs/heads/"];

fn resolve_base_entry(repo: &Path, name: &str) -> Result<Option<String>, GitFail> {
    if !is_branch_label(name) {
        return Ok(None);
    }
    for prefix in BRANCH_REF_PREFIXES {
        let probe = format!("{prefix}{name}^{{commit}}");
        if let Some(oid) = git_tristate(repo, &["rev-parse", "--verify", "--quiet", &probe])? {
            return Ok(Some(oid));
        }
    }
    Ok(None)
}

/// Commits `local` (the pinned `HEAD` OID) is ahead and behind `other` (the PR head OID).
/// `Ok(None)` when `other` is not in the object database — the PR head was never fetched
/// locally, a clean absence. Backs the PR `sync` indicator.
pub fn ahead_behind_oids(
    repo: &Path,
    local: &str,
    other: &str,
) -> Result<Option<(u32, u32)>, GitFail> {
    // Plain `-e` (no `^{commit}` peel): peeling a missing object exits 128, not the
    // clean-absence 1 this check relies on.
    if git_tristate(repo, &["cat-file", "-e", other])?.is_none() {
        return Ok(None);
    }
    let out =
        git_strict(repo, &["rev-list", "--left-right", "--count", &format!("{local}...{other}")])?;
    let mut it = out.split_whitespace();
    let parse = |s: Option<&str>| {
        s.and_then(|v| v.parse().ok())
            .ok_or_else(|| GitFail(format!("rev-list --left-right returned {out:?}")))
    };
    let ahead = parse(it.next())?;
    let behind = parse(it.next())?;
    Ok(Some((ahead, behind)))
}

/// The merge-base commit of the resolved base OID and `HEAD`
pub fn merge_base(repo: &Path, base_oid: &str) -> Option<String> {
    git_line(repo, &["merge-base", base_oid, "HEAD"])
}

/// The worktree files, tracked and untracked but not ignored, whose text contains `needle`
/// verbatim. Binary files are skipped. `git grep` exits 1 for no match, which is an empty
/// list here, not a failure.
pub fn files_containing(repo: &Path, needle: &str) -> Result<Vec<String>> {
    let args = ["grep", "-l", "-z", "-I", "--untracked", "--fixed-strings", "-e", needle];
    let out = crate::proc::command("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()),
        Some(1) => Ok(Vec::new()),
        _ => bail!("git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
    }
}

/// Whether git ignores `path` (`git check-ignore`).
pub fn is_ignored(repo: &Path, path: &str) -> bool {
    git_ok(repo, &["check-ignore", "-q", "--", path])
}

/// The content of `path` at `rev` (`git show <rev>:<path>`). Empty when the path does
/// not exist at that rev — an added file against its old side, say.
pub fn file_content(repo: &Path, rev: &str, path: &str) -> String {
    git_lenient(repo, &["show", &format!("{rev}:{path}")])
}

// --- base pick (branch scope) --------------------------------------------------
//
// One revision spelling per worktree: a blob under `refs/worktree/diff-reckoner/base-pick`.
// Git isolates that namespace, so sibling worktrees do not share a pick.

const BASE_PICK_REF: &str = "refs/worktree/diff-reckoner/base-pick";

/// The recorded pick's spelling, or `None` when no pick is recorded. One git call, so a
/// concurrent write from another pane of this worktree can never split the read the way
/// an exists-then-read pair would; a failed read is no pick, matching the chain's
/// skip-never-error contract.
pub fn read_base_pick(repo: &Path) -> Result<Option<String>, GitFail> {
    let out = run_git(repo, &["cat-file", "blob", BASE_PICK_REF])?;
    if !out.status.success() {
        return Ok(None);
    }
    let name = String::from_utf8_lossy(&out.stdout);
    let name = name.trim();
    Ok(pick_spelling_shaped(name).then(|| name.to_string()))
}

/// One printable line, not a git option. `HEAD~1` and a tag are
/// picks. Control bytes are not.
fn pick_spelling_shaped(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value.bytes().all(|byte| byte > b' ' && byte != 0x7f)
}

/// Shape of a branch name the origin-then-local walk will accept. `HEAD` and rev-walk
/// spellings (`HEAD~1`) are not: git would parse them through `origin/HEAD`.
fn branch_name_shaped(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.contains("..")
        && !value.contains("@{")
        && !value.contains(['~', '^', ':', '?', '*', '[', '\\'])
        && value.bytes().all(|byte| byte > b' ' && byte != 0x7f)
}

/// Record `name` as this worktree's pick. The ref write lands before the pick applies,
/// so a crash between the two loses nothing.
///
/// A name spelling the default branch is no pick: the ref is deleted instead, so the pane
/// follows the repo's next re-default. The default is read here, at the write, so a
/// picker row marked at open cannot go stale under a fetch that moved `origin/HEAD`.
pub fn write_base_pick(repo: &Path, name: &str) -> Result<(), GitFail> {
    if Some(name) == default_branch_name(repo)?.as_deref() {
        return delete_base_pick(repo);
    }
    let blob = git_stdin(repo, &["hash-object", "-w", "--stdin"], name)?;
    git_strict(repo, &["update-ref", BASE_PICK_REF, blob.trim()])?;
    Ok(())
}

/// Forget this worktree's pick, so the base is the default branch again. Deleting a ref
/// that does not exist succeeds: git's `-d` without an old value is idempotent.
pub fn delete_base_pick(repo: &Path) -> Result<(), GitFail> {
    git_strict(repo, &["update-ref", "-d", BASE_PICK_REF])?;
    Ok(())
}

/// Run git with `input` piped to stdin, any non-zero exit a failure.
fn git_stdin(repo: &Path, args: &[&str], input: &str) -> Result<String, GitFail> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = crate::proc::command("git")
        .arg("-C")
        .arg(repo)
        .env("LC_ALL", "C")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| GitFail(format!("git {args:?}: {e}")))?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(input.as_bytes())
        .map_err(|e| GitFail(format!("git {args:?}: {e}")))?;
    let out = child.wait_with_output().map_err(|e| GitFail(format!("git {args:?}: {e}")))?;
    if !out.status.success() {
        return Err(GitFail(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// git's well-known empty-tree object, used as the diff base when a repo has no commits.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// `HEAD` when the repo has a commit, else the empty tree (a commitless repo has no HEAD).
fn diff_base(repo: &Path) -> String {
    if git(repo, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        "HEAD".to_string()
    } else {
        EMPTY_TREE.to_string()
    }
}

/// The changed files for `scope`, sorted by path. `branch_base` is the resolved base OID
/// for the `branch` scope ([`resolve_base`]'s winner); with none the scope lists nothing.
pub fn changed_files(
    repo: &Path,
    scope: Scope,
    branch_base: Option<&str>,
) -> Result<Vec<ChangedFile>> {
    let (numstat, name_status) = match scope {
        Scope::Uncommitted => {
            // A repo with no commits has no HEAD; diff against the empty tree so a fresh
            // `git init` lists its files instead of erroring (which would kill the process).
            let base = diff_base(repo);
            (
                git(repo, &["diff", &base, "--numstat", "-z"])?,
                git(repo, &["diff", &base, "--name-status", "-z"])?,
            )
        }
        Scope::Branch => match branch_base.and_then(|b| merge_base(repo, b)) {
            Some(r) => (
                git(repo, &["diff", &r, "--numstat", "-z"])?,
                git(repo, &["diff", &r, "--name-status", "-z"])?,
            ),
            None => return Ok(Vec::new()),
        },
        // `commits` diffs through its own entry point.
        Scope::Commits => return Ok(Vec::new()),
    };
    // Branch diffs against the worktree, so like uncommitted it carries untracked files
    // that `git diff` never reports.
    let include_untracked = matches!(scope, Scope::Uncommitted | Scope::Branch);
    assemble(repo, &numstat, &name_status, include_untracked)
}

/// The changed files between two commits, `old` against `new`, for the `commits` scope:
/// both sides are committed trees, so no untracked pass runs. `old` may be the empty tree for a root commit.
pub fn changed_between(repo: &Path, old: &str, new: &str) -> Result<Vec<ChangedFile>> {
    let numstat = git(repo, &["diff", old, new, "--numstat", "-z"])?;
    let name_status = git(repo, &["diff", old, new, "--name-status", "-z"])?;
    assemble(repo, &numstat, &name_status, false)
}

/// `sha`'s first parent, or the empty tree when `sha` is a root commit: the old side of a
/// run whose oldest commit is `sha`. `None` when the
/// commit itself is missing. The parent is read from the raw commit object, so a parent the
/// repository lacks (a shallow clone's cut) is named, not mistaken for a root: the caller's
/// existence check then reports it `gone`.
pub fn parent_or_empty(repo: &Path, sha: &str) -> Option<String> {
    let object = git(repo, &["cat-file", "-p", &format!("{sha}^{{commit}}")]).ok()?;
    let parent = object
        .lines()
        .take_while(|l| !l.is_empty())
        .find_map(|l| l.strip_prefix("parent "))
        .map_or(EMPTY_TREE, str::trim);
    Some(parent.to_string())
}

/// The commit `HEAD` names, or `None` in an unborn repository. The commit picker's universe
/// is keyed by it, so a poll re-lists only when it moved.
pub fn head_oid(repo: &Path) -> Option<String> {
    git_line(repo, &["rev-parse", "--verify", "-q", "HEAD"])
}

/// `sha`'s subject line, for the header paint.
pub fn commit_subject(repo: &Path, sha: &str) -> Option<String> {
    git_line(repo, &["log", "-1", "--format=%s", sha])
}

/// Whether `sha` names a commit the repository still holds (`gone`).
pub fn commit_exists(repo: &Path, sha: &str) -> bool {
    git_ok(repo, &["cat-file", "-e", &format!("{sha}^{{commit}}")])
}

/// Whether `sha` is reachable from `HEAD` (`off branch`). A missing
/// commit is unreachable.
pub fn is_reachable(repo: &Path, sha: &str) -> bool {
    git_ok(repo, &["merge-base", "--is-ancestor", sha, "HEAD"])
}

/// How many commits `oldest..=newest` spans along the first-parent walk from `newest`
/// `None` when either end is missing, or `oldest` is not behind `newest`.
pub fn run_length(repo: &Path, oldest: &str, newest: &str) -> Option<usize> {
    let old = parent_or_empty(repo, oldest)?;
    run_length_from(repo, &old, oldest, newest)
}

/// [`run_length`] with `oldest`'s parent already resolved, so a build that has it spawns
/// nothing twice.
pub fn run_length_from(repo: &Path, old: &str, oldest: &str, newest: &str) -> Option<usize> {
    if oldest == newest {
        return Some(1);
    }
    let mut args = vec!["rev-list", "--count", "--first-parent", newest];
    let exclude;
    if old != EMPTY_TREE {
        if !git_ok(repo, &["merge-base", "--is-ancestor", oldest, newest]) {
            return None;
        }
        exclude = format!("^{old}");
        args.push(&exclude);
    }
    git_line(repo, &args)?.parse().ok()
}

/// One row of the commit picker: the full id, the subject,
/// the committer time as unix seconds, the author, the refs pointing at it, and whether it
/// is a merge.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitRow {
    pub sha: String,
    pub subject: String,
    pub time: u64,
    pub author: String,
    /// The refs pointing at the commit, `HEAD` and the checked-out branch dropped.
    pub refs: Vec<CommitRef>,
    pub merge: bool,
}

/// A ref a picker row can show, by kind, so the row's one ref ranks by what it is rather
/// than by how it is spelled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommitRef {
    /// A remote-tracking tip, shown as `origin/feature`.
    Remote(String),
    /// A tag, shown as `tag: v1`.
    Tag(String),
    /// A local branch other than the one checked out, shown by name.
    Branch(String),
}

impl CommitRef {
    pub fn label(&self) -> String {
        match self {
            Self::Remote(r) | Self::Branch(r) => r.clone(),
            Self::Tag(t) => format!("tag: {t}"),
        }
    }
}

/// The picker's universe, newest first, along the first-parent walk from `HEAD`:
/// `merge_base..HEAD` when the base has one, or the last 50 commits without. First-parent only, so any contiguous run of rows is one ancestor
/// chain and diffs as `A^..B`. An unborn repository lists nothing.
pub fn list_commits(repo: &Path, merge_base: Option<&str>) -> Result<Vec<CommitRow>> {
    if head_oid(repo).is_none() {
        return Ok(Vec::new());
    }
    let range = merge_base.map(|mb| format!("{mb}..HEAD"));
    let mut args = vec![
        "log",
        "--first-parent",
        "--decorate=full",
        "--format=%H%x00%s%x00%ct%x00%an%x00%D%x00%P",
        "-z",
    ];
    match &range {
        Some(r) => args.push(r),
        None => args.extend(["-50", "HEAD"]),
    }
    let out = git(repo, &args)?;
    Ok(parse_commit_log(&out))
}

/// Parse `git log --format=%H%x00%s%x00%ct%x00%an%x00%D%x00%P -z` output: six NUL-separated
/// fields per commit, commits themselves NUL-terminated.
fn parse_commit_log(out: &str) -> Vec<CommitRow> {
    let fields: Vec<&str> = out.split('\0').collect();
    fields
        .chunks(6)
        .filter(|c| c.len() == 6 && !c[0].is_empty())
        .map(|c| CommitRow {
            sha: c[0].to_string(),
            subject: c[1].to_string(),
            time: c[2].trim().parse().unwrap_or(0),
            author: c[3].to_string(),
            refs: parse_decorations(c[4]),
            merge: c[5].split_whitespace().count() > 1,
        })
        .collect()
}

/// `%D` under `--decorate=full` as typed refs: `HEAD -> refs/heads/feature,
/// refs/remotes/origin/feature, tag: refs/tags/v1` becomes `Remote("origin/feature")`,
/// `Tag("v1")`. `HEAD` and the branch it is on are dropped, since the top row is `HEAD` by
/// construction and its branch is the one being reviewed.
fn parse_decorations(d: &str) -> Vec<CommitRef> {
    d.split(", ")
        .map(str::trim)
        .filter(|r| !r.is_empty() && *r != "HEAD" && !r.starts_with("HEAD -> "))
        .filter_map(|r| {
            if let Some(t) = r.strip_prefix("tag: refs/tags/") {
                Some(CommitRef::Tag(t.to_string()))
            } else if let Some(t) = r.strip_prefix("refs/remotes/") {
                Some(CommitRef::Remote(t.to_string()))
            } else {
                r.strip_prefix("refs/heads/").map(|b| CommitRef::Branch(b.to_string()))
            }
        })
        .collect()
}

/// One entry in the `All files` worktree listing: a path plus whether git ignores it and
/// whether it is a (lazily-expanded) directory placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: String,
    pub ignored: bool,
    pub is_dir: bool,
}

/// Every entry in the worktree for the `All files` tab: tracked and
/// untracked-not-ignored files from one `ls-files --cached --others` pass, and the ignored
/// entries from [`ignored_entries`] — a wholly-ignored directory collapsed to one `is_dir`
/// placeholder, an individually-ignored file as itself. `.git` is never reported. Deduped and
/// sorted; `-z` keeps paths with spaces or special characters verbatim.
pub fn all_files(repo: &Path) -> Result<Vec<WorktreeEntry>> {
    // One spawn for tracked + untracked. `--others --exclude-standard` applies the same
    // standard exclude rules as the `status` untracked pass `changed_files` runs, so the
    // untracked sets match without a status walk.
    let listed = git(repo, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"])?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in listed.split('\0').filter(|s| !s.is_empty()) {
        if seen.insert(path.to_string()) {
            out.push(WorktreeEntry { path: path.to_string(), ignored: false, is_dir: false });
        }
    }
    for (path, is_dir) in ignored_entries(repo)? {
        if seen.insert(path.clone()) {
            out.push(WorktreeEntry { path, ignored: true, is_dir });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The ignored entries: a wholly-ignored directory comes back as `dir/` (mapped to
/// `is_dir = true`), an individually-ignored file as itself.
///
/// `ls-files --directory` prunes at each ignored directory instead of walking inside it, where
/// `git status --ignored` enumerates the whole tree — seconds against a large `node_modules`.
/// `--no-empty-directory` matches `status`'s output exactly, which skips empty ignored dirs.
fn ignored_entries(repo: &Path) -> Result<Vec<(String, bool)>> {
    let out = git(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
        ],
    )?;
    Ok(out
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|path| match path.strip_suffix('/') {
            Some(dir) => (dir.to_string(), true),
            None => (path.to_string(), false),
        })
        .collect())
}

/// The immediate children of a wholly-ignored directory, for lazy expansion in `All files`
/// Everything under an ignored directory is ignored, so this reads the
/// filesystem directly; sub-directories come back as `is_dir` placeholders to expand in turn.
/// An unreadable directory yields no children rather than failing the reload, so expansion is
/// best-effort.
pub fn list_ignored_dir(repo: &Path, dir: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(repo.join(dir)) else { return out };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else { continue };
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        out.push(WorktreeEntry { path: format!("{dir}/{name}"), ignored: true, is_dir });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Build the sorted `ChangedFile` list from `git diff` numstat + name-status output,
/// optionally appending untracked files (which a `git diff` never reports).
fn assemble(
    repo: &Path,
    numstat: &str,
    name_status: &str,
    include_untracked: bool,
) -> Result<Vec<ChangedFile>> {
    let counts = parse_numstat(numstat);
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for (kind, path, previous_path) in parse_name_status(name_status) {
        if !seen.insert(path.clone()) {
            continue;
        }
        let (additions, deletions) = counts.get(&path).copied().unwrap_or((0, 0));
        files.push(ChangedFile { path, kind, additions, deletions, previous_path });
    }

    if include_untracked {
        // Untracked-not-ignored files list as additions. One `ls-files --others` pass — the
        // same definition of untracked `all_files` uses, so the two views can't disagree.
        // `-z` keeps paths with spaces or special characters verbatim, and files inside a
        // brand-new directory list individually (.gitignore still applies).
        let others = git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        for path in others.split('\0').filter(|s| !s.is_empty()) {
            let path = path.to_string();
            if seen.insert(path.clone()) {
                let additions = untracked_additions(repo, &path);
                files.push(ChangedFile {
                    path,
                    kind: ChangeKind::Untracked,
                    additions,
                    deletions: 0,
                    previous_path: None,
                });
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Addition count of an untracked file: its line count, which is what `git diff` against
/// nothing reports (0 for empty or binary). Read locally rather than shelling
/// `git diff --no-index` per file — with `--untracked-files=all` a large untracked tree
/// would otherwise fork git once per file on every poll and freeze the UI.
fn untracked_additions(repo: &Path, path: &str) -> u32 {
    let Ok(bytes) = std::fs::read(repo.join(path)) else { return 0 };
    if bytes.is_empty() || bytes.contains(&0) {
        return 0; // empty, or binary (a NUL byte) — git reports no line additions
    }
    // Lines = newline count, plus one for a final line with no trailing newline. A plain
    // byte count is fine for one already-read file; no need for the bytecount crate.
    #[allow(clippy::naive_bytecount)]
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let trailing = usize::from(bytes.last() != Some(&b'\n'));
    (newlines + trailing) as u32
}

// --- pure parsers (unit-tested without a repo) ---------------------------------

/// Map of new-path to `(additions, deletions)` from `git diff --numstat -z`.
///
/// Under `-z` a non-rename record is `ADDS\tDELS\tPATH\0`; a rename/copy record is
/// `ADDS\tDELS\t\0OLD\0NEW\0` — the counts ride the front, then old and new arrive as
/// their own NUL fields (no `=>` arrow, no brace factoring). Binary files emit `-`/`-`,
/// which parse to 0. The counts key under the new path, matching `parse_name_status`.
fn parse_numstat(out: &str) -> HashMap<String, (u32, u32)> {
    let mut map = HashMap::new();
    let mut it = out.split('\0');
    while let Some(field) = it.next() {
        // `splitn(3)` keeps any tabs inside the path (verbatim under `-z`) intact.
        let mut parts = field.splitn(3, '\t');
        let add = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let del = parts.next().unwrap_or("0").parse().unwrap_or(0);
        match parts.next() {
            // Non-rename: the path rode this same field.
            Some(path) if !path.is_empty() => {
                map.insert(path.to_string(), (add, del));
            }
            // Rename/copy: the next two fields are the old and new paths.
            Some(_) => {
                let _old = it.next();
                if let Some(new) = it.next().filter(|n| !n.is_empty()) {
                    map.insert(new.to_string(), (add, del));
                }
            }
            // No tab fields — a trailing empty record after the final NUL.
            None => {}
        }
    }
    map
}

/// `(kind, path, previous_path)` from `git diff --name-status -z`. Under `-z` each record is
/// `STATUS\0PATH\0`, except a rename/copy is `R<score>\0OLD\0NEW\0` (status, then old and new
/// as separate fields). A rename or copy takes the new path and carries its old path; every
/// other kind has `previous_path == None`. Copy folds into `Renamed` — a copy's old content
/// lives at the old path exactly like a rename, which is what `content_sides` reads.
fn parse_name_status(out: &str) -> Vec<(ChangeKind, String, Option<String>)> {
    let mut rows = Vec::new();
    let mut it = out.split('\0');
    while let Some(status) = it.next() {
        let row = match status.chars().next() {
            Some('A') => it.next().map(|p| (ChangeKind::Added, p.to_string(), None)),
            Some('D') => it.next().map(|p| (ChangeKind::Deleted, p.to_string(), None)),
            Some('R' | 'C') => {
                let old = it.next();
                it.next().map(|new| (ChangeKind::Renamed, new.to_string(), old.map(str::to_string)))
            }
            // Modified, type-changed, etc.; also skips the trailing empty record.
            Some(_) => it.next().map(|p| (ChangeKind::Modified, p.to_string(), None)),
            None => None,
        };
        if let Some((kind, path, prev)) = row
            && !path.is_empty()
        {
            rows.push((kind, path, prev));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{ChangeKind, parse_name_status, parse_numstat};

    #[test]
    fn worktree_of_distinguishes_a_repo_from_a_plain_directory() {
        use super::{Worktree, worktree_of};
        // A plain directory git can read but that holds no worktree.
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(worktree_of(outside.path()), Worktree::Outside);
        // A real worktree resolves to its root. Compare against std canonicalization, an oracle
        // independent of `worktree_of` (both git and std resolve the temp dir's symlinks).
        let repo = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        let canonical = std::fs::canonicalize(repo.path()).unwrap();
        assert_eq!(worktree_of(repo.path()), Worktree::Root(canonical));
    }

    #[test]
    fn numstat_parses_counts_and_ignores_binary() {
        let m = parse_numstat("18\t8\tsrc/a.rs\0-\t-\tassets/logo.png\0");
        assert_eq!(m["src/a.rs"], (18, 8));
        assert_eq!(m["assets/logo.png"], (0, 0));
    }

    #[test]
    fn numstat_keys_renames_under_the_new_path() {
        // Under `-z` a rename is `ADDS\tDELS\t\0OLD\0NEW`: old and new are their own fields,
        // no `=>` arrow or brace form. Counts must key under the new path.
        let m = parse_numstat("3\t1\t\0src/old.rs\0src/new.rs\0");
        assert_eq!(m["src/new.rs"], (3, 1));
        assert!(!m.contains_key("src/old.rs"));
    }

    #[test]
    fn numstat_dir_removing_rename_has_no_double_slash() {
        // Regression: the old brace parser produced `a//file.rs` here, so counts never matched.
        let m = parse_numstat("4\t2\t\0a/b/file.rs\0a/file.rs\0");
        assert_eq!(m["a/file.rs"], (4, 2));
        assert!(!m.contains_key("a//file.rs"));
    }

    #[test]
    fn numstat_handles_a_mixed_stream() {
        // binary, plain, rename, in sequence — the rename lookahead must stay aligned.
        // `\x00` (= NUL) is used as the separator so the digits after it read clearly.
        let m = parse_numstat("-\t-\tlogo.png\x009\t1\tsrc/a.rs\x005\t4\t\x00o.rs\x00n.rs\x00");
        assert_eq!(m["logo.png"], (0, 0));
        assert_eq!(m["src/a.rs"], (9, 1));
        assert_eq!(m["n.rs"], (5, 4));
    }

    #[test]
    fn name_status_kinds_and_rename_target() {
        let rows =
            parse_name_status("M\0src/a.rs\0A\0src/b.rs\0D\0src/c.rs\0R100\0old.rs\0new.rs\0");
        assert_eq!(rows[0], (ChangeKind::Modified, "src/a.rs".to_string(), None));
        assert_eq!(rows[1], (ChangeKind::Added, "src/b.rs".to_string(), None));
        assert_eq!(rows[2], (ChangeKind::Deleted, "src/c.rs".to_string(), None));
        assert_eq!(
            rows[3],
            (ChangeKind::Renamed, "new.rs".to_string(), Some("old.rs".to_string()))
        );
    }

    #[test]
    fn name_status_copy_keeps_the_new_path() {
        // A copy carries old + new like a rename; it must key under the new path, not collapse
        // to a Modified entry on the source path.
        let rows = parse_name_status("C75\0orig.rs\0copy.rs\0");
        assert_eq!(
            rows[0],
            (ChangeKind::Renamed, "copy.rs".to_string(), Some("orig.rs".to_string()))
        );
    }
}
