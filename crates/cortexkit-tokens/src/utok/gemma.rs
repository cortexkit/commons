//! cortexkit-tokens: Gemma's tokenizer, which the Gemini family counts with.
//! This module is not from Oh My Pi.
//!
//! Gemma 4 publishes its tokenizer as a Hugging Face `tokenizer.json`: a BPE
//! that differs from the byte-level engine in `bpe.rs` in three ways.
//!
//! - There is no pre-tokenizer. Every space becomes `▁` (U+2581) and the
//!   whole text is one word, so merges run over the full text at once and
//!   can join anything, newlines and spaces included.
//! - Symbols start as Unicode characters, not bytes. A character missing
//!   from the vocabulary becomes one `<0xXX>` byte token per UTF-8 byte.
//! - Merge priority is the order of an explicit merges list, which differs
//!   from the vocabulary order, and several merges can produce the same
//!   piece. A rank table, where a merge's priority is the rank of the piece
//!   it makes, cannot express that, so this module keeps the list itself.
//!
//! The merge loop follows `tokenizers`' `Word::merge_all`: apply the
//! lowest-ranked merge among the current adjacent pairs, the leftmost one on
//! a tie, until no adjacent pair has a merge. Merges could in principle join
//! the whole text, so a request of hundreds of KB would be one word. In
//! practice almost no piece puts `▁` right after another character (Gemma 4
//! has one such piece, `>▁</`), so no merge can cross a space that follows
//! any character but `>` or another space. The text splits at those spaces
//! into segments, mostly single words, that merge independently with the
//! same result: short ones by rescanning their pairs, long ones (a run of
//! Chinese or Japanese text, which has no spaces, or of whitespace) with a
//! heap of candidate pairs in O(n log n). Special tokens written in the text
//! (`<bos>`, `<|turn>`) count as plain text, like every other family's
//! `encode_ordinary` semantics.
//!
//! The tables come from `tools/pack-gemma.py`; their layout is in
//! `data/families.json` (`gemma4`).

use std::{
    cell::RefCell,
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    hash::{BuildHasherDefault, Hasher},
    sync::{
        LazyLock,
        atomic::{AtomicU32, Ordering},
    },
};

use crate::utok::{utf::Unit, zst};

static GEMMA4: LazyLock<Gemma> = LazyLock::new(|| {
    Gemma::parse(
        include_bytes!("../../data/gemma4_vocab.bin.zst"),
        include_bytes!("../../data/gemma4_merges.bin.zst"),
    )
});

/// Content tokens of `units` under Gemma 4's tokenizer.
pub(crate) fn count<U: Unit>(units: &[U]) -> u32 {
    u32::try_from(GEMMA4.count(units)).expect("utoken: token count fits u32")
}

/// Token ids of `units` under Gemma 4's tokenizer, as Hugging Face's
/// `encode(text, add_special_tokens=False)` gives them for text without
/// special-token literals.
pub(crate) fn encode<U: Unit>(units: &[U]) -> Vec<u32> {
    GEMMA4.encode(units)
}

/// The embedded Gemma 4 table, for tests that check its internals.
#[cfg(test)]
pub(crate) fn gemma4() -> &'static Gemma {
    &GEMMA4
}

const VOCAB_MAGIC: &[u8] = b"GVOCAB1\n";
const MERGES_MAGIC: &[u8] = b"GMERGE1\n";

/// A symbol merged into its left neighbour.
const GONE: u32 = u32::MAX;
/// No neighbour on that side.
const NONE: u32 = u32::MAX;
/// Segments up to this many symbols merge by rescanning their pairs
/// ([`Gemma::merge_short`]); longer ones use the heap.
const SHORT: usize = 32;

/// The segments of `symbols` that start at `starts`, in order.
fn segments<'a>(symbols: &'a mut [u32], starts: &[usize]) -> impl Iterator<Item = &'a mut [u32]> {
    let mut rest = symbols;
    let mut offset = 0;
    starts
        .iter()
        .copied()
        .chain([usize::MAX])
        .map(move |start| {
            let end = start.min(offset + rest.len());
            let (segment, tail) = std::mem::take(&mut rest).split_at_mut(end - offset);
            offset = end;
            rest = tail;
            segment
        })
}

/// A SentencePiece-style BPE vocabulary with its merges list.
pub(crate) struct Gemma {
    /// The symbol of each ASCII character: its piece, or its byte token when
    /// the vocabulary lacks it (as it lacks `\r`). A space maps to the `▁`
    /// piece, which is the whole of the normalizer.
    ascii: [u32; 128],
    /// Whether a `▁` right after each ASCII character might merge with
    /// it: true for a space, and for a character some piece puts right
    /// before a `▁` (in Gemma 4 only `>`, in `>▁</`). A space after any
    /// other character starts a new segment.
    ascii_joins: [bool; 128],
    /// The non-ASCII characters some piece puts right before a `▁`,
    /// sorted.
    joiners: Vec<u32>,
    /// Whether any merge takes a byte token, so that a `▁` after a
    /// character the vocabulary lacks might merge with it.
    bytes_merge: bool,
    /// The piece of each non-ASCII character the vocabulary holds.
    chars: HashMap<u32, u32, Fx>,
    /// The `<0xXX>` token of each byte.
    bytes: [u32; 256],
    /// Rank of each mergeable pair of adjacent symbols.
    pairs: PairTable,
    /// The piece each merge produces, by rank.
    produces: Box<[u32]>,
    /// This table's name in [`SegmentMemo`] slots.
    id: u32,
}

/// Source of [`Gemma::id`]s; 0 marks an empty [`SegmentMemo`] slot.
static NEXT_TABLE_ID: AtomicU32 = AtomicU32::new(1);

thread_local! {
    static MEMO: RefCell<SegmentMemo> = RefCell::new(SegmentMemo::new());
}

/// Per-thread memo of short segments' token counts. Most segments are
/// single words, and words repeat: a hit costs one probe where merging
/// costs a pair lookup per merge in a table too large for the cache.
/// Direct-mapped, overwritten on collision, 4 Ki slots (288 KiB a thread).
/// A slot holds the whole segment (up to [`SegmentMemo::KEY`] symbols) and
/// the table it was counted under, so a hit is exact.
struct SegmentMemo {
    slots: Box<[MemoSlot]>,
}

#[derive(Clone, Copy)]
struct MemoSlot {
    /// The segment's symbols, then `u32::MAX` (never an id) as padding.
    key: [u32; SegmentMemo::KEY],
    table: u32,
    count: u32,
}

impl SegmentMemo {
    const KEY: usize = 16;
    const BITS: u32 = 12;

    fn new() -> Self {
        let empty = MemoSlot {
            key: [u32::MAX; Self::KEY],
            table: 0,
            count: 0,
        };
        Self {
            slots: vec![empty; 1 << Self::BITS].into_boxed_slice(),
        }
    }

    /// [`Gemma::merge_segment`]'s count for `segment`, from the memo when
    /// it holds the segment.
    fn count(&mut self, gemma: &Gemma, segment: &mut [u32]) -> usize {
        if !(2..=Self::KEY).contains(&segment.len()) {
            return gemma.merge_segment(segment);
        }
        let mut key = [u32::MAX; Self::KEY];
        key[..segment.len()].copy_from_slice(segment);
        let mut hash = u64::from(gemma.id);
        for word in key {
            hash = (hash.rotate_left(5) ^ u64::from(word)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
        let slot = &mut self.slots[(hash >> (64 - Self::BITS)) as usize];
        if slot.table == gemma.id && slot.key == key {
            return slot.count as usize;
        }
        let count = gemma.merge_segment(segment);
        *slot = MemoSlot {
            key,
            table: gemma.id,
            count: count as u32,
        };
        count
    }
}

impl Gemma {
    /// Parse the two zstd-compressed tables `tools/pack-gemma.py` writes.
    /// Panics on malformed data: the tables are compiled in, so that is a
    /// build defect, never bad input.
    pub(crate) fn parse(vocab_zst: &[u8], merges_zst: &[u8]) -> Self {
        let vocab_raw = zst::decode_all(vocab_zst);
        let mut rest = vocab_raw
            .strip_prefix(VOCAB_MAGIC)
            .expect("utoken: bad Gemma vocabulary magic");
        let piece_count = read_u32(&mut rest);
        let mut pieces: Vec<&[u8]> = Vec::with_capacity(piece_count);
        for _ in 0..piece_count {
            let end = rest
                .iter()
                .position(|&byte| byte == 0)
                .expect("utoken: Gemma piece is NUL-terminated");
            pieces.push(&rest[..end]);
            rest = &rest[end + 1..];
        }
        assert!(
            rest.is_empty(),
            "utoken: trailing bytes in Gemma vocabulary"
        );

        // Empty slots are pieces no count can use; they never get an id.
        let mut index: HashMap<&[u8], u32, Fx> =
            HashMap::with_capacity_and_hasher(pieces.len(), Fx::default());
        for (id, piece) in pieces.iter().enumerate() {
            if !piece.is_empty() {
                let id = u32::try_from(id).expect("utoken: Gemma id fits u32");
                assert!(
                    index.insert(piece, id).is_none(),
                    "utoken: duplicate Gemma piece"
                );
            }
        }
        let id_of = |piece: &[u8]| -> u32 {
            *index
                .get(piece)
                .unwrap_or_else(|| panic!("utoken: Gemma piece {piece:?} is missing"))
        };

        let bytes: [u32; 256] =
            std::array::from_fn(|byte| id_of(format!("<0x{byte:02X}>").as_bytes()));
        let space = id_of("\u{2581}".as_bytes());
        let ascii: [u32; 128] = std::array::from_fn(|c| match c {
            0x20 => space,
            _ => index.get([c as u8].as_slice()).copied().unwrap_or(bytes[c]),
        });
        let mut chars = HashMap::with_hasher(Fx::default());
        for (&piece, &id) in &index {
            if let Ok(text) = std::str::from_utf8(piece) {
                let mut it = text.chars();
                if let (Some(c), None) = (it.next(), it.next())
                    && !c.is_ascii()
                {
                    chars.insert(c as u32, id);
                }
            }
        }

        let merges_raw = zst::decode_all(merges_zst);
        let mut rest = merges_raw
            .strip_prefix(MERGES_MAGIC)
            .expect("utoken: bad Gemma merges magic");
        let merge_count = read_u32(&mut rest);
        let mut pairs = PairTable::new(pieces.len(), merge_count);
        let mut produces = Vec::with_capacity(merge_count);
        let mut is_byte = vec![false; pieces.len()];
        for &id in &bytes {
            is_byte[id as usize] = true;
        }
        let mut bytes_merge = false;
        let mut produced: i64 = 0;
        for rank in 0..merge_count {
            let delta = read_varint(&mut rest);
            // Zigzag: the low bit is the sign.
            produced += if delta & 1 == 0 {
                (delta >> 1) as i64
            } else {
                -((delta >> 1) as i64) - 1
            };
            let id = usize::try_from(produced).expect("utoken: Gemma merge id in range");
            let piece = pieces[id];
            let split = read_varint(&mut rest) as usize;
            let left = id_of(&piece[..split]);
            let right = id_of(&piece[split..]);
            bytes_merge |= is_byte[left as usize] || is_byte[right as usize];
            pairs.insert(left, right, rank as u32);
            produces.push(id as u32);
        }
        assert!(rest.is_empty(), "utoken: trailing bytes in Gemma merges");

        // Characters some piece puts right before `▁`: a `▁` after one
        // might merge with it, so no segment may start there. Checking every
        // piece, not only those a merge produces, only skips boundaries that
        // would have been safe, never keeps one a merge could cross.
        let mut joiners: Vec<u32> = vec![u32::from('\u{2581}')];
        for piece in &pieces {
            let text = std::str::from_utf8(piece).expect("utoken: Gemma piece is UTF-8");
            let mut it = text.chars().peekable();
            while let Some(c) = it.next() {
                if it.peek() == Some(&'\u{2581}') {
                    joiners.push(u32::from(c));
                }
            }
        }
        joiners.sort_unstable();
        joiners.dedup();
        let ascii_joins: [bool; 128] = std::array::from_fn(|c| {
            let fallback = ascii[c] == bytes[c] && c != 0x20;
            c == 0x20 || joiners.binary_search(&(c as u32)).is_ok() || (fallback && bytes_merge)
        });

        Self {
            ascii,
            ascii_joins,
            joiners,
            bytes_merge,
            chars,
            bytes,
            pairs,
            produces: produces.into_boxed_slice(),
            id: NEXT_TABLE_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Number of tokens `units` encodes to.
    pub(crate) fn count<U: Unit>(&self, units: &[U]) -> usize {
        let (mut symbols, starts) = self.symbols(units);
        MEMO.with_borrow_mut(|memo| {
            segments(&mut symbols, &starts)
                .map(|segment| memo.count(self, segment))
                .sum()
        })
    }

    /// Token ids `units` encodes to.
    pub(crate) fn encode<U: Unit>(&self, units: &[U]) -> Vec<u32> {
        let (mut symbols, starts) = self.symbols(units);
        let mut out = Vec::with_capacity(symbols.len());
        for segment in segments(&mut symbols, &starts) {
            let left = self.merge_segment(segment);
            out.extend_from_slice(&segment[..left]);
        }
        out
    }

    /// The count with the whole text merged as one word by the heap: no
    /// segments, no short-segment scan, no memo. Tests compare it with
    /// [`Gemma::count`] to check those shortcuts change nothing.
    #[cfg(test)]
    pub(crate) fn count_as_one_word<U: Unit>(&self, units: &[U]) -> usize {
        let (mut symbols, _) = self.symbols(units);
        self.merge_all(&mut symbols)
    }

    /// The text's starting symbols: one piece per character, or one byte
    /// token per UTF-8 byte of a character the vocabulary lacks. Also the
    /// symbol positions where a segment starts: a space whose preceding
    /// character no piece joins to `▁` (see [`Gemma::ascii_joins`]). No
    /// merge can cross such a boundary, so each segment merges on its own
    /// with the same result, and most segments are a single word.
    fn symbols<U: Unit>(&self, units: &[U]) -> (Vec<u32>, Vec<usize>) {
        let mut out = Vec::with_capacity(units.len());
        let mut starts = Vec::new();
        // The start of the text never needs a boundary.
        let mut previous_joins = true;
        let mut i = 0;
        while i < units.len() {
            if let Some(byte) = units[i].ascii() {
                if byte == b' ' && !previous_joins {
                    starts.push(out.len());
                }
                out.push(self.ascii[usize::from(byte)]);
                previous_joins = self.ascii_joins[usize::from(byte)];
                i += 1;
                continue;
            }
            let (c, len) = U::decode(units, i);
            i += len;
            let space = c == '\u{2581}';
            if space && !previous_joins {
                starts.push(out.len());
            }
            match self.chars.get(&(c as u32)) {
                Some(&id) => {
                    out.push(id);
                    previous_joins = space || self.joiners.binary_search(&(c as u32)).is_ok();
                }
                None => {
                    let mut buf = [0u8; 4];
                    out.extend(
                        c.encode_utf8(&mut buf)
                            .bytes()
                            .map(|byte| self.bytes[usize::from(byte)]),
                    );
                    previous_joins =
                        self.bytes_merge || self.joiners.binary_search(&(c as u32)).is_ok();
                }
            }
        }
        (out, starts)
    }

    /// Merge one segment in place, move the surviving tokens to its front,
    /// and return how many there are.
    fn merge_segment(&self, symbols: &mut [u32]) -> usize {
        if symbols.len() <= SHORT {
            return self.merge_short(symbols);
        }
        let left = self.merge_all(symbols);
        let mut kept = 0;
        for i in 0..symbols.len() {
            if symbols[i] != GONE {
                symbols[kept] = symbols[i];
                kept += 1;
            }
        }
        debug_assert_eq!(kept, left);
        left
    }

    /// [`Gemma::merge_all`] for a short segment: rescanning a few pair
    /// ranks after each merge is cheaper than a heap. Picks the same merge
    /// each step, the lowest rank and then the leftmost pair.
    fn merge_short(&self, symbols: &mut [u32]) -> usize {
        let mut len = symbols.len();
        if len < 2 {
            return len;
        }
        let rank = |left: u32, right: u32| self.pairs.get(left, right).unwrap_or(u32::MAX);
        // `ranks[i]` is the merge rank of `symbols[i]` with `symbols[i + 1]`.
        let mut ranks = [u32::MAX; SHORT];
        for i in 0..len - 1 {
            ranks[i] = rank(symbols[i], symbols[i + 1]);
        }
        loop {
            let mut best = u32::MAX;
            let mut at = 0;
            for (i, &r) in ranks[..len - 1].iter().enumerate() {
                if r < best {
                    best = r;
                    at = i;
                }
            }
            if best == u32::MAX {
                return len;
            }
            symbols[at] = self.produces[best as usize];
            symbols.copy_within(at + 2..len, at + 1);
            if at + 2 < len - 1 {
                ranks.copy_within(at + 2..len - 1, at + 1);
            }
            len -= 1;
            if at > 0 {
                ranks[at - 1] = rank(symbols[at - 1], symbols[at]);
            }
            if at + 1 < len {
                ranks[at] = rank(symbols[at], symbols[at + 1]);
            }
            if len < 2 {
                return len;
            }
        }
    }

    /// Apply every merge to `symbols` in place and return how many tokens
    /// remain. A merged-away symbol becomes [`GONE`]; the survivors keep
    /// their order.
    fn merge_all(&self, symbols: &mut [u32]) -> usize {
        let n = symbols.len();
        if n < 2 {
            return n;
        }
        let last = u32::try_from(n - 1).expect("utoken: text length fits u32");
        // The live neighbours of each position, as a doubly linked list.
        let mut next: Vec<u32> = (1..=last).chain([NONE]).collect();
        let mut prev: Vec<u32> = [NONE].into_iter().chain(0..last).collect();
        // A candidate merge is `rank << 32 | position of its left symbol`,
        // so the heap's minimum is the lowest rank, then the leftmost pair.
        let candidates: Vec<Reverse<u64>> = (0..n - 1)
            .filter_map(|pos| {
                let rank = self.pairs.get(symbols[pos], symbols[pos + 1])?;
                Some(Reverse(candidate(rank, pos)))
            })
            .collect();
        let mut heap = BinaryHeap::from(candidates);
        let mut remaining = n;
        while let Some(Reverse(top)) = heap.pop() {
            let rank = (top >> 32) as u32;
            let pos = (top & u64::from(u32::MAX)) as usize;
            if symbols[pos] == GONE || next[pos] == NONE {
                continue;
            }
            let right = next[pos] as usize;
            // A candidate is stale once either symbol has merged with
            // something else; the pair it named is then gone.
            if self.pairs.get(symbols[pos], symbols[right]) != Some(rank) {
                continue;
            }
            symbols[pos] = self.produces[rank as usize];
            symbols[right] = GONE;
            remaining -= 1;
            let after = next[right];
            next[pos] = after;
            if after != NONE {
                prev[after as usize] = pos as u32;
                if let Some(rank) = self.pairs.get(symbols[pos], symbols[after as usize]) {
                    heap.push(Reverse(candidate(rank, pos)));
                }
            }
            let before = prev[pos];
            if before != NONE
                && let Some(rank) = self.pairs.get(symbols[before as usize], symbols[pos])
            {
                heap.push(Reverse(candidate(rank, before as usize)));
            }
        }
        remaining
    }
}

#[inline]
fn candidate(rank: u32, pos: usize) -> u64 {
    u64::from(rank) << 32 | pos as u64
}

fn read_u32(data: &mut &[u8]) -> usize {
    let (head, rest) = data
        .split_first_chunk::<4>()
        .expect("utoken: truncated Gemma table");
    *data = rest;
    u32::from_le_bytes(*head) as usize
}

fn read_varint(data: &mut &[u8]) -> u64 {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let (&byte, rest) = data.split_first().expect("utoken: truncated Gemma varint");
        *data = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

/// Open-addressing map from a pair of symbol ids to its merge rank, packed
/// into one `u64` per slot: `left, right, rank` in `2 * id_bits + rank_bits`
/// bits. Gemma 4 needs 18 + 18 + 19 bits, so the 515k merges fit 8 MiB at a
/// load factor under one half; a general hash map takes about twice that.
struct PairTable {
    slots: Box<[u64]>,
    id_bits: u32,
    rank_bits: u32,
    /// `64 - log2(slots.len())`: the hash keeps its top bits.
    shift: u32,
}

impl PairTable {
    /// An empty slot. A packed entry uses at most 63 bits, so it never
    /// equals this.
    const EMPTY: u64 = u64::MAX;

    fn new(ids: usize, ranks: usize) -> Self {
        let id_bits = bits_for(ids);
        let rank_bits = bits_for(ranks);
        assert!(
            2 * id_bits + rank_bits <= 63,
            "utoken: Gemma pair table entry exceeds 63 bits"
        );
        let len = (ranks * 2).next_power_of_two().max(2);
        Self {
            slots: vec![Self::EMPTY; len].into_boxed_slice(),
            id_bits,
            rank_bits,
            shift: 64 - len.trailing_zeros(),
        }
    }

    #[inline]
    fn key(&self, left: u32, right: u32) -> u64 {
        u64::from(left) << self.id_bits | u64::from(right)
    }

    #[inline]
    fn home(&self, key: u64) -> usize {
        (key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> self.shift) as usize
    }

    fn insert(&mut self, left: u32, right: u32, rank: u32) {
        let key = self.key(left, right);
        let mask = self.slots.len() - 1;
        let mut slot = self.home(key);
        while self.slots[slot] != Self::EMPTY {
            assert_ne!(
                self.slots[slot] >> self.rank_bits,
                key,
                "utoken: duplicate Gemma merge"
            );
            slot = (slot + 1) & mask;
        }
        self.slots[slot] = key << self.rank_bits | u64::from(rank);
    }

    #[inline]
    fn get(&self, left: u32, right: u32) -> Option<u32> {
        // `GONE` and `NONE` never reach here: callers look up live symbols.
        let key = self.key(left, right);
        let mask = self.slots.len() - 1;
        let mut slot = self.home(key);
        loop {
            let entry = self.slots[slot];
            if entry == Self::EMPTY {
                return None;
            }
            if entry >> self.rank_bits == key {
                return Some((entry & ((1 << self.rank_bits) - 1)) as u32);
            }
            slot = (slot + 1) & mask;
        }
    }
}

/// Bits needed to store every value below `count`.
fn bits_for(count: usize) -> u32 {
    usize::BITS - count.saturating_sub(1).leading_zeros()
}

/// Firefox/rustc Fx hash, as in `bpe.rs`: the vocabulary index hashes 256k
/// short keys once per load, where `SipHash` would dominate.
#[derive(Default)]
struct FxHasher(u64);

type Fx = BuildHasherDefault<FxHasher>;

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        let mut h = self.0;
        let mut rest = bytes;
        while let Some((chunk, tail)) = rest.split_first_chunk::<8>() {
            h = (h.rotate_left(5) ^ u64::from_le_bytes(*chunk)).wrapping_mul(SEED);
            rest = tail;
        }
        for &byte in rest {
            h = (h.rotate_left(5) ^ u64::from(byte)).wrapping_mul(SEED);
        }
        self.0 = h;
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        self.0 = (self.0.rotate_left(5) ^ u64::from(value)).wrapping_mul(SEED);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}
