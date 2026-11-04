# Changelog

## 0.1.2

- 24 more `clean_reply` vectors: reply-cleaning edge cases (adjacent and
  doubled section signs, section references, broken tag endings, tag zero,
  oversized and lone tags, non-breaking spaces). The library's behaviour is
  unchanged, and `PLAN_VERSION` stays 2: the pinned rendering checksum covers
  the cases from before, and new cases are appended after them.

## 0.1.1

- Requires `cortexkit-role-compaction` 0.2.1.
- Inserted images (contract W6.5): `plan` refuses, as `unsupported_edit`
  naming the edit index, an image on a message whose role isn't user, a media
  type other than `image/png`, and data that isn't standard padded base64
  (standard alphabet, length a multiple of four, `=` only at the end, zero
  unused bits before it; PNG contents are not checked). The
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
