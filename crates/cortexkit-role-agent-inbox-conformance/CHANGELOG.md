# Changelog

## 0.1.0

- Add a transport-free Subject adapter, named live cases and rendered report.
- The crate's own tests run every case against a fake inbox, once correct and
  once with a single deliberate defect for that case, and check that exactly
  that case fails.
