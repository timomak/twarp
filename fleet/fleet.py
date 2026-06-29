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

def load():
    return json.loads(QUEUE.read_text())

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

def roadmap_sync():
    """Pull the next unchecked IMPL sub-phase of the active roadmap feature into the queue.
    Specs are human-gated: this only acts when the feature is `impl-pending`; for any other phase it
    pulls nothing and explains what's needed. Pulls ONE sub-phase at a time (they're sequential and
    share files). Returns a one-line status string."""
    feat = _active_feature()
    if not feat:
        return "no active feature in ROADMAP.md"
    status_md = SELF_REPO / "roadmap" / feat / "STATUS.md"
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
    q["items"].append({"id": sub_id, "title": f"{feat} {sub_id}: {sub_title}", "node": None,
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
    r = bash_on(name, run, timeout=1500)
    (LOG / f"{iid}.author.log").write_text(node_read(name, f"/tmp/fleet_{iid}.author.log"))
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


# ---------- gate (on the pod that authored) ----------
def gate(it, ref=None):
    """Build+test `ref` (default origin/fleet/<id>) on the item's pod. Returns (ok, tail)."""
    iid = it["id"]; name = it.get("node") or codex_node() or ACTIVE_PODS[0]
    verify = it["verify"]; ref = ref or f"origin/fleet/{iid}"
    repo = node_repo(name); wt = f"{node_wt(name)}/gate-{iid}"
    cmd = (fresh_worktree(name, wt, ref) +
           f"export CARGO_TARGET_DIR={repo}/target\ncd {wt}\n"
           f"({verify}) > /tmp/gate_{iid}.log 2>&1\necho GATE_EXIT_$?\ntail -25 /tmp/gate_{iid}.log\n")
    say(f"  [{iid}] gating ({verify}) on {name}…")
    with gatelock(name):                  # one cargo cache per pod — serialize within the pod
        r = bash_on(name, cmd, timeout=2400)
    ok = "GATE_EXIT_0" in r.stdout
    (LOG / f"{iid}.gate.log").write_text(r.stdout)
    return ok, r.stdout.strip().splitlines()[-8:]


# ---------- supervisor (merge queue) ----------
def speculative_gate(it):
    """Merge origin/master + origin/fleet/<id> on the item's pod, gate the combination."""
    iid = it["id"]; name = it.get("node") or codex_node() or ACTIVE_PODS[0]
    verify = it["verify"]; repo = node_repo(name); wt = f"{node_wt(name)}/spec-{iid}"
    cmd = (fresh_worktree(name, wt, "origin/master") +
           f"export CARGO_TARGET_DIR={repo}/target\ncd {wt}\n"
           f"if ! git -c user.email=fleet@local -c user.name=fleet merge --no-edit origin/fleet/{iid} > /tmp/spec_{iid}.log 2>&1; then\n"
           f"  echo MERGE_CONFLICT; tail -15 /tmp/spec_{iid}.log; exit 0\nfi\n"
           f"({verify}) >> /tmp/spec_{iid}.log 2>&1\necho SPEC_EXIT_$?\ntail -20 /tmp/spec_{iid}.log\n")
    with gatelock(name):
        r = bash_on(name, cmd, timeout=2400)
    (LOG / f"{iid}.spec.log").write_text(r.stdout)
    if "MERGE_CONFLICT" in r.stdout:
        return "conflict", r.stdout.strip().splitlines()[-6:]
    return ("ok" if "SPEC_EXIT_0" in r.stdout else "fail"), r.stdout.strip().splitlines()[-6:]

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
    line = next((l for l in out.splitlines() if l.strip().upper().startswith("ARCH")), "")
    if not line:
        return None, ""
    return ("approve" if "approve" in line.lower() else "changes"), line

REVIEW_PROMPT = (
    "You are a staff/principal engineer doing a PRE-MERGE review of a twarp change at ARCHITECTURE "
    "altitude — correctness, blast radius, fork discipline, reversibility, long-term "
    "maintainability — NOT style nitpicks. Approve unless there's a real blocking concern. "
    "Do NOT edit any files; only output your verdict.\n\n"
    "Diff:\n\n{diff}\n\n"
    "Reply with EXACTLY one line:\n"
    "  ARCH approve — <one line: why it's safe to merge>\n"
    "  ARCH changes — <the specific blocking issue(s) to fix>")

def _claude_review(prompt):
    """Review with claude on a claude pod (self) — or on the self pod if no claude pod is active
    (default mode runs the whole loop on the codex machine, which also has claude installed)."""
    r = sh(["claude", "-p", prompt, "--dangerously-skip-permissions"], timeout=300)
    return _parse_arch(r.stdout)

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
        verdict, note = _claude_review(prompt); reviewer = "claude"
    else:                                              # claude authored → codex reviews (independence)
        verdict, note = _codex_review(iid, prompt); reviewer = "codex"
        if verdict is None:                            # codex review unusable → independent fallback
            verdict, note = _claude_review(prompt); reviewer = "claude(fallback)"
    if verdict is None:
        verdict, note = "changes", "review produced no parseable verdict"
    return verdict, f"[{reviewer}] {note}"

def iterate(it):
    """Drive ONE PR to green + architect-approved: gate → fix → re-gate → architect → fix → … .
    Gates serialize within a pod; authoring/fixing/review run in parallel across items and pods."""
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
            uv, _ = uxgate(it.get("ux_test", UXTEST))
            if uv == "regression":
                say(f"  [{iid}] UX regression (round {rnd}) → fix-agent")
                fix_agent(it, "The UX visual gate found a regression vs the golden screenshot."); continue
        verdict, note = architect_review(it)
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
        # reflect just-merged changes locally (e.g. STATUS.md checkbox ticks) — best-effort
        sh(["git", "-C", str(SELF_REPO), "fetch", "-q", "origin"])
        sh(["git", "-C", str(SELF_REPO), "merge", "-q", "--ff-only", "origin/master"])
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
    say("=== run complete ===")


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
    args = ap.parse_args()
    if getattr(args, "self_pod", None):
        SELF = args.self_pod

    if args.cmd == "status":
        cmd_status(args)
    elif args.cmd == "roadmap-sync":
        print("roadmap:", roadmap_sync())
    elif args.cmd == "dispatch":
        ACTIVE_PODS = _resolve_pods(False)
        cmd_dispatch(args)
    elif args.cmd == "run":
        cmd_run(args)
    elif args.cmd in ("worker", "gate", "supervise", "uxgate"):
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

if __name__ == "__main__":
    main()
