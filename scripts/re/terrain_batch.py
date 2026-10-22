#!/usr/bin/env python3
"""Plan, run and watch an in-client batch terrain generation (docs/re/terrain/batch.md).

The client DLL's `gen_batch` feature runs the client's own terrain generator for a list of
`area seed topology orientation` jobs on worker threads and appends one line per job to
`batch-results.tsv` (columns: area seed topology orientation status tile_hash doodad_hash width
height ms detail). It needs no login and no server: the client sits at its login screen.
Private mode only; this script never touches the Steam install, never logs in anywhere and
refuses a GameFiles deployed in official mode.

    terrain_batch.py plan   <run> [--seeds 50] [--act 1,2] [--area G1_1,G2_*] [--kind tgr,dgr,town]
                                  [--master-seed N] [--vectors] [--limit N]
    terrain_batch.py run    <run> [--threads 4] [--no-deploy] [--max-restarts 300]
    terrain_batch.py status <run> [--watch SECONDS]
    terrain_batch.py verify <run>      # compare results with the known client vectors
    terrain_batch.py stop              # end a running `run` loop and its client

A run lives in dumps/<version>/terrain/batch/<run>/ (git-ignored): jobs.txt, export.txt
(optional: same format, jobs whose layout/doodads/grids are exported), batch-results.tsv,
crashes.tsv, state.json, seeds.json. `plan` seeds are drawn per combination from the master seed,
so the same plan gives the same jobs.
"""
import argparse
import fnmatch
import hashlib
import json
import os
import random
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from version import ROOT, dumps_dir  # noqa: E402

MATRIX = ROOT / "docs/status/terrain-matrix.tsv"
GAME = ROOT / "GameFiles"
HEADER = "area\tseed\ttopology\torientation\tstatus\ttile_hash\tdoodad_hash\twidth\theight\tms\tdetail"
DEFAULT_THREADS = 8
CRASH_LIMIT = 2  # crashes of one job (run alone) before it is marked crash-skip


def run_dir(run):
    return dumps_dir() / "terrain" / "batch" / run


def wine_path(p):
    """A Linux path as the game process sees it under Proton (Z: is the root)."""
    return "Z:" + str(Path(p).resolve()).replace("/", "\\")


# ---- plan ---------------------------------------------------------------------------------


def read_matrix():
    with open(MATRIX) as f:
        head = f.readline().rstrip("\n").split("\t")
        for line in f:
            row = dict(zip(head, line.rstrip("\n").split("\t")))
            yield row


def combos(args):
    """(area, topology, orientation) of the matrix rows that pass the filters."""
    acts = set(args.act.split(",")) if args.act else None
    areas = args.area.split(",") if args.area else None
    kinds = set(args.kind.split(",")) if args.kind else None
    for r in read_matrix():
        if r["kind"] == "fixed" or int(r["topology"]) < 1:
            continue
        if acts and r["act"] not in acts:
            continue
        if kinds and r["kind"] not in kinds:
            continue
        if areas and not any(fnmatch.fnmatchcase(r["area"], a) for a in areas):
            continue
        yield r["area"], int(r["topology"]), int(r["orientation"])


def seeds_for(master, area, topo, orient, n):
    h = hashlib.blake2b(f"{master}:{area}:{topo}:{orient}".encode(), digest_size=8).digest()
    rng = random.Random(int.from_bytes(h, "little"))
    out = set()
    while len(out) < n:
        out.add(rng.randrange(1, 0xFFFFFFFF))
    return sorted(out)


def known_vectors():
    """(area, seed) -> (tile hash, doodad hash, topology, orientation) of the client vectors the
    port reproduces (sweep-vectors.tsv + the combination area_vectors found)."""
    v = dumps_dir() / "terrain/vectors"
    hashes = {}
    for line in open(v / "sweep-vectors.tsv"):
        f = line.split()
        hashes[(f[0], f[2])] = (f[3], f[4])
    out = []
    for line in open(v / "area-vectors-results.txt"):
        m = re.match(r"(\S+)\t(\d+)\tmatch \((?:forced|natural) (\d+):(\d+)\)", line)
        if m and (m[1], m[2]) in hashes:
            out.append((m[1], int(m[2]), int(m[3]), int(m[4])) + hashes[(m[1], m[2])])
    return list(dict.fromkeys(out))


def cmd_plan(args):
    d = run_dir(args.run)
    d.mkdir(parents=True, exist_ok=True)
    jobs = []
    if args.vectors:
        jobs = [(a, s, t, o) for a, s, t, o, _, _ in known_vectors()]
        meta = {"vectors": True}
    else:
        master = args.master_seed
        for area, topo, orient in combos(args):
            jobs += [(area, s, topo, orient) for s in seeds_for(master, area, topo, orient, args.seeds)]
        meta = {"master_seed": master, "seeds_per_combination": args.seeds,
                "seed_rule": "blake2b(master:area:topology:orientation) seeds random.Random; "
                             f"{args.seeds} distinct values in 1..2^32-2, sorted",
                "filters": {"act": args.act, "area": args.area, "kind": args.kind}}
    if args.limit:
        jobs = jobs[: args.limit]
    with open(d / "jobs.txt", "w") as f:
        for j in jobs:
            f.write("%s %d %d %d\n" % j)
    combos_n = len({(a, t, o) for a, _, t, o in jobs})
    meta.update({"jobs": len(jobs), "combinations": combos_n,
                 "planned": time.strftime("%Y-%m-%d %H:%M:%S")})
    (d / "seeds.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(f"{d}: {len(jobs)} jobs, {combos_n} combinations")


# ---- results ------------------------------------------------------------------------------


def read_results(d):
    """key -> fields, for complete lines only."""
    out = {}
    p = d / "batch-results.tsv"
    if not p.exists():
        return out
    for line in p.read_text(errors="replace").splitlines():
        if line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) >= 10:
            out[tuple(f[:4])] = f
    return out


def read_jobs(p):
    out = []
    if p.exists():
        for line in p.read_text().splitlines():
            f = line.split()
            if len(f) == 4 and not line.startswith("#"):
                out.append(tuple(f))
    return out


# ---- run ----------------------------------------------------------------------------------


def client_pids():
    """Processes of the GameFiles client (not the Steam install's)."""
    pids = []
    for p in Path("/proc").iterdir():
        if not p.name.isdigit() or int(p.name) == os.getpid():
            continue
        try:
            argv = (p / "cmdline").read_bytes().decode(errors="replace").split("\0")
        except OSError:
            continue
        # Only the processes Proton makes for the GameFiles client (never a shell that mentions it).
        a0 = argv[0].lower()
        try:
            helper = a0.endswith("pathofexilesteam.exe") and Path(p / "cwd").resolve() == GAME.resolve()
        except OSError:
            helper = False  # e.g. the crash upload helper process
        if helper or (a0.startswith("z:") and a0.endswith("pathofexilesteam.exe")) or (
            a0.endswith("steam.exe") and any("GameFiles" in a for a in argv[1:2])
        ) or (len(argv) > 3 and "python" in a0 and argv[1].endswith("/proton") and "GameFiles" in argv[3]):
            pids.append(int(p.name))
    return pids


def client_rss_mb():
    """Resident memory of the GameFiles client under Proton (all its processes), in MB."""
    total = 0
    for pid in client_pids():
        try:
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                if line.startswith("VmRSS:"):
                    total += int(line.split()[1]) // 1024
        except OSError:
            pass
    return total


def stop_client(proc=None):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        pids = client_pids()
        if proc and proc.poll() is None:
            try:
                os.killpg(proc.pid, sig)
            except ProcessLookupError:
                pass
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        for _ in range(20):
            if not client_pids():
                break
            time.sleep(0.5)
        if not client_pids():
            return
    print("warning: client processes remain:", client_pids())


def check_private():
    cfg = (GAME / "poe-hook.toml").read_text()
    m = re.search(r'^mode\s*=\s*"(\w+)"', cfg, re.M)
    if not m or m[1] != "private":
        sys.exit("GameFiles/poe-hook.toml is not in private mode: refusing (cargo xtask deploy-client)")
    if subprocess.run(["pgrep", "-x", "steam"], capture_output=True).returncode != 0:
        sys.exit("Steam is not running (the client exits without it)")


def launches_total(st):
    return st["launches"]


def load_state(d):
    p = d / "state.json"
    if p.exists():
        return json.loads(p.read_text())
    return {"first_start": time.time(), "active_seconds": 0.0, "launches": 0, "crashes": 0, "hangs": 0}


def save_state(d, st):
    (d / "state.json").write_text(json.dumps(st, indent=2) + "\n")


def inflight(d):
    out = []
    for p in sorted(d.glob("inflight-*.txt")):
        t = p.read_text().strip().split("\t")
        if len(t) == 4:
            out.append(tuple(t))
        p.unlink()
    return out


def cmd_run(args):
    d = run_dir(args.run)
    if not (d / "jobs.txt").exists():
        sys.exit(f"{d}/jobs.txt missing: run plan first")
    if not args.no_deploy:
        subprocess.run(["cargo", "xtask", "deploy-client"], cwd=ROOT, check=True)
    check_private()
    jobs = read_jobs(d / "jobs.txt")
    st = load_state(d)
    crash_file = d / "crashes.tsv"
    crashes = {}
    if crash_file.exists():
        for line in crash_file.read_text().splitlines():
            f = line.split("\t")
            crashes[tuple(f[:4])] = int(f[4])
    results = d / "batch-results.tsv"
    restarts = 0
    peak_rss = st.get("peak_rss_mb", 0)
    log = open(ROOT / "runtime" / "terrain-batch-client.log", "ab")
    while True:
        done = read_results(d)
        todo = [j for j in jobs if j not in done]
        print(f"{time.strftime('%H:%M:%S')} {len(done)} results, {len(todo)} to go", flush=True)
        if not todo:
            break
        if restarts > args.max_restarts:
            sys.exit("too many restarts")
        stop_client()
        for marker in ("batch-done", "batch-stop", "batch-stopped"):
            (d / marker).unlink(missing_ok=True)
        # Jobs that were running when the client last died run first, alone.
        suspects = [j for j in todo if crashes.get(j, 0) >= 1]
        (d / "suspects.txt").write_text("".join("%s %s %s %s\n" % j for j in suspects))
        env = dict(os.environ)
        env["POE_HOOK_GEN_BATCH"] = wine_path(d)
        env["POE_HOOK_GEN_BATCH_THREADS"] = str(args.threads)
        env["POE_HOOK_WINDOW_TITLE"] = "PoE2 Batch Generator"
        (GAME / "poe-hook.log").unlink(missing_ok=True)
        (GAME / "poe-hook.keys").unlink(missing_ok=True)  # a stale `close` would end the next client at once
        proc = subprocess.Popen(["cargo", "xtask", "run-client"], cwd=ROOT, env=env, stdout=log,
                                stderr=subprocess.STDOUT, start_new_session=True)
        st["launches"] += 1
        started = last_progress = time.time()
        st["current_start"] = started
        st["peak_rss_mb"] = peak_rss
        save_state(d, st)
        last_size = results.stat().st_size if results.exists() else 0
        outcome = None
        launch_done = len(done)
        last_rss_log = 0
        cap_mb = int(args.max_rss_gb * 1024)
        pause_asked = None
        while outcome is None:
            time.sleep(2)
            size = results.stat().st_size if results.exists() else 0
            if size != last_size:
                last_size, last_progress = size, time.time()
            rss = client_rss_mb()
            if rss:
                peak_rss = max(peak_rss, rss)
            if time.time() - last_rss_log >= args.rss_interval:
                last_rss_log = time.time()
                with open(d / "rss.tsv", "a") as rf:
                    rf.write(f"{int(time.time())}\t{launches_total(st)}\t{len(read_results(d))}\t{rss}\t{args.threads}\n")
            # Memory cap / restart every N jobs: let the jobs in flight finish, then restart.
            if pause_asked is None:
                since = len(read_results(d)) - launch_done if args.restart_every else 0
                if rss > cap_mb:
                    pause_asked = f"RSS {rss} MB over the cap of {cap_mb} MB"
                elif args.restart_every and since >= args.restart_every:
                    pause_asked = f"{since} jobs since the start"
                if pause_asked:
                    print(f"{time.strftime('%H:%M:%S')} restarting the client: {pause_asked}", flush=True)
                    (d / "batch-stop").write_text("")
                    pause_at = time.time()
            elif time.time() - pause_at > 120 and not (d / "batch-stopped").exists():
                outcome = "hang"
            if (d / "batch-stopped").exists():
                outcome = "recycle"
            elif (d / "batch-done").exists():
                outcome = "done"
            elif proc.poll() is not None and not client_pids():
                outcome = "crash"
            elif time.time() - last_progress > args.stall and time.time() - started > 60:
                outcome = "hang"
        st["active_seconds"] += time.time() - started
        st.pop("current_start", None)
        if outcome == "recycle":
            st["recycles"] = st.get("recycles", 0) + 1
            made = len(read_results(d)) - launch_done
            if made < 100 and "RSS" in (pause_asked or ""):
                # the cap is too close to what the client needs just to run: it would restart forever
                args.max_rss_gb *= 1.25
                print(f"only {made} jobs in that launch: cap raised to {args.max_rss_gb:.1f} GB", flush=True)
            stop_client(proc)
            save_state(d, st)
            continue
        if outcome == "done":
            # a graceful close, then make sure
            (GAME / "poe-hook.keys").write_text("close\n")
            time.sleep(5)
            stop_client(proc)
            save_state(d, st)
            continue
        st["crashes" if outcome == "crash" else "hangs"] += 1
        stop_client(proc)
        restarts += 1
        running = inflight(d)
        print(f"client {outcome}; jobs in flight: {running}", flush=True)
        with open(results, "a") as rf:
            for j in running:
                crashes[j] = crashes.get(j, 0) + 1
                # a job that crashed the client when it ran alone twice is skipped
                if crashes[j] >= CRASH_LIMIT:
                    rf.write("\t".join(j) + f"\tcrash-skip\t0\t0\t0\t0\t0\tclient {outcome} {crashes[j]} times\n")
        crash_file.write_text("".join("\t".join(j) + f"\t{n}\n" for j, n in crashes.items()))
        save_state(d, st)
    save_state(d, st)
    print("all jobs have a result")


# ---- status / verify ----------------------------------------------------------------------


def cmd_status(args):
    d = run_dir(args.run)
    jobs = read_jobs(d / "jobs.txt")

    def snapshot():
        res = read_results(d)
        return res, sum(1 for j in jobs if j in res)

    res, n = snapshot()
    if args.watch:
        t0 = time.time()
        time.sleep(args.watch)
        res, n2 = snapshot()
        window = (n2 - n) / (time.time() - t0)
        n = n2
    else:
        window = None
    st = load_state(d) if (d / "state.json").exists() else None
    status = {}
    ms = []
    for j in jobs:
        r = res.get(j)
        if r:
            status[r[4]] = status.get(r[4], 0) + 1
            if r[4] == "ok":
                ms.append(float(r[9]))
    left = len(jobs) - n
    print(f"run {args.run}: {len(jobs)} jobs, {n} done ({', '.join(f'{k} {v}' for k, v in sorted(status.items()))}), {left} remaining")
    if ms:
        ms.sort()
        print(f"per layout (worker thread time): mean {sum(ms)/len(ms):.1f} ms, median {ms[len(ms)//2]:.1f} ms, p95 {ms[int(len(ms)*.95)]:.1f} ms, max {ms[-1]:.0f} ms")
    rate = window
    if st:
        active = st["active_seconds"] + (time.time() - st["current_start"] if "current_start" in st else 0)
    if st and active > 0:
        overall = n / active
        print(f"overall {overall:.2f} jobs/s over {active/3600:.2f} h client time (jobs of all launches); launches {st['launches']}, crashes {st['crashes']}, hangs {st['hangs']}")
        rate = rate or overall
    if st and st.get("peak_rss_mb"):
        print(f"client memory: peak {st['peak_rss_mb']} MB RSS (rss.tsv has the samples); restarts for the memory cap or --restart-every: {st.get('recycles', 0)}")
    if rate:
        print(f"{'current' if window else 'overall'} rate {rate:.2f} jobs/s; ETA {left/rate/3600:.2f} h")


def cmd_stop(args):
    """Ends a running `run` loop (any run) and the client it started."""
    for p in Path("/proc").iterdir():
        if not p.name.isdigit() or int(p.name) == os.getpid():
            continue
        try:
            argv = (p / "cmdline").read_bytes().decode(errors="replace").split("\0")
        except OSError:
            continue
        if len(argv) > 2 and "python" in argv[0] and argv[1].endswith("terrain_batch.py") and argv[2] == "run":
            os.kill(int(p.name), signal.SIGTERM)
            print("stopped run loop", p.name)
    time.sleep(1)
    stop_client()


def cmd_verify(args):
    d = run_dir(args.run)
    res = read_results(d)
    bad = ok = missing = 0
    for area, seed, topo, orient, tile, doodad in known_vectors():
        r = res.get((area, str(seed), str(topo), str(orient)))
        if not r:
            missing += 1
            continue
        good = r[4] == "ok" and r[5] == tile and r[6] == doodad
        ok += good
        bad += not good
        print(("ok   " if good else "FAIL ") + f"{area} {seed} {topo}:{orient} client {tile}/{doodad} batch {r[5]}/{r[6]} ({r[4]} {r[10]})")
    print(f"{ok} equal, {bad} different, {missing} not run")
    sys.exit(1 if bad else 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("plan")
    p.add_argument("run")
    p.add_argument("--seeds", type=int, default=50)
    p.add_argument("--act")
    p.add_argument("--area")
    p.add_argument("--kind")
    p.add_argument("--master-seed", type=int, default=20261022)
    p.add_argument("--vectors", action="store_true", help="the known client vectors instead of random seeds")
    p.add_argument("--limit", type=int)
    p.set_defaults(fn=cmd_plan)
    p = sub.add_parser("run")
    p.add_argument("run")
    p.add_argument("--threads", type=int, default=DEFAULT_THREADS)
    p.add_argument("--max-rss-gb", type=float, default=16.0,
                   help="resident memory cap of the client: above it the jobs in flight finish, the client restarts and the run resumes")
    p.add_argument("--restart-every", type=int, default=0, help="also restart the client after this many jobs (0: only for the memory cap)")
    p.add_argument("--rss-interval", type=int, default=10, help="seconds between lines of rss.tsv")
    p.add_argument("--no-deploy", action="store_true")
    p.add_argument("--max-restarts", type=int, default=300)
    p.add_argument("--stall", type=int, default=240, help="seconds without a new result before the client counts as hung")
    p.set_defaults(fn=cmd_run)
    p = sub.add_parser("status")
    p.add_argument("run")
    p.add_argument("--watch", type=int, default=0)
    p.set_defaults(fn=cmd_status)
    p = sub.add_parser("stop")
    p.set_defaults(fn=cmd_stop)
    p = sub.add_parser("verify")
    p.add_argument("run")
    p.set_defaults(fn=cmd_verify)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
