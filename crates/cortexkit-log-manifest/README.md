# cortexkit-log-manifest

> [!WARNING]
> **Alpha.** cortexkit-log-manifest is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Generate the log-template allow-list embedded by a module's `doctor manifest`
command. The tool reads one Cargo package without compiling it. It reports
logging calls that could not contribute a literal template on stderr.

```sh
cortexkit-log-manifest --crate ./my-module --module-id my-module \
  --repository https://github.com/example/my-module --log-glob 'logs/*.log'
cortexkit-log-manifest --crate ./my-module --check
```

See [CONTRACT.md](CONTRACT.md) for generation and check-mode behavior.
