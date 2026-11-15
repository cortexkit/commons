#!/usr/bin/env python3
"""Generate fixtures/gemma4.json, the differential fixture for the Gemini family.

Builds a few thousand varied strings from a fixed seed and records how many
tokens Hugging Face's `tokenizers` gives each under the pinned Gemma 4
`tokenizer.json` (`encode(text, add_special_tokens=False)`). The Rust test
`utok::tests::gemma` replays every case against the crate's own counter.

The strings mix sentences and code in many scripts (sliced at random
places), runs of random vocabulary pieces, random code points (which reach
the byte fallback), whitespace and punctuation runs, numbers, the pieces
whose merges Gemma 4 drops from its list, and a few long texts. No string
contains a special token's literal: `tokenizers` would emit that as one
special token, while the crate counts it as plain text. Those literals are
recorded separately, counted with `encode_special_tokens = True` (plain
text), under `special_token_literals`.

Usage, from this directory, with the same tokenizer.json `pack-gemma.py`
reads:

    python3 -m venv ../../../target/scratch/venv
    ../../../target/scratch/venv/bin/pip install tokenizers
    ../../../target/scratch/venv/bin/python gen-gemma-fixture.py \
      ../../../target/scratch/gemma4-tokenizer.json
"""

import hashlib
import json
import os
import random
import sys

import tokenizers
from tokenizers import Tokenizer

SOURCE_SHA256 = "9788bdb6a021b3170357cd3587b5d622b1177561ad3c5bf43d73728fd71aeacb"
SEED = 20261115
SPACE = "\u2581"

SEED_TEXTS = [
    # English prose and chat-like text.
    "The committee met on Tuesday to review the proposal. After a long discussion, "
    "they agreed to postpone the vote until the budget numbers were final. Nobody "
    "was entirely happy, but everyone could live with it.",
    "Q: Why does the build fail on Windows?\nA: The path separator. Use `Path::join` "
    "instead of formatting strings with '/'. Also check CRLF line endings.\r\n",
    "I'm sure we'll find it; it's probably in the drawer you didn't check. "
    "They've said they'd call back, but it's 5 o'clock already!",
    # Markdown.
    "# Release notes\n\n## Added\n\n- A `--dry-run` flag that prints the plan.\n"
    "- Support for **nested** _lists_:\n  1. first\n  2. second\n\n> Note: back up first.\n\n"
    "| name | value |\n|---|---|\n| retries | 3 |\n",
    # Code.
    "fn parse(input: &str) -> Result<Vec<u32>, ParseIntError> {\n"
    "    input.split(',').map(|s| s.trim().parse::<u32>()).collect()\n}\n",
    "def fib(n: int) -> int:\n    a, b = 0, 1\n    for _ in range(n):\n"
    "        a, b = b, a + b\n    return a\n\n\nif __name__ == \"__main__\":\n    print(fib(30))\n",
    "const handler = async (req, res) => {\n\tconst { id } = req.params;\n"
    "\tif (!id) return res.status(400).json({ error: 'missing id' });\n"
    "\tres.json(await db.find(id));\n};\nexport default handler;\n",
    "SELECT u.id, u.email, COUNT(o.id) AS orders\nFROM users u\n"
    "LEFT JOIN orders o ON o.user_id = u.id\nWHERE u.created_at >= '2024-01-01'\n"
    "GROUP BY u.id, u.email\nHAVING COUNT(o.id) > 3\nORDER BY orders DESC;",
    "#!/usr/bin/env bash\nset -euo pipefail\nfor f in \"$@\"; do\n"
    "  [[ -f \"$f\" ]] || { echo \"missing: $f\" >&2; exit 1; }\n  sha256sum \"$f\"\ndone\n",
    '<!DOCTYPE html>\n<html lang="en">\n<head><meta charset="utf-8"><title>Demo</title></head>\n'
    '<body>\n  <div class="card"><p>Hello&nbsp;there</p><br/></div>\n</body>\n</html>\n',
    '{"model":"example","messages":[{"role":"user","content":"hi"}],'
    '"temperature":0.2,"max_tokens":1024,"stream":true,"stop":["\\n\\n"]}',
    "server:\n  port: 8080\n  hosts:\n    - alpha.internal\n    - beta.internal\n"
    "logging:\n  level: debug  # verbose while testing\n",
    # Numbers, identifiers and links.
    "Version 2.14.3 shipped on 2025-07-04T09:30:00Z with 1,234,567 downloads, "
    "a 99.95% uptime SLA, and hash 0x7f3a9c21deadbeef. Pi is 3.14159265358979.",
    "getUserById fetch_all_items MAX_RETRY_COUNT kebab-case-name __init__ "
    "XMLHttpRequest i18n k8s OAuth2Client utf8ToUtf16 v0.4.45-rc.1",
    "https://example.org/a/b?x=1&y=two#frag mailto:someone@example.com "
    "file:///tmp/a%20b.txt ftp://mirror.example.net/pub/",
    # Other languages.
    "Şu an çalışıyoruz; ığdır, şişli, güneş, öğretmen, İstanbul ve IĞDIR.",
    "Größere Veränderungen müssen über den Ausschuss laufen, sagte die Präsidentin.",
    "Le château est à côté de l'église ; on y accède par une allée bordée d'arbres.",
    "¿Dónde está la estación? Necesito llegar antes de las ocho, ¡por favor!",
    "Não sei se ele já chegou, mas a reunião começa às três horas.",
    "Zażółć gęślą jaźń. Przyjdź jutro o piątej, proszę.",
    "Съешь же ещё этих мягких французских булок, да выпей чаю.",
    "Η γρήγορη καφέ αλεπού πηδάει πάνω από τον τεμπέλη σκύλο.",
    "مرحبا بكم في المكتبة. يمكنكم استعارة ثلاثة كتب في كل زيارة.",
    "שלום עולם, זהו משפט לדוגמה בעברית עם מספרים 123.",
    "یک روز آفتابی بود و ما به پارک رفتیم.",
    "नमस्ते दुनिया! आज मौसम बहुत अच्छा है और हम बाहर जा रहे हैं।",
    "আমি বাংলায় গান গাই। আজ আকাশ খুব পরিষ্কার।",
    "வணக்கம் உலகம். இன்று நல்ல நாள்.",
    "สวัสดีครับ วันนี้อากาศดีมาก เราจะไปเที่ยวทะเลกัน",
    "Tiếng Việt có nhiều dấu: ă, â, ê, ô, ơ, ư, đ và các thanh điệu.",
    "Kami akan berangkat besok pagi dan kembali hari Minggu.",
    "我们今天讨论了新的设计方案，大家都觉得还需要进一步优化。",
    "這是一個繁體中文的句子，包含標點符號與數字２０２５。",
    "東京は日本の首都です。今日はとても暑いので、冷たいお茶を飲みましょう。カタカナもテスト。",
    "한국어 문장입니다. 오늘 회의는 오후 세 시에 시작합니다.",
    "Habari za asubuhi? Tunaenda sokoni kununua matunda.",
    # Symbols and emoji.
    "∀x∈ℝ: x² ≥ 0 ⇒ √(x²) = |x|; ∑ₖ₌₁ⁿ k = n(n+1)/2 ≈ ½n² ≠ ∞ ≤ ≥ ±",
    "Emoji: 😀😃😄 👍🏽👍🏿 👨\u200d👩\u200d👧\u200d👦 🏳️\u200d🌈 🇹🇷🇯🇵🇺🇸 ❤️\u200d🔥 🧑🏻\u200d💻 ✅❌⚠️",
    "Quotes “double” ‘single’ «guillemets» „low“ — en–dash … ellipsis • bullet † ‡ § ¶ © ® ™",
    # Whitespace oddities.
    "tabs\there\t\tand\t\t\tthere\nnbsp\u00a0between\u00a0words, ideographic\u3000space, "
    "zero\u200bwidth, narrow\u202fnbsp, em\u2003space, line\u2028separator",
    "trailing spaces   \nmixed \t \t indent\r\n\r\n\n\n\n    deep\n        deeper\n",
    "literal \u2581 lower one eighth block and ▁▁ twice",
]

RANDOM_BLOCKS = [
    (0x20, 0x7E, 6),
    (0x00, 0x1F, 1),
    (0xA0, 0xFF, 2),
    (0x100, 0x24F, 2),
    (0x300, 0x36F, 1),
    (0x370, 0x3FF, 1),
    (0x400, 0x4FF, 1),
    (0x590, 0x6FF, 1),
    (0x900, 0x97F, 1),
    (0xE00, 0xE7F, 1),
    (0x1100, 0x11FF, 1),
    (0x2000, 0x206F, 1),
    (0x2190, 0x22FF, 1),
    (0x3040, 0x30FF, 2),
    (0x4E00, 0x9FFF, 3),
    (0xAC00, 0xD7A3, 2),
    (0xE000, 0xF8FF, 1),
    (0xFE00, 0xFE0F, 1),
    (0xFF00, 0xFFEF, 1),
    (0x1D400, 0x1D7FF, 1),
    (0x1F300, 0x1FAFF, 3),
    (0x20000, 0x2A6DF, 1),
    (0xE0000, 0xE007F, 1),
    (0x10FFF0, 0x10FFFD, 1),
]

RUN_ALPHABET = [
    " ", "\t", "\n", "\r", "\r\n", "\u00a0", "\u3000", "\u200b", SPACE,
    ".", ",", "-", "_", "=", "#", "*", "/", "\\", "|", '"', "'", "`",
    "(", ")", "{", "}", "[", "]", "<", ">", "a", "x", "0", "1", "é", "中",
]

WORDS = [
    "get", "set", "user", "item", "list", "count", "token", "parse", "value", "index",
    "http", "json", "async", "request", "handler", "config", "max", "min", "id", "data",
]


def fail(message):
    sys.exit(f"gen-gemma-fixture: {message}")


def random_code_point(rng):
    weights = [weight for _, _, weight in RANDOM_BLOCKS]
    low, high, _ = rng.choices(RANDOM_BLOCKS, weights)[0]
    return chr(rng.randint(low, high))


def slices(rng, corpus, n):
    out = []
    for _ in range(n):
        length = min(int(rng.expovariate(1 / 35)) + 1, 200)
        start = rng.randrange(len(corpus))
        out.append(corpus[start : start + length])
    return out


def piece_runs(rng, pieces, n):
    out = []
    for _ in range(n):
        chosen = [rng.choice(pieces) for _ in range(rng.randint(1, 8))]
        joiner = rng.choice(["", "", "", " ", "\n"])
        out.append(joiner.join(chosen).replace(SPACE, " "))
    return out


def code_points(rng, n):
    return ["".join(random_code_point(rng) for _ in range(rng.randint(1, 24))) for _ in range(n)]


def symbol_runs(rng, n):
    out = []
    for _ in range(n):
        runs = [rng.choice(RUN_ALPHABET) * rng.randint(1, 40) for _ in range(rng.randint(1, 6))]
        out.append("".join(runs))
    return out


def numbers_and_names(rng, n):
    out = []
    for _ in range(n):
        kind = rng.randrange(5)
        if kind == 0:
            text = str(rng.randrange(10 ** rng.randint(1, 18)))
        elif kind == 1:
            text = f"{rng.uniform(-1e6, 1e6):.{rng.randint(0, 9)}f}"
        elif kind == 2:
            text = hex(rng.getrandbits(rng.randint(4, 64)))
        elif kind == 3:
            text = ".".join(str(rng.randint(0, 120)) for _ in range(rng.randint(2, 4)))
        else:
            words = [rng.choice(WORDS) for _ in range(rng.randint(1, 4))]
            style = rng.randrange(3)
            if style == 0:
                text = words[0] + "".join(word.title() for word in words[1:])
            elif style == 1:
                text = "_".join(words)
            else:
                text = "-".join(words).upper()
        out.append(text)
    return out


def dropped_merges(vocab, merges):
    """Pieces with a split into two pieces that the merges list leaves out.

    Gemma 4 lists most such splits as merges but omits a few hundred; a
    counter that derived merges from the vocabulary would merge them.
    """
    listed = set(merges)
    out = []
    for piece in vocab:
        for k in range(1, len(piece)):
            left, right = piece[:k], piece[k:]
            if left in vocab and right in vocab and (left, right) not in listed:
                out.append(piece.replace(SPACE, " "))
                break
    return sorted(out)


def long_texts(rng):
    out = []
    for _ in range(2):
        out.append("\n\n".join(rng.choice(SEED_TEXTS) for _ in range(rng.randint(8, 12))))
    out.append(" " * 3000)
    out.append("\n" * 500 + "end")
    out.append("".join(SEED_TEXTS[-11:-4]) * 3)
    return out


def main():
    if len(sys.argv) != 2:
        fail("usage: gen-gemma-fixture.py <path to tokenizer.json>")
    source = sys.argv[1]
    raw = open(source, "rb").read()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != SOURCE_SHA256:
        fail(f"sha256 {digest} is not the pinned {SOURCE_SHA256}")
    spec = json.loads(raw)
    tokenizer = Tokenizer.from_file(source)

    vocab = spec["model"]["vocab"]
    merges = [tuple(merge) for merge in spec["model"]["merges"]]
    special = [token["content"] for token in spec["added_tokens"] if token["special"]]
    # Pieces a text can actually spell: no special, unused or byte tokens.
    pieces = sorted(
        piece
        for piece in vocab
        if piece not in special
        and not (piece.startswith("<unused") or piece.startswith("<0x"))
    )

    here = os.path.dirname(os.path.abspath(__file__))
    corpus_path = os.path.join(here, "..", "fixtures", "corpus.json")
    upstream_corpus = json.load(open(corpus_path, encoding="utf-8"))

    rng = random.Random(SEED)
    joined = "\n\n".join(SEED_TEXTS)
    texts = (
        upstream_corpus
        + SEED_TEXTS
        + slices(rng, joined, 1200)
        + piece_runs(rng, pieces, 700)
        + code_points(rng, 400)
        + symbol_runs(rng, 400)
        + numbers_and_names(rng, 200)
        + dropped_merges(vocab, merges)
        + long_texts(rng)
    )
    texts = [text for text in texts if not any(literal in text for literal in special)]

    counts = [len(encoding.ids) for encoding in tokenizer.encode_batch(texts, add_special_tokens=False)]

    literals = [
        "<bos>",
        "<eos><pad>",
        "<|turn>user\nhello<turn|>",
        "<|tool_call>call:f{}<tool_call|>",
        '<|"|>quoted<|"|>',
        "<|image|> and <|audio|>",
        "a <mask> b",
        "<unk><unk>",
    ]
    tokenizer.encode_special_tokens = True
    literal_counts = [
        len(encoding.ids) for encoding in tokenizer.encode_batch(literals, add_special_tokens=False)
    ]

    generator = (
        f"tools/gen-gemma-fixture.py: tokenizers {tokenizers.__version__} "
        "Tokenizer.from_file(google/gemma-4-12B-it tokenizer.json at "
        f"43c9d014b231474fa7212dbb48a762566c2b255d, sha256 {SOURCE_SHA256})"
        ".encode(text, add_special_tokens=False); special_token_literals with "
        "encode_special_tokens = True"
    )

    def rows(pairs):
        return ",\n".join(json.dumps([text, count], ensure_ascii=False) for text, count in pairs)

    out_path = os.path.join(here, "..", "fixtures", "gemma4.json")
    with open(out_path, "w", encoding="utf-8") as out:
        out.write("{\n")
        out.write(f'"generator": {json.dumps(generator)},\n')
        out.write('"cases": [\n' + rows(zip(texts, counts)) + "\n],\n")
        out.write(
            '"special_token_literals": [\n' + rows(zip(literals, literal_counts)) + "\n]\n"
        )
        out.write("}\n")
    print(f"{len(texts)} cases, {sum(counts)} tokens, {os.path.getsize(out_path)} bytes")


if __name__ == "__main__":
    main()
