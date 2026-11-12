//! A tool part boxes its attachments, approval and delivered text only when
//! it has one of them. Edits that leave all three empty allocate nothing.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use cortexkit_role_compaction::part::{ToolPart, ToolStatus};
use cortexkit_role_compaction::tokens::TokenCount;
use serde_json::json;

/// The system allocator, counting the allocations each thread makes.
struct Counting;

#[global_allocator]
static ALLOC: Counting = Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn counted() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: every call is forwarded to `System` with the same arguments; the
// counting touches only a thread-local cell and never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        counted();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        counted();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        counted();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn allocations_in(f: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

fn plain() -> ToolPart {
    ToolPart::new("call_1", "read", json!({}), ToolStatus::Completed)
}

#[test]
fn editing_delivered_text_in_place_on_a_part_without_any_allocates_nothing() {
    let mut tool = plain();
    let allocations = allocations_in(|| {
        for delivered in tool.delivered_mut() {
            delivered.tokens = Some(TokenCount::new(1));
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(tool.delivered(), None);
}

#[test]
fn setting_the_boxed_fields_to_empty_allocates_nothing() {
    let mut tool = plain();
    let allocations = allocations_in(|| {
        tool.set_attachments(Vec::new());
        tool.set_approval(None);
        tool.set_delivered(Vec::new());
    });
    assert_eq!(allocations, 0);
    let fresh = plain();
    let allocations = allocations_in(|| {
        let built = fresh
            .with_attachments(Vec::new())
            .with_delivered(Vec::new());
        assert!(built.attachments().is_empty());
        assert_eq!(built.delivered(), None);
    });
    assert_eq!(allocations, 0);
}

/// Building and decoding an input check its text but compute no digest. The
/// first `canonical_digest` computes it; later reads, and clones, allocate
/// nothing for it.
#[test]
fn the_canonical_digest_is_computed_once_on_first_use_and_cloned_with_the_input() {
    use cortexkit_role_compaction::part::ToolInput;

    let text = r#"{"path":"src/lib.rs","offset":120,"limit":200}"#;
    let mut input = None;
    // One allocation: the shared buffer the text is copied into.
    assert_eq!(
        allocations_in(|| input = Some(ToolInput::try_from(text).unwrap())),
        1
    );
    let input = input.unwrap();
    let wire = format!("[{text}]");
    let mut decoded = None;
    // The list, the raw text and the shared buffer.
    assert_eq!(
        allocations_in(|| decoded = Some(serde_json::from_str::<Vec<ToolInput>>(&wire).unwrap())),
        3
    );
    assert!(allocations_in(|| drop(input.canonical_digest())) > 0);
    assert_eq!(allocations_in(|| drop(input.canonical_digest())), 0);
    assert_eq!(
        allocations_in(|| {
            let clone = input.clone();
            assert_eq!(clone.canonical_digest(), input.canonical_digest());
        }),
        0
    );
    assert_eq!(decoded.unwrap()[0], input);
}
