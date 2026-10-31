# App reader admission

`admission.json` pins the optional plan member's group gate and shape, exact
principal matching, the six read operations, implicit readers, own-send status,
and the absence of write grants. `app_readers` unit tests consume these cases.
An absent list preserves the runner's current admission; an empty list does not.

Refresh tests additionally check that accepted reader and sender lists replace
the old lists before model input applies, while the active compaction provider
retains its implicit read grant until the provider switches.
