# Changelog

## 0.1.0

- First release: `Device`, `Report`, `State`, `DeviceKind`, `Where`,
  `Watching`, `Summary` (with `same_attention`), `summarize`, `Deliver` and
  `route`. Location precedence: an active phone, then an active or background
  computer, then any fresh report as away, else unknown. Inline suppression
  lasts at most 60 seconds after the newest active report for that agent.
