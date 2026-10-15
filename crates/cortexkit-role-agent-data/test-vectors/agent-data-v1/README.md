# `agent-data/v1` test vectors

These files are the shared bytes that every `agent-data/v1` participant and
entorhinal check. The
contract is `CONTRACT.md` at the root of the `cortexkit-role-agent-data`
crate. These files ship in the crate's package; with the crate's
`test-vectors` feature, `cortexkit_role_agent_data::vectors` exposes their
contents, so a participant that depends on the crate by version loads
exactly these bytes.

| File | What it pins | Checked by |
|---|---|---|
| `requests.json` | `requests`: one valid request per op, including `busy` with no refs, the same handle under two kinds (two refs), and `prepare` at the cap of 256 refs; `refused`: requests that do not decode and are refused `invalid_request` (a member on `describe`, an unknown top-level field, 257 refs on `prepare` and on `busy`, an uppercase, short, non-hex or missing `cutId`, a `cutId` on `busy`, `release` without `holder`, a ref with an unknown member or an empty handle, a ref listed twice (on `prepare`, `busy` and `release`), `refs` that is not a list, an unknown op) | the wire test's request round-trip and refusal tests |
| `replies.json` | `replies`: at least one valid reply per op (describe with and without external data (the only external data is the Claude Code transcripts behind THALAMUS's harness) and an opaque `detail`, busy in request order with and without a `reason_code`, status, prepare frozen and not frozen with an external entry and an opaque `check` and `format`, export, diverged in all three states, import, the two `{}` replies (`activate`, `abort`), `release` with `enforced` true and false, the largest `endOffset`); `refused`: replies that do not decode (unknown top-level or typed member, a state or step outside its set, a missing `format` or `check`, a negative `endOffset`, a member on `{}`, a `release` reply without `enforced` or with a non-boolean one) | the wire test's reply round-trip and refusal tests |
| `diverged-reasons.json` | valid `diverged` replies with reasons on `unknown` and `advanced`, and refused reasons on `at_cut`, empty, over 256 UTF-8 bytes, and containing a control character | the wire test's diverged-reason validation test |
| `refusals.json` | `refusals`: one ERROR body per code in the closed set (and a second `invalid_state` for a ref-set mismatch, naming the expected refs, and a second and third `busy` refused by `activate` and `import`, and `local_advanced` refused by `activate` and by `import`, each naming the op it came from), each with its driver action and whether it is retried, and two codes outside the set, which decode and are never retried; `tolerated`: an extra member on the ERROR body, ignored; `detail_refused`: bodies whose typed `detail` is missing or does not decode | the wire test's refusal tests |
| `idempotency.json` | pairs of requests for the same participant with the outcome of the second: `replay` (identical, the same members in another order on the wire, or the same refs reordered, since refs compare as a set), `id_reused` (fewer refs, other refs, another `holder`), `new_key` (another `cutId` or another op), `exempt` (the four reads) | the wire test's idempotency test |
| `steps.json` | every (op, step) pair: the step it moves to, or the refusal (`cut_unknown`, or `invalid_state` with the step `detail.step` names) | the wire test's step-machine test |
| `ref-sets.json` | the one-ref-set-per-`cutId` rule: an op on a `cutId` at a step, the refs the `cutId` started with (`null` when the participant has none), the request's refs, and the outcome, `ok` or the `invalid_state` refusal with its detail; covers `export`, `release`, `activate` and both sides' `abort`, reordered sets, subsets and supersets, an `abort` of a `cutId` never seen, and the first ops and reads the rule never checks | the wire test's ref-set test |
| `op-refusals.json` | which op may refuse with which code: `ops` gives each op's row, the same table as CONTRACT.md §6.3, and `cases` asks about single (op, code) pairs, among them `busy` allowed on `prepare`, `import` and `activate` only, `local_advanced` on `import` and `activate` only, `cut_not_ready` on `diverged`, and a code outside the closed set in no row | the wire test's per-op refusal test |
| `session-refs.json` | the inputs behind the `sessionRef` values the other files use, each with the value `cortexkit_role_llm_runner::session_ref` returns for them | the wire test's sessionRef test, against the llm-runner crate |

A case with `expand_refs: N` replaces its params' `refs` with N refs
`{"ref_kind": "broca.session", "ref": "ses_<i>"}`, for `i` from 0 to N-1,
before decoding. This pins the 256-ref cap without checking in a list of
hundreds of refs.

Every canonical vector round-trips: decoding it as its op's type and
encoding the result gives back the same RFC 8785 (JCS) canonical bytes. The
`describe.detail`, `format` and `check` values in `replies.json` are
deliberately arbitrary, to show they pass through untouched.

`SHA256SUMS` pins every file in this directory, this README included, in
`sha256sum` format. Changing a vector changes the contract: bump the crate's
version, regenerate `SHA256SUMS`, and say why in the commit.
