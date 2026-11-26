#[path = "same_dir.rs"]
mod nearby;
mod inline {
    #[path = "inside.rs"]
    mod from_file;
}
