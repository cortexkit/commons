# Changelog

## 0.1.1

- Requires `cortexkit-role-compaction` 0.2.1.
- Inserted images (contract W6.5): `plan` refuses, as `unsupported_edit`
  naming the edit, an image on a message whose role isn't user, a media type
  other than `image/png`, and data that isn't standard padded base64. The
  render plan lists each insert's images, and the neutral render adds one
  inline file part per image after the texts, with the decoded size.
- `split_leading` returns the first leading prefix's number only. When it
  doesn't fit a `u64`, the result is `None`; a later prefix no longer stands
  in for it (W4.6, T7).
- `PLAN_VERSION` is 2.

## 0.1.0

- First release of the shared compaction view library, with render plans, view
  application, provider-rule checks, reply cleaning, tag numbering, and JSON
  vectors.
