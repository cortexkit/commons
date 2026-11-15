# Changelog

## 0.2.0

- Add the `Gemini` family: Gemini 3 onward and Gemma 4 onward, counted with
  Gemma 4's published vocabulary (behind the new `gemma` feature, on by
  default, Apache-2.0 data). Its counts equal Hugging Face `tokenizers`'
  `encode(text, add_special_tokens=False)` on that vocabulary, which matched
  Gemini's own `countTokens` text counts exactly. Its label is `gemini/1`.
- `family_for_model` maps Gemini 3 onward and Gemma 4 onward to `Gemini` as
  exact, including provider-prefixed and Antigravity route names. Earlier
  Gemini and Gemma, and names with no revision, map to `Gemini` marked not
  exact; they counted with o200k before.
- Breaking: `Family` is now `#[non_exhaustive]`, and `Family::ALL` has 11
  entries. A `match` on `Family` outside this crate needs a wildcard arm;
  later families will not break it again.
- Existing families' counts and labels are unchanged.

## 0.1.0

- First release: offline token counts per model family (o200k, cl100k, four
  Claude generations, Qwen 3, DeepSeek V3, Kimi K2, GLM-5), model-to-family
  name rules, image token rules and replayed-reasoning measurement.
