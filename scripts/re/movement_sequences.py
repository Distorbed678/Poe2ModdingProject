#!/usr/bin/env python3
"""Movement message sequences from official captures, with frame timing (WP2.1).

Reads `packets.jsonl` written by the client DLL (official or local sessions) and prints the
movement-related messages decoded, with the *frame* each lands in. Frames are counted from the
server's `0x02f4` SimulationTick: a message that arrives after `n` ticks belongs to frame `n`
(server simulation messages arrive before the tick that closes their frame). A client message
sent after `n` ticks is "input in frame n"; the server's earliest possible reply is applied by
the client in frame n+1 or later, so the interesting number is `reply_frame - input_frame`.

Usage:
  movement_sequences.py list   [session ...]            summary of opcode counts per session
  movement_sequences.py events <session> [--from N] [--to N] [--all] [--aim]
                                                         decoded timeline (message index range)
  movement_sequences.py scen   [session ...] [--max N] [--scenario NAME ...]
                                                         per-scenario sequences (see SCENARIOS)
  movement_sequences.py never  [session ...]            S->C opcodes that are never sent for movement

`session` is a folder name under dumps/0.5.5e.hotfix1/captures/official/ (or local/ with --local), a path
to a session folder, or a path to a packets.jsonl. Without sessions, all official ones are used.
Analysis only: nothing here connects to anything.

Scenarios (detected from the C->S inputs; see docs/protocol/movement.md):
  wasd        0x63 runs: start, turn, stop
  click       0x1a5 Move (skill 0x20002909): single click and held click (repeats with target flags 0x10)
  roll        0x1a5 with a roll skill (small id or 0x80000040...): standing, moving, chained
  sprint      0x2e6 held=1 ... held=0 and what follows
  stance      0x2f9 ComponentDelta on the player (combat stance)
  aim         0x62 AimTarget -> 0x1dc SetAimTarget
"""

import argparse
import collections
import glob
import json
import os
import struct
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.dirname(__file__))
from version import current_version  # noqa: E402

CAPS = os.path.join(ROOT, "dumps", current_version(), "captures")

TICK = 0x02F4
MOVE_SKILL = 0x20002909

C2S_NAMES = {
    0x0062: "AimTarget", 0x0063: "MoveDirection", 0x01A5: "UseSkill", 0x01A8: "UpdateSkillTarget",
    0x01AA: "UpdateSkillTargetEntity", 0x01AC: "SkillInputEnd", 0x01AD: "SkillReleased",
    0x02E6: "SprintInput", 0x0200: "StanceRequest", 0x0063: "MoveDirection",
}
S2C_NAMES = {
    0x0064: "MoveDirectionUpdate", 0x01C8: "StartAction", 0x01CB: "UpdateActionTarget",
    0x01DF: "SetOrientation", 0x01DC: "SetAimTarget", 0x01AE: "UseSkillAck",
    0x01AF: "UpdateSkillTargetAck", 0x01B0: "UpdateSkillTargetEntityAck", 0x01B1: "SkillInputEndAck",
    0x01B2: "SkillReleasedAck", 0x01CA: "ActionEvent1ca", 0x01EB: "BuffAdded", 0x01EC: "BuffRemoved",
    0x02E7: "SprintState",
    0x01D3: "StatsUpdate", 0x01D8: "SkillStatsUpdate", 0x02F9: "ComponentDelta",
    0x02A6: "OrientationLerp",
}


class R:
    def __init__(self, b):
        self.b, self.p = b, 0

    def left(self):
        return len(self.b) - self.p

    def take(self, n):
        if self.p + n > len(self.b):
            raise ValueError("short")
        v = self.b[self.p:self.p + n]
        self.p += n
        return v

    def u8(self): return self.take(1)[0]
    def u16(self): return struct.unpack(">H", self.take(2))[0]
    def u32(self): return struct.unpack(">I", self.take(4))[0]
    def i32(self): return struct.unpack(">i", self.take(4))[0]

    def varuint(self):
        first = self.u8()
        extra = 0
        while extra < 8 and first & (0x80 >> extra):
            extra += 1
        if extra >= 4:
            return self.u32()
        v = first & (0x7F >> extra)
        for c in self.take(extra):
            v = (v << 8) | c
        return v

    def varint(self):
        first = self.u8()
        extra = 0
        while extra < 8 and first & (0x80 >> extra):
            extra += 1
        if extra >= 4:
            return self.i32()
        v = first & (0x7F >> extra)
        for c in self.take(extra):
            v = (v << 8) | c
        bits = 7 + 7 * extra
        if v & (1 << (bits - 1)):
            v -= 1 << bits
        return v


def hdr(r):
    return dict(ent=r.u32(), u14=r.u32(), u18=r.u8(), u1a=r.u16())


def params(r):
    d = dict(skill=r.u32(), flags=r.u16(), u13=r.u8(), tf=r.u8())
    if d["tf"] & 8:
        d["target"] = r.u32()
    d["x"], d["y"] = r.i32(), r.i32()
    return d


def decode(m):
    """Decode one capture record's body (after the opcode) into a dict, or None."""
    op = int(m["op"], 16)
    r = R(bytes.fromhex(m["hex"])[2:])
    try:
        if m["dir"] == "out":
            if op == 0x63:
                return dict(u0=r.u8(), x=r.varint(), y=r.varint(), tail=r.take(3).hex())
            if op == 0x1A5:
                d = dict(player=r.u8())
                d.update(params(r))
                return d
            if op == 0x1A8:
                return dict(player=r.u8(), x=r.i32(), y=r.i32(), skill=r.u32())
            if op == 0x1AA:
                return dict(player=r.u8(), entity=r.u32(), skill=r.u32())
            if op == 0x1AC:
                return dict(player=r.u8())
            if op == 0x1AD:
                return dict(player=r.u8(), skill=r.u32(), u=r.u8())
            if op == 0x2E6:
                return dict(player=r.u8(), held=r.u8())
            if op == 0x200:
                return dict(player=r.u8(), value=r.u8())
            if op == 0x62:
                d = dict(player=r.u8(), mode=r.u8())
                if d["mode"] == 2:
                    d["x"], d["y"] = r.i32(), r.i32()
                elif d["mode"] == 3:
                    d["entity"] = r.u32()
                return d
            return None
        d = hdr(r)
        if op == 0x64:
            d.update(dx=r.varint(), dy=r.varint(), gx=r.varint(), gy=r.varint(), last=r.u8())
        elif op == 0x1C8:
            fl = r.u16()
            d["flags"] = fl & ~0x80
            if fl & 0x80:
                d["target"] = r.u32()
            d.update(gx=r.varint(), gy=r.varint(), tx=r.varint(), ty=r.varint(),
                     skill=r.u32(), action=r.u16(), last=r.u8())
        elif op == 0x1AE:
            d.update(params(r))
        elif op == 0x1AF:
            d.update(x=r.i32(), y=r.i32(), skill=r.u32())
        elif op in (0x1B1, 0x1CA):
            pass
        elif op == 0x1B0:
            d.update(entity=r.u32(), x=r.i32(), y=r.i32(), skill=r.u32())
        elif op == 0x1B2:
            d.update(skill=r.u32(), flag=r.u8())
        elif op == 0x1EC:
            d["index"] = r.u16()
        elif op == 0x1EB:
            # BuffAdded: index, buff row, u32, u16, flags; optional parts by flags (partial decode)
            d.update(index=r.u16(), buff=r.u16(), u64=r.u32(), u60=r.u16(), flags=r.u16())
            fl = d["flags"]
            if fl & 1:
                d["f1"] = (r.varuint(), r.u32(), r.u8(), r.u16())
            if fl & 2:
                d["f2"] = r.u32()
            if fl & 0x80:
                d["f80"] = r.u32()
            if fl & 4:
                d["f4"] = r.u16()
            if fl & 8:
                d["f8"] = (r.u16(), r.u32(), r.u8())
            if fl & 0x20:
                d["f20"] = r.u8()
            d["count"] = r.u32()
            d["vec"] = [r.u32() for _ in range(d["count"])]
        elif op == 0x1CB:
            d.update(action=r.u16(), x=r.i32(), y=r.i32())
        elif op == 0x1DC:
            d.update(kind=r.u8(), x=r.i32(), y=r.i32())
        elif op == 0x2E7:
            d["held"] = r.u8()
        elif op == 0x1D3:
            d["set"] = r.u8()
            n = r.varuint()
            d["stats"] = [(r.varuint(), r.varint()) for _ in range(n)]
        elif op == 0x1D8:
            d["skill"] = r.u32()
            d["container"] = r.u8()
            n = r.varuint()
            d["stats"] = [(r.varuint(), r.varint()) for _ in range(n)]
        elif op == 0x2A6:
            d["f32"] = struct.unpack(">f", r.take(4))[0]
            d["u8"] = r.u8()
        elif op == 0x2F9:
            n = r.varuint()
            blob = r.take(n)
            d["comp"], d["delta"] = blob[0], blob[1:].hex()
        else:
            return None
        if r.left():
            d["_extra"] = r.take(r.left()).hex()
        return d
    except ValueError:
        return {"_short": True}


def resolve(session, local=False):
    if os.path.isfile(session):
        return session
    if os.path.isdir(session):
        return os.path.join(session, "packets.jsonl")
    base = os.path.join(CAPS, "local" if local else "official")
    return os.path.join(base, session, "packets.jsonl")


def official_sessions():
    return sorted(glob.glob(os.path.join(CAPS, "official", "*", "packets.jsonl")))


class Msg:
    __slots__ = ("i", "t", "dir", "op", "name", "frame", "d", "hex")

    def __repr__(self):
        return f"<{self.t} f{self.frame} {self.dir} {self.op:#x}>"


def load(path):
    """Instance-channel messages with frame numbers (ticks received so far) and decoded bodies."""
    out, ticks, t0 = [], 0, None
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = json.loads(line)
            if m["chan"] != "instance":
                continue
            x = Msg()
            x.i = len(out)
            x.t = m["t"]
            if t0 is None:
                t0 = x.t
            x.dir, x.op, x.name, x.hex = m["dir"], int(m["op"], 16), m["name"], m["hex"]
            x.frame = ticks
            x.d = decode(m)
            if x.op == TICK and x.dir == "in":
                ticks += 1
            out.append(x)
    return out


def _load_names(fname):
    """key -> name from GameFiles/poe-hook-data (written by `cargo xtask dll-data`), if present."""
    out = {}
    try:
        with open(os.path.join(ROOT, "GameFiles", "poe-hook-data", fname), encoding="utf-8") as f:
            for line in f:
                if line.startswith("#") or "\t" not in line:
                    continue
                k, v = line.rstrip("\n").split("\t")[:2]
                out[int(k)] = v
    except OSError:
        pass
    return out


STAT_NAMES = _load_names("stats.tsv")
ACTION_NAMES = _load_names("action_types.tsv")
MAX_STATS = 6


def skill_name(skill):
    if skill & 0xE0000000 == 0x20000000:
        return ACTION_NAMES.get(skill & 0xFFFF, "?")
    return None


def fmt_stats(stats):
    named = [f"{STAT_NAMES.get(k, k)}={v}" for k, v in stats[:MAX_STATS]]
    more = f" +{len(stats) - MAX_STATS} more" if len(stats) > MAX_STATS else ""
    return "[" + ", ".join(named) + more + "]"


def fmt(m):
    nm = (C2S_NAMES if m.dir == "out" else S2C_NAMES).get(m.op, m.name or "?")
    arrow = "C>S" if m.dir == "out" else "S>C"
    d = dict(m.d or {})
    if "stats" in d:
        d["stats"] = fmt_stats(d["stats"])
    if "skill" in d and skill_name(d["skill"]):
        d["skill"] = f"{d['skill']:#x}({skill_name(d['skill'])})"
    for k in ("ent", "u14"):
        if d.get(k) == 0:
            d.pop(k)
    if d.get("u18") == 0:
        d.pop("u18")
    if d.get("u1a") == 0:
        d.pop("u1a")
    body = " ".join(f"{k}={v:#x}" if k == "skill" and isinstance(v, int) else f"{k}={v}" for k, v in d.items())
    return f"{arrow} {m.op:#06x} {nm:<20} {body}"


MOVE_OPS_OUT = {0x63, 0x1A5, 0x1A8, 0x1AA, 0x1AC, 0x1AD, 0x2E6, 0x200}
MOVE_OPS_IN = {0x64, 0x1C8, 0x1CB, 0x1DF, 0x1AE, 0x1AF, 0x1B0, 0x1B1, 0x1B2, 0x1CA, 0x1EB, 0x1EC,
               0x2E7, 0x1D3, 0x1D8, 0x2F9, 0x2A6}
AIM_OPS = {0x62, 0x1DC}


def interesting(m, aim=False, everything=False):
    if everything:
        return True
    if m.dir == "out":
        return m.op in MOVE_OPS_OUT or (aim and m.op in AIM_OPS)
    return m.op in MOVE_OPS_IN or (aim and m.op in AIM_OPS)


def cmd_events(a):
    msgs = load(resolve(a.session, a.local))
    lo, hi = a.start, a.end or len(msgs)
    t0 = msgs[lo].t if lo < len(msgs) else 0
    for m in msgs[lo:hi]:
        if m.op == TICK and not a.all:
            continue
        if not interesting(m, a.aim, a.all):
            continue
        print(f"{m.i:6d} +{m.t - t0:6d}ms f{m.frame:5d}  {fmt(m)}")


def cmd_list(a):
    sessions = a.sessions or official_sessions()
    for s in sessions:
        msgs = load(resolve(s, a.local))
        c = collections.Counter((m.dir, m.op) for m in msgs)
        print(os.path.basename(os.path.dirname(resolve(s, a.local))), f"{len(msgs)} instance msgs,",
              f"{c[('in', TICK)]} ticks")
        for (dr, op), n in sorted(c.items()):
            if (dr == "out" and op in MOVE_OPS_OUT | AIM_OPS) or (dr == "in" and op in MOVE_OPS_IN | AIM_OPS):
                nm = (C2S_NAMES if dr == "out" else S2C_NAMES).get(op, "?")
                print(f"    {dr:3} {op:#06x} {nm:<22} {n}")


# ---------------------------------------------------------------- scenarios
# Facts these detectors rely on (docs/protocol/movement.md):
#   dodge roll skill (official)  0x80000040        attack skill 0x80400040      Move 0x20002909
#   stats: is_dodge_rolling 19305, is_sprinting 22342, must_sprint (see stats.tsv)

ROLL_SKILL = 0x80000040
SPRINT_SCALE = 550 / 180  # walking vectors have length ~180, sprinting replies ~550
K_ROLLING, K_SPRINTING = 19305, 22342


def player_entity(msgs):
    """The local player's entity id: the one the server acknowledges skill uses for."""
    c = collections.Counter(m.d["ent"] for m in msgs if m.dir == "in" and m.op == 0x1AE and m.d)
    return c.most_common(1)[0][0] if c else None


def is_move(m):
    return m.dir == "out" and m.op == 0x1A5 and m.d.get("skill") == MOVE_SKILL


def replies_after(msgs, i, span):
    """S>C movement-related messages in frames [f, f+span] after the input at index i."""
    f = msgs[i].frame
    out = []
    for m in msgs[i + 1:]:
        if m.frame > f + span:
            break
        if m.dir == "in" and m.op in MOVE_OPS_IN:
            out.append(m)
    return out


def show_replies(msgs, i, span, limit=14, indent="      ", only_ent=None):
    """Replies relative to the input frame (f+k) and to the first reply batch (r+k)."""
    f = msgs[i].frame
    rs = [m for m in replies_after(msgs, i, span) if only_ent is None or m.d.get("ent") in (only_ent, None, 0)]
    base = rs[0].frame if rs else f
    for n, m in enumerate(rs):
        if n >= limit:
            print(f"{indent}...")
            break
        print(f"{indent}f+{m.frame - f} r+{m.frame - base} (+{m.t - msgs[i].t}ms) {fmt(m)}")


def scen_wasd(msgs, a):
    """0x63 runs; every 0x63 is answered by exactly one 0x64 with the same vector."""
    pe = player_entity(msgs)
    outs = [m for m in msgs if m.dir == "out" and m.op == 0x63]
    pins = [m for m in msgs if m.dir == "in" and m.op == 0x64 and m.d["ent"] == pe]
    used, offs, lat, miss, nscaled = set(), collections.Counter(), [], 0, 0
    for o in outs:
        want = (o.d["x"], o.d["y"])
        scaled = (round(want[0] * SPRINT_SCALE), round(want[1] * SPRINT_SCALE))
        k = next((q for q in pins if q.i > o.i and q.i not in used
                  and (q.d["dx"], q.d["dy"]) in (want, scaled) and q.frame - o.frame < 6), None)
        if k is None:
            miss += 1
            continue
        used.add(k.i)
        offs[k.frame - o.frame] += 1
        if (k.d["dx"], k.d["dy"]) == scaled and want != (0, 0):
            nscaled += 1
        lat.append(k.t - o.t)
    extra = [q for q in pins if q.i not in used]
    # extras are the per-frame sprint stream (see `sprint`), not answers to 0x63
    lat.sort()
    print(f"  {len(outs)} x 0x63 -> matched 0x64 (same vector) {sum(offs.values())}; unmatched inputs {miss}; "
          f"0x64 not answering an input (sprint stream etc.) {len(extra)}; answered with the vector x{SPRINT_SCALE:.3f} (sprinting): {nscaled}")
    print(f"  reply frame offset (frames between input and its 0x64): {dict(sorted(offs.items()))}; "
          f"latency ms median {lat[len(lat) // 2] if lat else '-'}")
    vec = collections.Counter((m.d["x"], m.d["y"]) for m in outs)
    print(f"  0x63 vectors: {dict(vec.most_common(10))}")
    print(f"  0x63 tail bytes: {dict(collections.Counter(m.d['tail'] for m in outs))}; u0: {dict(collections.Counter(m.d['u0'] for m in outs))}")
    walk = [q for q in pins if max(abs(q.d['dx']), abs(q.d['dy'])) in (127, 180)]
    print(f"  0x64 flag byte (walk replies): {dict(collections.Counter(q.d['last'] for q in walk))}")
    shown = 0
    # start / turn / stop examples
    runs, cur = [], None
    for m in outs:
        moving = (m.d["x"], m.d["y"]) != (0, 0)
        if cur is None and moving:
            cur = [m]
        elif cur is not None:
            cur.append(m)
            if not moving:
                runs.append(cur)
                cur = None
    print(f"  {len(runs)} runs (first moving 0x63 .. stop (0,0))")
    for run in runs[:a.max]:
        print(f"  run of {len(run) - 1} direction inputs over {(run[-1].t - run[0].t)} ms; start, first turn, stop:")
        for label, m in (("start", run[0]), ("turn", run[1] if len(run) > 2 else None), ("stop", run[-1])):
            if m is None:
                continue
            print(f"    {label:5} in f{m.frame} {fmt(m)}")
            r = next((q for q in replies_after(msgs, m.i, 5) if q.op == 0x64 and (q.d['dx'], q.d['dy']) == (m.d['x'], m.d['y'])), None)
            if r:
                print(f"          reply f+{r.frame - m.frame} (+{r.t - m.t}ms) {fmt(r)}")


def scen_click(msgs, a):
    """Move-skill clicks. Official sessions are WASD only: the only Move click is the intro's."""
    clicks = [m for m in msgs if is_move(m)]
    held = [m for m in clicks if m.d["tf"] & 0x10]
    print(f"  Move (0x20002909) UseSkill: {len(clicks)}, held repeats (tf bit 0x10): {len(held)}")
    for m in clicks[:a.max]:
        print(f"    in f{m.frame} {fmt(m)}")
        show_replies(msgs, m.i, 4, 12, "        ")
        rel = next((x for x in msgs[m.i + 1:m.i + 40] if x.dir == "out" and x.op == 0x1AD), None)
        if rel:
            print(f"    in f{rel.frame} {fmt(rel)}")
            show_replies(msgs, rel.i, 4, 6, "        ")
    real = [m for m in clicks if not m.d["tf"] & 0x10 and any(x.dir == "in" and x.op == 0x1C8 and x.d["skill"] == MOVE_SKILL
                                      for x in msgs[m.i:m.i + 60])]
    if clicks and not real:
        print("  NOTE: this click is the intro wake-up (reply StartAction skill 0x2000b7d9, not a walk); "
              "this session has no real click-to-move.")
    elif real:
        print(f"  {len(real)} presses are real click-to-move (reply StartAction Move 0x20002909), plus "
              f"{len(held)} held repeats. Use `events <session> --from F --to F` for a window.")


def scen_roll(msgs, a):
    """Per roll: timeline relative to the roll's StartAction frame (t0) and classification."""
    pe = player_entity(msgs)
    starts = [m for m in msgs if m.dir == "in" and m.op == 0x1C8 and m.d["skill"] == ROLL_SKILL and m.d["ent"] == pe]
    prev_t0 = None
    for s in starts[:a.max]:
        inp = next((x for x in reversed(msgs[max(0, s.i - 300):s.i])
                    if x.dir == "out" and x.op == 0x1A5 and x.d["skill"] == ROLL_SKILL and not x.d["tf"] & 0x10), None)
        down = next((x for x in reversed(msgs[max(0, s.i - 300):s.i]) if x.dir == "out" and x.op == 0x2E6 and x.d["held"] == 1), None)
        up = next((x for x in msgs[(down.i if down else s.i):] if x.dir == "out" and x.op == 0x2E6 and x.d["held"] == 0), None)
        w = next((x for x in reversed(msgs[:(inp.i if inp else s.i)]) if x.dir == "out" and x.op == 0x63), None)
        wasd = "standing" if w is None or (w.d["x"], w.d["y"]) == (0, 0) else "moving"
        held_f = (up.frame - down.frame) if up and down else None
        kind = "hold(sprint)" if held_f is not None and held_f >= 16 else "tap"
        print(f"  roll t0=f{s.frame}: {wasd}, key {kind} held {held_f}f, input f{inp.frame if inp else '?'} "
              f"rel=({inp.d['x']},{inp.d['y']}) -> StartAction {s.d['gx'],s.d['gy']}->{s.d['tx'],s.d['ty']} "
              f"action={s.d['action']} flags={s.d['flags']:#x}; since previous roll: "
              f"{s.frame - prev_t0 if prev_t0 else '-'}f")
        prev_t0 = s.frame
        if a.verbose:
            show_replies(msgs, inp.i, 3, 20, "      ")
        ev = []
        for x in msgs[s.i:s.i + 800]:
            if x.frame - s.frame > 130:
                break
            if x.dir == "in" and x.op == 0x1D3 and x.d["ent"] == pe:
                names = [STAT_NAMES.get(k, str(k)) for k, v in x.d["stats"]]
                v = dict(x.d["stats"])
                if K_ROLLING in v:
                    ev.append((x.frame - s.frame, f"is_dodge_rolling={v[K_ROLLING]}"))
                if K_SPRINTING in v:
                    ev.append((x.frame - s.frame, f"is_sprinting={v[K_SPRINTING]}"))
                if "base_move_speed_+%_from_sprint" in names:
                    ev.append((x.frame - s.frame, f"sprint speed stats ={v.get(next(k for k in v if STAT_NAMES.get(k) == 'base_move_speed_+%_from_sprint'))}"))
                if names == ["must_sprint"]:
                    ev.append((x.frame - s.frame, "must_sprint=0"))
            elif x.dir == "in" and x.op in (0x2E7, 0x1B1, 0x1EB, 0x1EC) and (x.d or {}).get("ent") in (pe, None):
                nm = {0x2E7: f"SprintState held={(x.d or {}).get('held')}", 0x1B1: "SkillInputEndAck",
                      0x1EB: "BuffAdded", 0x1EC: "BuffRemoved"}[x.op]
                ev.append((x.frame - s.frame, nm))
            if x.dir == "out" and x.op in (0x2E6, 0x1AC):
                ev.append((x.frame - s.frame, f"C>S {C2S_NAMES[x.op]}{' held=%d' % x.d['held'] if x.op == 0x2E6 else ''}"))
            if x.dir == "in" and x.op == 0x1C8 and x.d["ent"] == pe and x is not s:
                ev.append((x.frame - s.frame, f"StartAction skill={x.d['skill']:#x}"))
        print("      timeline (frames after t0): " + "; ".join(f"+{f} {t}" for f, t in ev[:24]))


def scen_sprint(msgs, a):
    """Sprint = roll with the key held; then a 0x64 every frame with a 550-long vector."""
    pe = player_entity(msgs)
    on = [m for m in msgs if m.dir == "in" and m.op == 0x1D3 and m.d["ent"] == pe and dict(m.d["stats"]).get(K_SPRINTING) == 1]
    off = [m for m in msgs if m.dir == "in" and m.op == 0x1D3 and m.d["ent"] == pe and dict(m.d["stats"]).get(K_SPRINTING) == 0]
    print(f"  sprints started (is_sprinting=1): {len(on)}, ended: {len(off)}")
    for s in on[:a.max]:
        e = next((x for x in off if x.i > s.i), None)
        end_f = e.frame if e else s.frame + 200
        stream = [x for x in msgs[s.i:] if x.dir == "in" and x.op == 0x64 and x.d["ent"] == pe and s.frame <= x.frame <= end_f]
        per = collections.Counter(x.frame for x in stream)
        gaps = [f for f in range(s.frame, end_f + 1) if per.get(f, 0) == 0]
        mags = collections.Counter(round((x.d["dx"] ** 2 + x.d["dy"] ** 2) ** 0.5) for x in stream if (x.d["dx"], x.d["dy"]) != (0, 0))
        zero = [x.frame - s.frame for x in stream if not x.d["last"]]
        print(f"  sprint on f{s.frame}, off f{e.frame if e else None}: 0x64 stream {len(stream)} msgs in "
              f"{end_f - s.frame + 1} frames (frames without one: {len(gaps)}); vector lengths {dict(mags.most_common(4))}; "
              f"flag byte 0 at +{zero[:8]}")
        sp = [(x.d['gx'], x.d['gy']) for x in stream]
        if len(sp) > 10:
            import math
            d = [math.hypot(b[0] - c[0], b[1] - c[1]) for b, c in zip(sp[5:], sp[4:-1])]
            print(f"      mean step {sum(d) / len(d):.2f} cells/frame (walk 1.23 expected, sprint 1.85)")
        # sequences at start and end
        print("      start (relative to is_sprinting=1):")
        for x in msgs[s.i - 3:s.i + 4]:
            if x.dir == "in" and x.op in (0x1EB, 0x1D3, 0x64) and x.frame - s.frame <= 1 and x.d.get("ent") in (pe, None):
                print(f"        f+{x.frame - s.frame} {fmt(x)[:170]}")
        if e:
            print("      end:")
            for x in msgs[e.i - 12:e.i + 10]:
                if x.dir == "in" and x.op in (0x2E7, 0x1B1, 0x1EC, 0x1D3, 0x1CA, 0x64) and abs(x.frame - e.frame) <= 1 and (x.d or {}).get("ent") in (pe, None, 0):
                    print(f"        f{x.frame - e.frame:+d} {fmt(x)[:170]}")
    # sprint ended by an attack
    ends = collections.Counter()
    for e in off:
        why = "key up"
        for x in reversed(msgs[max(0, e.i - 60):e.i]):
            if x.dir == "out" and x.op == 0x2E6 and x.d["held"] == 0:
                break
            if x.dir == "out" and x.op == 0x1A5 and x.d["skill"] == 0x80400040:
                why = "attack skill used"
                break
        ends[why] += 1
    print(f"  sprint end causes: {dict(ends)}")


def scen_stance(msgs, a):
    """0x02f9 ComponentDelta on the player's Player component (component 6)."""
    pe = player_entity(msgs)
    last_in = None
    for m in msgs:
        if m.dir == "out" and m.op in (0x63, 0x1A5, 0x2E6, 0x62, 0x1AD):
            last_in = m
        if m.dir == "in" and m.op == 0x2F9 and m.d.get("ent") == pe and m.d.get("comp") == 6:
            b = bytes.fromhex(m.d["delta"])
            mask = b[0] | b[1] << 8
            if mask & 0x0400 or not a.all_deltas:
                if not mask & 0x0400:
                    continue
            li = f"{C2S_NAMES[last_in.op]}(f-{m.frame - last_in.frame})" if last_in else "-"
            print(f"  f{m.frame}: mask={mask:#06x} tail={b[2:].hex()[:24]} (stance {'bit' if mask & 0x0400 else 'no stance'}) after {li}")


def scen_aim(msgs, a):
    """0x62 AimTarget -> 0x1dc SetAimTarget; suppressed while rolling/sprinting/attacking."""
    pe = player_entity(msgs)
    roll = spr = False
    atk = set()
    for m in msgs:
        if m.dir == "in" and m.op == 0x1C8 and m.d["ent"] == pe and m.d["skill"] not in (ROLL_SKILL, MOVE_SKILL, 0x2000B7D9):
            atk.update(range(m.frame, m.frame + 25))
    state = {}
    for m in msgs:
        if m.dir == "in" and m.op == 0x1D3 and m.d["ent"] == pe:
            v = dict(m.d["stats"])
            roll = bool(v[K_ROLLING]) if K_ROLLING in v else roll
            spr = bool(v[K_SPRINTING]) if K_SPRINTING in v else spr
        state[m.frame] = (roll, spr)
    inp = collections.defaultdict(list)
    out = collections.defaultdict(list)
    modes = collections.Counter()
    for m in msgs:
        if m.dir == "out" and m.op == 0x62:
            modes[m.d["mode"]] += 1
            if m.d["mode"] == 2:
                inp[m.frame].append((m.d["x"], m.d["y"]))
        if m.dir == "in" and m.op == 0x1DC:
            out[m.frame].append((m.d["x"], m.d["y"], m.d["kind"]))
    tot, miss, offs = collections.Counter(), collections.Counter(), collections.Counter()
    for f, v in inp.items():
        r, s = state.get(f, (False, False))
        k = "roll" if r else "sprint" if s else "attack" if f in atk else "idle/walk"
        tot[k] += 1
        hit = [(g, o) for g in (f, f + 1, f + 2, f + 3) for o in out.get(g, []) if o[:2] == v[-1]]
        if hit:
            offs[hit[0][0] - f] += 1
        else:
            miss[k] += 1
    print(f"  0x62 by mode: {dict(modes)} (mode 2 = absolute cell, 3 = entity, 0/1 once at entry)")
    print(f"  mode-2 inputs: one per frame ({dict(collections.Counter(len(v) for v in inp.values()))} per frame); "
          f"echoed by 0x1dc kind 0 with the same cell at frame offset {dict(sorted(offs.items()))}")
    print(f"  not echoed / total by state: " + ", ".join(f"{k} {miss[k]}/{tot[k]}" for k in tot))
    kinds = collections.Counter(o[2] for v in out.values() for o in v)
    print(f"  0x1dc kinds: {dict(kinds)}; 0x1dc header: ent=player, rest 0")


SCENARIOS = {
    "wasd": scen_wasd, "click": scen_click, "roll": scen_roll, "sprint": scen_sprint,
    "stance": scen_stance, "aim": scen_aim,
}


def cmd_scen(a):
    sessions = a.sessions or official_sessions()
    names = a.scenario or list(SCENARIOS)
    for s in sessions:
        path = resolve(s, a.local)
        msgs = load(path)
        print(f"== {os.path.basename(os.path.dirname(path))}: {len(msgs)} msgs")
        for n in names:
            print(f" [{n}]")
            SCENARIOS[n](msgs, a)


def cmd_never(a):
    sessions = a.sessions or official_sessions()
    seen = collections.Counter()
    for s in sessions:
        for m in load(resolve(s, a.local)):
            if m.dir == "in":
                seen[m.op] += 1
    for op in (0x1CB, 0x1DF, 0x64, 0x1C8, 0x1AE, 0x1AF, 0x1B1, 0x2E7, 0x1D3, 0x1D8, 0x2F9, 0x1DC):
        print(f"  S>C {op:#06x} {S2C_NAMES.get(op, '?'):<20} {seen[op]} total over {len(sessions)} sessions")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("list")
    p.add_argument("sessions", nargs="*")
    p.add_argument("--local", action="store_true")
    p.set_defaults(fn=cmd_list)
    p = sub.add_parser("events")
    p.add_argument("session")
    p.add_argument("--from", dest="start", type=int, default=0)
    p.add_argument("--to", dest="end", type=int, default=0)
    p.add_argument("--all", action="store_true")
    p.add_argument("--aim", action="store_true")
    p.add_argument("--local", action="store_true")
    p.set_defaults(fn=cmd_events)
    p = sub.add_parser("scen")
    p.add_argument("sessions", nargs="*")
    p.add_argument("--scenario", nargs="*", choices=list(SCENARIOS))
    p.add_argument("--max", type=int, default=6)
    p.add_argument("--rep", type=int, default=4, help="held-click repeats to print")
    p.add_argument("--span", type=int, default=6, help="frames of replies to print after a roll")
    p.add_argument("--limit", type=int, default=16)
    p.add_argument("--verbose", action="store_true", help="roll: print the reply batches too")
    p.add_argument("--all-deltas", action="store_true", help="stance: also non-stance Player deltas")
    p.add_argument("--local", action="store_true")
    p.set_defaults(fn=cmd_scen)
    p = sub.add_parser("never")
    p.add_argument("sessions", nargs="*")
    p.add_argument("--local", action="store_true")
    p.set_defaults(fn=cmd_never)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
