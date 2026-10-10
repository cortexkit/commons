# Changelog

## 0.6.0

- Requires subc-protocol 0.30 and cortexkit-role-tool-provider 0.6. No change to
  the cases.

## 0.5.2

- Add the unconditional `catalog_reply_valid` conformance case, refusing
  invalid per-tool reply metadata by tool and member with `check_reply`.
- Test the new case against a deliberately broken provider stand-in and
  depend on the optional reply deadline types and validator added in
  `cortexkit-role-tool-provider` 0.5.2.
