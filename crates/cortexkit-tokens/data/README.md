# utok vocabulary data

> This document is copied from Oh My Pi at the revision recorded in
> `../NOTICE.md`; "this repository" below means Oh My Pi. The generator
> scripts and `cache/` paths are upstream-only: regenerate in
> `crates/pi-natives` of that pinned upstream checkout, then copy the
> resulting files here. The exception is the Gemma 4 section at the end,
> which is this crate's own: its scripts are the only ones in this crate's
> `../tools/`.

`ctok_v3.bin.zst` and `ctok_v4_7.bin.zst` are **generated** — do not
hand-edit. They are compacted from the measured vocabulary files of
[sanderland/ctok](https://github.com/sanderland/ctok) v1.0.0 (revision
`df3b59b5e645289a5eadc8e24036b99d39c333c4`), MIT licensed — see
`LICENSE.ctok`. The vocabulary data is Sander Land's measurement work
("On the biology of Claude's tokenizer",
<https://tokencontributions.substack.com/p/on-the-biology-of-claudes-tokenizer>);
the Rust implementation was written in Oh My Pi and is vendored in
`../src/utok/claude/`.

Upstream ships every piece with a `count_tokens` witness probe; compaction
drops that metadata, parses the public `⟨bow⟩the⟨eow⟩` key notation into the
compact C0 marker alphabet (single bytes `0x01`–`0x05`; safe because `nfc`
strips C0 controls from input), adds the glued contraction spellings, and
front-codes the sorted piece list into the version-2 binary format produced
by `../tools/gen-ctok-vocab.ts` (~4.7 MB of upstream JSON →
~254 KB front-coded → ~106 KB after zstd -19).

Regenerate the front-coded binaries, then compress them here:

```sh
cd ../tools
bun gen-ctok-vocab.ts   # fetch upstream, emit raw bins into cache/
bun pack-ctok.ts        # zstd -19 into ../data/
```

If the upstream pin moves, also regenerate
`../src/utok/claude/testdata/fixtures.json` against the same ctok release
(see the fixture doc in `../src/utok/claude/mod.rs`).

The other `*.bin.zst` files here are the UTOK1 BPE rank tables packed by
the per-family scripts in `../tools/` (container format and per-family
split specs: `families.json` in this directory), except the Gemma 4 tables
below.

## Gemma 4 (`gemma4_vocab.bin.zst`, `gemma4_merges.bin.zst`)

Not from Oh My Pi. These are **generated** in this crate from Gemma 4's
published `tokenizer.json` (source, revision and hash in `../NOTICE.md`),
Apache-2.0, see `LICENSE.gemma`. Gemma ranks merges by an explicit list
rather than by vocabulary order, so a UTOK1 rank table cannot hold it: the
pieces and the merges list are packed as two tables (layouts: `gemma4` in
`families.json`), about 1.0 MB and 0.23 MB after zstd -19. The vocabulary
would compress to about 1.15 MB with each piece length-prefixed as in UTOK1;
NUL terminators compress better.

Regenerate both tables, and the differential fixture
`../fixtures/gemma4.json`, from the pinned file:

```sh
cd ../tools
curl -L -o ../../../target/scratch/gemma4-tokenizer.json --create-dirs \
  https://huggingface.co/google/gemma-4-12B-it/resolve/43c9d014b231474fa7212dbb48a762566c2b255d/tokenizer.json
python3 pack-gemma.py ../../../target/scratch/gemma4-tokenizer.json         # needs the zstd CLI
python3 gen-gemma-fixture.py ../../../target/scratch/gemma4-tokenizer.json  # needs `pip install tokenizers`
```

Both scripts refuse a file with another hash. `pack-gemma.py` also refuses a
tokenizer whose normalizer, pre-tokenizer or BPE options differ from what
`../src/utok/gemma.rs` implements. The download (32 MB) goes under the
workspace's ignored `target/`.
