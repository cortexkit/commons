mod nested;

mod inline {
    pub(super) fn emit() {
        tracing::info!(ok = true, "inline event");
    }
}

pub fn emit() {
    nested::emit();
    inline::emit();
}
