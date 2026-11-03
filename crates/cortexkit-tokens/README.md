# cortexkit-tokens

Token counts that every CortexKit module computes the same way. A runner (the
module that renders model requests and holds each message's content) counts
each part once with this crate and sends the counts; a compaction provider
(a module that rewrites a session's history) counts the text it writes itself
with the same crate.

```sh
cargo add cortexkit-tokens@0.1
```

- `count(family, text)`: content tokens of a text, with no message frame.
  Each family is an offline reconstruction of one provider tokenizer (OpenAI
  o200k and cl100k, four Claude generations, Qwen 3, DeepSeek V3, Kimi K2,
  GLM-5), with no correction ratio on top.
- `family_for_model(model)`: the family to count a model's text with, from
  name rules. An unrecognized model, including GPT-4o and older GPT models,
  gets o200k with no guaranteed error bound, and the choice says it is not
  exact; so does a known approximation (Haiku 5.5 counted with Sonnet 5's
  vocabulary).
- `image::rule_for` and `image::tokens`: an estimate of what one image costs
  its reader, by pixel size. Unrecognized models use their API's fallback,
  which is an assumption, not an upper bound.
- `reasoning::generated` and `reasoning::measured`: what a step's reasoning
  costs when it is sent again, first from the step's usage, then as the
  residual of two consecutive requests that share a prefix, capped by the
  step's reported output. The caller must establish the prefix and count
  everything else appended, framing included.
- `Family::token_family()`: a compatibility label such as `claude-v5/1`, the
  family and its counting revision. Counts are only comparable under the same
  label; callers store it and recompute counts under another. Maintainers bump
  a revision whenever a change can alter its counts, and a test pins one
  sample count per family as a regression guard.

Each family's vocabulary is behind a cargo feature, so a runner builds only
what it serves. o200k is always built. `count` returns `None` for a family
that isn't built in. The vocabularies carry their own licenses:

| Feature | Families | Data license | Default |
|---|---|---|---|
| (always) | o200k | MIT (tiktoken) | yes |
| `cl100k` | cl100k | MIT (tiktoken) | yes |
| `claude` | Claude v3, v4.7, v5, v5 Sonnet | MIT (ctok) | yes |
| `qwen` | Qwen 3 | Apache-2.0 | yes |
| `deepseek` | DeepSeek V3 to V4 | MIT | yes |
| `glm` | GLM-5 | MIT per the model card; no license file in the release | yes |
| `kimi` | Kimi K2 | Modified MIT: a product over 100M monthly users or US$20M monthly revenue must prominently display "Kimi K2" in its UI | no |

The code is MIT. `NOTICE.md` gives each file's exact source and revision.

The tokenizer, model-mapping and image-rule code is adapted from
[Oh My Pi](https://github.com/can1357/oh-my-pi) under MIT. The vocabulary data
has its own sources and licenses (table above), including Apache-2.0 and
Modified MIT; the Claude measurement data is Sander Land's
[ctok](https://github.com/sanderland/ctok) under MIT. See `NOTICE.md` for the
exact sources and the changes made to build on stable Rust.
