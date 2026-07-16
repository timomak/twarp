#!/usr/bin/env python3
"""twarp dev fleet orchestrator.

Roles (all driven from this file):
  dispatch   pick eligible, file-disjoint, dependency-unblocked items and lease them
  worker     author one item on its pod (codex / claude) -> push branch
  gate       build + targeted tests for a branch on the pod that authored it
  supervise  bors-style merge queue: speculative-merge each green branch, re-gate, auto-merge
  run        full loop: dispatch -> workers (parallel) -> gate -> supervise

Node-pod model:
  * A POD is one machine = a pool of builder sessions + its own gate. Pods build/test LOCALLY
    (warm cache, no cross-machine branch shipping). Gates run serial WITHIN a pod, parallel ACROSS
    pods (one lock per pod).
  * The orchestrator runs ON one of the pods — the "self" pod (run commands locally) — and reaches
    any other pod over SSH. `--self NAME` (or $FLEET_SELF) says which pod that is, so the same script
    runs correctly on either machine. Paths derive from this file's location, so "self" is wherever
    the checkout lives.
  * Default: one pod (other-mac, codex) and the loop runs there → this Mac is idle.
    `--both`: loop runs on this Mac with two pods (local=claude here + other-mac=codex over SSH) →
    4 builders / 2 gates.
  * Auto-merge only happens after the functional gate passes on the *merge with current master*
    (catches green-alone-but-break-together semantic conflicts). gh runs on the self pod.
"""
import argparse, concurrent.futures as cf, json, os, re, subprocess, sys, threading, time
from pathlib import Path

_qlock = threading.Lock()        # serialize all access to queue.json across worker threads
_screenlock = threading.Lock()   # a display pod has ONE display — serialize real-display captures
_gatelocks = {}                  # one lock per pod — each machine has one cargo cache
_gatelocks_guard = threading.Lock()
MAX_ROUNDS = 4   # per-PR fix-until-green+approved attempts before giving up

# Paths derive from THIS file so the script runs correctly on whichever machine is "self".
SELF_REPO = Path(__file__).resolve().parents[1]
QUEUE = SELF_REPO / "fleet" / "queue.json"
SELF_WT = SELF_REPO.parent / (SELF_REPO.name + "-fleet-wt")
LOG = SELF_REPO / "fleet" / "runs"
ROADMAP = SELF_REPO / "roadmap" / "ROADMAP.md"
GOLDEN = SELF_REPO / "fleet" / "golden"

SELF = os.environ.get("FLEET_SELF", "local")   # which pod this process runs on; set by --self
ACTIVE_PODS = []                               # filled by cmd_run / _resolve_pods
INFLIGHT = {"leased", "building", "authored", "iterating", "gating", "gated", "ready", "merging"}


# ---------- io helpers ----------
def now():
    return time.strftime("%H:%M:%S")

def say(msg):
    print(f"[{now()}] {msg}", flush=True)

DEFAULT_VERIFY = "cargo build --bin twarp-oss"   # matches the roadmap-bridge default (bin renamed in 9c)

def _normalize(it):
    """Manual queue items follow the documented schema (id/node/touches/depends_on/task/
    verify/ux) and may omit 'title' and 'verify' — only roadmap-bridged items set those.
    Apply defaults on read so no worker/gate path KeyErrors on a hand-added item."""
    it.setdefault("verify", DEFAULT_VERIFY)
    if not it.get("title"):
        it["title"] = it["task"].strip().splitlines()[0][:60] if it.get("task") else it["id"]
    return it

def load():
    q = json.loads(QUEUE.read_text())
    for it in q.get("items", []):
        _normalize(it)
    return q

def save(q):
    # Atomic replace so concurrent readers (the many lock-free node_*/load() calls in worker threads)
    # never observe a half-written or empty file. Writers are serialized by _qlock; readers always
    # see a complete snapshot (old or new), never a torn one.
    tmp = QUEUE.with_name(f"{QUEUE.name}.tmp.{os.getpid()}.{threading.get_ident()}")
    tmp.write_text(json.dumps(q, indent=2) + "\n")
    os.replace(str(tmp), str(QUEUE))

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
    log_event(iid, "status", status=status,
              **{k: v for k, v in extra.items() if isinstance(v, (str, int, float, bool))})
    if status in ("merged", "failed", "exhausted", "needs-rebase"):
        notify(f"{iid} → {status}", "info" if status == "merged" else "warn")

# ---------- traceability: structured event ledger + off-box notifications ----------
EVENTS = LOG / "events.jsonl"
_evlock = threading.Lock()

def log_event(iid, stage, **fields):
    """Append a timestamped structured record to fleet/runs/events.jsonl — the machine-readable audit
    trail that queue.json's in-place status overwrite can't provide (durations, gate rounds, verdicts,
    evidence paths). Best-effort: never fails the run."""
    rec = {"ts": round(time.time(), 3), "iid": iid, "stage": stage, **fields}
    try:
        LOG.mkdir(parents=True, exist_ok=True)
        with _evlock, open(EVENTS, "a") as f:
            f.write(json.dumps(rec) + "\n")
    except Exception:
        pass
    return rec

def notify(msg, level="info"):
    """Push a run event off-box so the loop is observable when you're remote. POSTs to a Slack-style
    incoming webhook if FLEET_WEBHOOK (env) or config.webhook is set; always mirrors to run.log. No-op
    (never fails the run) if unconfigured."""
    say(f"[notify:{level}] {msg}")
    try:
        url = os.environ.get("FLEET_WEBHOOK") or cfg().get("webhook")
    except Exception:
        url = None
    if not url:
        return
    try:
        import urllib.request
        data = json.dumps({"text": f":robot_face: twarp-fleet [{level}] {msg}"}).encode()
        urllib.request.urlopen(
            urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"}),
            timeout=10)
    except Exception as e:
        say(f"  notify webhook failed: {e}")

def _stamp_ux(iid, level, verdict, evid):
    """Record HOW an item was UX-verified (live-drive / bootstrap-screenshot / none) + the verdict and
    evidence path, on the item itself. This is the provenance that was missing when all of 14a-e merged
    `ux:False` with nothing recording they were never actually driven."""
    try:
        with _qlock:
            q = load(); it = item(q, iid)
            it["ux_level"] = level; it["ux_verdict"] = verdict
            it["ux_evidence"] = str(evid) if evid else None
            save(q)
    except Exception:
        pass
    log_event(iid, "ux", level=level, verdict=verdict, evidence=str(evid) if evid else None)
    return verdict, evid

def sh(cmd, cwd=None, timeout=None, check=False):
    r = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str),
                       capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"cmd failed ({r.returncode}): {cmd}\n{r.stdout}\n{r.stderr}")
    return r


# ---------- node/pod layer ----------
def cfg():
    return load()["config"]

def nodes_cfg():
    """Pod definitions. Falls back to a legacy two-pod layout if `config.nodes` is absent so old
    queue.json files keep working (self=local claude here, other-mac codex over SSH)."""
    n = cfg().get("nodes")
    if n:
        return n
    return {
        "local": {"host": "self", "kind": "claude", "builders": 2,
                  "repo": str(SELF_REPO), "wt": str(SELF_WT), "env": ""},
        "other-mac": {"host": "other-mac", "kind": "codex", "builders": 2, "display": True,
                      "repo": "$HOME/Development/twarp", "wt": "$HOME/Development/twarp-fleet-wt",
                      "env": "source ~/.config/twarp-fleet/env; set -a; source ~/.codex/.env-foundry; set +a"},
    }

def node_repo(name):
    return str(SELF_REPO) if name == SELF else nodes_cfg()[name]["repo"]

def node_wt(name):
    return str(SELF_WT) if name == SELF else nodes_cfg()[name]["wt"]

def node_env(name):
    return nodes_cfg().get(name, {}).get("env", "")

def node_kind(name):
    return nodes_cfg().get(name, {}).get("kind", "codex")

def node_host(name):
    return nodes_cfg()[name].get("host", name)

def node_builders(name):
    return nodes_cfg().get(name, {}).get("builders", 2)

def gatelock(name):
    with _gatelocks_guard:
        return _gatelocks.setdefault(name, threading.Lock())

def codex_node(pods=None):
    for p in (pods or ACTIVE_PODS):
        if node_kind(p) == "codex":
            return p
    return None

def claude_node(pods=None):
    for p in (pods or ACTIVE_PODS):
        if node_kind(p) == "claude":
            return p
    return None

def display_node(pods=None):
    pods = pods or ACTIVE_PODS
    for p in pods:
        if nodes_cfg().get(p, {}).get("display"):
            return p
    return pods[0] if pods else None

def bash_on(name, script, timeout=None, check=False):
    """Run a bash script on pod `name`: locally if it's the self pod, else over SSH. The pod's env
    prefix (PATH for ~/.local/bin tools, Foundry key, etc.) is sourced first. Fed via stdin so the
    remote login shell never re-parses it (the `ssh host bash -lc "a; b"` trap runs everything after
    the first `;` under the remote default shell)."""
    env = node_env(name)
    full = (env + "\n" + script) if env else script
    if name == SELF:
        argv = ["bash", "-s"]
    else:
        argv = ["ssh", "-o", "ConnectTimeout=10", node_host(name), "bash", "-s"]
    r = subprocess.run(argv, input=full, capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"bash_on({name}) failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r

def put_file(name, local_path, dest):
    """Copy a local file to `dest` on pod `name` (cp if self, scp otherwise)."""
    if name == SELF:
        sh(["cp", str(local_path), dest], check=True)
    else:
        sh(["scp", "-q", str(local_path), f"{node_host(name)}:{dest}"], check=True)

def node_read(name, path):
    """Read a file from pod `name` (local read if self, ssh cat otherwise). '' if missing."""
    if name == SELF:
        try:
            return Path(path).read_text()
        except FileNotFoundError:
            return ""
    return sh(["ssh", "-o", "ConnectTimeout=10", node_host(name), "cat", path]).stdout

def get_file(name, remote_path, local_path):
    """Pull a file from pod `name` to a local path (cp if self, scp otherwise)."""
    if name == SELF:
        sh(["cp", remote_path, str(local_path)], check=True)
    else:
        sh(["scp", "-q", f"{node_host(name)}:{remote_path}", str(local_path)], check=True)

def fresh_worktree(name, wt, ref, branch=None):
    """Script that force-creates a clean worktree at `wt` on pod `name` (robust against orphan dirs
    left by killed runs). Returned for bash_on (which prepends the pod env)."""
    repo = node_repo(name)
    bflag = f"-B {branch}" if branch else "--detach"
    delbranch = f"git branch -D {branch} 2>/dev/null || true\n" if branch else ""
    return (f"cd {repo}\ngit fetch -q origin\n"
            f"git worktree remove --force {wt} 2>/dev/null || true\n"
            f"rm -rf {wt}\ngit worktree prune\n{delbranch}"
            f"git worktree add -f {bflag} {wt} {ref}\n")

def reap_worktrees(iid, name):
    """Remove an item's worktrees (author + gate- + spec- helpers) on its pod, freeing the multi-GB
    `target/` each carries. Boot disks are small/shared; leftover merged/failed worktrees pile up and
    eventually crash workers with `No space left on device`. Never touches the master worktree."""
    wt = node_wt(name); repo = node_repo(name)
    names = [iid, f"gate-{iid}", f"spec-{iid}"]
    rm = "".join(f"git worktree remove --force {wt}/{n} 2>/dev/null || true\nrm -rf {wt}/{n}\n"
                 for n in names)
    try:
        bash_on(name, f"cd {repo}\n{rm}git worktree prune\n", timeout=120)
    except Exception as e:
        say(f"  [{iid}] reap warning on {name}: {e}")


# ---------- roadmap bridge ----------
def _active_feature():
    if not ROADMAP.exists():
        return None
    m = re.search(r"\*\*Currently active:\*\*\s*`([^`]+)`", ROADMAP.read_text())
    return m.group(1) if m else None

def _roadmap_features():
    """Parse the ROADMAP.md feature table → ordered list of (slug, phase) rows, e.g.
    ('15-computer-control', 'not-started'). Table order is authoritative for auto-advance."""
    rows = []
    if not ROADMAP.exists():
        return rows
    for line in ROADMAP.read_text().splitlines():
        m = re.match(r"\|\s*\d+\s*\|\s*\[[^\]]+\]\(([^/)]+)/STATUS\.md\)\s*\|\s*`?([a-z-]+)`?\s*\|", line)
        if m:
            rows.append((m.group(1), m.group(2)))
    return rows

def _enqueue(q, it):
    """Append an item iff one with that id isn't already present (any status). Returns True if added."""
    if any(x["id"] == it["id"] for x in q["items"]):
        return False
    q["items"].append(it)
    return True

def roadmap_sync():
    """Drive the active roadmap feature one step at a time by enqueuing the right fleet item for its
    current phase. FULLY AUTONOMOUS — no human gate: specs are authored + opposite-model-reviewed +
    auto-merged just like impl, and a finished feature auto-advances `Currently active:` to the next
    `not-started` feature in ROADMAP table order. The owner steers only by editing ROADMAP.md (the
    table order / the `Currently active:` pointer); the fleet never reorders it. Each enqueued item's
    own committed edits carry the phase/pointer transitions, so they persist via the merge queue.
    Pulls ONE item at a time (steps are sequential and share files). Returns a one-line status."""
    feat = _active_feature()
    if not feat:
        return "no active feature in ROADMAP.md"
    status_md = SELF_REPO / "roadmap" / feat / "STATUS.md"
    if not status_md.exists():
        return f"{feat}: no STATUS.md"
    text = status_md.read_text()
    # Tolerate authors bolding or backticking the phase value: `**Phase:** **impl-pending**` etc.
    pm = re.search(r"\*\*Phase:\*\*\s*(?:\*\*)?`?([a-z-]+)`?(?:\*\*)?", text)
    phase = pm.group(1) if pm else "unknown"
    q = load()

    # ---- SPEC step: author PRODUCT.md + TECH.md, then flip phase to impl-pending ----
    if phase in ("not-started", "spec-pending", "spec-in-review"):
        iid = f"{feat}-spec"
        task = (
            f"Author the product + technical specs for roadmap feature {feat}. Read "
            f"roadmap/{feat}/STATUS.md (and roadmap/{feat}/PLAN.md if it exists) for scope. If the "
            f"twarp `write-product-spec` / `write-tech-spec` skills are available to you, use them.\n"
            f"1) Write roadmap/{feat}/PRODUCT.md: detailed user-facing behavior. It MUST end with a "
            f"`## Smoke test` section — a numbered list of concrete steps to validate against a built "
            f"twarp binary (per-sub-phase sub-headings if the feature is sub-phased).\n"
            f"2) Write roadmap/{feat}/TECH.md: an implementation plan grounded in the CURRENT codebase "
            f"(files/crates to touch, constraints, a sub-phase breakdown that matches the sub-phases "
            f"listed in STATUS.md).\n"
            f"3) In roadmap/{feat}/STATUS.md change the `**Phase:**` line to `**Phase:** impl-pending`.\n"
            f"Write ONLY spec/markdown — no product code.")
        verify = (f"test -f roadmap/{feat}/PRODUCT.md && test -f roadmap/{feat}/TECH.md && "
                  f"grep -q '## Smoke test' roadmap/{feat}/PRODUCT.md")
        added = _enqueue(q, {"id": iid, "title": f"{feat}: author specs", "node": None,
                             "status": "queued", "depends_on": [], "touches": [f"roadmap/{feat}/**"],
                             "barrier": False, "task": task, "verify": verify, "ux": False})
        if not added:
            return f"{iid}: already in queue"
        save(q)
        return f"queued {iid} — author specs for {feat}"

    # ---- IMPL step: next unchecked sub-phase (specs are merged) ----
    if phase in ("impl-pending", "impl-in-review"):
        # Two checkbox formats appear in STATUS.md files:
        #   A: `- [ ] **16a — Title.** description`   (bold spans id + title)
        #   B: `- [ ] **16a** — Title text.`          (bold id only, no separate description)
        m = re.search(r"^- \[ \] \*\*([0-9]+[a-z]) — ([^*]+?)\.?\*\*\s*(.*)$", text, re.M)
        if m:
            sub_id, sub_title, sub_desc = m.group(1), m.group(2).strip(), m.group(3).strip()
        else:
            m = re.search(r"^- \[ \] \*\*([0-9]+[a-z])\*\*\s*—\s*(.+?)\s*$", text, re.M)
            if m:
                sub_id, sub_title, sub_desc = m.group(1), m.group(2).strip().rstrip("."), ""
        if m:
            if any(it["id"] == sub_id for it in q["items"]):
                return f"{sub_id}: already in queue"
            # Inherit touches/verify from the feature's base item or a sibling impl sub-phase —
            # never from -spec/-advance items (their grep verifies would wedge an impl item).
            num = feat.split("-")[0]
            base = next((it for it in q["items"] if it["id"] == feat), None) \
                or next((it for it in q["items"] if re.fullmatch(rf"{num}[a-z]", it["id"])), None)
            touches = (base["touches"][:] if base else ["app/**"]) + [f"roadmap/{feat}/STATUS.md"]
            verify = base["verify"] if base else "cargo build --bin twarp-oss"
            task = (f"Implement sub-phase {sub_id} of roadmap feature {feat}. FIRST read the merged "
                    f"specs roadmap/{feat}/PRODUCT.md and roadmap/{feat}/TECH.md. Sub-phase {sub_id} — "
                    f"{sub_title}: {sub_desc}\nImplement ONLY this sub-phase, scoped to its files. When "
                    f"done, tick this sub-phase's checkbox in roadmap/{feat}/STATUS.md from `- [ ]` to "
                    f"`- [x]`.")
            # Impl sub-phases get the dynamic UX gate: criteria come from the feature PRODUCT.md's
            # `## Smoke test` section (the gate degrades to the screenshot fallback, never blocks).
            _enqueue(q, {"id": sub_id, "title": f"{feat} {sub_id}: {sub_title}", "node": None,
                         "status": "queued", "depends_on": [], "touches": touches, "barrier": False,
                         "task": task, "verify": verify, "ux": True})
            save(q)
            return f"queued {sub_id} — {sub_title}"
        # no unchecked sub-phase left → impl is done → fall through to advance
        phase = "merged"

    # ---- ADVANCE step: this feature is done → point `Currently active:` at the next not-started ----
    if phase == "merged":
        nxt = next((slug for slug, ph in _roadmap_features()
                    if ph == "not-started" and slug != feat), None)
        if not nxt:
            return f"{feat}: merged — no `not-started` feature left; roadmap drained"
        iid = f"{feat}-advance"
        task = (
            f"Roadmap bookkeeping ONLY — no product code. Feature {feat} is fully merged.\n"
            f"1) In roadmap/ROADMAP.md set feature {feat}'s table-row Phase cell to `merged`, and "
            f"change the `**Currently active:**` line to `{nxt}` (keep the format `` `{nxt}` ``).\n"
            f"2) In roadmap/{feat}/STATUS.md set the `**Phase:**` line to `merged`.\n"
            f"3) In roadmap/{nxt}/STATUS.md set the `**Phase:**` line to `spec-pending`.\n"
            f"Make ONLY these edits. Do NOT reorder the ROADMAP table.")
        verify = f"grep -q 'Currently active:.*{nxt}' roadmap/ROADMAP.md"
        added = _enqueue(q, {"id": iid, "title": f"advance active feature {feat} → {nxt}", "node": None,
                             "status": "queued", "depends_on": [], "touches": ["roadmap/**"],
                             "barrier": False, "task": task, "verify": verify, "ux": False})
        if not added:
            return f"{iid}: already in queue"
        save(q)
        return f"queued {iid} — advance active feature {feat} → {nxt}"

    return f"{feat}: phase={phase} — no fleet action"


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

def assign_pods(picked):
    """Assign each picked item to an active pod (least-loaded under per-pod builder capacity).
    UX items pin to a pod with a real display. Sets it['node'] in the queue."""
    load_ct = {p: 0 for p in ACTIVE_PODS}
    for it in picked:
        cands = [p for p in ACTIVE_PODS if (not it.get("ux")) or nodes_cfg().get(p, {}).get("display")]
        cands = cands or ACTIVE_PODS
        pod = min(cands, key=lambda p: (load_ct[p] / max(1, node_builders(p))))
        load_ct[pod] += 1
        it["node"] = pod
        set_status(it["id"], "leased", node=pod)
    return picked

def cmd_status(_):
    q = load()
    pods = _resolve_pods_for_status(q)
    print(f"\ntwarp fleet — self={SELF} pods={pods} "
          f"builders={sum(node_builders(p) for p in pods)} gates={len(pods)}\n")
    for it in q["items"]:
        dep = f" deps={it['depends_on']}" if it["depends_on"] else ""
        bar = " [BARRIER]" if it.get("barrier") else ""
        nd = f" @{it.get('node')}" if it.get("node") else ""
        br = f" {it.get('branch','')}" if it.get("branch") else ""
        print(f"  {it['status']:>9}  {it['id']:<26}{bar}{dep}{nd}{br}")
    cap = sum(node_builders(p) for p in pods)
    elig = eligible(q, cap=cap)
    print(f"\n  eligible now: {[i['id'] for i in elig] or '—'}\n")

def _resolve_pods_for_status(q):
    # status is read-only and pod-agnostic; show whatever's configured as the default set.
    return q["config"].get("pods_default", [SELF] if SELF in nodes_cfg() else list(nodes_cfg())[:1])


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

def worker_claude(it, name, ref="origin/master", prompt_text=None):
    """Author on a claude pod. claude runs where its auth lives → only on the self pod."""
    iid = it["id"]; repo = node_repo(name); wt = Path(node_wt(name)) / iid
    if name != SELF:
        return False, f"claude pod {name} is not self — claude can only author locally"
    sh(["git", "-C", repo, "fetch", "-q", "origin"], check=True)
    sh(["git", "-C", repo, "worktree", "remove", "--force", str(wt)])
    sh(f"rm -rf {wt}")
    sh(["git", "-C", repo, "worktree", "prune"])
    sh(["git", "-C", repo, "branch", "-D", f"fleet/{iid}"])
    sh(["git", "-C", repo, "worktree", "add", "-B", f"fleet/{iid}", str(wt), ref], check=True)
    prompt = write_prompt(it) if prompt_text is None else write_text_prompt(iid, prompt_text)
    say(f"  [{iid}] claude worker authoring on {name}…")
    r = sh(["claude", "-p", prompt.read_text(), "--dangerously-skip-permissions"],
           cwd=str(wt), timeout=1200)
    (LOG / f"{iid}.author.log").write_text(r.stdout + "\n---STDERR---\n" + r.stderr)
    sh(["git", "-C", str(wt), "add", "-A"], check=True)
    diff = sh(["git", "-C", str(wt), "diff", "--cached", "--stat"]).stdout.strip()
    if not diff:
        return False, "no changes produced"
    sh(["git", "-C", str(wt), "commit", "-m", f"fleet({iid}): {it['title']}"], check=True)
    sh(["git", "-C", str(wt), "push", "-q", "-f", "-u", "origin", f"fleet/{iid}"], check=True)
    return True, diff

def worker_codex(it, name, ref="origin/master", prompt_text=None):
    """Author on a codex pod (local if self, else over SSH)."""
    iid = it["id"]; wt = f"{node_wt(name)}/{iid}"
    prompt = write_prompt(it) if prompt_text is None else write_text_prompt(iid, prompt_text)
    put_file(name, prompt, f"/tmp/fleet_{iid}.prompt")
    bash_on(name, fresh_worktree(name, wt, ref=ref, branch=f"fleet/{iid}"), timeout=120, check=True)
    say(f"  [{iid}] codex worker authoring on {name}…")
    run = (f"cd {wt}\n"
           f"cat /tmp/fleet_{iid}.prompt | $HOME/.local/bin/codex exec "
           f"--dangerously-bypass-approvals-and-sandbox -c model_reasoning_effort='high' "
           f"> /tmp/fleet_{iid}.author.log 2>&1\n"
           f"echo CODEX_EXIT_$?\n")
    auth_start = time.time()
    r = bash_on(name, run, timeout=3600)
    (LOG / f"{iid}.author.log").write_text(node_read(name, f"/tmp/fleet_{iid}.author.log"))
    _collect_codex_session(iid, name, auth_start)   # structured author trace (parity with driver.jsonl)
    if "CODEX_EXIT_0" not in r.stdout:
        return False, f"codex did not exit clean: {r.stdout.strip()[-200:]}"
    commit = (f"set -e\ncd {wt}\ngit add -A\n"
              f"if git diff --cached --quiet; then echo NO_CHANGES; exit 0; fi\n"
              f"git commit -q -m 'fleet({iid}): {it['title']}'\n"
              f"git push -q -f -u origin fleet/{iid}\n"
              f"git diff --stat origin/master..origin/fleet/{iid} | tail -4\n"
              f"echo PUSHED_$(git rev-parse --short HEAD)\n")
    c = bash_on(name, commit, timeout=120)
    if "NO_CHANGES" in c.stdout:
        return False, "no changes produced"
    ok = "PUSHED_" in c.stdout and bool(
        bash_on(name, f"cd {node_repo(name)} && git ls-remote origin fleet/{iid}").stdout.strip())
    return ok, c.stdout.strip()

def author(it, name, ref="origin/master", prompt_text=None):
    return (worker_codex if node_kind(name) == "codex" else worker_claude)(it, name, ref, prompt_text)

def run_worker(it):
    name = it.get("node") or codex_node() or ACTIVE_PODS[0]
    # Manual queue items (documented schema: id/node/touches/depends_on/task/verify) omit
    # 'title'; only roadmap-bridged items set it. Derive one so the worker never KeyErrors.
    it.setdefault("title", (it.get("task", "").strip().splitlines()[0][:60] if it.get("task") else it["id"]))
    try:
        set_status(it["id"], "building", node=name, branch=f"fleet/{it['id']}")
        ok, info = author(it, name)
        set_status(it["id"], "authored" if ok else "failed")
        say(f"  [{it['id']}] author on {name} {'OK' if ok else 'FAILED'}: {info.splitlines()[-1] if info else ''}")
        return it["id"], ok
    except Exception as e:
        set_status(it["id"], "failed")
        say(f"  [{it['id']}] worker EXC: {e}")
        return it["id"], False


# ---------- UX gate (real-display screenshot + golden baseline) ----------
UXTEST = "test_video_recording"   # bootstraps the UI and captures after_bootstrap.png

def vision_review(new_png, golden_png):
    """A headless vision agent compares the new screenshot against the approved golden and judges
    whether there is a USER-VISIBLE regression. Returns (verdict, note) in {'pass','regression','error'}."""
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
    # Parse the token right after VERDICT — a prose tail like "no regression observed"
    # inside a pass line must not flip the verdict (same class of bug as _parse_arch).
    m = re.search(r"\bVERDICT\b\W{0,4}(pass|regression|fail|error)", line, re.I)
    if m:
        tok = m.group(1).lower()
        return ("regression" if tok == "fail" else tok), line
    low = line.lower()
    if "regression" in low:
        return "regression", line
    if "pass" in low:
        return "pass", line
    return "error", line or "no verdict from vision agent"


def uxgate(test=UXTEST, png="after_bootstrap.png"):
    """Render twarp on a display pod's REAL display, capture a screenshot, pull it to the self pod,
    and vision-compare to the golden baseline. Returns (verdict, local_png_path)."""
    GOLDEN.mkdir(parents=True, exist_ok=True); LOG.mkdir(parents=True, exist_ok=True)
    name = display_node()
    repo = node_repo(name); art = "/tmp/uxgate"
    cmd = (f"export CARGO_TARGET_DIR={repo}/target\n"
           f"export WARP_INTEGRATION_TEST_ARTIFACTS_DIR={art}\n"
           f"export WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1\n"
           f"caffeinate -dimsu & CAF=$!\n"
           f"rm -rf {art}; mkdir -p {art}\ncd {repo}\n"
           f"./target/debug/integration {test} > /tmp/uxgate.log 2>&1 || true\n"
           f"kill $CAF 2>/dev/null\n"
           f"find {art} -name '{png}' -print | head -1\n")
    say(f"  uxgate: rendering {test} on {name}'s real display…")
    with _screenlock:                       # one display → one capture at a time
        r = bash_on(name, cmd, timeout=600)
    remote_png = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else ""
    if not remote_png.endswith(".png"):
        return "fail", None
    local = LOG / f"uxgate_{png}"
    get_file(name, remote_png, local)
    golden = GOLDEN / png
    if not golden.exists():
        sh(["cp", str(local), str(golden)]); return "golden-saved", local
    verdict, note = vision_review(str(local), str(golden))
    say(f"  uxgate vision → {note}")
    return verdict, local


# ---------- dynamic UX gate (computer-use: drive the real app, judge vs criteria) ----------
# Unlike uxgate() (one bootstrap screenshot vs a golden), this LAUNCHES the PR's build and lets a
# Claude agent drive the live app — navigate to the feature, click/type, and judge it against the
# feature's acceptance criteria. Needs the display pod's in-session injector (UidriveAgent) trusted;
# falls back to uxgate() if injection isn't available, so a missing grant degrades, never blocks.
def ux_inject_ready(name):
    """True if the display pod's IN-SESSION injector holds the Accessibility grant. A fresh
    `uidrive trusted` launched over SSH is sshd-attributed and ALWAYS false — the grant lives in the
    UidriveAgent `serve` process running inside the GUI login session. So kick the agent to re-log its
    state, then read the serve log's latest startup line."""
    try:
        bash_on(name, "launchctl kickstart -k gui/$(id -u)/com.twarp.uidrive >/dev/null 2>&1 || true\n"
                      "sleep 1\n", timeout=30)
        r = bash_on(name, "grep 'serve started' ~/.uidrive/log 2>/dev/null | tail -1\n", timeout=20)
        return "trusted=true" in (r.stdout or "")
    except Exception:
        return False

def ux_capture_ready(name):
    """True if the display pod's IN-SESSION capturer holds the Screen Recording grant. Same SSH-TCC
    rule as injection: a `screencapture` over SSH is sshd-attributed and silently returns a
    privacy-limited (desktop-only, no app windows) frame. The grant lives in the UicaptureAgent
    `serve` process. Ask it to re-check via its `prompt` command and read the log."""
    try:
        bash_on(name, "printf 'prompt\\n' > ~/.uicapture/in 2>/dev/null || true\nsleep 1\n", timeout=30)
        r = bash_on(name, "grep 'granted=' ~/.uicapture/log 2>/dev/null | tail -1\n", timeout=20)
        return "granted=true" in (r.stdout or "")
    except Exception:
        return False

def _ux_criteria(it):
    """What the UX agent judges against: explicit per-item `ux_criteria` wins, else the feature's
    PRODUCT.md `## Smoke test` section, else a generic 'exercise what the diff changed'."""
    if it.get("ux_criteria"):
        return it["ux_criteria"]
    feat = it.get("feature") or _active_feature()
    if feat:
        pm = SELF_REPO / "roadmap" / feat / "PRODUCT.md"
        if pm.exists():
            m = re.search(r"##\s*Smoke test\s*\n(.+?)(?:\n##\s|\Z)", pm.read_text(), re.S)
            if m:
                sec = m.group(1)
                # A sub-phase item (id like `16a`) with its own `### 16a — …` sub-heading is judged
                # on that subsection only — not on sibling sub-phases that aren't built yet.
                sm = re.search(rf"###[^\n]*\b{re.escape(it['id'])}\b[^\n]*\n(.+?)(?:\n###\s|\Z)",
                               sec, re.S)
                return (sm.group(1) if sm else sec).strip()[:4000]
    return ("Exercise the user-facing behavior introduced by this PR's diff and confirm it works "
            "without visual glitches.")

UX_DRIVE_PROMPT = """You are a UX QA agent debugging the twarp macOS app live, like a human tester. \
A freshly-built twarp from this PR is running on screen. Drive it with two shell helpers (Bash tool):

  {shot} <out.png>     -> captures the screen to <out.png> and prints its scale (pixels-per-point).
                          Then use your Read tool on <out.png> to SEE the screen.
  {act} "<command>"    -> injects input. Commands:
                            click X Y | dblclick X Y | move X Y      (X Y in POINTS)
                            type <text> | key <code> [cmd,shift,opt,ctrl]

CRITICAL coordinate rule: screenshots are RETINA. Coordinates you read off the PNG are PIXELS; \
DIVIDE by the printed scale (usually 2) to get POINTS before passing to {act}. E.g. a button at \
pixel (800,460) on a scale-2 shot -> `{act} "click 400 230"`.

KNOWN INJECTOR QUIRKS (work around these, don't fight them):
- `key <name>` only accepts NUMERIC macOS keycodes — a word like `return` silently becomes keycode \
0 (the 'a' key). Use numbers: Return=36, Escape=53, Tab=48, Delete=51, Space=49, arrows L/R/D/U=\
123/124/125/126. E.g. submit a URL with `{act} "key 36"`, not `key return`.
- `type` can DROP the first keystrokes if the field just gained focus. After typing, screenshot to \
confirm the text landed; if it didn't, click the field again and re-issue `type`. Prefer one \
`type <word>` then verify over many rapid calls.

Your job: verify the feature below actually works in the running twarp. Capture first to see the \
state, navigate to the feature, exercise it, and inspect the result after each action. If twarp \
isn't frontmost, click its window first. Save your final evidence screenshot to {final}.

=== FEATURE / ACCEPTANCE CRITERIA ===
{criteria}

=== WHAT THIS PR CHANGED (diff excerpt) ===
{diff}

Work in at most ~12 actions. When done, reply with EXACTLY one final line:
  VERDICT pass — <what you confirmed works>
  VERDICT regression — <what is broken or missing>
"""

def _collect_codex_session(iid, name, started_at):
    """Collect the codex AUTHOR's structured session (tool calls + reasoning) into the evidence dir —
    parity with the UX driver's driver.jsonl. Codex writes ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl
    (timestamp-named, so lexical order = chronological). Best-effort; never fails the run."""
    try:
        r = bash_on(name, "ls -1 ~/.codex/sessions/*/*/*/*.jsonl 2>/dev/null | tail -60", timeout=30)
        files = [l for l in (r.stdout or "").splitlines() if l.strip()]
        if not files:
            return
        newest = sorted(files)[-1]
        mt = bash_on(name, f"stat -f %m '{newest}' 2>/dev/null || stat -c %Y '{newest}' 2>/dev/null",
                     timeout=20).stdout.strip()
        if mt and float(mt) < started_at - 5:   # nothing authored this run
            return
        content = node_read(name, newest)
        if content:
            (LOG / f"{iid}.author.session.jsonl").write_text(content)
            log_event(iid, "author_session", file=newest, bytes=len(content))
    except Exception as e:
        say(f"  [{iid}] codex session collection warning: {e}")

def _collect_driver_trace(evid, started_at):
    """Capture the UX driver's FULL conversation for later review. Claude Code persists every session
    to ~/.claude/projects/<cwd-slug>/<session-id>.jsonl, so the driver's step-by-step reasoning, every
    ux_shot/ux_act call, and the verdict are already on disk — the gate just wasn't collecting them.
    Copy the raw jsonl into the evidence dir and render a readable trace (driver.md) so `what worked /
    what didn't` is reviewable without re-running. Best-effort: never let this fail the gate."""
    try:
        slug = str(SELF_REPO).replace("/", "-")
        proj = Path.home() / ".claude" / "projects" / slug
        cands = [p for p in proj.glob("*.jsonl") if p.stat().st_mtime >= started_at - 2]
        if not cands:
            return
        src = max(cands, key=lambda p: p.stat().st_mtime)
        rows = [json.loads(l) for l in src.read_text().splitlines() if l.strip()]
        (evid / "driver.jsonl").write_text(src.read_text())
        lines = []
        for r in rows:
            for b in (r.get("message", {}).get("content") or []):
                if not isinstance(b, dict):
                    continue
                if b.get("type") == "text" and b.get("text", "").strip():
                    lines.append(f"\n**reason:** {b['text'].strip()}")
                elif b.get("type") == "tool_use":
                    cmd = (b.get("input", {}) or {}).get("command", "")
                    act = cmd.split("ux_act ", 1)[-1] if "ux_act" in cmd else (
                        "screenshot" if "ux_shot" in cmd else cmd)
                    lines.append(f"- `{act[:140]}`")
        (evid / "driver.md").write_text(
            f"# UX driver trace — {evid.name}\n\nSource session: `{src.name}`\n\n" + "\n".join(lines))
    except Exception as e:
        say(f"  UX trace collection warning: {e}")

def _ux_launch(it, name):
    """Build the item's branch as a real GUI .app bundle on the display pod and launch it via
    LaunchServices (`open`), so it attaches to the user's GUI session and renders on the real display.
    A plain `cargo build --bin twarp-oss` does NOT work: it omits the `gui` feature and produces a bare
    binary that fatally exits ('no asset exists at path bundled/png/local.png'). `script/run` is the
    only correct macOS launch — it `cargo bundle`s with FEATURES=gui, stages resources, and codesigns,
    producing target/debug/bundle/osx/<App>.app. The bundle name is resolved at run time (ls glob),
    not hardcoded — 09-rebrand renames it, and a branch mid-rebrand must still gate with whatever
    name it builds."""
    iid = it["id"]; repo = node_repo(name); wt = f"{node_wt(name)}/ux-{iid}"
    cmd = (fresh_worktree(name, wt, f"origin/fleet/{iid}") +
           f"export CARGO_TARGET_DIR={wt}/target\ncd {wt}\n"   # bundle resolves assets via its own tree
           f"./script/run --dont-open > /tmp/ux_build_{iid}.log 2>&1\necho BUILD_$?\n"
           f'APP=$(ls -d target/debug/bundle/osx/*.app 2>/dev/null | head -1)\n'
           f'test -n "$APP" && echo BUNDLE_OK || echo BUNDLE_MISSING\n'
           # Detach caffeinate's fds from the ssh pipe: left running through the drive phase, an
           # inherited stdout/stderr keeps the remote `bash -s` channel open so bash_on() never
           # returns and the gate hangs until timeout (never reaching the driver). </dev/null too.
           f"caffeinate -dimsu </dev/null >/dev/null 2>&1 & echo $! > /tmp/ux_caf_{iid}\n"
           f'open "$APP" > /tmp/ux_open_{iid}.log 2>&1; echo OPEN_$?\n'
           # match the worktree's bundle path, not the app name — per-item precise + rename-proof
           f"sleep 12\npgrep -f '{wt}/target/debug/bundle' >/dev/null && echo LAUNCH_UP || echo LAUNCH_DOWN\n")
    say(f"  [{iid}] UX: building GUI bundle + launching on {name}'s display (multi-min build)…")
    with gatelock(name):                       # one cargo cache per pod
        r = bash_on(name, cmd, timeout=3600)
    # Gate on the app actually being up (BUNDLE_OK + LAUNCH_UP), NOT on script/run's exit code:
    # ad-hoc codesigning flakily returns `errSecInternalComponent` (BUILD_1) while still producing a
    # launchable, running bundle. LAUNCH_UP is the real signal; a non-zero BUILD alone must not veto
    # it. Distinct up/down tokens: the old RUNNING/NOTRUNNING pair made the substring check always
    # true, so a dead app still went to the driver (which then judged whatever else was on screen).
    ok = ("BUNDLE_OK" in r.stdout and "LAUNCH_UP" in r.stdout)
    (LOG / f"{iid}.uxlaunch.log").write_text(r.stdout)
    return ok, r.stdout

def _ux_teardown(it, name):
    iid = it["id"]; wt = f"{node_wt(name)}/ux-{iid}"; repo = node_repo(name)
    try:
        bash_on(name, f"pkill -f '{wt}/target/debug/bundle' 2>/dev/null || true\n"
                      f"kill $(cat /tmp/ux_caf_{iid} 2>/dev/null) 2>/dev/null || true\n"
                      f"cd {repo}\ngit worktree remove --force {wt} 2>/dev/null || true\n"
                      f"rm -rf {wt}\ngit worktree prune\n", timeout=120)
    except Exception as e:
        say(f"  [{iid}] UX teardown warning on {name}: {e}")

def ux_drive_gate(it):
    """Dynamic UX gate: launch the PR's build, let a Claude agent drive the live app and judge it vs
    acceptance criteria. Falls back to the bootstrap screenshot gate if the display pod can't inject.
    Returns (verdict, evidence_dir) — 'regression' blocks merge; 'pass'/'error' do not."""
    iid = it["id"]; name = display_node()
    if not name:
        return _stamp_ux(iid, "none", "error", None)
    if not ux_inject_ready(name):
        say(f"  [{iid}] UX: injector not trusted on {name} (grant Accessibility to UidriveAgent) "
            f"→ falling back to bootstrap screenshot gate")
        return _stamp_ux(iid, "bootstrap", *uxgate(it.get("ux_test", UXTEST)))
    if not ux_capture_ready(name):
        say(f"  [{iid}] UX: capturer not granted on {name} (grant Screen Recording to UicaptureAgent; "
            f"else captures are privacy-limited desktop-only) → falling back to bootstrap screenshot gate")
        return _stamp_ux(iid, "bootstrap", *uxgate(it.get("ux_test", UXTEST)))
    host = node_host(name); is_self = (name == SELF)
    LOG.mkdir(parents=True, exist_ok=True)
    evid = LOG / f"ux_{iid}"; evid.mkdir(exist_ok=True)
    shot = evid / "ux_shot"; act = evid / "ux_act"
    if is_self:
        shot.write_text('#!/bin/bash\n~/.local/bin/uishot "$1"\n')
        act.write_text('#!/bin/bash\n~/.local/bin/uinject "$*"\n')
    else:
        shot.write_text(f"#!/bin/bash\ninfo=$(ssh {host} '~/.local/bin/uishot /tmp/uxframe.png')\n"
                        f'scp -q {host}:/tmp/uxframe.png "$1"\necho "$1 $info"\n')
        act.write_text(f'#!/bin/bash\nssh {host} "~/.local/bin/uinject \\"$*\\""\n')
    os.chmod(shot, 0o755); os.chmod(act, 0o755)

    with _screenlock:                          # one display → one UX session at a time
        launched, ltail = _ux_launch(it, name)
        if not launched:
            say(f"  [{iid}] UX: app build/launch failed → bootstrap gate")
            _ux_teardown(it, name)
            return _stamp_ux(iid, "bootstrap", *uxgate(it.get("ux_test", UXTEST)))
        try:
            sh(["git", "-C", str(SELF_REPO), "fetch", "-q", "origin"])
            diff = sh(["git", "-C", str(SELF_REPO), "diff",
                       f"origin/master...origin/fleet/{iid}"]).stdout[:8000]
            prompt = UX_DRIVE_PROMPT.format(shot=shot, act=act, final=evid / "final.png",
                                            criteria=_ux_criteria(it), diff=diff or "(no diff)")
            drv_start = time.time()
            r = sh(["claude", "-p", prompt, "--dangerously-skip-permissions"], timeout=1200)
            _collect_driver_trace(evid, drv_start)   # persist the driver's full conversation for review
        finally:
            _ux_teardown(it, name)
    out = (r.stdout or "").strip()
    (evid / "transcript.txt").write_text(out)
    line = next((l for l in out.splitlines() if "VERDICT" in l.upper()),
                out.splitlines()[-1] if out else "")
    say(f"  [{iid}] UX drive → {line}")
    if "VERDICT" not in out.upper() and _LIMIT_RE.search(out):
        # The driver itself is quota-limited — that's not a pass and not an app error; the caller
        # waits and re-drives rather than letting the item merge un-verified.
        return _stamp_ux(iid, "live", "unavailable", evid)
    # Token right after VERDICT decides; prose like "no regression observed" in a pass
    # line must not flip the verdict (bit 19b round 2: pass misrouted to fix-agent).
    m = re.search(r"\bVERDICT\b\W{0,4}(pass|regression|fail|error)", line, re.I)
    if m:
        verdict = "regression" if m.group(1).lower() == "fail" else m.group(1).lower()
    else:
        low = line.lower()
        verdict = ("regression" if ("regression" in low or "verdict fail" in low)
                   else "pass" if "pass" in low else "error")
    return _stamp_ux(iid, "live", verdict, evid)


# ---------- gate (on the pod that authored) ----------
def gate(it, ref=None):
    """Build+test `ref` (default origin/fleet/<id>) on the item's pod. Returns (ok, tail)."""
    iid = it["id"]; name = it.get("node") or codex_node() or ACTIVE_PODS[0]
    verify = it.get("verify") or DEFAULT_VERIFY; ref = ref or f"origin/fleet/{iid}"
    repo = node_repo(name); wt = f"{node_wt(name)}/gate-{iid}"
    cmd = (fresh_worktree(name, wt, ref) +
           f"export CARGO_TARGET_DIR={repo}/target\ncd {wt}\n"
           f"({verify}) > /tmp/gate_{iid}.log 2>&1\necho GATE_EXIT_$?\ntail -25 /tmp/gate_{iid}.log\n")
    say(f"  [{iid}] gating ({verify}) on {name}…")
    with gatelock(name):                  # one cargo cache per pod — serialize within the pod
        r = bash_on(name, cmd, timeout=2400)
    ok = "GATE_EXIT_0" in r.stdout
    full = node_read(name, f"/tmp/gate_{iid}.log") or r.stdout   # FULL build/test output, not the -25 tail
    (LOG / f"{iid}.gate.log").write_text(full)
    log_event(iid, "gate", ok=ok)
    return ok, r.stdout.strip().splitlines()[-8:]


# ---------- supervisor (merge queue) ----------
def speculative_gate(it):
    """Merge origin/master + origin/fleet/<id> on the item's pod, gate the combination."""
    iid = it["id"]; name = it.get("node") or codex_node() or ACTIVE_PODS[0]
    verify = it.get("verify") or DEFAULT_VERIFY; repo = node_repo(name); wt = f"{node_wt(name)}/spec-{iid}"
    cmd = (fresh_worktree(name, wt, "origin/master") +
           f"export CARGO_TARGET_DIR={repo}/target\ncd {wt}\n"
           f"if ! git -c user.email=fleet@local -c user.name=fleet merge --no-edit origin/fleet/{iid} > /tmp/spec_{iid}.log 2>&1; then\n"
           f"  echo MERGE_CONFLICT; tail -15 /tmp/spec_{iid}.log; exit 0\nfi\n"
           f"({verify}) >> /tmp/spec_{iid}.log 2>&1\necho SPEC_EXIT_$?\ntail -20 /tmp/spec_{iid}.log\n")
    with gatelock(name):
        r = bash_on(name, cmd, timeout=2400)
    full = node_read(name, f"/tmp/spec_{iid}.log") or r.stdout   # FULL merge+test output, not the tail
    (LOG / f"{iid}.spec.log").write_text(full)
    verdict = ("conflict" if "MERGE_CONFLICT" in r.stdout
               else "ok" if "SPEC_EXIT_0" in r.stdout else "fail")
    log_event(iid, "spec", verdict=verdict)
    return verdict, r.stdout.strip().splitlines()[-6:]

def auto_merge(iid, title):
    repo = cfg()["repo"]
    make_pr(iid, title)   # idempotent — PR already opened during iterate()
    r = sh(["gh", "pr", "merge", f"fleet/{iid}", "--repo", repo, "--squash", "--delete-branch"],
           cwd=str(SELF_REPO))
    return r.returncode == 0, (r.stdout + r.stderr).strip()


# ---------- per-PR loop: fix-until-green + staff-architect review ----------
def make_pr(iid, title):
    """Open a PR for fleet/<id> if one isn't already open (idempotent). gh runs on the self pod."""
    repo = cfg()["repo"]; base = cfg()["base"]
    ex = sh(["gh", "pr", "list", "--repo", repo, "--head", f"fleet/{iid}", "--json", "number",
             "-q", ".[0].number"], cwd=str(SELF_REPO))
    if ex.stdout.strip():
        return ex.stdout.strip()
    body = ("Opened by the twarp fleet. Auto-iterated until the functional gate + staff-architect "
            "review pass, then merged via the speculative merge-queue.\n\n"
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)")
    sh(["gh", "pr", "create", "--repo", repo, "--base", base, "--head", f"timomak:fleet/{iid}",
        "--title", title, "--body", body], cwd=str(SELF_REPO))
    return "created"

def fix_agent(it, errors):
    """Re-run the item's pod agent ON ITS EXISTING branch with the failure as context."""
    iid = it["id"]; name = it.get("node") or codex_node() or ACTIVE_PODS[0]
    fix_prompt = (
        f"You are fixing your own branch fleet/{iid} (task: {it['title']}). Read AGENTS.md. The fleet "
        f"gate FAILED — fix the ROOT CAUSE, stay in scope, do not revert prior good work.\n\n"
        f"Original task: {it['task']}\n\nFAILURE OUTPUT:\n{errors}\n\n"
        f"Output one final line: WORKER_DONE {iid}")
    return author(it, name, ref=f"origin/fleet/{iid}", prompt_text=fix_prompt)

def _parse_arch(out):
    """Pull the `ARCH approve|changes — …` verdict line out of a reviewer's output. Returns
    (verdict|None, note); None means no parseable verdict (caller decides the fallback)."""
    out = (out or "").strip()
    # Reviewers wrap the verdict in markdown ("**ARCH approve** — …", "`ARCH changes`…"), so match
    # the token pair anywhere in a line instead of requiring the line to start with ARCH.
    pat = re.compile(r"\bARCH\b\W{0,4}(approve|changes)", re.I)
    line = next((l.strip() for l in out.splitlines() if pat.search(l)), "")
    if not line:
        return None, ""
    return pat.search(line).group(1).lower(), line

REVIEW_PROMPT = (
    "You are a staff/principal engineer doing a PRE-MERGE review of a twarp change at ARCHITECTURE "
    "altitude — correctness, blast radius, fork discipline, reversibility, long-term "
    "maintainability — NOT style nitpicks. Approve unless there's a real blocking concern. "
    "Do NOT edit any files; only output your verdict.\n\n"
    "Diff:\n\n{diff}\n\n"
    "Reply with EXACTLY one line:\n"
    "  ARCH approve — <one line: why it's safe to merge>\n"
    "  ARCH changes — <the specific blocking issue(s) to fix>")

_LIMIT_RE = re.compile(r"session limit|usage limit|rate.?limit|quota|overloaded|resets \d", re.I)

def _claude_review(prompt, iid=None):
    """Review with claude on a claude pod (self) — or on the self pod if no claude pod is active
    (default mode runs the whole loop on the codex machine, which also has claude installed).
    Returns ('unavailable', why) when claude is quota/rate-limited — that is NOT a changes verdict."""
    r = sh(["claude", "-p", prompt, "--dangerously-skip-permissions"], timeout=300)
    out = (r.stdout or "") + (r.stderr or "")
    if iid:
        (LOG / f"{iid}.review.claude.log").write_text(out)
    verdict, note = _parse_arch(r.stdout)
    if verdict is None and _LIMIT_RE.search(out):
        return "unavailable", out.strip()[:160]
    return verdict, note

def _codex_review(iid, prompt):
    """Review with codex on a codex pod, isolated in a throwaway tmp dir with --skip-git-repo-check
    so it can't touch any worktree. The diff is embedded in the prompt. Returns (verdict|None, note)."""
    name = codex_node()
    if name is None:
        return None, ""
    p = LOG / f"{iid}.review.prompt"; p.write_text(prompt)
    put_file(name, p, f"/tmp/fleet_{iid}.review")
    run = (f"rm -rf /tmp/fleet_rev_{iid}\nmkdir -p /tmp/fleet_rev_{iid}\ncd /tmp/fleet_rev_{iid}\n"
           f"cat /tmp/fleet_{iid}.review | $HOME/.local/bin/codex exec "
           f"--dangerously-bypass-approvals-and-sandbox --skip-git-repo-check "
           f"-c model_reasoning_effort='high' > /tmp/fleet_{iid}.review.log 2>&1\n"
           f"echo CODEX_EXIT_$?\n")
    r = bash_on(name, run, timeout=600)
    log = node_read(name, f"/tmp/fleet_{iid}.review.log")
    (LOG / f"{iid}.review.log").write_text(log)
    if "CODEX_EXIT_0" not in r.stdout:
        return None, ""
    return _parse_arch(log)

def architect_review(it):
    """Staff/principal-altitude pre-merge review of the branch diff. Returns (verdict, note).
    INDEPENDENT reviewer: review with the model that did NOT author the item — codex-authored work
    is reviewed by claude, claude-authored work is reviewed by codex — so no model rubber-stamps its
    own diff. Codex-review failures fall back to claude (never blind-approves)."""
    iid = it["id"]
    sh(["git", "-C", str(SELF_REPO), "fetch", "-q", "origin"])
    diff = sh(["git", "-C", str(SELF_REPO), "diff", f"origin/master...origin/fleet/{iid}"]).stdout
    if len(diff) > 60000:
        diff = diff[:60000] + "\n...[diff truncated]..."
    prompt = REVIEW_PROMPT.format(diff=diff)
    if node_kind(it.get("node") or "") == "codex":     # codex authored → claude reviews
        verdict, note = _claude_review(prompt, iid); reviewer = "claude"
    else:                                              # claude authored → codex reviews (independence)
        verdict, note = _codex_review(iid, prompt); reviewer = "codex"
        if verdict is None:                            # codex review unusable → independent fallback
            verdict, note = _claude_review(prompt, iid); reviewer = "claude(fallback)"
    if verdict is None:
        verdict, note = "changes", "review produced no parseable verdict"
    return verdict, f"[{reviewer}] {note}"  # verdict ∈ {approve, changes, unavailable}

def iterate(it):
    """Drive ONE PR to green + architect-approved: gate → fix → re-gate → architect → fix → … .
    Gates serialize within a pod; authoring/fixing/review run in parallel across items and pods."""
    try:
        return _iterate(it)
    except Exception as e:
        # A per-item failure must not crash the whole batch (ex.map re-raises on iteration).
        say(f"  [{it['id']}] iterate EXC: {e}"); set_status(it["id"], "failed")
        return False

def _retry_while_unavailable(iid, what, fn):
    """claude quota-limited ('unavailable') is not a code problem: wait for the window to reset
    (15-min retries, cap ~5h) instead of burning author fix-rounds or waving work through. Returns
    the final (verdict, extra); the caller parks the item if it is STILL 'unavailable'."""
    verdict, extra = fn()
    waits = 0
    while verdict == "unavailable" and waits < 20:
        waits += 1
        say(f"  [{iid}] {what} unavailable (claude quota-limited) — retry {waits}/20 in 15 min")
        time.sleep(900)
        verdict, extra = fn()
    return verdict, extra

def _iterate(it):
    iid = it["id"]
    set_status(iid, "iterating")
    make_pr(iid, it["title"])
    for rnd in range(1, MAX_ROUNDS + 1):
        ok, tail = gate(it)
        if not ok:
            say(f"  [{iid}] gate FAIL (round {rnd}) → fix-agent")
            fixed, _ = fix_agent(it, "\n".join(tail))
            if not fixed:
                say(f"  [{iid}] fix produced no change — giving up"); set_status(iid, "failed"); return False
            continue
        if it.get("ux"):
            # drive the live app vs criteria; bootstrap-gate fallback
            uv, _ = _retry_while_unavailable(iid, "UX driver", lambda: ux_drive_gate(it))
            if uv == "unavailable":
                say(f"  [{iid}] UX driver still unavailable after ~5h — parking as failed")
                set_status(iid, "failed"); return False
            if uv == "regression":
                say(f"  [{iid}] UX regression (round {rnd}) → fix-agent")
                fix_agent(it, "The UX gate drove the live app and found the feature broken or missing. "
                              "Re-read the acceptance criteria and fix the user-facing behavior."); continue
        verdict, note = _retry_while_unavailable(iid, "architect reviewer", lambda: architect_review(it))
        if verdict == "unavailable":
            say(f"  [{iid}] reviewer still unavailable after ~5h — parking as failed")
            set_status(iid, "failed"); return False
        say(f"  [{iid}] architect (round {rnd}): {note}")
        if verdict != "approve":
            fix_agent(it, f"Staff-architect review requested changes: {note}"); continue
        set_status(iid, "ready"); return True
    say(f"  [{iid}] exhausted {MAX_ROUNDS} rounds without green+approved"); set_status(iid, "exhausted")
    return False


# ---------- orchestration ----------
def _resolve_pods(both):
    """Pick the active pod set. `--both` uses config.pods_both (default all nodes); otherwise
    config.pods_default (default [SELF])."""
    c = cfg()
    if both:
        pods = c.get("pods_both") or list(nodes_cfg().keys())
    else:
        pods = c.get("pods_default") or ([SELF] if SELF in nodes_cfg() else list(nodes_cfg())[:1])
    return [p for p in pods if p in nodes_cfg()]

def cmd_dispatch(args):
    q = load()
    picked = eligible(q)
    if not picked:
        say("dispatch: nothing eligible")
        return []
    say(f"dispatch: leasing {[i['id'] for i in picked]}")
    assign_pods(picked)
    return picked

def merge_one(it):
    """Speculative-merge gate + auto-merge a single ready PR. Serialized by the caller."""
    try:
        return _merge_one(it)
    except Exception as e:
        say(f"  [{it['id']}] merge EXC: {e}"); set_status(it["id"], "failed")
        return False

def _merge_one(it):
    iid = it["id"]
    set_status(iid, "merging")
    verdict, tail = speculative_gate(it)
    if verdict == "conflict":
        say(f"  [{iid}] speculative merge CONFLICT — needs rebase"); set_status(iid, "needs-rebase"); return False
    if verdict != "ok":
        say(f"  [{iid}] speculative gate FAILED (semantic conflict) — ejected"); set_status(iid, "failed"); return False
    ok, out = auto_merge(iid, it["title"])
    say(f"  [{iid}] auto-merge {'OK' if ok else 'FAILED'}: {out.splitlines()[-1] if out else ''}")
    set_status(iid, "merged" if ok else "ready"); return ok

def write_run_report():
    """Post-run human report (report.md) + machine index (index.json) from events.jsonl + queue.json:
    per-item final status, gate rounds, UX level+verdict+evidence, and wall-clock duration — the single
    'what happened this run' artifact. Flags any item that merged WITHOUT a live UX drive (the 14-class
    gap). Returns (index, risky_rows)."""
    q = load(); items = {i["id"]: i for i in q["items"]}
    events = []
    if EVENTS.exists():
        for l in EVENTS.read_text().splitlines():
            if l.strip():
                try: events.append(json.loads(l))
                except Exception: pass
    per = {}
    for e in events:
        d = per.setdefault(e["iid"], {"first": e["ts"], "last": e["ts"], "gate_rounds": 0})
        d["first"] = min(d["first"], e["ts"]); d["last"] = max(d["last"], e["ts"])
        if e["stage"] == "gate": d["gate_rounds"] += 1
    rows = []
    for iid, d in per.items():
        it = items.get(iid, {})
        rows.append({"id": iid, "status": it.get("status"), "ux": bool(it.get("ux")),
                     "ux_level": it.get("ux_level"), "ux_verdict": it.get("ux_verdict"),
                     "ux_evidence": it.get("ux_evidence"), "gate_rounds": d["gate_rounds"],
                     "duration_s": round(d["last"] - d["first"], 1)})
    rows.sort(key=lambda r: r["id"])
    index = {"generated": round(time.time(), 3), "items": rows}
    (LOG / "index.json").write_text(json.dumps(index, indent=2))
    risky = [r for r in rows if r["status"] == "merged" and r["ux"] and r["ux_level"] != "live"]
    lines = ["# fleet run report", "",
             "| item | status | ux | ux_level | ux_verdict | gate_rounds | dur(s) | evidence |",
             "|------|--------|----|----------|-----------|-------------|--------|----------|"]
    for r in rows:
        ev = f"`{r['ux_evidence']}`" if r["ux_evidence"] else "—"
        lines.append(f"| {r['id']} | {r['status']} | {'✓' if r['ux'] else ''} | {r['ux_level'] or '—'} | "
                     f"{r['ux_verdict'] or '—'} | {r['gate_rounds']} | {r['duration_s']} | {ev} |")
    if risky:
        lines += ["", "## ⚠️ merged WITHOUT a live UX drive",
                  *[f"- **{r['id']}** (ux_level={r['ux_level'] or 'none'}) — was never driven; verify manually"
                    for r in risky]]
    (LOG / "report.md").write_text("\n".join(lines) + "\n")
    return index, risky

def cmd_run(args):
    """Continuous batch loop: fill up to total-builder-capacity ready items → author in parallel
    across pods → drive each PR to green + architect-approved (parallel authoring/fixing, per-pod
    serialized gates) → merge-queue → report → refill → repeat until the queue+roadmap are drained."""
    global ACTIVE_PODS
    ACTIVE_PODS = _resolve_pods(getattr(args, "both", False))
    for p in ACTIVE_PODS:
        gatelock(p)   # pre-create per-pod locks
    LOG.mkdir(parents=True, exist_ok=True)
    cap = sum(node_builders(p) for p in ACTIVE_PODS) or 1
    say(f"=== fleet up — self={SELF} pods={ACTIVE_PODS} builders={cap} gates={len(ACTIVE_PODS)} ===")
    batch_no = 0
    while True:
        # reflect just-merged changes locally (e.g. STATUS.md checkbox ticks). queue.json is
        # machine-local runtime state this loop mutates in place, so it is always dirty here and a
        # plain ff-merge silently refuses whenever an incoming commit touches it — set the checkout
        # copy aside, merge, then restore the live ledger over whatever snapshot came in.
        sh(["git", "-C", str(SELF_REPO), "fetch", "-q", "origin"])
        with _qlock:
            q = load()
            sh(["git", "-C", str(SELF_REPO), "checkout", "-q", "--", "fleet/queue.json"])
            r = sh(["git", "-C", str(SELF_REPO), "merge", "-q", "--ff-only", "origin/master"])
            save(q)
        if r.returncode != 0:
            say(f"  WARN: ff-merge of origin/master failed — roadmap view may be stale: "
                f"{(r.stderr or r.stdout).strip().splitlines()[0] if (r.stderr or r.stdout).strip() else '?'}")
        say(f"roadmap: {roadmap_sync()}")            # top up from the roadmap each batch
        picked = eligible(load(), cap=cap)           # up to total builder capacity, file-disjoint
        if not picked:
            say("=== queue drained — nothing ready ==="); break
        batch_no += 1
        assign_pods(picked)
        say(f"=== batch {batch_no}: authoring {len(picked)} across pods → "
            f"{[(i['id'], i['node']) for i in picked]} ===")

        # 1) author in parallel across pods
        with cf.ThreadPoolExecutor(max_workers=len(picked)) as ex:
            authored = list(ex.map(run_worker, picked))
        live = [item(load(), iid) for iid, ok in authored if ok]

        # 2) drive each PR to green + architect-approved (gates serialize per-pod, parallel across)
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
        # reap worktrees for every item that reached a terminal state this batch (merged or failed)
        # on its own pod, so the multi-GB target/ dirs don't accumulate and fill that machine's disk.
        for it in picked:
            cur = item(load(), it["id"])
            if cur.get("status") in ("merged", "failed"):
                reap_worktrees(it["id"], cur.get("node") or it["node"])
        cmd_status(args)
    idx, risky = write_run_report()
    merged_ids = [r["id"] for r in idx["items"] if r["status"] == "merged"]
    failed_ids = [r["id"] for r in idx["items"]
                  if r["status"] in ("failed", "exhausted", "needs-rebase")]
    msg = f"run complete — merged {merged_ids or '[]'}, failed {failed_ids or '[]'}"
    if risky:
        msg += f", ⚠️ merged-without-live-UX {[r['id'] for r in risky]}"
    notify(msg, "warn" if (failed_ids or risky) else "info")
    say(f"=== run complete — report: {LOG / 'report.md'} ===")


def main():
    global SELF, ACTIVE_PODS
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("status", "dispatch", "roadmap-sync"):
        sub.add_parser(name)
    r = sub.add_parser("run")
    r.add_argument("--self", dest="self_pod", default=None, help="which pod this process runs on")
    r.add_argument("--both", action="store_true", help="enlist both machines (4 builders / 2 gates)")
    w = sub.add_parser("worker"); w.add_argument("id"); w.add_argument("--self", dest="self_pod", default=None)
    g = sub.add_parser("gate"); g.add_argument("id"); g.add_argument("--self", dest="self_pod", default=None)
    s = sub.add_parser("supervise"); s.add_argument("id"); s.add_argument("--self", dest="self_pod", default=None)
    u = sub.add_parser("uxgate"); u.add_argument("test", nargs="?", default=UXTEST)
    u.add_argument("--self", dest="self_pod", default=None)
    ud = sub.add_parser("uxdrive"); ud.add_argument("id")
    ud.add_argument("--self", dest="self_pod", default=None)
    sub.add_parser("report")   # regenerate report.md / index.json from events.jsonl + queue.json
    args = ap.parse_args()
    if getattr(args, "self_pod", None):
        SELF = args.self_pod

    if args.cmd == "status":
        cmd_status(args)
    elif args.cmd == "roadmap-sync":
        print("roadmap:", roadmap_sync())
    elif args.cmd == "report":
        idx, risky = write_run_report()
        print(f"report: {LOG / 'report.md'}  ({len(idx['items'])} items)")
        if risky:
            print("⚠️ merged without a live UX drive:", [r["id"] for r in risky])
    elif args.cmd == "dispatch":
        ACTIVE_PODS = _resolve_pods(False)
        cmd_dispatch(args)
    elif args.cmd == "run":
        cmd_run(args)
    elif args.cmd in ("worker", "gate", "supervise", "uxgate", "uxdrive"):
        ACTIVE_PODS = _resolve_pods(False)
        if args.cmd == "worker":
            run_worker(item(load(), args.id))
        elif args.cmd == "gate":
            it = item(load(), args.id)
            ok, tail = gate(it)
            print("GATE", "PASS" if ok else "FAIL"); [print(" |", l) for l in tail]
        elif args.cmd == "supervise":
            print(speculative_gate(item(load(), args.id)))
        elif args.cmd == "uxgate":
            verdict, png = uxgate(args.test)
            print(f"UXGATE {verdict} png={png}")
        elif args.cmd == "uxdrive":
            verdict, evid = ux_drive_gate(item(load(), args.id))
            print(f"UXDRIVE {verdict} evidence={evid}")

if __name__ == "__main__":
    main()
