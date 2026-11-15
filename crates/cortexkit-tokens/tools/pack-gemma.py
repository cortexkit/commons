#!/usr/bin/env python3
"""Pack Gemma 4's published tokenizer into the crate's Gemini tables.

Reads the pinned `tokenizer.json` of google/gemma-4-12B-it, checks its hash
and every property the Rust counter (`src/utok/gemma.rs`) relies on, and
writes two zstd-compressed tables into `../data/`:

- `gemma4_vocab.bin.zst`: the pieces in id order, so a piece's index is its
  token id.
- `gemma4_merges.bin.zst`: the merges in rank order, each named by the piece
  it produces and where that piece splits into its two halves.

The layout of both is in `../data/families.json` (`gemma4`). A piece that no
count can ever use (special and unused tokens, and pieces that are neither a
single character nor part of any merge) is written as an empty slot, which
keeps ids unchanged and the table smaller.

Usage, from this directory:

    curl -L -o ../../../target/scratch/gemma4-tokenizer.json --create-dirs \
      https://huggingface.co/google/gemma-4-12B-it/resolve/43c9d014b231474fa7212dbb48a762566c2b255d/tokenizer.json
    python3 pack-gemma.py ../../../target/scratch/gemma4-tokenizer.json

Needs Python 3.9+ and the `zstd` command-line tool. The output is
deterministic for one zstd version; another version may compress to
different bytes that decode to the same tables.
"""

import hashlib
import json
import os
import struct
import subprocess
import sys

SOURCE_SHA256 = "9788bdb6a021b3170357cd3587b5d622b1177561ad3c5bf43d73728fd71aeacb"
VOCAB_MAGIC = b"GVOCAB1\n"
MERGES_MAGIC = b"GMERGE1\n"
SPACE = "\u2581"  # ▁, the piece a space becomes after normalization


def fail(message):
    sys.exit(f"pack-gemma: {message}")


def varint(value):
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def zigzag(value):
    return value << 1 if value >= 0 else ((-value) << 1) - 1


def check_shape(tokenizer):
    """Refuse a file whose pipeline differs from what the counter implements.

    The counter turns every space into ▁ and runs one BPE over the whole
    text: characters first, `<0xXX>` byte tokens for characters missing from
    the vocabulary, then the merges by rank. Any other normalizer,
    pre-tokenizer or model option would change counts silently.
    """
    expected_normalizer = {"type": "Replace", "pattern": {"String": " "}, "content": SPACE}
    if tokenizer["normalizer"] != expected_normalizer:
        fail(f"unexpected normalizer {tokenizer['normalizer']!r}")
    # Splitting on a space after every space became ▁ never splits anything.
    expected_pre = {
        "type": "Split",
        "pattern": {"String": " "},
        "behavior": "MergedWithPrevious",
        "invert": False,
    }
    if tokenizer["pre_tokenizer"] != expected_pre:
        fail(f"unexpected pre_tokenizer {tokenizer['pre_tokenizer']!r}")
    post = tokenizer["post_processor"]
    if post is not None and (
        post.get("type") != "TemplateProcessing"
        or post.get("single") != [{"Sequence": {"id": "A", "type_id": 0}}]
    ):
        fail(f"post_processor adds tokens to a single text: {post!r}")
    model = tokenizer["model"]
    expected_model = {
        "type": "BPE",
        "dropout": None,
        "continuing_subword_prefix": None,
        "end_of_word_suffix": None,
        "byte_fallback": True,
        "ignore_merges": False,
    }
    for key, value in expected_model.items():
        if model.get(key) != value:
            fail(f"model.{key} is {model.get(key)!r}, expected {value!r}")
    # Added tokens that are not special would be matched inside plain text
    # and split it; the counter does not do that.
    plain_added = [t["content"] for t in tokenizer["added_tokens"] if not t["special"]]
    if plain_added:
        fail(f"{len(plain_added)} non-special added tokens, e.g. {plain_added[:5]!r}")


def main():
    if len(sys.argv) != 2:
        fail("usage: pack-gemma.py <path to tokenizer.json>")
    source = sys.argv[1]
    raw = open(source, "rb").read()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != SOURCE_SHA256:
        fail(f"sha256 {digest} is not the pinned {SOURCE_SHA256}")
    tokenizer = json.loads(raw)
    check_shape(tokenizer)

    vocab = tokenizer["model"]["vocab"]
    pieces = [None] * len(vocab)
    for piece, token_id in vocab.items():
        if not 0 <= token_id < len(pieces) or pieces[token_id] is not None:
            fail(f"ids are not a permutation of 0..{len(pieces)} (at {piece!r})")
        pieces[token_id] = piece

    merges = [tuple(merge) for merge in tokenizer["model"]["merges"]]
    if len(set(merges)) != len(merges):
        fail("duplicate merges")
    byte_tokens = [f"<0x{byte:02X}>" for byte in range(256)]
    for token in byte_tokens:
        if token not in vocab:
            fail(f"byte fallback token {token} is missing")
    if SPACE not in vocab:
        fail("the ▁ piece is missing")

    needed = {piece for piece in pieces if len(piece) == 1}
    needed.update(byte_tokens)
    for left, right in merges:
        if left not in vocab or right not in vocab or left + right not in vocab:
            fail(f"merge {left!r} + {right!r} names a piece outside the vocabulary")
        needed.update((left, right, left + right))

    vocab_blob = bytearray(VOCAB_MAGIC)
    vocab_blob += struct.pack("<I", len(pieces))
    for piece in pieces:
        encoded = piece.encode("utf-8") if piece in needed else b""
        # NUL terminates each piece, so no piece may contain one. None can:
        # NUL is not a vocabulary character, so no merge can produce it.
        if b"\x00" in encoded:
            fail(f"piece {piece!r} contains NUL")
        vocab_blob += encoded + b"\x00"

    merges_blob = bytearray(MERGES_MAGIC)
    merges_blob += struct.pack("<I", len(merges))
    previous = 0
    for left, right in merges:
        produced = vocab[left + right]
        merges_blob += varint(zigzag(produced - previous))
        merges_blob += varint(len(left.encode("utf-8")))
        previous = produced

    data_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "data")
    for name, blob in [("gemma4_vocab.bin.zst", vocab_blob), ("gemma4_merges.bin.zst", merges_blob)]:
        path = os.path.join(data_dir, name)
        subprocess.run(
            ["zstd", "-19", "-q", "-f", "-o", path],
            input=bytes(blob),
            check=True,
        )
        print(f"{name}: {len(blob)} bytes raw, {os.path.getsize(path)} packed")
    kept = sum(1 for piece in pieces if piece in needed)
    print(f"{len(pieces)} ids ({kept} kept), {len(merges)} merges")


if __name__ == "__main__":
    main()
