# Changelog

## 0.3.5

- On Windows, log files and newly created log directories are now owner-only:
  only the user running the program can open them, as with mode 0600 and 0700
  on Unix.
- When a log directory is narrowed on Windows, files already inside it,
  including nested ones, lose broader access too.

## 0.3.4

- Stop credential query redaction at `)` and `]` so URL wrappers and following diagnostic text survive.
