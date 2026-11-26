# `awaited-work/v1` test vectors

These wire examples ship with the crate and are exposed by the `test-vectors`
feature. `SHA256SUMS` pins their bytes, including this README, in `FILES` order.
All IDs and messages are invented.

- `requests.json`: one valid request envelope per operation, a finish request
  with `more: null`, and malformed or unknown members that prevent decoding.
- `bounds.json`: valid boundary inputs and invalid inputs that decode but fail
  bounds checks with `invalid_params`. Each rejected input names its field.
  Multibyte strings distinguish scalar counts from byte counts.
- `refusals.json`: every v1 refusal code and one future code. All are terminal:
  the caller falls back rather than retrying the unchanged request.
- `replies.json`: item, sync, and list replies with future members, plus all v1
  item states and one future item state.
