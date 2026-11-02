# cortexkit-compaction-view-wasm

The WebAssembly adapter for `cortexkit-compaction-view`, with a JavaScript loader
for runners using Node or Bun. It uses the shared Rust implementation for render
plans, rule checks, reply cleaning, tag numbering, and checksums.

For Rust dependencies:

```sh
cargo add cortexkit-compaction-view-wasm@0.1
```

For JavaScript consumers, obtain a source checkout or unpack the crate source,
build the WebAssembly module with Rust/Cargo and the `wasm32-unknown-unknown`
target, then vendor the binary and loader together. The published crate includes
`js/index.mjs`; the `cargo add` command alone does not install the JavaScript
files.

From the crate's source directory:

```sh
rustup target add wasm32-unknown-unknown
CARGO_TARGET_DIR=target cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/cortexkit_compaction_view_wasm.wasm js/compaction_view.wasm
```

Vendor both `js/index.mjs` and `js/compaction_view.wasm`. The loader reads the
binary beside itself by default, or accepts explicit bytes:

```js
import { load } from "./index.mjs";

const view = await load();
const answer = view.call("plan", { entries, edits });
```

In the commons workspace, `bash crates/cortexkit-compaction-view-wasm/build.sh`
builds the module into the workspace's `target` directory, copies it beside the
loader, and runs every shared vector through Node. The suite requires Bash and
Node in addition to the Rust target; it tests the Node loader path, not Bun. The
Rust and JavaScript adapter tests use vectors from the sibling view crate and are
not standalone tests of an unpacked wasm package. Linux CI installs an LTS Node
and runs this same end-to-end script.
