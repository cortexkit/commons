# cortexkit-log-manifest

Generate the log-template allow-list embedded by a module's `doctor manifest`
command. The tool reads one Cargo package without compiling it. It reports
logging calls that could not contribute a literal template on stderr.

```sh
cortexkit-log-manifest --crate ./my-module --module-id my-module \
  --repository https://github.com/example/my-module --log-glob 'logs/*.log'
cortexkit-log-manifest --crate ./my-module --check
```

See [CONTRACT.md](CONTRACT.md) for generation and check-mode behavior.
