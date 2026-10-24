#!/usr/bin/env python3
"""Regenerate the persona/v1 golden vectors and SHA256SUMS.

This is a second implementation of SPEC.md section 6 (newline normalisation,
layer digests, fragment digests, the source revision and the rendered digest),
written without the Rust code. The golden test compares the crate's output with
what this script writes, so a match is evidence for the rules rather than the
crate agreeing with itself.

It is not a template engine: the expected rendered text of each case is
written out by hand below, from the fixture templates and SPEC.md's rendering
rules. Every digest and every report field is computed here.

Run it with Python 3.8 or later from anywhere:

    python3 crates/cortexkit-persona/test-vectors/generate.py

It rewrites the echo fixtures (one LF and one CRLF copy of the same role),
every `cases/*` directory and `SHA256SUMS`. The scribe fixtures are written by
hand and only read. Changing any expected byte or digest is a breaking release
of the crate (SPEC.md section 6).
"""

import hashlib
import json
import os
import re

# Paths are relative to the crate root, as the Rust tests see them.
os.chdir(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

ROOT = "test-vectors"
V = f"{ROOT}/persona-v1"
BARE = re.compile(r"^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$")


def norm(b):
    """CRLF and lone CR become LF (SPEC.md section 6)."""
    return b.replace(b"\r\n", b"\n").replace(b"\r", b"\n")


def sha(b):
    return "sha256:" + hashlib.sha256(b).hexdigest()


def layer_files(d):
    """A layer is PERSONA.md plus fragments/<bare-id>.md; nothing else."""
    files = {"PERSONA.md": norm(open(f"{d}/PERSONA.md", "rb").read())}
    fd = f"{d}/fragments"
    if os.path.isdir(fd):
        for n in os.listdir(fd):
            if n.endswith(".md") and BARE.match(n[:-3]) and os.path.isfile(f"{fd}/{n}"):
                files["fragments/" + n] = norm(open(f"{fd}/{n}", "rb").read())
    return files


def layer_digest(d):
    """Files in byte order of their logical paths, each framed as
    path NUL decimal-length NUL content."""
    files = layer_files(d)
    h = hashlib.sha256()
    for p in sorted(files):
        c = files[p]
        h.update(p.encode() + b"\0" + str(len(c)).encode() + b"\0" + c)
    return "sha256:" + h.hexdigest()


def canon(o):
    """RFC 8785 canonical JSON. For these documents (ASCII keys, integers and
    strings only) sorted keys and no whitespace are exactly RFC 8785."""
    return json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def frag_digest(path):
    return sha(norm(open(path, "rb").read()))


def tier_dir(t, role):
    return f"{V}/{t['root']}/{t['roles_path']}/{role}"


def write_case(case, cfg, text, applied, approval_states, manifest, closure, entry_tier, fragments):
    """Write case.json, expected.txt and expected-report.json for one case.

    `applied` names the tiers whose layer is used, `approval_states` the
    approval of each repository tier, `closure` the fragment ids in the entry
    template's include closure, and `fragments` the composed fragment list as
    (id, origin, source file) in declared order.
    """
    d = f"{V}/cases/{case}"
    os.makedirs(d, exist_ok=True)
    open(f"{d}/case.json", "w", newline="\n").write(json.dumps(cfg, indent=2) + "\n")
    open(f"{d}/expected.txt", "wb").write(text.encode())
    role = cfg["role"]
    tiers = []
    layers = []
    for t in cfg["tiers"]:
        dd = tier_dir(t, role)
        present = os.path.isfile(f"{dd}/PERSONA.md")
        dig = layer_digest(dd) if present else None
        used = t["name"] in applied
        if used:
            layers.append({"tier": t["name"], "role": role, "digest": dig})
        tiers.append({
            # The Rust test joins the tier root with `/` on every OS.
            "name": t["name"], "trust": t["trust"], "root": f"{V}/{t['root']}",
            "relative_path": f"{t['roles_path']}/{role}", "present": present,
            "approval": approval_states.get(t["name"], "not_required"),
            "layer_digest": dig, "used": used,
        })
    reported = []
    revision_fragments = []
    for fid, origin, path in fragments:
        dg = frag_digest(path)
        reported.append({"id": fid, "origin": origin, "digest": dg, "included": fid in closure})
        if fid in closure:
            revision_fragments.append({"id": fid, "origin": origin, "digest": dg})
    revision = sha(canon({"format": "persona/v1", "role": role, "layers": layers,
                          "manifest": manifest, "fragments": revision_fragments}))
    report = {
        "format": "persona/v1", "module": cfg["module"], "role": role, "tiers": tiers,
        "parents": [], "fragments": reported, "entry_tier": entry_tier,
        "source_revision": revision, "rendered_digest": sha(text.encode()),
    }
    open(f"{d}/expected-report.json", "w", newline="\n").write(json.dumps(report, indent=2, ensure_ascii=False) + "\n")


def write_echo_fixtures():
    """The same one-tier role twice: once with LF and once with CRLF endings."""
    persona = """---
name: echo
version: 1
display_name: Echo
fragments: [footer]
---
Line one for {{ agent.role }} on {{ harness.id }}.
{% if harness.capabilities.submit_tool %}Use the submit tool to answer.
{% endif %}Line three.
{% include "footer" %}
"""
    footer = "Footer: {{ env.timezone }}.\nLast line.\n"
    for variant, nl in (("lf", "\n"), ("crlf", "\r\n")):
        base = f"{V}/echo-{variant}/tiers/base/roles/echo"
        os.makedirs(f"{base}/fragments", exist_ok=True)
        for path, text in (("PERSONA.md", persona), ("fragments/footer.md", footer)):
            with open(f"{base}/{path}", "wb") as f:
                f.write(text.replace("\n", nl).encode())
    context = """{
  "agent": {"role": "echo"},
  "model": {"provider": "example", "id": "model-small-1", "variant": "fast"},
  "harness": {"id": "example-harness", "capabilities": {"submit_tool": true}},
  "tools": {},
  "mcp": {},
  "modules": {},
  "demand": {"iq": 30, "eq": 30},
  "env": {"os": "windows", "timezone": "UTC", "locale": "tr-TR"},
  "module": null
}
"""
    open(f"{V}/echo-context.json", "w", newline="\n").write(context)


def write_scribe_cases():
    tiers = [
        {"name": "base", "trust": "operator", "root": "scribe/tiers/base", "roles_path": "roles"},
        {"name": "user", "trust": "operator", "root": "scribe/tiers/user", "roles_path": "cortexkit/demo/roles"},
        {"name": "project", "trust": "repository", "root": "scribe/tiers/project", "roles_path": ".cortexkit/demo/roles"},
    ]
    project_digest = layer_digest(tier_dir(tiers[2], "scribe"))
    approval = {"module": "demo", "role": "scribe", "project_key": "pj-demo-1",
                "relative_path": ".cortexkit/demo/roles/scribe", "layer_digest": project_digest}
    # Approved with an older digest: the layer has changed since.
    stale = dict(approval, layer_digest=sha(b"an earlier version of the layer"))
    # Approved for another project: it does not apply here.
    other_project = dict(approval, project_key="pj-other-2")

    # `tone` is overridden by the user tier; the qualified id is supplied.
    fragments = [
        ("rules", {"kind": "bundle", "tier": "base"}, f"{V}/scribe/tiers/base/roles/scribe/fragments/rules.md"),
        ("tone", {"kind": "bundle", "tier": "user"}, f"{V}/scribe/tiers/user/cortexkit/demo/roles/scribe/fragments/tone.md"),
        ("mc/memory-profile@1", {"kind": "supplied"}, f"{V}/scribe/supplied/memory-profile.md"),
    ]
    closure = {"rules", "tone", "mc/memory-profile@1"}
    cfg = {"module": "demo", "role": "scribe", "project_key": "pj-demo-1", "extensions": ["limits"],
           "tiers": tiers, "context": "scribe/context.json",
           "supplied": {"mc/memory-profile@1": "scribe/supplied/memory-profile.md"}}
    closing = ("Tone: warm and brief.\n\nMemory profile: the user prefers numbered citations.\n\n"
               "Keep answers under 400 words.")
    rules = "Rules:\n- Cite your sources.\n- Show your reasoning step by step.\n- Answer in en-GB."

    # All three layers: `intro` (project) and `tools` (user) are disabled,
    # the project extends the `rules` block and lowers `limits`.
    write_case("scribe-approved", dict(cfg, approvals=[approval]),
               "\n\n" + rules + "\n- Follow the project's style guide.\n\n\n\n" + closing,
               {"base", "user", "project"}, {"project": "approved"},
               {"name": "scribe", "version": 2, "display_name": "Scribe (project)",
                "fragments": ["rules", "tone", "mc/memory-profile@1"], "disabled_blocks": ["intro", "tools"],
                "limits": {"max_words": 250}},
               closure, "project", fragments)

    # The project layer is skipped whole; base and user stand.
    skipped_text = "You are Quill, writing with example/model-large-2 on linux.\n\n" + rules + "\n\n\n\n" + closing
    skipped_manifest = {"name": "scribe", "version": 2, "display_name": "Scribe",
                        "fragments": ["rules", "tone", "mc/memory-profile@1"], "disabled_blocks": ["tools"],
                        "limits": {"max_words": 400}}
    write_case("scribe-changed", dict(cfg, approvals=[stale]), skipped_text, {"base", "user"},
               {"project": "changed"}, skipped_manifest, closure, "user", fragments)
    write_case("scribe-unapproved", dict(cfg, approvals=[other_project]), skipped_text, {"base", "user"},
               {"project": "unapproved"}, skipped_manifest, closure, "user", fragments)


def write_echo_cases():
    text = "Line one for echo on example-harness.\nUse the submit tool to answer.\nLine three.\nFooter: UTC.\nLast line."
    for variant in ("lf", "crlf"):
        cfg = {"module": "demo", "role": "echo", "project_key": None, "extensions": [],
               "tiers": [{"name": "base", "trust": "operator", "root": f"echo-{variant}/tiers/base", "roles_path": "roles"}],
               "approvals": [], "context": "echo-context.json", "supplied": {}}
        write_case(f"echo-{variant}", cfg, text, {"base"}, {},
                   {"name": "echo", "version": 1, "display_name": "Echo", "fragments": ["footer"], "disabled_blocks": []},
                   {"footer"}, "base",
                   [("footer", {"kind": "bundle", "tier": "base"},
                     f"{V}/echo-{variant}/tiers/base/roles/echo/fragments/footer.md")])


def write_sums():
    """SHA256SUMS lists every file under test-vectors/ except itself."""
    lines = []
    for d, _, names in os.walk(ROOT):
        for n in names:
            p = os.path.relpath(os.path.join(d, n), ROOT).replace(os.sep, "/")
            if p != "SHA256SUMS":
                lines.append((p, hashlib.sha256(open(os.path.join(d, n), "rb").read()).hexdigest()))
    lines.sort()
    with open(f"{ROOT}/SHA256SUMS", "w", newline="\n") as f:
        f.write("".join(f"{h}  {p}\n" for p, h in lines))


if __name__ == "__main__":
    write_echo_fixtures()
    write_scribe_cases()
    write_echo_cases()
    write_sums()
