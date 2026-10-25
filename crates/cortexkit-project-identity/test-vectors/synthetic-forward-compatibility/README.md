# Synthetic forward-compatibility samples

These files are **invented decoder inputs, not handler captures**. Their members
are named by the shape under test, not by a transport operation envelope.

- `unknown-values.json` replaces each of the eight open string vocabularies
  with a `future-*` spelling. Decoding must choose `Other(String)` and retain the
  exact spelling on re-encoding. It does not assert the handler emits these
  combinations of identity, approval, and binding state.
- `unknown-keys.json` adds `futureMember` to requests, parameters, a captured
  execution binding, reply envelopes, resolve/id/enumerate results, root
  records, root keys, checkout incarnations, approvals, projects, workspaces,
  and a refusal. Those members must be ignored, including their arbitrary
  nested content. The inputs add a synthetic binding and label key only to
  exercise nested unknown members, not to claim served sample coverage.

Populated Git remotes are tested as isolated data types, not manufactured as
served reply samples. The real-handler sample gaps are listed in the crate
README.
