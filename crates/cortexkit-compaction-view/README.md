# cortexkit-compaction-view

The shared view library for the CortexKit compaction contract. A runner and a
compaction provider use the same implementation to compute render plans, apply
views, check provider rules, clean replies, and number tags.

```sh
cargo add cortexkit-compaction-view@0.2
```

The wire types come from
[`cortexkit-role-compaction`](https://crates.io/crates/cortexkit-role-compaction).
The crate ships JSON vectors in `vectors/cases.json`. `PLAN_VERSION` identifies
rendering semantics: supported rendering behavior changes require a version bump.
A regression test pins representative JSON outputs for selected rendering
operations and a sample tag prefix. It is not exhaustive and does not hash final
host requests; retaining historical pins and requiring version bumps are
release-review obligations. Runners requiring replay compatibility must persist
and check the version themselves.

Reply cleaning has two versions in the vectors, pinned on the same inputs:
`clean_reply` is the current rule (plan version 4), and `clean_reply_for_plan`
at 3 reproduces the rule before it. Version 3 exists only to reproduce bytes
recorded before a runner adopted version 4: a resumed run's requests, and
offline checks that rebuild recorded requests. It is never for a new render or
a served read.

The companion
[`cortexkit-compaction-view-wasm`](https://crates.io/crates/cortexkit-compaction-view-wasm)
crate exposes the same implementation to JavaScript runners. On the JavaScript
side reply cleaning stays current-only: a JavaScript runner calls `clean_reply`
(version 4), because nothing on that side replays recorded requests. A
JavaScript implementation checked against `vectors/cases.json` implements
`clean_reply` and skips the `clean_reply_for_plan` cases.

Use `render::plan_view(entries, view, rules, floor)` (JSON `plan_view` in
WebAssembly) to check a whole view and honor `new_signed: "strip"`. It drops invalidated
signed parts only in entries strictly after the view's active-path
`built_through` message id. Recheck at every step boundary, because a revert
can remove the anchor. `plan_view_at` selects a supported plan version;
`apply_with_rules` produces neutral rendered messages with the same policy.
The edit-only APIs remain strict and cannot authorize stripping. Encrypted
reasoning is unaffected, and the final protected tool part always stays.
Providers must decode the new wire members before runners send them.

Pass the session's signed floor (compaction contract rule W3.5) as
`rules::Floor::through(mid)`, or `Floor::none()` when it has none, to
`plan_view`, `plan_view_at`, `apply_with_rules`, `rules::check` and
`rules::invalid_signed_parts` (JSON: an optional `signed_floor` member holding
the `through` message id). Signed parts at or before it are treated as an
already-stripped oldest run: never invalid and never one side of a gap. Edits
before the floor still invalidate signed parts after it, and a floor naming no
message in the entries covers nothing. The floor only decides validity: covered
parts stay in the plan, and removing them from the request is the runner's
strip.
