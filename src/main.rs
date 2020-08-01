extern crate codespan_reporting; // Tested with codespan_reporting v0.9.5
extern crate salsa; // Tested with salsa v0.15.1

use codespan_reporting::{
    diagnostic::{Diagnostic, Label, LabelStyle},
    files::Files,
    term::{
        self,
        termcolor::{ColorChoice, StandardStream},
        Config,
    },
};
use std::{cmp::Ordering, fmt, ops::Range, sync::Arc};

fn main() {
    let mut database = Database::default();
    database.set_file_name(FileId(0), Arc::new("crime.rs".to_owned()));
    database.set_source_text(FileId(0), Arc::new(include_str!("main.rs").to_owned()));

    database.parse(FileId(0));
}

// A standard salsa database to hold all our query info
#[salsa::database(SourceDatabaseStorage, ParseDatabaseStorage)]
#[derive(Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
}

impl salsa::Database for Database {}

// Implement upcasting for the main database into every query group it holds
impl Upcast<dyn SourceDatabase> for Database {
    fn upcast(&self) -> &dyn SourceDatabase {
        &*self
    }
}

impl Upcast<dyn ParseDatabase> for Database {
    fn upcast(&self) -> &dyn ParseDatabase {
        &*self
    }
}

// This is the key trait here, it allows all of our trait shenanigans
pub trait Upcast<T: ?Sized> {
    fn upcast(&self) -> &T;
}

// If we want to be able to use `&dyn ParseDatabase` for rendering errors we must have `Upcast<dyn SourceDatabase>` as a supertrait
#[salsa::query_group(ParseDatabaseStorage)]
pub trait ParseDatabase: salsa::Database + SourceDatabase + Upcast<dyn SourceDatabase> {
    // Salsa currently doesn't allow returning unit in a non-explicit way, see https://github.com/salsa-rs/salsa/issues/149
    fn parse(&self, file: FileId) -> ();
}

// Right now all this does is emit an error, but that's just an example
fn parse(db: &dyn ParseDatabase, file: FileId) {
    let writer = StandardStream::stderr(ColorChoice::Auto);
    let config = Config::default();

    let diag = Diagnostic::error()
        .with_message("This is a crime")
        .with_labels(vec![Label::new(
            LabelStyle::Primary,
            FileId(0),
            db.line_range(file, 14).unwrap().start..db.line_range(file, 20).unwrap().end - 1,
        )]);

    // Using `FileCache::upcast` we can take anything that implements `Upcast<dyn SourceDatabase` and use it for emitting errors
    term::emit(&mut writer.lock(), &config, &FileCache::upcast(db), &diag).unwrap();
}

/// The database that holds all source files
#[salsa::query_group(SourceDatabaseStorage)]
pub trait SourceDatabase: salsa::Database {
    /// Get the name of a source file
    #[salsa::input]
    fn file_name(&self, file: FileId) -> Arc<String>;

    /// The source text of a file
    #[salsa::input]
    fn source_text(&self, file: FileId) -> Arc<String>;

    /// The length of a source file
    fn source_length(&self, file: FileId) -> usize;

    /// The indices of every line start for the file
    fn line_starts(&self, file: FileId) -> Arc<Vec<usize>>;

    /// The index a line starts at
    fn line_start(&self, file: FileId, line_index: usize) -> Option<usize>;

    /// The line which a byte index falls on
    fn line_index(&self, file: FileId, byte_index: usize) -> Option<usize>;

    /// The range of a single line
    fn line_range(&self, file: FileId, line_index: usize) -> Option<Range<usize>>;
}

fn source_length(db: &dyn SourceDatabase, file: FileId) -> usize {
    db.source_text(file).len()
}

fn line_starts(db: &dyn SourceDatabase, file: FileId) -> Arc<Vec<usize>> {
    Arc::new(
        core::iter::once(0)
            .chain(db.source_text(file).match_indices('\n').map(|(i, _)| i + 1))
            .collect(),
    )
}

fn line_start(db: &dyn SourceDatabase, file: FileId, line_index: usize) -> Option<usize> {
    let line_starts = db.line_starts(file);

    match line_index.cmp(&line_starts.len()) {
        Ordering::Less => line_starts.get(line_index).cloned(),
        Ordering::Equal => Some(db.source_length(file)),
        Ordering::Greater => None,
    }
}

fn line_index(db: &dyn SourceDatabase, file: FileId, byte_index: usize) -> Option<usize> {
    match db.line_starts(file).binary_search(&byte_index) {
        Ok(line) => Some(line),
        Err(next_line) => Some(next_line - 1),
    }
}

fn line_range(db: &dyn SourceDatabase, file: FileId, line_index: usize) -> Option<Range<usize>> {
    let start = db.line_start(file, line_index)?;
    let end = db.line_start(file, line_index + 1)?;

    Some(start..end)
}

#[derive(Copy, Clone)]
pub struct FileCache<'a> {
    // The advantage of using `SourceDatabase` for rendering errors is that things are done on-demand.
    // Calculating line ranges and indices can be expensive, especially with lots of large files, and
    // using queries means that things are only calculated when they're needed and then cached from then on,
    // making further uses essentially free
    source: &'a dyn SourceDatabase,
}

impl<'a> FileCache<'a> {
    pub fn new(source: &'a dyn SourceDatabase) -> Self {
        Self { source }
    }

    pub fn upcast<T>(source: &'a T) -> Self
    where
        T: Upcast<dyn SourceDatabase> + ?Sized,
    {
        Self::new(source.upcast())
    }
}

// Note that most methods here will make salsa panic if the requested file hasn't been input yet
impl<'a> Files<'a> for FileCache<'a> {
    type FileId = FileId;
    // The owning here isn't ideal, but `Arc<String>` doesn't implement `Display` or `AsRef<str>`
    type Name = String;
    type Source = String;

    fn name(&self, file: FileId) -> Option<String> {
        Some(self.source.file_name(file).as_ref().clone())
    }

    fn source(&self, file: FileId) -> Option<String> {
        Some(self.source.source_text(file).as_ref().clone())
    }

    fn line_index(&self, file: FileId, byte_index: usize) -> Option<usize> {
        self.source.line_index(file, byte_index)
    }

    fn line_range(&self, file: FileId, line_index: usize) -> Option<Range<usize>> {
        self.source.line_range(file, line_index)
    }
}

impl fmt::Debug for FileCache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileCache").finish()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct FileId(pub u32);
