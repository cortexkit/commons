# Upfront tool definitions

Each JSON sample carries a plan and whether the runner declares `tool_upfront`.
`offered` is the item's offered tool names after its exclusions, given to the
helper as input. `resolved` is the
unchanged rendering hint, never a replacement tool set. `refusal` pins the exact
code and detail for admission or post-fetch validation.

- `valid.json`: two listed tools are offered; another offered tool stays offered.
- `undeclared.json`: even a well-formed list requires the group.
- `empty.json`, `duplicate.json`, `non-string.json`, `non-array.json`: malformed
  lists name the indexed item field in `invalid_params`.
- `stale.json`: an excluded tool cannot be made offered by listing it upfront;
  `plan_stale` carries `detail.differences` with exactly one entry,
  `{kind: "upfront", provider, name}`.
- `without-upfront.json`: absence needs no group and passes `check_plan`; the
  helpers leave the plan unchanged.

`tests/upfront.rs` decodes every sample and checks the admission and post-fetch
helpers. It also pins the pre-existing `plans.json` bytes, so adding this hint
does not rewrite plans without the member. Tool names have no extra identifier
grammar in this role: the list uses non-empty strings, distinct by exact spelling.
