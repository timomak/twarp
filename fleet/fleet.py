#!/usr/bin/env python3
"""twarp dev fleet orchestrator.

Roles (all driven from this file):
  dispatch   pick eligible, file-disjoint, dependency-unblocked items and lease them
  worker     author one item on its node (local=claude / other-mac=codex) -> push branch
  gate       build + targeted tests for a branch on the build node (other-mac)
  supervise  bors-style merge queue: speculative-merge each green branch, re-gate, auto-merge
  run        full loop: dispatch -> workers (parallel) -> gate -> supervise

Design notes:
  * other-mac is the single BUILD NODE (warm cache + working toolchain). All building/testing
    happens there, regardless of which node authored the branch. This Mac orchestrates + merges.
  * Workers run in isolated git worktrees so parallel authors never share a working tree.
  * Auto-merge only happens after the functional gate passes on the *merge with current master*
    (catches green-alone-but-break-together semantic conflicts).
"""
import argparse, concurrent.futures as cf, json, os, re, subprocess, sys, threading, time
from pathlib import Path

_qlock = threading.Lock()   # serialize all access to queue.json across worker threads
_screenlock = threading.Lock()  # the build node has ONE display — serialize real-display captures
_gatelock = threading.Lock()    # the build node has ONE cargo cache — serialize all builds/tests
MAX_ROUNDS = 4   # per-PR fix-until-green+approved attempts before giving up

LOCAL_REPO = Path("/Users/thirdfacedev/Development/twarp")
QUEUE = LOCAL_REPO / "fleet" / "queue.json"
LOCAL_WT = Path("/Users/thirdfacedev/Development/twarp-fleet-wt")
REMOTE = "other-mac"
REMOTE_REPO = "$HOME/Development/twarp"
REMOTE_WT = "$HOME/Development/twarp-fleet-wt"
REMOTE_ENV = "source ~/.config/twarp-fleet/env; set -a; source ~/.codex/.env-foundry; set +a"
LOG = LOCAL_REPO / "fleet" / "runs"
ROADMAP = LOCAL_REPO / "roadmap" / "ROADMAP.md"
INFLIGHT = {"leased", "building", "authored", "iterating", "gating", "gated", "ready", "merging"}


# ---------- io helpers ----------
def now():
    return time.strftime("%H:%M:%S")

def say(msg):
    print(f"[{now()}] {msg}", flush=True)

def load():
    return json.loads(QUEUE.read_text())

def save(q):
    QUEUE.write_text(json.dumps(q, indent=2) + "\n")

def item(q, iid):
    return next(i for i in q["items"] if i["id"] == iid)

def set_status(iid, status, **extra):
    with _qlock:
        q = load()
        it = item(q, iid)
        it["status"] = status
        it.update(extra)
        save(q)
    say(f"  {iid} -> {status}")

def sh(cmd, cwd=None, timeout=None, check=False):
    r = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str),
                       capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"cmd failed ({r.returncode}): {cmd}\n{r.stdout}\n{r.stderr}")
    return r

def remote_fresh_worktree(wt, ref, branch=None):
    """Script that force-creates a clean worktree at `wt` checked out to `ref` (robust against
    orphan dirs left by killed runs)."""
    bflag = f"-B {branch}" if branch else "--detach"
    delbranch = f"git branch -D {branch} 2>/dev/null || true\n" if branch else ""
    return (f"{REMOTE_ENV}\ncd {REMOTE_REPO}\ngit fetch -q origin\n"
            f"git worktree remove --force {wt} 2>/dev/null || true\n"
            f"rm -rf {wt}\ngit worktree prune\n{delbranch}"
            f"git worktree add -f {bflag} {wt} {ref}\n")


def ssh(remote_cmd, timeout=None, check=False):
    """Run a bash script on the build node. The script is fed via stdin so the remote login
    shell never re-parses it — this avoids the `ssh host bash -lc "a; b"` trap where everything
    after the first `;` is run by the remote default shell (zsh) instead of bash."""
    r = subprocess.run(["ssh", "-o", "ConnectTimeout=10", REMOTE, "bash", "-s"],
                       input=remote_cmd, capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"ssh failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r


# ---------- roadmap bridge ----------
def _active_feature():
    if not ROADMAP.exists():
        return None
    m = re.search(r"\*\*Currently active:\*\*\s*`([^`]+)`", ROADMAP.read_text())
    return m.group(1) if m else None

def roadmap_sync():
    """Pull the next unchecked IMPL sub-phase of the active roadmap feature into the queue.
    Specs are human-gated: this only acts when the feature is `impl-pending`; for any other phase it
    pulls nothing and explains what's needed. Pulls ONE sub-phase at a time (they're sequential and
    share files). Returns a one-line status string."""
    feat = _active_feature()
    if not feat:
        return "no active feature in ROADMAP.md"
    status_md = LOCAL_REPO / "roadmap" / feat / "STATUS.md"
    if not status_md.exists():
        return f"{feat}: no STATUS.md"
    text = status_md.read_text()
    pm = re.search(r"\*\*Phase:\*\*\s*`?([a-z-]+)`?", text)
    phase = pm.group(1) if pm else "unknown"
    if phase != "impl-pending":
        hint = {"spec-in-review": "review/merge the spec PR", "spec-pending": "run /twarp-next to write specs",
                "not-started": "run /twarp-next to start specs", "impl-in-review": "review/merge the open impl PR",
                "merged": "feature done — advance ROADMAP to the next feature"}.get(phase, "no fleet action")
        return f"{feat}: phase={phase} — {hint}; nothing pulled (specs stay human)"
    m = re.search(r"^- \[ \] \*\*([0-9]+[a-z]) — ([^*]+?)\.?\*\*\s*(.*)$", text, re.M)
    if not m:
        return f"{feat}: impl-pending but no unchecked sub-phase found"
    sub_id, sub_title, sub_desc = m.group(1), m.group(2).strip(), m.group(3).strip()
    q = load()
    if any(it["id"] == sub_id for it in q["items"]):
        return f"{sub_id}: already in queue"
    base = next((it for it in q["items"]
                 if it["id"] == feat or it["id"].split("-")[0] == feat.split("-")[0]), None)
    touches = (base["touches"][:] if base else ["app/**"]) + [f"roadmap/{feat}/STATUS.md"]
    verify = base["verify"] if base else "cargo build --bin warp-oss"
    task = (f"Implement sub-phase {sub_id} of roadmap feature {feat}. FIRST read the merged specs "
            f"roadmap/{feat}/PRODUCT.md and roadmap/{feat}/TECH.md. Sub-phase {sub_id} — {sub_title}: "
            f"{sub_desc}\nImplement ONLY this sub-phase, scoped to its files. When done, tick this "
            f"sub-phase's checkbox in roadmap/{feat}/STATUS.md from `- [ ]` to `- [x]`.")
    q["items"].append({"id": sub_id, "title": f"{feat} {sub_id}: {sub_title}", "node": "other-mac",
                       "status": "queued", "depends_on": [], "touches": touches, "barrier": False,
                       "task": task, "verify": verify, "ux": False})
    save(q)
    return f"queued {sub_id} — {sub_title}"


# ---------- dispatcher ----------
def glob_prefix(g):
    return g.split("*")[0].rstrip("/")

def touches_overlap(a, b):
    for ga in a:
        pa = glob_prefix(ga)
        for gb in b:
            pb = glob_prefix(gb)
            if pa == "" or pb == "" or pa == pb or pa.startswith(pb + "/") or pb.startswith(pa + "/"):
                return True
    return False

def eligible(q, cap=None):
    items = q["items"]
    merged = {i["id"] for i in items if i["status"] == "merged"}
    inflight = [i for i in items if i["status"] in INFLIGHT]
    cap = cap if cap is not None else q["config"].get("concurrency", 2)
    if any(i.get("barrier") for i in inflight):
        return []  # a barrier is running -> nothing else
    claimed_touches = [i["touches"] for i in inflight]
    picked = []
    for it in items:
        if len(inflight) + len(picked) >= cap:
            break
        if it["status"] != "queued":
            continue
        if it.get("barrier"):
            continue  # barriers are launched explicitly, solo
        if not set(it["depends_on"]).issubset(merged):
            continue
        if any(touches_overlap(it["touches"], t) for t in claimed_touches + [p["touches"] for p in picked]):
            continue
        picked.append(it)
    return picked

def cmd_status(_):
    q = load()
    print(f"\ntwarp fleet — concurrency={q['config']['concurrency']} build_node={q['config']['build_node']}\n")
    for it in q["items"]:
        dep = f" deps={it['depends_on']}" if it["depends_on"] else ""
        bar = " [BARRIER]" if it.get("barrier") else ""
        br = f" {it.get('branch','')}" if it.get("branch") else ""
        print(f"  {it['status']:>9}  {it['id']:<26}{bar}{dep}{br}")
    elig = eligible(q)
    print(f"\n  eligible now: {[i['id'] for i in elig] or '—'}\n")


# ---------- worker ----------
def write_prompt(it):
    LOG.mkdir(parents=True, exist_ok=True)
    p = LOG / f"{it['id']}.prompt.txt"
    body = (
        f"You are a twarp fleet worker. Read AGENTS.md at the repo root first (origin-only, never "
        f"upstream, never merge, stay in scope). You are on a fresh branch off origin/master in a "
        f"dedicated worktree. Implement EXACTLY this task and nothing else:\n\n{it['task']}\n\n"
        f"When done, ensure the working tree contains only the intended changes. Do not commit or "
        f"push — the fleet harness handles that. Output one final line: WORKER_DONE {it['id']}\n"
    )
    p.write_text(body)
    return p

def write_text_prompt(iid, text):
    LOG.mkdir(parents=True, exist_ok=True)
    p = LOG / f"{iid}.prompt.txt"
    p.write_text(text)
    return p

def worker_local(it, ref="origin/master", prompt_text=None):
    iid = it["id"]
    wt = LOCAL_WT / iid
    sh(["git", "-C", str(LOCAL_REPO), "fetch", "-q", "origin"], check=True)
    sh(["git", "-C", str(LOCAL_REPO), "worktree", "remove", "--force", str(wt)])
    sh(f"rm -rf {wt}")
    sh(["git", "-C", str(LOCAL_REPO), "worktree", "prune"])
    sh(["git", "-C", str(LOCAL_REPO), "branch", "-D", f"fleet/{iid}"])
    sh(["git", "-C", str(LOCAL_REPO), "worktree", "add", "-B", f"fleet/{iid}", str(wt), ref], check=True)
    prompt = write_prompt(it) if prompt_text is None else write_text_prompt(iid, prompt_text)
    say(f"  [{iid}] local Claude worker authoring…")
    r = sh(["claude", "-p", prompt.read_text(), "--dangerously-skip-permissions"],
           cwd=str(wt), timeout=1200)
    (LOG / f"{iid}.author.log").write_text(r.stdout + "\n---STDERR---\n" + r.stderr)
    return _commit_push_local(it, wt)

def _commit_push_local(it, wt):
    iid = it["id"]
    sh(["git", "-C", str(wt), "add", "-A"], check=True)
    diff = sh(["git", "-C", str(wt), "diff", "--cached", "--stat"]).stdout.strip()
    if not diff:
        return False, "no changes produced"
    sh(["git", "-C", str(wt), "commit", "-m", f"fleet({iid}): {it['title']}"], check=True)
    sh(["git", "-C", str(wt), "push", "-q", "-f", "-u", "origin", f"fleet/{iid}"], check=True)
    return True, diff

def worker_remote(it, ref="origin/master", prompt_text=None):
    iid = it["id"]
    prompt = write_prompt(it) if prompt_text is None else write_text_prompt(iid, prompt_text)
    sh(["scp", "-q", str(prompt), f"{REMOTE}:/tmp/fleet_{iid}.prompt"], check=True)
    wt = f"{REMOTE_WT}/{iid}"
    ssh(remote_fresh_worktree(wt, ref=ref, branch=f"fleet/{iid}"), timeout=120, check=True)
    say(f"  [{iid}] remote Codex worker authoring…")
    run = (f"{REMOTE_ENV}\n"
           f"cd {wt}\n"
           f"cat /tmp/fleet_{iid}.prompt | $HOME/.local/bin/codex exec "
           f"--dangerously-bypass-approvals-and-sandbox -c model_reasoning_effort='high' "
           f"> /tmp/fleet_{iid}.author.log 2>&1\n"
           f"echo CODEX_EXIT_$?\n")
    r = ssh(run, timeout=1500)
    (LOG / f"{iid}.author.log").write_text(ssh(f"cat /tmp/fleet_{iid}.author.log").stdout)
    if "CODEX_EXIT_0" not in r.stdout:
        return False, f"codex did not exit clean: {r.stdout.strip()[-200:]}"
    # commit + push on remote, then VERIFY the branch is really on origin
    commit = (f"set -e\ncd {wt}\ngit add -A\n"
              f"if git diff --cached --quiet; then echo NO_CHANGES; exit 0; fi\n"
              f"git commit -q -m 'fleet({iid}): {it['title']}'\n"
              f"git push -q -f -u origin fleet/{iid}\n"
              f"git diff --stat origin/master..origin/fleet/{iid} | tail -4\n"
              f"echo PUSHED_$(git rev-parse --short HEAD)\n")
    c = ssh(commit, timeout=120)
    if "NO_CHANGES" in c.stdout:
        return False, "no changes produced"
    ok = "PUSHED_" in c.stdout and bool(ssh(f"cd {REMOTE_REPO} && git ls-remote origin fleet/{iid}").stdout.strip())
    return ok, c.stdout.strip()

def run_worker(it):
    try:
        set_status(it["id"], "building", branch=f"fleet/{it['id']}")
        ok, info = worker_remote(it) if it["node"] == REMOTE else worker_local(it)
        set_status(it["id"], "authored" if ok else "failed")
        say(f"  [{it['id']}] author {'OK' if ok else 'FAILED'}: {info.splitlines()[-1] if info else ''}")
        return it["id"], ok
    except Exception as e:
        set_status(it["id"], "failed")
        say(f"  [{it['id']}] worker EXC: {e}")
        return it["id"], False


# ---------- UX gate (real-display screenshot + golden baseline) ----------
UXTEST = "test_video_recording"   # bootstraps the UI and captures after_bootstrap.png
GOLDEN = LOCAL_REPO / "fleet" / "golden"

def vision_review(new_png, golden_png):
    """A headless vision agent compares the new screenshot against the approved golden and judges
    whether there is a USER-VISIBLE regression. Robust to anti-aliasing/cursor/timestamp noise in a
    way a pixel diff is not. Returns (verdict, note) with verdict in {'pass','regression','error'}."""
    prompt = (
        "You are a UX visual-regression gate for the twarp terminal app. Two screenshots:\n"
        f"  GOLDEN (approved baseline): {golden_png}\n"
        f"  NEW (current build):        {new_png}\n"
        "Use your Read tool to view BOTH images, then decide if NEW has any real user-visible "
        "regression vs GOLDEN: misaligned / overlapping / clipped / missing UI, broken layout, "
        "wrong colors or contrast, unstyled controls. IGNORE benign differences (anti-aliasing, "
        "cursor blink, timestamps, tiny 1-2px shifts).\n"
        "Reply with EXACTLY one line:\n"
        "  VERDICT pass — <short reason>        (if visually equivalent / acceptable)\n"
        "  VERDICT regression — <what broke>    (if there is a real regression)")
    r = sh(["claude", "-p", prompt, "--dangerously-skip-permissions"], timeout=300)
    out = (r.stdout or "").strip()
    line = next((l for l in out.splitlines() if "VERDICT" in l.upper()),
                out.splitlines()[-1] if out else "")
    low = line.lower()
    if "regression" in low:
        return "regression", line
    if "pass" in low:
        return "pass", line
    return "error", line or "no verdict from vision agent"


def uxgate(test=UXTEST, png="after_bootstrap.png"):
    """Render twarp on the build node's REAL display, capture a screenshot, pull it here, and have a
    vision agent compare it to the golden baseline. Returns (verdict, local_png_path):
      'golden-saved' first run (baseline stored), 'pass' / 'regression' from the vision agent,
      'fail' no screenshot produced, 'error' no verdict. Captures hold the screen semaphore so only
      one real-display render runs at a time."""
    GOLDEN.mkdir(parents=True, exist_ok=True); LOG.mkdir(parents=True, exist_ok=True)
    art = "/tmp/uxgate"
    cmd = (f"{REMOTE_ENV}\nexport CARGO_TARGET_DIR={REMOTE_REPO}/target\n"
           f"export WARP_INTEGRATION_TEST_ARTIFACTS_DIR={art}\n"
           f"export WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1\n"
           f"caffeinate -dimsu & CAF=$!\n"
           f"rm -rf {art}; mkdir -p {art}\ncd {REMOTE_REPO}\n"
           f"./target/debug/integration {test} > /tmp/uxgate.log 2>&1 || true\n"
           f"kill $CAF 2>/dev/null\n"
           f"find {art} -name '{png}' -print | head -1\n")
    say(f"  uxgate: rendering {test} on {REMOTE}'s real display…")
    with _screenlock:                       # one display → one capture at a time
        r = ssh(cmd, timeout=600)
    remote_png = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else ""
    if not remote_png.endswith(".png"):
        return "fail", None
    local = LOG / f"uxgate_{png}"
    sh(["scp", "-q", f"{REMOTE}:{remote_png}", str(local)], check=True)
    golden = GOLDEN / png
    if not golden.exists():
        sh(["cp", str(local), str(golden)]); return "golden-saved", local
    verdict, note = vision_review(str(local), str(golden))
    say(f"  uxgate vision → {note}")
    return verdict, local


# ---------- gate (always on build node) ----------
def gate(iid, verify, ref=None):
    """Build+test `ref` (default origin/fleet/<id>) on the build node. Returns (ok, tail)."""
    ref = ref or f"origin/fleet/{iid}"
    wt = f"{REMOTE_WT}/gate-{iid}"
    cmd = (remote_fresh_worktree(wt, ref) +
           f"export CARGO_TARGET_DIR={REMOTE_REPO}/target\ncd {wt}\n"
           f"({verify}) > /tmp/gate_{iid}.log 2>&1\necho GATE_EXIT_$?\ntail -25 /tmp/gate_{iid}.log\n")
    say(f"  [{iid}] gating ({verify}) on {REMOTE}…")
    with _gatelock:                  # one build node, one cargo cache — serialize
        r = ssh(cmd, timeout=2400)
    ok = "GATE_EXIT_0" in r.stdout
    (LOG / f"{iid}.gate.log").write_text(r.stdout)
    return ok, r.stdout.strip().splitlines()[-8:]


# ---------- supervisor (merge queue) ----------
def speculative_gate(iid, verify):
    """Merge origin/master + origin/fleet/<id> on the build node, gate the combination."""
    wt = f"{REMOTE_WT}/spec-{iid}"
    cmd = (remote_fresh_worktree(wt, "origin/master") +
           f"export CARGO_TARGET_DIR={REMOTE_REPO}/target\ncd {wt}\n"
           f"if ! git -c user.email=fleet@local -c user.name=fleet merge --no-edit origin/fleet/{iid} > /tmp/spec_{iid}.log 2>&1; then\n"
           f"  echo MERGE_CONFLICT; tail -15 /tmp/spec_{iid}.log; exit 0\nfi\n"
           f"({verify}) >> /tmp/spec_{iid}.log 2>&1\necho SPEC_EXIT_$?\ntail -20 /tmp/spec_{iid}.log\n")
    with _gatelock:                  # serialize on the build node
        r = ssh(cmd, timeout=2400)
    (LOG / f"{iid}.spec.log").write_text(r.stdout)
    if "MERGE_CONFLICT" in r.stdout:
        return "conflict", r.stdout.strip().splitlines()[-6:]
    return ("ok" if "SPEC_EXIT_0" in r.stdout else "fail"), r.stdout.strip().splitlines()[-6:]

def auto_merge(iid, title):
    repo = load()["config"]["repo"]
    make_pr(iid, title)   # idempotent — PR already opened during iterate()
    r = sh(["gh", "pr", "merge", f"fleet/{iid}", "--repo", repo, "--squash", "--delete-branch"],
           cwd=str(LOCAL_REPO))
    return r.returncode == 0, (r.stdout + r.stderr).strip()


# ---------- per-PR loop: fix-until-green + staff-architect review ----------
def make_pr(iid, title):
    """Open a PR for fleet/<id> if one isn't already open (idempotent)."""
    repo = load()["config"]["repo"]; base = load()["config"]["base"]
    ex = sh(["gh", "pr", "list", "--repo", repo, "--head", f"fleet/{iid}", "--json", "number",
             "-q", ".[0].number"], cwd=str(LOCAL_REPO))
    if ex.stdout.strip():
        return ex.stdout.strip()
    body = ("Opened by the twarp fleet. Auto-iterated until the functional gate + staff-architect "
            "review pass, then merged via the speculative merge-queue.\n\n"
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)")
    sh(["gh", "pr", "create", "--repo", repo, "--base", base, "--head", f"timomak:fleet/{iid}",
        "--title", title, "--body", body], cwd=str(LOCAL_REPO))
    return "created"

def fix_agent(it, errors):
    """Re-run the item's node agent ON ITS EXISTING branch with the failure as context."""
    iid = it["id"]
    fix_prompt = (
        f"You are fixing your own branch fleet/{iid} (task: {it['title']}). Read AGENTS.md. The fleet "
        f"gate FAILED — fix the ROOT CAUSE, stay in scope, do not revert prior good work.\n\n"
        f"Original task: {it['task']}\n\nFAILURE OUTPUT:\n{errors}\n\n"
        f"Output one final line: WORKER_DONE {iid}")
    ref = f"origin/fleet/{iid}"
    return worker_remote(it, ref=ref, prompt_text=fix_prompt) if it["node"] == REMOTE \
        else worker_local(it, ref=ref, prompt_text=fix_prompt)

def architect_review(iid):
    """Staff/principal-altitude pre-merge review of the branch diff. Returns (verdict, note)."""
    repo = load()["config"]["repo"]
    sh(["git", "-C", str(LOCAL_REPO), "fetch", "-q", "origin"])
    diff = sh(["git", "-C", str(LOCAL_REPO), "diff", f"origin/master...origin/fleet/{iid}"]).stdout
    if len(diff) > 60000:
        diff = diff[:60000] + "\n...[diff truncated]..."
    prompt = (
        "You are a staff/principal engineer doing a PRE-MERGE review of a twarp change at ARCHITECTURE "
        "altitude — correctness, blast radius, fork discipline, reversibility, long-term "
        "maintainability — NOT style nitpicks. Approve unless there's a real blocking concern.\n\n"
        f"Diff (origin/master...fleet/{iid}):\n\n{diff}\n\n"
        "Reply with EXACTLY one line:\n"
        "  ARCH approve — <one line: why it's safe to merge>\n"
        "  ARCH changes — <the specific blocking issue(s) to fix>")
    r = sh(["claude", "-p", prompt, "--dangerously-skip-permissions"], timeout=300)
    out = (r.stdout or "").strip()
    line = next((l for l in out.splitlines() if l.strip().upper().startswith("ARCH")),
                out.splitlines()[-1] if out else "")
    return ("approve" if "approve" in line.lower() else "changes"), line

def iterate(it):
    """Drive ONE PR to green + architect-approved: gate → fix → re-gate → architect → fix → … .
    Gates serialize on the build node; authoring/fixing/review run in parallel across items."""
    iid = it["id"]
    set_status(iid, "iterating")
    make_pr(iid, it["title"])
    for rnd in range(1, MAX_ROUNDS + 1):
        ok, tail = gate(iid, it["verify"])
        if not ok:
            say(f"  [{iid}] gate FAIL (round {rnd}) → fix-agent")
            fixed, _ = fix_agent(it, "\n".join(tail))
            if not fixed:
                say(f"  [{iid}] fix produced no change — giving up"); set_status(iid, "failed"); return False
            continue
        if it.get("ux"):
            uv, _ = uxgate(it.get("ux_test", UXTEST))
            if uv == "regression":
                say(f"  [{iid}] UX regression (round {rnd}) → fix-agent")
                fix_agent(it, "The UX visual gate found a regression vs the golden screenshot."); continue
        verdict, note = architect_review(iid)
        say(f"  [{iid}] architect (round {rnd}): {note}")
        if verdict != "approve":
            fix_agent(it, f"Staff-architect review requested changes: {note}"); continue
        set_status(iid, "ready"); return True
    say(f"  [{iid}] exhausted {MAX_ROUNDS} rounds without green+approved"); set_status(iid, "exhausted")
    return False


# ---------- orchestration ----------
def cmd_dispatch(args):
    q = load()
    picked = eligible(q)
    if not picked:
        say("dispatch: nothing eligible")
        return []
    say(f"dispatch: leasing {[i['id'] for i in picked]}")
    for it in picked:
        set_status(it["id"], "leased")
    return picked

def merge_one(it):
    """Speculative-merge gate + auto-merge a single ready PR. Serialized by the caller."""
    iid = it["id"]
    set_status(iid, "merging")
    verdict, tail = speculative_gate(iid, it["verify"])
    if verdict == "conflict":
        say(f"  [{iid}] speculative merge CONFLICT — needs rebase"); set_status(iid, "needs-rebase"); return False
    if verdict != "ok":
        say(f"  [{iid}] speculative gate FAILED (semantic conflict) — ejected"); set_status(iid, "failed"); return False
    ok, out = auto_merge(iid, it["title"])
    say(f"  [{iid}] auto-merge {'OK' if ok else 'FAILED'}: {out.splitlines()[-1] if out else ''}")
    set_status(iid, "merged" if ok else "ready"); return ok

def cmd_run(args):
    """Continuous batch loop: fill up to `batch` ready items → author in parallel → drive each PR to
    green + architect-approved (parallel authoring/fixing, serialized gates) → merge-queue → report →
    refill → repeat until the queue (and roadmap) are drained."""
    LOG.mkdir(parents=True, exist_ok=True)
    batch_size = load()["config"].get("batch", 5)
    batch_no = 0
    while True:
        # reflect just-merged changes locally (e.g. STATUS.md checkbox ticks) — best-effort
        sh(["git", "-C", str(LOCAL_REPO), "fetch", "-q", "origin"])
        sh(["git", "-C", str(LOCAL_REPO), "merge", "-q", "--ff-only", "origin/master"])
        say(f"roadmap: {roadmap_sync()}")            # top up from the roadmap each batch
        picked = eligible(load(), cap=batch_size)    # up to `batch` file-disjoint ready items
        if not picked:
            say("=== queue drained — nothing ready ==="); break
        batch_no += 1
        say(f"=== batch {batch_no}: authoring {len(picked)} in parallel → {[i['id'] for i in picked]} ===")
        for it in picked:
            set_status(it["id"], "leased")

        # 1) author in parallel (N sessions across Claude + Codex)
        with cf.ThreadPoolExecutor(max_workers=len(picked)) as ex:
            authored = list(ex.map(run_worker, picked))
        live = [item(load(), iid) for iid, ok in authored if ok]

        # 2) drive each PR to green + architect-approved (gates serialize via _gatelock)
        if live:
            with cf.ThreadPoolExecutor(max_workers=len(live)) as ex:
                flags = list(ex.map(iterate, live))
            ready = [it for it, ok in zip(live, flags) if ok]
        else:
            ready = []

        # 3) merge queue: serialized speculative-merge + auto-merge
        say(f"=== batch {batch_no} merge queue: {[i['id'] for i in ready]} ===")
        merged = [it["id"] for it in ready if merge_one(it)]
        say(f"=== batch {batch_no} done — merged {merged} ===")
        cmd_status(args)
    say("=== run complete ===")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("status", "dispatch", "run", "roadmap-sync"):
        sub.add_parser(name)
    w = sub.add_parser("worker"); w.add_argument("id")
    g = sub.add_parser("gate"); g.add_argument("id")
    s = sub.add_parser("supervise"); s.add_argument("id")
    u = sub.add_parser("uxgate"); u.add_argument("test", nargs="?", default=UXTEST)
    args = ap.parse_args()

    if args.cmd == "status":
        cmd_status(args)
    elif args.cmd == "roadmap-sync":
        print("roadmap:", roadmap_sync())
    elif args.cmd == "dispatch":
        cmd_dispatch(args)
    elif args.cmd == "run":
        cmd_run(args)
    elif args.cmd == "worker":
        run_worker(item(load(), args.id))
    elif args.cmd == "gate":
        it = item(load(), args.id)
        ok, tail = gate(args.id, it["verify"])
        print("GATE", "PASS" if ok else "FAIL"); [print(" |", l) for l in tail]
    elif args.cmd == "supervise":
        it = item(load(), args.id)
        print(speculative_gate(args.id, it["verify"]))
    elif args.cmd == "uxgate":
        verdict, png = uxgate(args.test)
        print(f"UXGATE {verdict} png={png}")

if __name__ == "__main__":
    main()
