# Changelog

## 0.2.0

- Add optional `notice_record: {writer, id}` display-record references to
  `awaited_work.finish` and `awaited_work.progress`, including bounds checks,
  wire vectors and the caller-writer refusal rule. Omission preserves the old
  request bytes. The added public fields break struct-literal construction of
  `FinishParams` and `ProgressParams`.

## 0.1.0

- First release: the `awaited-work/v1` operation family (`awaited_work.open`,
  `finish`, `progress`, `cancel`, `sync`, `list`), its item and refusal
  shapes, bounds checks and test vectors.
