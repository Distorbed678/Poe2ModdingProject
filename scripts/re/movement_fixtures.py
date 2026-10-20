#!/usr/bin/env python3
"""Builds the replay fixtures of `crates/poe-game/tests/fixtures/` from official captures (WP2.2).

A fixture is a short window of one capture, reduced to *movement* messages of the local player:
the client inputs and the server replies, as semantic events (the names of
`poe_game::movement::MoveEvent`), with frame numbers relative to the first input. No account
names, chat or other entities' messages are kept. Captures stay in `dumps/` (git-ignored); only
these derived files are committed.

Frame model (docs/protocol/movement.md): a message that arrives after `n` ticks is in frame `n`.
The server processes an input in the tick that emits the batch with the first reply, so an input
`arrives` one frame before its first reply: `arrive = first_reply_frame - 1`, never before the
previous input. The replay submits inputs at `arrive` and expects each reply at its own frame.

Usage:
  movement_fixtures.py show  <session> --from F --to F      print the window as fixture JSON
  movement_fixtures.py build [--out DIR]                    write all PRESETS (needs the dumps)

Needs `GameFiles/poe-hook-data/stats.tsv` (cargo xtask dll-data) to name stats; analysis only.

Notes (WP2.1b): `action_start` carries the 1c8 flags (the 0x1a5 flags plus 0x10 for a start an
input caused; a server-issued restart has the last request's flags without it). Sprint replies
of `0x63` are matched with the vector normalised to length 550 and truncated (and with the
older x 550/180).
"""

import argparse
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import movement_sequences as ms  # noqa: E402

OUT_DEFAULT = os.path.join(ms.ROOT, "crates", "poe-game", "tests", "fixtures")

NEW = "2026-10-20_18-41-14_336"

# name -> (session, first frame, last frame, description[, drop aim inputs[, options]]). Frames
# are the capture's own. Options: `shift` {capture frame of an input: frames to arrive later}.
PRESETS = {
    "wasd_start_turn_stop": (
        "2026-10-14_22-23-02_336", 1352, 1373,
        "WASD start, two turns and stop, with one aim input per frame echoed (all 16)"),
    "tap_roll_while_walking": (
        "2026-10-14_22-23-02_336", 1081, 1106,
        "walk, tap-roll (Space released 4 frames in), steering by 0x1a8, WASD during the roll, "
        "roll end at t0+16; aims during the roll are not echoed"),
    "tap_rolls_chained": (
        "2026-10-14_22-23-02_336", 1081, 1135,
        "two tap rolls, the second 30 frames after the first started"),
    "sprint_hold_release": (
        "2026-10-17_15-28-54_340", 1098, 1200,
        "roll with Space held: sprint at t0+15, roll end t0+30, must_sprint off t0+35, a 0x64 per "
        "frame from the first WASD vector after the sprint start, scaled; Space released"),
    "sprint_ended_by_attack": (
        "2026-10-14_22-23-02_336", 2340, 2382,
        "a short sprint ended by an attack in the same frame as the key release "
        "(aim inputs dropped: aims are mostly not echoed while an attack runs)", True),
    # ---- 2026-10-20 session (gap-targeted, WP2.1b) -------------------------------------------
    "standing_tap_roll": (
        NEW, 510, 550,
        "roll from standing (no WASD), Space released 7 frames after the press: the roll batch "
        "is the same as when walking, steering by 0x1a8, end at t0+16, no 0x64 at any time"),
    "standing_hold_sprint_cursor_follow": (
        NEW, 1128, 1175,
        "roll from standing with Space held: sprint at t0+15, then the client sends a 0x63 per "
        "frame towards the cursor (no WASD) and the server streams 0x64 scaled to length 550; "
        "roll end t0+30, must_sprint off t0+35"),
    "chained_rolls_during_a_roll": (
        NEW, 1568, 1650,
        "roll (press and release in one frame, after a pause), a roll asked for 9 frames later "
        "(tap, dropped), one at 28 frames (tap, starts at t0+30), one at 31 frames (starts at once)"),
    "walk_tap_roll_then_turn": (
        NEW, 2452, 2500,
        "WASD walk, tap roll (7 frames), WASD turn at t0+15 answered at once, stop; the 0x64 "
        "after the roll"),
    "sprint_wasd_released_then_space_up": (
        NEW, 1720, 1800,
        "roll with Space held while walking; WASD vectors during and after the roll; WASD keys "
        "released while Space is still down (the client then sends the cursor direction, not "
        "(0, 0)); Space released, 0x64 scaled then unscaled, (0, 0) after"),
    "attack_during_a_tap_roll": (
        NEW, 1930, 1998,
        "tap roll followed by an attack asked for 11 frames in: acked at once, the attack "
        "starts at t0+24, the earlier acks of the repeated attack carry no start",
        True),
    "attack_waits_for_sprint_end": (
        NEW, 4018, 4060,
        "roll with Space held; an attack asked for during the roll is acked at once and starts "
        "with the end of the sprint at t0+35 (must_sprint off, MoveAck, StartAction, sprint "
        "off); cursor-follow 0x63 vectors in between",
        True),
    "click_single": (
        NEW, 3372, 3382,
        "click-to-move: press and release 2 frames later: StartAction Move + UseSkillAck in the "
        "next batch, SkillReleasedAck one batch after the release"),
    "click_held_reaim": (
        NEW, 3322, 3372,
        "held click for 41 frames with the cursor sweeping: a UseSkill repeat (target flags "
        "0x10) every 2 frames, each acked and answered with a StartAction Move; a periodic "
        "StartAction every 15 frames; SkillReleasedAck on release",
        True),
    "held_click_with_rolls": (
        NEW, 3525, 3700,
        "mouse mode: a click held for 4 s; a roll with Space released at t0+19 (sprint cut short, "
        "its end at t0+35), then a tap roll; repeats are acked during the rolls and the walk "
        "resumes with a StartAction at the end of the roll",
        True),
    "stance_to_mouse_mode": (
        NEW, 3284, 3290,
        "input mode toggled to click-to-move: AimMode(0) and the stance request in one frame, "
        "stance 0 one frame later", False, {"stance": True}),
    "stance_to_combat_mode": (
        NEW, 4660, 4666,
        "input mode toggled back to WASD: AimMode(1) and the stance request in one frame, "
        "stance 1 one frame later", False, {"stance": False}),
}


def stat_name(k):
    return ms.STAT_NAMES.get(k, f"stat_{k}")


def struct_f32(u):
    return struct.unpack(">f", struct.pack(">I", u))[0]


def to_input(m):
    d = m.d
    if m.op == 0x63:
        return {"op": "wasd", "x": d["x"], "y": d["y"]}
    if m.op == 0x1A5:
        o = {"op": "use_skill", "skill": d["skill"], "flags": d["flags"], "unk": d["u13"],
             "target_flags": d["tf"], "x": d["x"], "y": d["y"]}
        if "target" in d:
            o["entity"] = d["target"]
        return o
    if m.op == 0x1A8:
        return {"op": "update_target", "x": d["x"], "y": d["y"], "skill": d["skill"]}
    if m.op == 0x1AC:
        return {"op": "input_end"}
    if m.op == 0x2E6:
        return {"op": "sprint", "held": bool(d["held"])}
    if m.op == 0x1AD:
        return {"op": "released", "skill": d["skill"], "flag": d["u"]}
    if m.op == 0x62 and d["mode"] == 2:
        return {"op": "aim", "x": d["x"], "y": d["y"]}
    if m.op == 0x62 and d["mode"] in (0, 1):
        return {"op": "aim_mode", "mode": d["mode"]}
    return None


def to_event(m, buff_base):
    d = m.d
    if m.op == 0x64:
        return {"ev": "move_ack", "dir": [d["dx"], d["dy"]], "cell": [d["gx"], d["gy"]]}
    if m.op == 0x1C8:
        o = {"ev": "action_start", "skill": d["skill"], "flags": d["flags"],
             "cell": [d["gx"], d["gy"]], "target": [d["tx"], d["ty"]]}
        return o
    if m.op == 0x1AE:
        o = {"ev": "skill_ack", "skill": d["skill"], "flags": d["flags"], "unk": d["u13"],
             "target_flags": d["tf"] & ~4, "x": d["x"], "y": d["y"]}
        if "target" in d:
            o["entity"] = d["target"]
        return o
    if m.op == 0x1AF:
        return {"ev": "target_ack", "skill": d["skill"], "x": d["x"], "y": d["y"]}
    if m.op == 0x1B1:
        return {"ev": "input_end_ack"}
    if m.op == 0x1B2:
        return {"ev": "released_ack", "skill": d["skill"], "flag": d["flag"]}
    if m.op == 0x2F9:
        if d.get("comp") != 6:
            return None
        b = bytes.fromhex(d["delta"])
        mask = b[0] | b[1] << 8
        if not mask & 0x0400:
            return None
        return {"ev": "stance", "combat": bool(b[2])}
    if m.op == 0x1CA:
        return {"ev": "action_event"}
    if m.op == 0x1D3:
        return {"ev": "stats", "stats": [[stat_name(k), v] for k, v in d["stats"]]}
    if m.op == 0x1EB:
        o = {"ev": "buff_added", "index": d["index"] - buff_base, "row": d["buff"]}
        if d["flags"] & 2:
            o["fade"] = round(struct_f32(d["f2"]), 3)
        return o
    if m.op == 0x1EC:
        return {"ev": "buff_removed", "index": d["index"] - buff_base}
    if m.op == 0x2E7:
        return {"ev": "sprint", "held": bool(d["held"])}
    if m.op == 0x1DC:
        return {"ev": "aim_echo", "kind": d["kind"], "x": d["x"], "y": d["y"]}
    return None


PAIR = {0x63: (0x64,), 0x1A5: (0x1AE,), 0x1A8: (0x1AF,), 0x1AC: (0x1B1,), 0x2E6: (0x2E7,),
        0x62: (0x1DC,), 0x1AD: (0x1B2,)}
REPLY_OPS = (0x64, 0x1C8, 0x1AE, 0x1AF, 0x1B1, 0x1B2, 0x1CA, 0x1D3, 0x1EB, 0x1EC, 0x2E7, 0x1DC,
             0x2F9)


def window(session, lo, hi, name="", desc="", drop_aim=False, opts=None):
    opts = opts or {}
    shift = opts.get("shift", {})
    msgs = ms.load(ms.resolve(session))
    pe = ms.player_entity(msgs)
    ins = [m for m in msgs if m.dir == "out" and lo <= m.frame <= hi and to_input(m)
           and not (drop_aim and m.op == 0x62)]
    outs = [m for m in msgs if m.dir == "in" and lo <= m.frame <= hi + 3 and m.d
            and m.d.get("ent") == pe and m.op in REPLY_OPS and not (drop_aim and m.op == 0x1DC)]
    used, arrive, prev = set(), {}, lo
    for m in ins:
        kinds = PAIR.get(m.op, ())
        if m.op == 0x62 and m.d["mode"] != 2:
            kinds = (0x2F9,)
        want = (m.d.get("x"), m.d.get("y"))
        r = None
        for q in outs:
            if q.i <= m.i or q.i in used or q.op not in kinds or q.frame > m.frame + 3:
                continue
            if m.op == 0x63:
                ln = math.hypot(*want) or 1.0
                sc = (int(want[0] * 550 / 180), int(want[1] * 550 / 180))  # the older rule
                nm = (int(want[0] * 550 / ln), int(want[1] * 550 / ln))  # normalised, truncated
                if (q.d["dx"], q.d["dy"]) not in (want, sc, nm):
                    continue
            if m.op == 0x62 and m.d["mode"] == 2 and (q.d["x"], q.d["y"]) != want:
                continue
            if m.op == 0x62 and m.d["mode"] != 2 and q.op != 0x2F9:
                continue
            r = q
            break
        if r is not None:
            used.add(r.i)
            a = max(prev, r.frame - 1)
        else:
            a = max(prev, m.frame)
        a += shift.get(m.frame, 0)
        arrive[m.i] = a
        prev = a
    base = arrive[ins[0].i] if ins else lo
    buffs = [q.d["index"] for q in outs if q.op == 0x1EB]
    buff_base = min(buffs) if buffs else 0
    inputs = [dict(f=arrive[m.i] - base, **to_input(m)) for m in ins]
    events, first_cell = [], None
    for q in outs:
        e = to_event(q, buff_base)
        if e is None or q.frame - base < 1 or q.frame > hi:
            continue
        events.append(dict(f=q.frame - base, **e))
        if first_cell is None and "cell" in e:
            first_cell = e["cell"]
    extra = {"combat_stance": opts["stance"]} if "stance" in opts else {}
    return {
        **extra,
        "name": name,
        "description": desc,
        "source": os.path.basename(os.path.dirname(ms.resolve(session))),
        "capture_frames": [lo, hi],
        "start_cell": first_cell or [50, 50],
        "inputs": inputs,
        "expect": events,
    }


def cmd_show(a):
    print(json.dumps(window(a.session, a.start, a.end), indent=None, separators=(",", ":")).replace("},{", "},\n{"))


def cmd_build(a):
    os.makedirs(a.out, exist_ok=True)
    for name, (session, lo, hi, desc, *opts) in PRESETS.items():
        w = window(session, lo, hi, name, desc, drop_aim=bool(opts and opts[0]),
                   opts=opts[1] if len(opts) > 1 else None)
        path = os.path.join(a.out, name + ".json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(w, f, separators=(",", ":"))
            f.write("\n")
        print(f"{path}: {len(w['inputs'])} inputs, {len(w['expect'])} events")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("show")
    p.add_argument("session")
    p.add_argument("--from", dest="start", type=int, required=True)
    p.add_argument("--to", dest="end", type=int, required=True)
    p.set_defaults(fn=cmd_show)
    p = sub.add_parser("build")
    p.add_argument("--out", default=OUT_DEFAULT)
    p.set_defaults(fn=cmd_build)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
