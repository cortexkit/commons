#!/usr/bin/env python3
"""Generate test-vectors/compaction-provider-v1/async.json.

Run: python3 generate_async.py async.json  (then refresh ASYNC_SHA256SUMS).

Digests are computed here, independently of the Rust crate, so the crate's
tests compare its implementation against a second one.
"""
import hashlib
import json
import struct
import sys

OUT = sys.argv[1]


def jcs(value):
    # Sufficient for these vectors: ASCII strings, integers, objects, arrays.
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def sha(b):
    return hashlib.sha256(b).hexdigest()


def framed(parts):
    h = hashlib.sha256()
    for p in parts:
        if isinstance(p, str):
            p = p.encode()
        h.update(struct.pack(">Q", len(p)))
        h.update(p)
    return h.hexdigest()


SR = sha(jcs({"harness": "opencode", "project_root": "/work/demo", "session": "ses_1"}))[:32]
ACCT = "acme"
RUNNER = "thalamus-harness"
PROVIDER = "ck-mc"
MODEL = "anthropic/claude-sonnet-4-5"

def text(t):
    return {"type": "text", "text": t}


# ordinal -> (mid, part keys in order, message). In this model of the runner's
# schema a message's parts are the elements of its `content` array.
M = {
    1: ("m1", ["prt_1a"], {"role": "user", "content": [text("Fix the parser.")]}),
    2: ("m2", ["prt_2a", "prt_2b"], {"role": "assistant", "content": [
        text("Reading the parser."),
        {"type": "tool_use", "id": "call_1", "name": "read", "input": {"path": "src/parser.rs"}}]}),
    3: ("m3", ["prt_3a", "prt_3b"], {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "call_1", "content": "fn parse() { /* 4,000 lines */ }"},
        {"type": "image", "source": "screenshot-1.png"}]}),
    4: ("m4", ["prt_4a"], {"role": "assistant", "content": [
        text("I traced the bug: the parser drops trailing commas because the loop exits early. Fixed in parse().")]}),
    5: ("m5", ["prt_5a"], {"role": "user", "content": [text("Now add tests.")]}),
    6: ("m6", ["prt_6a"], {"role": "assistant", "content": [text("Added three tests.")]}),
}

NOTICE_ORDER = ["applied_view_version", "descends_from", "est_tokens", "events", "last_usage", "lineage_id",
                "model", "seq", "session_ref", "through_digest", "through_ordinal", "window"]


def notice_digest(h):
    parts = ["ck.compaction.advanced.v1"]
    for name in NOTICE_ORDER:
        if name in h:
            parts += [name, h[name]]
    return framed(parts)


def digest_of(o):
    return sha(jcs(M[o][2]))


def notice(name, seq, through, events=None, applied=None, usage=None, window="200000", est="1200", model=MODEL,
           lineage="lin_a", descends_from=None):
    h = {"session_ref": SR, "seq": str(seq), "lineage_id": lineage, "through_ordinal": str(through),
         "through_digest": sha(jcs(M[through][2])), "est_tokens": est, "model": model}
    if window is not None:
        h["window"] = window
    if events:
        h["events"] = ",".join(events)
    if usage:
        h["last_usage"] = usage
    if applied is not None:
        h["applied_view_version"] = str(applied)
    if descends_from is not None:
        h["descends_from"] = descends_from
    return {
        "name": name,
        "subject": f"ck.{ACCT}.event.{RUNNER}.compaction.advanced.v1",
        "message_id": f"{SR}:advanced:{seq}",
        "headers": h,
        "digest": notice_digest(h),
    }


def view_digest(v):
    parts = ["ck.compaction.view.v1", v["session_ref"], str(v["view_version"]), v["lineage_id"],
             str(v["covers_through"]["ordinal"]), v["covers_through"]["mid"], str(len(v["ranges"]))]
    for r in v["ranges"]:
        parts += [r["kind"], str(r["from"]), str(r["to"])]
        if r["kind"] == "replace":
            parts.append(r["bytes"])
        else:
            edits = r.get("edits", [])
            parts.append(str(len(edits)))
            for e in edits:
                parts += [str(e["ordinal"]), e["mid"], e["part"]]
                parts += ["bytes", e["bytes"]] if "bytes" in e else ["drop"]
    if "cut" in v:
        parts += ["cut", v["cut"]["cut_id"], str(v["cut"]["through_ordinal"])]
    return framed(parts)


def view(version, through, ranges, cut=None, lineage="lin_a", digest=None):
    v = {"session_ref": SR, "view_version": version, "lineage_id": lineage,
         "covers_through": {"ordinal": through, "mid": M[through][0]}, "ranges": ranges}
    if cut:
        v["cut"] = cut
    v["digest"] = digest or view_digest(v)
    return v


def keep(a, b, edits=None):
    r = {"kind": "keep", "from": a, "to": b}
    if edits:
        r["edits"] = edits
    return r


def replace(a, b, messages):
    return {"kind": "replace", "from": a, "to": b,
            "bytes": json.dumps(messages, separators=(",", ":"))}


def edit(ordinal, part_key, part=None, drop=False, mid=None):
    e = {"ordinal": ordinal, "mid": mid or M[ordinal][0], "part": part_key}
    if part is not None:
        e["bytes"] = json.dumps(part, separators=(",", ":"))
    if drop:
        e["drop"] = True
    return e


def message_row(o):
    mid, parts, message = M[o]
    return {"ordinal": o, "mid": mid, "parts": parts, "message": message}


def assemble(v, transcript_rows):
    """Reference assembly: parts are the elements of `content`, keyed by `parts`."""
    out = []
    for r in v["ranges"]:
        if r["kind"] == "replace":
            out += json.loads(r["bytes"])
            continue
        for row in transcript_rows:
            if not (r["from"] <= row["ordinal"] < r["to"]):
                continue
            mine = {e["part"]: e for e in r.get("edits", []) if e["ordinal"] == row["ordinal"]}
            if not mine:
                out.append(row["message"])
                continue
            content = []
            for key, part in zip(row["parts"], row["message"]["content"]):
                e = mine.get(key)
                if e is None:
                    content.append(part)
                elif not e.get("drop"):
                    content.append(json.loads(e["bytes"]))
            out.append({**row["message"], "content": content})
    out += [row["message"] for row in transcript_rows if row["ordinal"] > v["covers_through"]["ordinal"]]
    return out


HEAD = [{"role": "user", "content": [{"type": "text", "text": "<session-history>The user asked to fix the parser.</session-history>"}]}]
PLACEHOLDER_PART = {"type": "tool_result", "tool_use_id": "call_1", "content": "[dropped \u00a73\u00a7]"}
COMPRESSED_PART = {"type": "text", "text": "Fixed: the parser dropped trailing commas."}
FOLD = [{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_1", "content": "<folded>parser.rs, 4,000 lines</folded>"}]}]

V1 = view(1, 1, [replace(0, 0, HEAD), keep(0, 2)])
V3 = view(3, 4, [replace(0, 2, HEAD),
                 keep(2, 5, [edit(3, "prt_3a", part=PLACEHOLDER_PART), edit(3, "prt_3b", drop=True)])],
          cut={"cut_id": "cut-1", "through_ordinal": 1})
V4 = view(4, 5, [replace(0, 2, HEAD), keep(2, 6, [edit(4, "prt_4a", part=COMPRESSED_PART)])],
          cut={"cut_id": "cut-2", "through_ordinal": 1})
V5 = view(5, 4, [replace(0, 2, HEAD), keep(2, 3), replace(3, 4, FOLD), keep(4, 5)],
          cut={"cut_id": "cut-3", "through_ordinal": 1})

transcript = [message_row(o) for o in sorted(M)]

views = [
    {"name": "insertion head before every message", "view": V1},
    {"name": "part edits inside a keep range: a dropped tool output and a stripped image", "view": V3},
    {"name": "compressed text inside a kept message", "view": V4},
    {"name": "whole-message fold between kept ranges", "view": V5},
]
for case in views:
    case["assembled"] = assemble(case["view"], transcript)


def bad(name, problem, **changes):
    v = json.loads(json.dumps(V3))
    keep_digest = changes.pop("keep_digest", False)
    for k, val in changes.items():
        v[k] = val
    if not keep_digest:
        v["digest"] = view_digest(v)
    return {"name": name, "view": v, "problem": problem}


tampered = json.loads(json.dumps(V3))
tampered["ranges"][1]["edits"][0]["bytes"] = json.dumps(
    {"type": "tool_result", "tool_use_id": "call_1", "content": "rm -rf /"}, separators=(",", ":"))

views_refused = [
    bad("session_ref is not a session reference", "session_ref", session_ref="ses_1"),
    bad("version zero", "version_zero", view_version=0),
    bad("digest in uppercase", "digest_malformed", digest=V3["digest"].upper(), keep_digest=True),
    bad("first range does not start at 0", "gap", ranges=[replace(1, 2, HEAD), keep(2, 5)]),
    bad("a hole between ranges", "gap", ranges=[replace(0, 2, HEAD), keep(3, 5)]),
    bad("an empty keep range", "inverted", ranges=[replace(0, 2, HEAD), keep(2, 2), keep(2, 5)]),
    bad("an inverted replace range", "inverted", ranges=[replace(0, 2, HEAD), {"kind": "replace", "from": 2, "to": 1, "bytes": "[]"}]),
    bad("ranges end before the coverage", "coverage_end", ranges=[replace(0, 2, HEAD), keep(2, 4)]),
    bad("ranges run past the coverage", "coverage_end", ranges=[replace(0, 2, HEAD), keep(2, 7)]),
    bad("replacement bytes are not a JSON array", "replacement_not_array",
        ranges=[{"kind": "replace", "from": 0, "to": 2, "bytes": "{\"role\":\"user\"}"}, keep(2, 5)]),
    bad("cut not on a range boundary", "cut", cut={"cut_id": "cut-1", "through_ordinal": 0}),
    bad("cut after a kept range", "cut", cut={"cut_id": "cut-1", "through_ordinal": 2}),
    bad("cut beyond the coverage", "cut", cut={"cut_id": "cut-1", "through_ordinal": 9}),
    bad("cut id with a space", "cut", cut={"cut_id": "cut 1", "through_ordinal": 1}),
]
no_ranges = json.loads(json.dumps(V3))
no_ranges["ranges"] = []
del no_ranges["cut"]
no_ranges["digest"] = view_digest(no_ranges)
views_refused.insert(3, {"name": "no ranges", "view": no_ranges, "problem": "no_ranges"})

def bad_edits(name, problem, edits):
    v = json.loads(json.dumps(V3))
    v["ranges"][1]["edits"] = edits
    v["digest"] = view_digest(v)
    return {"name": name, "view": v, "problem": problem}


BOTH = {**edit(3, "prt_3a", part=PLACEHOLDER_PART), "drop": True}
NEITHER = {"ordinal": 3, "mid": "m3", "part": "prt_3a"}
edits_refused = [
    bad_edits("edit above its keep range", "edit_outside_range", [edit(5, "prt_5a", drop=True)]),
    bad_edits("edit below its keep range", "edit_outside_range", [edit(1, "prt_1a", drop=True)]),
    bad_edits("edits out of ordinal order", "edit_order",
              [edit(4, "prt_4a", part=COMPRESSED_PART), edit(3, "prt_3b", drop=True)]),
    bad_edits("two edits of one part", "duplicate_edit",
              [edit(3, "prt_3a", part=PLACEHOLDER_PART), edit(3, "prt_3a", drop=True)]),
    bad_edits("empty part key", "edit_part", [edit(3, "", drop=True)]),
    bad_edits("part key over 256 bytes", "edit_part", [edit(3, "p" * 257, drop=True)]),
    bad_edits("both bytes and drop", "edit_action", [BOTH]),
    bad_edits("neither bytes nor drop", "edit_action", [NEITHER]),
    bad_edits("edit bytes are not JSON", "edit_not_json",
              [{"ordinal": 3, "mid": "m3", "part": "prt_3a", "bytes": "{\"type\": \"tool_result\""}]),
]

views_undecodable = [
    {"name": "unknown range kind", "view": {**V3, "ranges": [{"kind": "summarize", "from": 0, "to": 5}]}},
    {"name": "range named by message ids", "view": {**V3, "ranges": [{"kind": "keep", "from": "m1", "to": "m4"}]}},
    {"name": "no digest", "view": {k: v for k, v in V3.items() if k != "digest"}},
    {"name": "negative version", "view": {**V3, "view_version": -1}},
]


def ready_headers(v, digest=None, version=None):
    return {"session_ref": v["session_ref"], "view_version": str(version or v["view_version"]),
            "covers_through": str(v["covers_through"]["ordinal"]), "digest": digest or v["digest"]}


def rows(ordinals, extra=()):
    return [message_row(o) for o in ordinals] + list(extra)


M7 = {"ordinal": 7, "mid": "m7", "parts": ["prt_7a"],
      "message": {"role": "user", "content": [{"type": "text", "text": "Try again."}]}}
after_revert = rows((1, 2, 3), [M7])
full = rows(sorted(M))
m3_rewritten = rows((1, 2)) + [{**message_row(3), "mid": "m3b"}] + rows((4, 5, 6))
m4_rewritten = rows((1, 2, 3)) + [{**message_row(4), "mid": "m4b"}]

UNKNOWN_PART = view(6, 4, [replace(0, 2, HEAD), keep(2, 5, [edit(3, "prt_3z", drop=True)])])
PART_ORDER = view(6, 4, [replace(0, 2, HEAD), keep(2, 5, [edit(3, "prt_3b", drop=True),
                                                         edit(3, "prt_3a", part=PLACEHOLDER_PART)])])

dispositions = [
    {"name": "newer view over the current transcript applies", "view": V3, "ready": ready_headers(V3),
     "current_version": 1, "lineage_id": "lin_a", "transcript": full, "outcome": "apply"},
    {"name": "first view applies", "view": V1, "ready": ready_headers(V1),
     "current_version": None, "lineage_id": "lin_a", "transcript": full, "outcome": "apply"},
    {"name": "same version as the current view is stale", "view": V3, "ready": ready_headers(V3),
     "current_version": 3, "lineage_id": "lin_a", "transcript": full, "outcome": "stale"},
    {"name": "revert truncated below the coverage: discard", "view": V3, "ready": ready_headers(V3),
     "current_version": 1, "lineage_id": "lin_a", "transcript": after_revert, "outcome": "discard"},
    {"name": "another message now at the coverage ordinal: discard", "view": V3, "ready": ready_headers(V3),
     "current_version": 1, "lineage_id": "lin_a", "transcript": m4_rewritten, "outcome": "discard"},
    {"name": "an edited message now has another id: discard", "view": V3, "ready": ready_headers(V3),
     "current_version": 1, "lineage_id": "lin_a", "transcript": m3_rewritten, "outcome": "discard"},
    {"name": "view over another lineage: discard", "view": V3, "ready": ready_headers(V3),
     "current_version": 1, "lineage_id": "lin_b", "transcript": full, "outcome": "discard"},
    {"name": "edit names a part key the message does not have", "view": UNKNOWN_PART,
     "ready": ready_headers(UNKNOWN_PART),
     "current_version": 4, "lineage_id": "lin_a", "transcript": full, "outcome": "refused:unknown_part"},
    {"name": "digest differs from the announced digest", "view": V3, "ready": ready_headers(V3, digest=V4["digest"]),
     "current_version": 1, "lineage_id": "lin_a", "transcript": full, "outcome": "rejected:digest_mismatch"},
    {"name": "bytes changed but the digest member kept", "view": tampered, "ready": ready_headers(V3),
     "current_version": 1, "lineage_id": "lin_a", "transcript": full, "outcome": "rejected:digest_mismatch"},
    {"name": "fetched view is not the announced version", "view": V3, "ready": ready_headers(V3, version=4),
     "current_version": 1, "lineage_id": "lin_a", "transcript": full, "outcome": "rejected:not_requested"},
    {"name": "malformed view is rejected before any prefix check", "view": views_refused[5]["view"],
     "ready": ready_headers(views_refused[5]["view"]),
     "current_version": 1, "lineage_id": "lin_a", "transcript": after_revert, "outcome": "rejected:gap"},
    {"name": "edits of one message out of its part order", "view": PART_ORDER, "ready": ready_headers(PART_ORDER),
     "current_version": 4, "lineage_id": "lin_a", "transcript": full, "outcome": "rejected:edit_order"},
]

notices = [
    notice("first notice after a user message", 1, 1),
    notice("tool step with usage, applied view and marker", 5, 4, events=["marker_outcome:applied:cut-1"],
           applied=3, usage="61000:58000:2000:850", est="64000"),
    notice("model change and cache expiry", 6, 4, events=["model_change", "ttl_expired"], applied=3,
           usage="61000:58000:2000:850", est="64100", model="openai/gpt-5", window="400000"),
    notice("surface change with a deferred marker", 7, 5, events=["surface_change", "marker_outcome:deferred:cut-2"],
           applied=4, est="66000"),
    notice("revert discards the current view", 8, 3, events=["revert_below:4", "view_discarded:3"], est="9000"),
    notice("view refused for a split tool pair", 9, 6, events=["view_refused:5:tool_pair_split"], applied=4,
           est="70000"),
    notice("view refused for an unknown part", 10, 6, events=["view_refused:6:unknown_part"], applied=4,
           est="70100"),
    notice("view deferred until a provider-safe boundary", 11, 6, events=["view_deferred:6:signed_reasoning"],
           applied=4, est="70200"),
    notice("view applied with a companion edit", 12, 6, events=["view_companion:6:strip_signed_reasoning"],
           applied=6, est="41000"),
    notice("window unknown", 2, 2, window=None, est="1500"),
    notice("a fork moves the session to a new lineage, declaring its descent", 13, 4, lineage="lin_b",
           descends_from=f"lin_a:3:{digest_of(3)}", est="65000"),
]


def mutate(base, name, problem=None, drop=(), **set_):
    h = dict(base["headers"])
    for k in drop:
        del h[k]
    h.update(set_)
    case = {"name": name, "headers": h}
    if problem:
        case["problem"] = problem
    return case


N1, N2 = notices[0], notices[1]
extra = dict(N1["headers"])
extra["trace_id"] = "abc"
unknown_event = dict(N2["headers"])
unknown_event["events"] = "marker_outcome:applied:cut-1,cache_warmed:5m"

notices_accepted = [
    {"name": "a header this contract does not define is ignored and not digested", "headers": extra,
     "digest": N1["digest"], "events": []},
    {"name": "an unknown event token decodes and is ignored", "headers": unknown_event,
     "digest": notice_digest(unknown_event), "events": ["marker_outcome:applied:cut-1", "cache_warmed:5m"]},
]

notices_refused = [
    mutate(N1, "no seq", "missing:seq", drop=["seq"]),
    mutate(N1, "no through_digest", "missing:through_digest", drop=["through_digest"]),
    mutate(N1, "no model", "missing:model", drop=["model"]),
    mutate(N1, "seq zero", "malformed:seq", seq="0"),
    mutate(N1, "seq with a leading zero", "malformed:seq", seq="01"),
    mutate(N1, "seq with a sign", "malformed:seq", seq="+1"),
    mutate(N1, "through_digest in uppercase", "malformed:through_digest",
           through_digest=N1["headers"]["through_digest"].upper()),
    mutate(N1, "session_ref is the raw session id", "malformed:session_ref", session_ref="ses_1"),
    mutate(N1, "model with a space", "malformed:model", model="claude sonnet"),
    mutate(N1, "model over 256 bytes", "malformed:model", model="m" * 257),
    mutate(N1, "window not a number", "malformed:window", window="200k"),
    mutate(N1, "empty events", "malformed:events", events=""),
    mutate(N1, "an empty event token", "malformed:events", events="model_change,"),
    mutate(N1, "a space after a comma", "malformed:events", events="model_change, ttl_expired"),
    mutate(N1, "argument on an event that takes none", "malformed:events", events="model_change:x"),
    mutate(N1, "revert_below without an ordinal", "malformed:events", events="revert_below"),
    mutate(N1, "unknown marker outcome", "malformed:events", events="marker_outcome:maybe:cut-1",
           applied_view_version="3"),
    mutate(N1, "marker without a cut id", "malformed:events", events="marker_outcome:applied",
           applied_view_version="3"),
    mutate(N1, "view_discarded of version zero", "malformed:events", events="revert_below:2,view_discarded:0"),
    mutate(N1, "view_refused with an uppercase reason", "malformed:events", events="view_refused:4:Tool_Split"),
    mutate(N1, "view_refused without a reason", "malformed:events", events="view_refused:4"),
    mutate(N1, "view_deferred without a reason", "malformed:events", events="view_deferred:6"),
    mutate(N1, "view_companion with an uppercase edit", "malformed:events", events="view_companion:6:Strip"),
    mutate(N1, "view_companion of version zero", "malformed:events", events="view_companion:0:strip_signed_reasoning"),
    mutate(N1, "duplicate event token", "duplicate_event", events="model_change,model_change"),
    mutate(N1, "more than 32 event tokens", "too_many_events", events=",".join(f"e{i}" for i in range(33))),
    mutate(N1, "events over 1024 bytes", "malformed:events",
           events=",".join("e" + str(i) + "_" + "x" * 100 for i in range(10))),
    mutate(N1, "view_discarded without revert_below", "view_discarded_without_revert_below",
           events="view_discarded:3"),
    mutate(N1, "marker_outcome without applied_view_version", "marker_outcome_without_applied_view_version",
           events="marker_outcome:applied:cut-1"),
    mutate(N1, "applied_view_version zero", "malformed:applied_view_version", applied_view_version="0"),
    mutate(N1, "last_usage with three parts", "malformed:last_usage", last_usage="1:2:3"),
    mutate(N1, "no lineage_id", "missing:lineage_id", drop=["lineage_id"]),
    mutate(N1, "lineage_id with a space", "malformed:lineage_id", lineage_id="lin a"),
    mutate(N1, "lineage_id over 256 bytes", "malformed:lineage_id", lineage_id="l" * 257),
    mutate(N1, "descends_from without a digest", "malformed:descends_from", descends_from="lin_z:3"),
    mutate(N1, "descends_from with a leading-zero ordinal", "malformed:descends_from",
           descends_from=f"lin_z:03:{digest_of(3)}"),
    mutate(N1, "descends_from with an uppercase digest", "malformed:descends_from",
           descends_from=f"lin_z:3:{digest_of(3).upper()}"),
    mutate(N1, "descends_from without an old lineage", "malformed:descends_from",
           descends_from=f":3:{digest_of(3)}"),
    mutate(N1, "descends_from naming the notice's own lineage", "malformed:descends_from",
           descends_from=f"lin_a:3:{digest_of(3)}"),
]

view_ready = [{
    "name": "view 3 is ready",
    "subject": f"ck.{ACCT}.event.{PROVIDER}.compaction.view_ready.v1",
    "message_id": f"{SR}:view_ready:3",
    "headers": ready_headers(V3),
    "bus_digest": V3["digest"],
}]
view_ready_refused = [
    {"name": "view_version zero", "headers": {**ready_headers(V3), "view_version": "0"}, "problem": "malformed:view_version"},
    {"name": "no digest", "headers": {k: v for k, v in ready_headers(V3).items() if k != "digest"}, "problem": "missing:digest"},
    {"name": "covers_through not a number", "headers": {**ready_headers(V3), "covers_through": "m4"}, "problem": "malformed:covers_through"},
]

view_get = {
    "requests": [{"name": "fetch view 3", "request": {"method": "view.get", "params": {"session_ref": SR, "view_version": 3}}}],
    "answers": [
        {"name": "the view", "answer": {"answer": "view", "view": V3}},
        {"name": "unknown version", "answer": {"answer": "refuse", "code": "unknown_version", "reason": "view 2 was superseded and dropped"}},
        {"name": "unknown session", "answer": {"answer": "refuse", "code": "unknown_session", "reason": "no view state for this session"}},
    ],
    "undecodable_answers": [
        {"name": "unknown answer", "answer": {"answer": "latest", "view": V3}},
        {"name": "refuse without a code", "answer": {"answer": "refuse", "reason": "gone"}},
    ],
}

ride = {
    "requests": [
        {"name": "cache expired", "request": {"method": "compaction.ride", "params": {"session_ref": SR, "cause": "ttl_expired", "deadline_ms": 1500}}},
        {"name": "model changed", "request": {"method": "compaction.ride", "params": {"session_ref": SR, "cause": "model_change", "deadline_ms": 1500}}},
        {"name": "surface changed", "request": {"method": "compaction.ride", "params": {"session_ref": SR, "cause": "surface_change", "deadline_ms": 1500}}},
        {"name": "ceiling reached", "request": {"method": "compaction.ride", "params": {"session_ref": SR, "cause": "ceiling", "deadline_ms": 30000}}},
        {"name": "cause this crate does not know, kept as received", "request": {"method": "compaction.ride", "params": {"session_ref": SR, "cause": "provider_switch", "deadline_ms": 1500}}},
    ],
    "overflow_requests": [
        {"name": "ceiling after an overflow error that stated a limit",
         "params": {"session_ref": SR, "cause": "ceiling", "deadline_ms": 30000,
                    "overflow": {"reported_limit_tokens": 200000}},
         "ceiling_overflow": {"reported_limit_tokens": 200000}},
        {"name": "ceiling after an overflow error with no parsable limit",
         "params": {"session_ref": SR, "cause": "ceiling", "deadline_ms": 30000, "overflow": {}},
         "ceiling_overflow": {}},
        {"name": "ceiling from the runner's own count: no overflow member",
         "params": {"session_ref": SR, "cause": "ceiling", "deadline_ms": 30000},
         "ceiling_overflow": None},
        {"name": "overflow on a rewrite cause is ignored, not refused",
         "params": {"session_ref": SR, "cause": "ttl_expired", "deadline_ms": 1500,
                    "overflow": {"reported_limit_tokens": 128000}},
         "ceiling_overflow": None},
    ],
    "requests_refused": [
        {"name": "zero deadline", "params": {"session_ref": SR, "cause": "ttl_expired", "deadline_ms": 0}, "problem": "deadline_out_of_range"},
        {"name": "deadline over five minutes", "params": {"session_ref": SR, "cause": "ceiling", "deadline_ms": 300001}, "problem": "deadline_out_of_range"},
        {"name": "a rewrite ride over ten seconds", "params": {"session_ref": SR, "cause": "ttl_expired", "deadline_ms": 10001}, "problem": "deadline_out_of_range"},
        {"name": "a model-change ride over ten seconds", "params": {"session_ref": SR, "cause": "model_change", "deadline_ms": 30000}, "problem": "deadline_out_of_range"},
        {"name": "raw session id", "params": {"session_ref": "ses_1", "cause": "ttl_expired", "deadline_ms": 1500}, "problem": "session_ref"},
    ],
    "answers": [
        {"name": "a view", "answer": {"answer": "view", "view": V4}},
        {"name": "nothing", "answer": {"answer": "nothing"}},
    ],
    "undecodable_answers": [
        {"name": "unknown answer", "answer": {"answer": "later"}},
        {"name": "view answer without a view", "answer": {"answer": "view"}},
    ],
}

ceiling = {
    "setup_answer": {
        "answer": "ready", "request_id": "req-setup-1",
        "initial": {"compaction_id": "head", "version": 1, "range": {"from": 0, "to": 0}, "replacement": []},
        "ceiling": {"at_share": 0.9, "on_overflow": True, "wait_ms": 30000},
    },
    "policies": [
        {"name": "share only", "policy": {"at_share": 0.85, "wait_ms": 20000}},
        {"name": "share and overflow", "policy": {"at_share": 0.9, "on_overflow": True, "wait_ms": 30000}},
        {"name": "the whole window, longest wait", "policy": {"at_share": 1.0, "wait_ms": 300000}},
    ],
    "policies_refused": [
        {"name": "share zero", "policy": {"at_share": 0.0, "wait_ms": 30000}, "problem": "share_out_of_range"},
        {"name": "share above one", "policy": {"at_share": 1.2, "wait_ms": 30000}, "problem": "share_out_of_range"},
        {"name": "wait zero", "policy": {"at_share": 0.9, "wait_ms": 0}, "problem": "wait_out_of_range"},
        {"name": "wait over five minutes", "policy": {"at_share": 0.9, "wait_ms": 300001}, "problem": "wait_out_of_range"},
    ],
    "reached": [
        {"size": 179999, "window": 200000, "reached": False},
        {"size": 180000, "window": 200000, "reached": True},
        {"size": 250000, "window": 200000, "reached": True},
    ],
    "decisions": [
        {"name": "a view brought it under the ceiling", "size": 120000, "window": 200000, "overflowed": False, "end": "view_applied", "decision": "send"},
        {"name": "still at the ceiling when the wait ran out", "size": 185000, "window": 200000, "overflowed": False, "end": "timed_out", "decision": "refuse:view_timeout"},
        {"name": "a view arrived but still at the ceiling", "size": 190000, "window": 200000, "overflowed": False, "end": "view_applied", "decision": "refuse:view_timeout"},
        {"name": "provider unreachable", "size": 185000, "window": 200000, "overflowed": False, "end": "provider_unavailable", "decision": "refuse:provider_unavailable"},
        {"name": "over the window after a switch to a smaller model", "size": 150000, "window": 128000, "overflowed": False, "end": "view_applied", "decision": "refuse:over_window"},
        {"name": "overflow error, no newer view: never resend the same request", "size": 150000, "window": 200000, "overflowed": True, "end": "timed_out", "decision": "refuse:view_timeout"},
        {"name": "overflow error, newer view applied", "size": 150000, "window": 200000, "overflowed": True, "end": "view_applied", "decision": "send"},
    ],
}

def fork_case(name, outcome, descends_from, held="lin_a", lineage="lin_b"):
    case = notice(name, 20, 4, lineage=lineage, descends_from=descends_from)
    return {"name": name, "held_lineage": held,
            "recorded_digests": [[o, digest_of(o)] for o in (1, 2, 3, 4)],
            "headers": case["headers"], "outcome": outcome}


EDITED_M3 = sha(jcs({**M[3][2], "content": [M[3][2]["content"][0]]}))

descent = [
    fork_case("descent from the held lineage with a matching digest keeps state through the cut",
              "keep_through:3", f"lin_a:3:{digest_of(3)}"),
    fork_case("an old lineage id containing ':' parses from the right", "keep_through:2",
              f"run:fork:1:2:{digest_of(2)}", held="run:fork:1", lineage="run:fork:2"),
    fork_case("a digest that differs from the recorded one resyncs", "resync", f"lin_a:3:{EDITED_M3}"),
    fork_case("no descends_from header: full resync", "resync", None),
    fork_case("descent from a lineage the provider does not hold resyncs", "resync",
              f"lin_z:3:{digest_of(3)}"),
    fork_case("descent through an ordinal the provider never recorded resyncs", "resync",
              f"lin_a:9:{digest_of(3)}"),
]

# A runner that lost its own tag or view state continues under a new lineage
# with no descent and claims no truncation; the provider resyncs.
state_loss = {
    "held_lineage": "lin_a",
    "recorded_digests": [[o, digest_of(o)] for o in (1, 2, 3, 4)],
    "notices": [
        notice("before the loss", 30, 4, applied=3, est="64000")["headers"],
        notice("after restoring an older store: new lineage, no descent, no revert", 31, 4,
               lineage="lin_c", est="9000")["headers"],
    ],
    "outcome": "resync",
}

id_collision = {
    "notice_headers": notice("notice 3", 3, 3)["headers"],
    "view_ready_headers": ready_headers(V3),
    "notice_id": f"{SR}:advanced:3",
    "view_ready_id": f"{SR}:view_ready:3",
}

doc = {
    "session": {"harness": "opencode", "project_root": "/work/demo", "session": "ses_1", "session_ref": SR,
                "account": ACCT, "runner_module_id": RUNNER, "provider_module_id": PROVIDER},
    "transcript": {"lineage_id": "lin_a", "messages": transcript},
    "notices": notices,
    "notices_accepted": notices_accepted,
    "notices_refused": notices_refused,
    "view_ready": view_ready,
    "view_ready_refused": view_ready_refused,
    "views": views,
    "views_refused": views_refused,
    "views_undecodable": views_undecodable,
    "edits_refused": edits_refused,
    "dispositions": dispositions,
    "view_get": view_get,
    "ride": ride,
    "ceiling": ceiling,
    "descent": descent,
    "id_collision": id_collision,
    "state_loss": state_loss,
}

with open(OUT, "w") as f:
    json.dump(doc, f, indent=2, ensure_ascii=False)
    f.write("\n")
