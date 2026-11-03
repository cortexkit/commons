# Third-party code and data

## Code

`src/utok/` is copied from Oh My Pi's `crates/pi-natives/src/utok/`
(<https://github.com/can1357/oh-my-pi>, MIT, see `LICENSE-oh-my-pi`) at
commit `0b5c27ab24719c86a5986b95ee55f4e75bc1aaa0` (2026-10-22). `fixtures/`
holds that crate's test fixtures from the same commit. Every change is marked
with a `cortexkit-tokens:` comment:

- **Unicode data on stable Rust.** Upstream reads Unicode data from `xutf`,
  which needs nightly Rust. `src/utok/ucd.rs` gives the same lookups over the
  stable `unicode-normalization`, `unicode-properties` and `unicode-script`
  crates, pinned to the same Unicode 16.0.0 data. All of upstream's fixture
  and differential tests pass on it.
- **A cargo feature per family**, so a runner builds only the vocabularies it
  serves. o200k is always built.
- **Pure-Rust zstd.** Upstream decodes the tables with `zstd`, which
  compiles C; `src/utok/zst.rs` decodes with `ruzstd`, so the crate builds
  for WebAssembly without a C toolchain.
- **The Jev family is removed.** Its vocabulary was measured from a live API
  rather than published, and no model maps to it here. Its data, tests and
  documentation are removed with it.

The image rules in `src/image.rs` and the model-to-family mapping in
`src/family.rs` are ported from Oh My Pi's model catalog
(`packages/catalog/src/compat/image-tokenization.ts`,
`packages/catalog/src/compat/rules/`, `packages/catalog/src/model-tokenizer.ts`)
at the same commit.

## Vocabulary data

`data/` holds Oh My Pi's packed vocabularies from the same commit, made from
the source files below. The BPE vocabularies are packed into UTOK1 tables
that keep the ranks of retained entries; merge-unreachable entries become
empty slots. The Claude measurement data is converted to the compact marker
format `data/README.md` describes, with its witness metadata removed and
glued contraction spellings added. On 2026-11-03 we checked the *source
files* Oh My Pi packed from: each hashes to the published file, or, where the
published file has since been reformatted, parses to the same JSON. The
packed output itself is checked only by the tests.

| File | Source | License |
|---|---|---|
| `ctok_v3.bin.zst`, `ctok_v4_7.bin.zst` | Sander Land's ctok v1.0.0 measurements, <https://github.com/sanderland/ctok> at `df3b59b5e645289a5eadc8e24036b99d39c333c4` | MIT, `data/LICENSE.ctok` |
| `o200k_base.bin.zst` | `https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken`, sha256 `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d` (the hash tiktoken checks) | MIT, `data/LICENSE.tiktoken` |
| `cl100k_base.bin.zst` | `https://openaipublic.blob.core.windows.net/encodings/cl100k_base.tiktoken`, sha256 `223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7` | MIT, `data/LICENSE.tiktoken` |
| `qwen3.bin.zst` | `tokenizer.json` of <https://huggingface.co/Qwen/Qwen3.8-27B> at `1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0` | Apache-2.0, `data/LICENSE.qwen` |
| `deepseek3.bin.zst` | `tokenizer.json` of <https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash> at `60d8d70770c6776ff598c94bb586a859a38244f1` | MIT, `data/LICENSE.deepseek` |
| `kimi_k2.bin.zst` | `tiktoken.model` of <https://huggingface.co/moonshotai/Kimi-K2-Instruct> at `fd1984e2b7a3350dbf7305fe73a4ede25c14de50`, sha256 `b6c497a7469b33ced9c38afb1ad6e47f03f5e5dc05f15930799210ec050c5103` | Modified MIT, `data/LICENSE.kimi` |
| `glm5.bin.zst` | `tokenizer.json` of <https://huggingface.co/zai-org/GLM-5> at `c183ef8c61faee82855eca1ed9bb3a9a7ce3b0b2` | `mit` in the model card's metadata; the repository has no license file |

The Qwen vocabulary is redistributed under Apache-2.0: `data/LICENSE.qwen`
is its license, and this notice states the one change made to it, the
repacking above. The Kimi K2 license adds a term to MIT: a commercial
product or service built on the software or a derivative work, with more
than 100 million monthly active users or more than US$20 million monthly
revenue, must prominently display "Kimi K2" in its user interface.

## Updating

To update from upstream, copy the module again, reapply the marked changes,
and run the tests. If any family's counts change, bump that family's
`Family::revision` and update its pin in `src/lib.rs`, so callers can
invalidate stored counts made under the old revision.
