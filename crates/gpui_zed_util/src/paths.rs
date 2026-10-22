use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use itertools::Itertools;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::mem;
use std::path::StripPrefixError;
use std::sync::Arc;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use path::rel_path::RelPath;
use path::rel_path::RelPathBuf;

pub use path::PathStyle;

/// Returns the path to the user's home directory.
#[cfg(not(target_family = "wasm"))]
pub fn home_dir() -> &'static PathBuf {
    static HOME_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    HOME_DIR.get_or_init(|| {
        if cfg!(any(test, feature = "test-support")) {
            if cfg!(target_os = "macos") {
                PathBuf::from("/Users/zed")
            } else if cfg!(target_os = "windows") {
                PathBuf::from("C:\\Users\\zed")
            } else {
                PathBuf::from("/home/zed")
            }
        } else {
            dirs::home_dir().expect("failed to determine home directory")
        }
    })
}

pub trait PathExt {
    /// On Linux and macOS, replaces the user's home directory prefix with `~`.
    /// Returns the original path if the prefix is absent or on other platforms.
    fn compact(&self) -> PathBuf;

    /// Returns a file's extension or, if the file is hidden, its name without the leading dot
    fn extension_or_hidden_file_name(&self) -> Option<&str>;

    fn try_from_bytes<'a>(bytes: &'a [u8]) -> anyhow::Result<Self>
    where
        Self: From<&'a Path>,
    {
        #[cfg(target_family = "wasm")]
        {
            std::str::from_utf8(bytes)
                .map(Path::new)
                .map(Into::into)
                .map_err(Into::into)
        }
        #[cfg(unix)]
        {
            use std::os::unix::prelude::OsStrExt;
            Ok(Self::from(Path::new(OsStr::from_bytes(bytes))))
        }
        #[cfg(windows)]
        {
            use anyhow::Context;
            use tendril::fmt::{Format, WTF8};
            WTF8::validate(bytes)
                .then(|| {
                    // Safety: bytes are valid WTF-8 sequence.
                    Self::from(Path::new(unsafe {
                        OsStr::from_encoded_bytes_unchecked(bytes)
                    }))
                })
                .with_context(|| format!("Invalid WTF-8 sequence: {bytes:?}"))
        }
    }

    /// Converts a local path to one that can be used inside of WSL.
    /// Returns `None` if the path cannot be converted into a WSL one (network share).
    fn local_to_wsl(&self) -> Option<PathBuf>;

    /// Returns joined extensions, such as `tar.gz` for `File.tar.gz` or
    /// `stories.tsx` for `Component.stories.tsx`.
    fn multiple_extensions(&self) -> Option<String>;

    /// Try to make a shell-safe representation of the path.
    #[cfg(not(target_family = "wasm"))]
    fn try_shell_safe(&self, shell_kind: crate::shell::ShellKind) -> anyhow::Result<String>;
}

impl<T: AsRef<Path>> PathExt for T {
    fn compact(&self) -> PathBuf {
        #[cfg(target_family = "wasm")]
        {
            self.as_ref().to_path_buf()
        }
        #[cfg(not(target_family = "wasm"))]
        if cfg!(any(target_os = "linux", target_os = "freebsd")) || cfg!(target_os = "macos") {
            match self.as_ref().strip_prefix(home_dir().as_path()) {
                Ok(relative_path) => {
                    let mut shortened_path = PathBuf::new();
                    shortened_path.push("~");
                    shortened_path.push(relative_path);
                    shortened_path
                }
                Err(_) => self.as_ref().to_path_buf(),
            }
        } else {
            self.as_ref().to_path_buf()
        }
    }

    fn extension_or_hidden_file_name(&self) -> Option<&str> {
        let path = self.as_ref();
        let file_name = path.file_name()?.to_str()?;
        if file_name.starts_with('.') {
            return file_name.strip_prefix('.');
        }

        path.extension()
            .and_then(|e| e.to_str())
            .or_else(|| path.file_stem()?.to_str())
    }

    fn local_to_wsl(&self) -> Option<PathBuf> {
        // quite sketchy to convert this back to path at the end, but a lot of functions only accept paths
        // todo: ideally rework them..?
        let mut new_path = std::ffi::OsString::new();
        for component in self.as_ref().components() {
            match component {
                std::path::Component::Prefix(prefix) => {
                    let drive_letter = prefix.as_os_str().to_string_lossy().to_lowercase();
                    let drive_letter = drive_letter.strip_suffix(':')?;

                    new_path.push(format!("/mnt/{}", drive_letter));
                }
                std::path::Component::RootDir => {}
                std::path::Component::CurDir => {
                    new_path.push("/.");
                }
                std::path::Component::ParentDir => {
                    new_path.push("/..");
                }
                std::path::Component::Normal(os_str) => {
                    new_path.push("/");
                    new_path.push(os_str);
                }
            }
        }

        Some(new_path.into())
    }

    fn multiple_extensions(&self) -> Option<String> {
        let path = self.as_ref();
        let file_name = path.file_name()?.to_str()?;

        let parts: Vec<&str> = file_name
            .split('.')
            // Skip the part with the file name extension
            .skip(1)
            .collect();

        if parts.len() < 2 {
            return None;
        }

        Some(parts.into_iter().join("."))
    }

    #[cfg(not(target_family = "wasm"))]
    fn try_shell_safe(&self, shell_kind: crate::shell::ShellKind) -> anyhow::Result<String> {
        use anyhow::Context;
        let path_str = self
            .as_ref()
            .to_str()
            .with_context(|| "Path contains invalid UTF-8")?;
        shell_kind
            .try_quote(path_str)
            .as_deref()
            .map(ToOwned::to_owned)
            .context("Failed to quote path")
    }
}

pub fn path_ends_with(base: &Path, suffix: &Path) -> bool {
    strip_path_suffix(base, suffix).is_some()
}

/// Case-insensitive ASCII comparison of a path component to a literal
/// folder name. macOS and Windows use case-insensitive filesystems by
/// default, so a path like `.ZED/settings.json` resolves to the same
/// inode as the lowercase form. A case-sensitive `==` check would miss
/// those and let a malicious settings author bypass classifiers with
/// unusual casing. Callers should restrict `name` to ASCII; for ASCII
/// inputs `eq_ignore_ascii_case` is safe and stable across platforms.
pub fn component_matches_ignore_ascii_case(component: &OsStr, name: &str) -> bool {
    component
        .to_str()
        .is_some_and(|s| s.eq_ignore_ascii_case(name))
}

pub fn strip_path_suffix<'a>(base: &'a Path, suffix: &Path) -> Option<&'a Path> {
    if let Some(remainder) = base
        .as_os_str()
        .as_encoded_bytes()
        .strip_suffix(suffix.as_os_str().as_encoded_bytes())
        && remainder
            .last()
            .is_none_or(|last_byte| std::path::is_separator(*last_byte as char))
    {
        let os_str = unsafe {
            OsStr::from_encoded_bytes_unchecked(&remainder[0..remainder.len().saturating_sub(1)])
        };
        return Some(Path::new(os_str));
    }
    None
}

/// In memory, this is identical to `Path`. On non-Windows conversions to this type are no-ops. On
/// windows, these conversions sanitize UNC paths by removing the `\\\\?\\` prefix.
#[derive(Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct SanitizedPath(Path);

impl SanitizedPath {
    pub fn new<T: AsRef<Path> + ?Sized>(path: &T) -> &Self {
        #[cfg(not(target_os = "windows"))]
        return Self::unchecked_new(path.as_ref());

        #[cfg(target_os = "windows")]
        return Self::unchecked_new(dunce::simplified(path.as_ref()));
    }

    pub fn unchecked_new<T: AsRef<Path> + ?Sized>(path: &T) -> &Self {
        // safe because `Path` and `SanitizedPath` have the same repr and Drop impl
        unsafe { mem::transmute::<&Path, &Self>(path.as_ref()) }
    }

    pub fn from_arc(path: Arc<Path>) -> Arc<Self> {
        // safe because `Path` and `SanitizedPath` have the same repr and Drop impl
        #[cfg(not(target_os = "windows"))]
        return unsafe { mem::transmute::<Arc<Path>, Arc<Self>>(path) };

        #[cfg(target_os = "windows")]
        {
            let simplified = dunce::simplified(path.as_ref());
            if simplified == path.as_ref() {
                // safe because `Path` and `SanitizedPath` have the same repr and Drop impl
                unsafe { mem::transmute::<Arc<Path>, Arc<Self>>(path) }
            } else {
                Self::unchecked_new(simplified).into()
            }
        }
    }

    pub fn new_arc<T: AsRef<Path> + ?Sized>(path: &T) -> Arc<Self> {
        Self::new(path).into()
    }

    pub fn cast_arc(path: Arc<Self>) -> Arc<Path> {
        // safe because `Path` and `SanitizedPath` have the same repr and Drop impl
        unsafe { mem::transmute::<Arc<Self>, Arc<Path>>(path) }
    }

    pub fn cast_arc_ref(path: &Arc<Self>) -> &Arc<Path> {
        // safe because `Path` and `SanitizedPath` have the same repr and Drop impl
        unsafe { mem::transmute::<&Arc<Self>, &Arc<Path>>(path) }
    }

    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.0.starts_with(&prefix.0)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn file_name(&self) -> Option<&std::ffi::OsStr> {
        self.0.file_name()
    }

    pub fn extension(&self) -> Option<&std::ffi::OsStr> {
        self.0.extension()
    }

    pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.0.join(path)
    }

    pub fn parent(&self) -> Option<&Self> {
        self.0.parent().map(Self::unchecked_new)
    }

    pub fn strip_prefix(&self, base: &Self) -> Result<&Path, StripPrefixError> {
        self.0.strip_prefix(base.as_path())
    }

    pub fn to_str(&self) -> Option<&str> {
        self.0.to_str()
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.0.to_path_buf()
    }
}

impl std::fmt::Debug for SanitizedPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, formatter)
    }
}

impl Display for SanitizedPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl From<&SanitizedPath> for Arc<SanitizedPath> {
    fn from(sanitized_path: &SanitizedPath) -> Self {
        let path: Arc<Path> = sanitized_path.0.into();
        // safe because `Path` and `SanitizedPath` have the same repr and Drop impl
        unsafe { mem::transmute(path) }
    }
}

impl From<&SanitizedPath> for PathBuf {
    fn from(sanitized_path: &SanitizedPath) -> Self {
        sanitized_path.as_path().into()
    }
}

impl AsRef<Path> for SanitizedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct RemotePathBuf {
    style: PathStyle,
    string: String,
}

impl RemotePathBuf {
    pub fn new(string: String, style: PathStyle) -> Self {
        Self { style, string }
    }

    pub fn from_str(path: &str, style: PathStyle) -> Self {
        Self::new(path.to_string(), style)
    }

    pub fn path_style(&self) -> PathStyle {
        self.style
    }

    pub fn to_proto(&self) -> String {
        self.string.clone()
    }
}

impl Display for RemotePathBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.string)
    }
}

pub fn is_absolute(path_like: &str, path_style: PathStyle) -> bool {
    path_like.starts_with('/')
        || path_style == PathStyle::Windows
            && (path_like.starts_with('\\')
                || path_like
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                    && path_like[1..]
                        .strip_prefix(':')
                        .is_some_and(|path| path.starts_with('/') || path.starts_with('\\')))
}

#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct NormalizeError;

impl Error for NormalizeError {}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("parent reference `..` points outside of base directory")
    }
}

/// Copied from stdlib where it's unstable.
///
/// Normalize a path, including `..` without traversing the filesystem.
///
/// Returns an error if normalization would leave leading `..` components.
///
/// <div class="warning">
///
/// This function always resolves `..` to the "lexical" parent.
/// That is "a/b/../c" will always resolve to `a/c` which can change the meaning of the path.
/// In particular, `a/c` and `a/b/../c` are distinct on many systems because `b` may be a symbolic link, so its parent isn't `a`.
///
/// </div>
///
/// [`path::absolute`](absolute) is an alternative that preserves `..`.
/// Or [`Path::canonicalize`] can be used to resolve any `..` by querying the filesystem.
pub fn normalize_lexically(path: &Path) -> Result<PathBuf, NormalizeError> {
    use std::path::Component;

    let mut lexical = PathBuf::new();
    let mut iter = path.components().peekable();

    // Find the root, if any, and add it to the lexical path.
    // Here we treat the Windows path "C:\" as a single "root" even though
    // `components` splits it into two: (Prefix, RootDir).
    let root = match iter.peek() {
        Some(Component::ParentDir) => return Err(NormalizeError),
        Some(p @ Component::RootDir) | Some(p @ Component::CurDir) => {
            lexical.push(p);
            iter.next();
            lexical.as_os_str().len()
        }
        Some(Component::Prefix(prefix)) => {
            lexical.push(prefix.as_os_str());
            iter.next();
            if let Some(p @ Component::RootDir) = iter.peek() {
                lexical.push(p);
                iter.next();
            }
            lexical.as_os_str().len()
        }
        None => return Ok(PathBuf::new()),
        Some(Component::Normal(_)) => 0,
    };

    for component in iter {
        match component {
            Component::RootDir => unreachable!(),
            Component::Prefix(_) => return Err(NormalizeError),
            Component::CurDir => continue,
            Component::ParentDir => {
                // It's an error if ParentDir causes us to go above the "root".
                if lexical.as_os_str().len() == root {
                    return Err(NormalizeError);
                } else {
                    lexical.pop();
                }
            }
            Component::Normal(path) => lexical.push(path),
        }
    }
    Ok(lexical)
}

/// Insert `path` into a set of "subtree" grants, keeping the set minimal.
///
/// A subtree grant covers a path and all of its descendants. Insertion is a
/// no-op when `path` is already covered by an existing (equal-or-broader)
/// entry; otherwise `path` is added and any now-subsumed descendant entries
/// are pruned. Containment is purely lexical (component-wise `starts_with`),
/// so callers should normalize paths (e.g. via [`normalize_lexically`]) before
/// inserting, otherwise `..` components can defeat the containment checks.
pub fn insert_subtree(subtrees: &mut Vec<PathBuf>, path: PathBuf) {
    if subtrees.iter().any(|existing| path.starts_with(existing)) {
        return;
    }
    subtrees.retain(|existing| !existing.starts_with(&path));
    subtrees.push(path);
}

/// Whether `path` sits under (or exactly equals) any of the given subtree
/// grants. As with [`insert_subtree`], containment is purely lexical, so
/// callers should pass normalized paths.
pub fn path_within_subtree<'a>(path: &Path, mut subtrees: impl Iterator<Item = &'a Path>) -> bool {
    subtrees.any(|granted| path.starts_with(granted))
}

/// A delimiter to use in `path_query:row_number:column_number` strings parsing.
pub const FILE_ROW_COLUMN_DELIMITER: char = ':';

const ROW_COL_CAPTURE_REGEX: &str = r"(?xs)
    ([^\(]+)\:(?:
        \((\d+)[,:](\d+)\) # filename:(row,column), filename:(row:column)
        |
        \((\d+)\)()     # filename:(row)
    )
    |
    ([^\(]+)(?:
        \((\d+)[,:](\d+)\) # filename(row,column), filename(row:column)
        |
        \((\d+)\)()     # filename(row)
    )
    \:*$
    |
    (.+?)(?:
        \:+(\d+)\:(\d+)\:*$  # filename:row:column
        |
        \:+(\d+)\:*()$       # filename:row
        |
        \:+()()$
    )";

/// A representation of a path-like string with optional row and column numbers.
/// Matching values example: `te`, `test.rs:22`, `te:22:5`, `test.c(22)`, `test.c(22,5)`etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct PathWithPosition {
    pub path: PathBuf,
    pub row: Option<u32>,
    // Absent if row is absent.
    pub column: Option<u32>,
}

impl PathWithPosition {
    /// Returns a PathWithPosition from a path.
    pub fn from_path(path: PathBuf) -> Self {
        Self {
            path,
            row: None,
            column: None,
        }
    }

    /// Parses a string that possibly has `:row:column` or `(row, column)` suffix.
    /// Parenthesis format is used by [MSBuild](https://learn.microsoft.com/en-us/visualstudio/msbuild/msbuild-diagnostic-format-for-tasks) compatible tools
    /// Ignores trailing `:`s, so `test.rs:22:` is parsed as `test.rs:22`.
    /// If the suffix parsing fails, the whole string is parsed as a path.
    ///
    /// Be mindful that `test_file:10:1:` is a valid posix filename.
    /// `PathWithPosition` class assumes that the ending position-like suffix is **not** part of the filename.
    ///
    /// # Examples
    ///
    /// ```
    /// # use util::paths::PathWithPosition;
    /// # use std::path::PathBuf;
    /// assert_eq!(PathWithPosition::parse_str("test_file"), PathWithPosition {
    ///     path: PathBuf::from("test_file"),
    ///     row: None,
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file:10"), PathWithPosition {
    ///     path: PathBuf::from("test_file"),
    ///     row: Some(10),
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: None,
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:1"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: Some(1),
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:1:2"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: Some(1),
    ///     column: Some(2),
    /// });
    /// ```
    ///
    /// # Expected parsing results when encounter ill-formatted inputs.
    /// ```
    /// # use util::paths::PathWithPosition;
    /// # use std::path::PathBuf;
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:a"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs:a"),
    ///     row: None,
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:a:b"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs:a:b"),
    ///     row: None,
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: None,
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs::1"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: Some(1),
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:1::"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: Some(1),
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs::1:2"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs"),
    ///     row: Some(1),
    ///     column: Some(2),
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:1::2"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs:1"),
    ///     row: Some(2),
    ///     column: None,
    /// });
    /// assert_eq!(PathWithPosition::parse_str("test_file.rs:1:2:3"), PathWithPosition {
    ///     path: PathBuf::from("test_file.rs:1"),
    ///     row: Some(2),
    ///     column: Some(3),
    /// });
    /// ```
    pub fn parse_str(s: &str) -> Self {
        let trimmed = s.trim();
        let path = Path::new(trimmed);
        let Some(maybe_file_name_with_row_col) = path.file_name().unwrap_or_default().to_str()
        else {
            return Self {
                path: Path::new(s).to_path_buf(),
                row: None,
                column: None,
            };
        };
        if maybe_file_name_with_row_col.is_empty() {
            return Self {
                path: Path::new(s).to_path_buf(),
                row: None,
                column: None,
            };
        }

        // Reuse initialization. Thread contention is possible, but this is not currently
        // called from multiple hot paths, so contention seems unlikely.
        static SUFFIX_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(ROW_COL_CAPTURE_REGEX).unwrap());
        match SUFFIX_RE
            .captures(maybe_file_name_with_row_col)
            .map(|caps| caps.extract())
        {
            Some((_, [file_name, maybe_row, maybe_column])) => {
                let row = maybe_row.parse::<u32>().ok();
                let column = maybe_column.parse::<u32>().ok();

                let (_, suffix) = trimmed.split_once(file_name).unwrap();
                let path_without_suffix = &trimmed[..trimmed.len() - suffix.len()];

                Self {
                    path: Path::new(path_without_suffix).to_path_buf(),
                    row,
                    column,
                }
            }
            None => {
                // Handle suffixes such as `foo/bar.py:22:in` outside the digits-only regex.
                let delimiter = ':';
                let mut path_parts = s
                    .rsplitn(3, delimiter)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .fuse();
                let mut path_string = path_parts.next().expect("rsplitn should have the rest of the string as its last parameter that we reversed").to_owned();
                let mut row = None;
                let mut column = None;
                if let Some(maybe_row) = path_parts.next() {
                    if let Ok(parsed_row) = maybe_row.parse::<u32>() {
                        row = Some(parsed_row);
                        if let Some(parsed_column) = path_parts
                            .next()
                            .and_then(|maybe_col| maybe_col.parse::<u32>().ok())
                        {
                            column = Some(parsed_column);
                        }
                    } else {
                        path_string.push(delimiter);
                        path_string.push_str(maybe_row);
                    }
                }
                for split in path_parts {
                    path_string.push(delimiter);
                    path_string.push_str(split);
                }

                Self {
                    path: PathBuf::from(path_string),
                    row,
                    column,
                }
            }
        }
    }

    pub fn map_path<E>(
        self,
        mapping: impl FnOnce(PathBuf) -> Result<PathBuf, E>,
    ) -> Result<PathWithPosition, E> {
        Ok(PathWithPosition {
            path: mapping(self.path)?,
            row: self.row,
            column: self.column,
        })
    }

    pub fn to_string(&self, path_to_string: &dyn Fn(&PathBuf) -> String) -> String {
        let path_string = path_to_string(&self.path);
        if let Some(row) = self.row {
            if let Some(column) = self.column {
                format!("{path_string}:{row}:{column}")
            } else {
                format!("{path_string}:{row}")
            }
        } else {
            path_string
        }
    }
}

#[derive(Clone)]
pub struct PathMatcher {
    sources: Vec<(String, RelPathBuf, /*trailing separator*/ bool)>,
    glob: GlobSet,
    path_style: PathStyle,
}

impl std::fmt::Debug for PathMatcher {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathMatcher")
            .field("sources", &self.sources)
            .field("path_style", &self.path_style)
            .finish()
    }
}

impl PartialEq for PathMatcher {
    fn eq(&self, other: &Self) -> bool {
        self.sources.eq(&other.sources)
    }
}

impl Eq for PathMatcher {}

impl PathMatcher {
    pub fn new(
        globs: impl IntoIterator<Item = impl AsRef<str>>,
        path_style: PathStyle,
    ) -> Result<Self, globset::Error> {
        let globs = globs
            .into_iter()
            .map(|as_str| {
                GlobBuilder::new(as_str.as_ref())
                    .backslash_escape(path_style.is_posix())
                    .build()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sources = globs
            .iter()
            .filter_map(|glob| {
                let glob = glob.glob();
                Some((
                    glob.to_string(),
                    RelPath::new(glob.as_ref(), path_style)
                        .ok()
                        .map(std::borrow::Cow::into_owned)?,
                    glob.ends_with(path_style.separators_ch()),
                ))
            })
            .collect();
        let mut glob_builder = GlobSetBuilder::new();
        for single_glob in globs {
            glob_builder.add(single_glob);
        }
        let glob = glob_builder.build()?;
        Ok(PathMatcher {
            glob,
            sources,
            path_style,
        })
    }

    pub fn sources(&self) -> impl Iterator<Item = &str> + Clone {
        self.sources.iter().map(|(source, ..)| source.as_str())
    }

    pub fn is_match<P: AsRef<RelPath>>(&self, other: P) -> bool {
        let other = other.as_ref();
        if self
            .sources
            .iter()
            .any(|(_, source, _)| other.starts_with(source) || other.ends_with(source))
        {
            return true;
        }
        let other_path = other.display(self.path_style);

        if self.glob.is_match(&*other_path) {
            return true;
        }

        self.glob
            .is_match(other_path.into_owned() + self.path_style.primary_separator())
    }

    pub fn is_match_std_path<P: AsRef<Path>>(&self, other: P) -> bool {
        let other = other.as_ref();
        if self.sources.iter().any(|(_, source, _)| {
            other.starts_with(source.as_std_path()) || other.ends_with(source.as_std_path())
        }) {
            return true;
        }
        self.glob.is_match(other)
    }
}

impl Default for PathMatcher {
    fn default() -> Self {
        Self {
            path_style: PathStyle::local(),
            glob: GlobSet::empty(),
            sources: vec![],
        }
    }
}

/// Compares consecutive digit sequences by numeric value for natural sorting.
/// Equal values sort by length, so `"002" > "2"`. Values that overflow `u128`
/// fall back to string comparison.
///
/// # Examples
///
/// ```text
/// "1" vs "2"      -> Less       (different values)
/// "2" vs "10"     -> Less       (numeric comparison)
/// "002" vs "2"    -> Greater    (leading zeros)
/// "10" vs "010"   -> Less       (leading zeros)
/// "999..." vs "1000..." -> Less (large number comparison)
/// ```
///
/// Collects digit strings, comparing equal lengths lexically and differing lengths
/// numerically when they fit in `u128`. Advances both iterators past their digit
/// sequences regardless of the result.
fn compare_numeric_segments<I>(
    a_iter: &mut std::iter::Peekable<I>,
    b_iter: &mut std::iter::Peekable<I>,
) -> Ordering
where
    I: Iterator<Item = char>,
{
    let mut a_num_str = String::new();
    let mut b_num_str = String::new();

    while let Some(&c) = a_iter.peek() {
        if !c.is_ascii_digit() {
            break;
        }

        a_num_str.push(c);
        a_iter.next();
    }

    while let Some(&c) = b_iter.peek() {
        if !c.is_ascii_digit() {
            break;
        }

        b_num_str.push(c);
        b_iter.next();
    }

    match a_num_str.len().cmp(&b_num_str.len()) {
        Ordering::Equal => match a_num_str.cmp(&b_num_str) {
            Ordering::Equal => Ordering::Equal,
            ordering => ordering,
        },

        // Different lengths but same value means leading zeros
        ordering => {
            if let (Ok(a_val), Ok(b_val)) = (a_num_str.parse::<u128>(), b_num_str.parse::<u128>()) {
                match a_val.cmp(&b_val) {
                    Ordering::Equal => ordering, // Same value, longer one is greater (leading zeros)
                    ord => ord,
                }
            } else {
                // If parsing fails (overflow), compare as strings
                a_num_str.cmp(&b_num_str)
            }
        }
    }
}

/// Compares strings case-insensitively with consecutive digits treated as numbers,
/// so `"file2"` sorts before `"file10"`.
///
/// Equal numeric values sort by digit count, with longer sequences last.
/// Values above `u128::MAX` fall back to string comparison. Case breaks ties
/// only after all characters match case-insensitively, with lowercase first.
pub fn natural_sort(a: &str, b: &str) -> Ordering {
    let mut a_iter = a.chars().peekable();
    let mut b_iter = b.chars().peekable();

    loop {
        match (a_iter.peek(), b_iter.peek()) {
            (None, None) => {
                return b.cmp(a);
            }
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(&a_char), Some(&b_char)) => {
                if a_char.is_ascii_digit() && b_char.is_ascii_digit() {
                    match compare_numeric_segments(&mut a_iter, &mut b_iter) {
                        Ordering::Equal => continue,
                        ordering => return ordering,
                    }
                } else {
                    match a_char
                        .to_ascii_lowercase()
                        .cmp(&b_char.to_ascii_lowercase())
                    {
                        Ordering::Equal => {
                            a_iter.next();
                            b_iter.next();
                        }
                        ordering => return ordering,
                    }
                }
            }
        }
    }
}

/// Defers the case tie-breaker so deeper path components can decide the order first.
fn natural_sort_no_tiebreak(a: &str, b: &str) -> Ordering {
    if a.eq_ignore_ascii_case(b) {
        Ordering::Equal
    } else {
        natural_sort(a, b)
    }
}

fn stem_and_extension(filename: &str) -> (Option<&str>, Option<&str>) {
    if filename.is_empty() {
        return (None, None);
    }

    match filename.rsplit_once('.') {
        None => (Some(filename), None),

        Some((before, after)) => {
            // A dotfile such as ".bashrc" has no extension.
            if before.is_empty() {
                (Some(filename), None)
            } else {
                (Some(before), Some(after))
            }
        }
    }
}

/// Controls the lexicographic sorting of file and folder names.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    /// Case-insensitive natural sort with lowercase preferred in ties.
    /// Numbers in file names are compared by value (e.g., `file2` before `file10`).
    #[default]
    Default,
    /// Uppercase names are grouped before lowercase names, with case-insensitive
    /// natural sort within each group. Dot-prefixed names sort before both groups.
    Upper,
    /// Lowercase names are grouped before uppercase names, with case-insensitive
    /// natural sort within each group. Dot-prefixed names sort before both groups.
    Lower,
    /// Pure Unicode codepoint comparison. No case folding, no natural number sorting.
    /// Uppercase ASCII sorts before lowercase. Accented characters sort after ASCII.
    Unicode,
}

/// Controls how files and directories are ordered relative to each other.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SortMode {
    /// Directories are listed before files at each level.
    #[default]
    DirectoriesFirst,
    /// Files and directories are interleaved alphabetically.
    Mixed,
    /// Files are listed before directories at each level.
    FilesFirst,
}

fn case_group_key(name: &str, order: SortOrder) -> u8 {
    let first = match name.chars().next() {
        Some(c) => c,
        None => return 0,
    };
    match order {
        SortOrder::Upper if first.is_lowercase() => 1,
        SortOrder::Upper => 0,
        SortOrder::Lower if first.is_uppercase() => 1,
        SortOrder::Lower => 0,
        _ => 0,
    }
}

fn compare_strings(a: &str, b: &str, order: SortOrder) -> Ordering {
    match order {
        SortOrder::Unicode => a.cmp(b),
        _ => natural_sort(a, b),
    }
}

fn compare_strings_no_tiebreak(a: &str, b: &str, order: SortOrder) -> Ordering {
    match order {
        SortOrder::Unicode => a.cmp(b),
        _ => natural_sort_no_tiebreak(a, b),
    }
}

pub fn compare_rel_paths(
    (path_a, a_is_file): (&RelPath, bool),
    (path_b, b_is_file): (&RelPath, bool),
) -> Ordering {
    compare_rel_paths_by(
        (path_a, a_is_file),
        (path_b, b_is_file),
        SortMode::DirectoriesFirst,
        SortOrder::Default,
    )
}

pub fn compare_rel_paths_by(
    (path_a, a_is_file): (&RelPath, bool),
    (path_b, b_is_file): (&RelPath, bool),
    mode: SortMode,
    order: SortOrder,
) -> Ordering {
    let needs_final_tiebreak =
        mode != SortMode::DirectoriesFirst && !(std::ptr::eq(path_a, path_b) || path_a == path_b);

    let mut components_a = path_a.components();
    let mut components_b = path_b.components();

    loop {
        match (components_a.next(), components_b.next()) {
            (Some(component_a), Some(component_b)) => {
                let a_leaf_file = a_is_file && components_a.rest().is_empty();
                let b_leaf_file = b_is_file && components_b.rest().is_empty();

                let file_dir_ordering = match mode {
                    SortMode::DirectoriesFirst => a_leaf_file.cmp(&b_leaf_file),
                    SortMode::FilesFirst => b_leaf_file.cmp(&a_leaf_file),
                    SortMode::Mixed => Ordering::Equal,
                };

                if !file_dir_ordering.is_eq() {
                    return file_dir_ordering;
                }

                let (a_stem, a_ext) = if a_leaf_file {
                    stem_and_extension(component_a)
                } else {
                    Default::default()
                };
                let (b_stem, b_ext) = if b_leaf_file {
                    stem_and_extension(component_b)
                } else {
                    Default::default()
                };
                let a_key = if a_leaf_file {
                    a_stem
                } else {
                    Some(component_a)
                };
                let b_key = if b_leaf_file {
                    b_stem
                } else {
                    Some(component_b)
                };

                let ordering = match (a_key, b_key) {
                    (Some(a), Some(b)) => {
                        let name_cmp = case_group_key(a, order)
                            .cmp(&case_group_key(b, order))
                            .then_with(|| match mode {
                                SortMode::DirectoriesFirst => compare_strings(a, b, order),
                                _ => compare_strings_no_tiebreak(a, b, order),
                            });

                        let name_cmp = if mode == SortMode::Mixed {
                            name_cmp.then_with(|| match (a_leaf_file, b_leaf_file) {
                                (true, false) if a.eq_ignore_ascii_case(b) => Ordering::Greater,
                                (false, true) if a.eq_ignore_ascii_case(b) => Ordering::Less,
                                _ => Ordering::Equal,
                            })
                        } else {
                            name_cmp
                        };

                        name_cmp.then_with(|| {
                            if a_leaf_file && b_leaf_file {
                                match order {
                                    SortOrder::Unicode => {
                                        a_ext.unwrap_or_default().cmp(b_ext.unwrap_or_default())
                                    }
                                    _ => {
                                        let a_ext_str = a_ext.unwrap_or_default().to_lowercase();
                                        let b_ext_str = b_ext.unwrap_or_default().to_lowercase();
                                        a_ext_str.cmp(&b_ext_str)
                                    }
                                }
                            } else {
                                Ordering::Equal
                            }
                        })
                    }
                    (Some(_), None) => Ordering::Greater,
                    (None, Some(_)) => Ordering::Less,
                    (None, None) => Ordering::Equal,
                };

                if !ordering.is_eq() {
                    return ordering;
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => {
                if needs_final_tiebreak {
                    return compare_strings(path_a.as_unix_str(), path_b.as_unix_str(), order);
                }
                return Ordering::Equal;
            }
        }
    }
}

pub fn compare_paths(
    (path_a, a_is_file): (&Path, bool),
    (path_b, b_is_file): (&Path, bool),
) -> Ordering {
    let mut components_a = path_a.components().peekable();
    let mut components_b = path_b.components().peekable();

    loop {
        match (components_a.next(), components_b.next()) {
            (Some(component_a), Some(component_b)) => {
                let a_is_file = components_a.peek().is_none() && a_is_file;
                let b_is_file = components_b.peek().is_none() && b_is_file;

                let ordering = a_is_file.cmp(&b_is_file).then_with(|| {
                    let path_a = Path::new(component_a.as_os_str());
                    let path_string_a = if a_is_file {
                        path_a.file_stem()
                    } else {
                        path_a.file_name()
                    }
                    .map(|s| s.to_string_lossy());

                    let path_b = Path::new(component_b.as_os_str());
                    let path_string_b = if b_is_file {
                        path_b.file_stem()
                    } else {
                        path_b.file_name()
                    }
                    .map(|s| s.to_string_lossy());

                    let compare_components = match (path_string_a, path_string_b) {
                        (Some(a), Some(b)) => natural_sort(&a, &b),
                        (Some(_), None) => Ordering::Greater,
                        (None, Some(_)) => Ordering::Less,
                        (None, None) => Ordering::Equal,
                    };

                    compare_components.then_with(|| {
                        if a_is_file && b_is_file {
                            let ext_a = path_a.extension().unwrap_or_default();
                            let ext_b = path_b.extension().unwrap_or_default();
                            ext_a.cmp(ext_b)
                        } else {
                            Ordering::Equal
                        }
                    })
                });

                if !ordering.is_eq() {
                    return ordering;
                }
            }
            (Some(_), None) => break Ordering::Greater,
            (None, Some(_)) => break Ordering::Less,
            (None, None) => break Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslPath {
    pub distro: String,

    // Use OsString instead of a path type to represent a Unix path with `/`
    // separators on Windows. `from_path` constructs it from Windows path components.
    pub path: std::ffi::OsString,
}

impl WslPath {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Option<WslPath> {
        if cfg!(not(target_os = "windows")) {
            return None;
        }
        use std::{
            ffi::OsString,
            path::{Component, Prefix},
        };

        let mut components = path.as_ref().components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return None;
        };
        let (server, distro) = match prefix.kind() {
            Prefix::UNC(server, distro) => (server, distro),
            Prefix::VerbatimUNC(server, distro) => (server, distro),
            _ => return None,
        };
        let Some(Component::RootDir) = components.next() else {
            return None;
        };

        let server_str = server.to_string_lossy();
        if server_str == "wsl.localhost" || server_str == "wsl$" {
            let mut result = OsString::from("");
            for c in components {
                use Component::*;
                match c {
                    Prefix(p) => unreachable!("got {p:?}, but already stripped prefix"),
                    RootDir => unreachable!("got root dir, but already stripped root"),
                    CurDir => continue,
                    ParentDir => result.push("/.."),
                    Normal(s) => {
                        result.push("/");
                        result.push(s);
                    }
                }
            }
            if result.is_empty() {
                result.push("/");
            }
            Some(WslPath {
                distro: distro.to_string_lossy().to_string(),
                path: result,
            })
        } else {
            None
        }
    }
}

/// Error returned when a [`url::Url`] cannot be converted into a [`PathBuf`]
/// via [`UrlExt::to_file_path_ext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToFilePathError;

impl std::fmt::Display for ToFilePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("failed to convert URL to file path")
    }
}

impl std::error::Error for ToFilePathError {}

pub trait UrlExt {
    /// Like `url::Url::to_file_path`, but uses `PathStyle` instead of the host platform.
    /// Use for cross-platform paths, such as remoting interactions.
    fn to_file_path_ext(&self, path_style: PathStyle) -> Result<PathBuf, ToFilePathError>;
}

impl UrlExt for url::Url {
    // Copied from `url::Url::to_file_path`, but the `cfg` handling is replaced with runtime branching on `PathStyle`
    fn to_file_path_ext(&self, source_path_style: PathStyle) -> Result<PathBuf, ToFilePathError> {
        if let Some(segments) = self.path_segments() {
            let host = match self.host() {
                None | Some(url::Host::Domain("localhost")) => None,
                Some(_) if source_path_style.is_windows() && self.scheme() == "file" => {
                    self.host_str()
                }
                _ => return Err(ToFilePathError),
            };

            let str_len = self.as_str().len();
            let estimated_capacity = if source_path_style.is_windows() {
                // remove scheme: - has possible \\ for hostname
                str_len.saturating_sub(self.scheme().len() + 1)
            } else {
                // remove scheme://
                str_len.saturating_sub(self.scheme().len() + 3)
            };
            return match source_path_style {
                PathStyle::Unix => {
                    file_url_segments_to_pathbuf_posix(estimated_capacity, host, segments)
                }
                PathStyle::Windows => {
                    file_url_segments_to_pathbuf_windows(estimated_capacity, host, segments)
                }
            };
        }

        fn file_url_segments_to_pathbuf_posix(
            estimated_capacity: usize,
            host: Option<&str>,
            segments: std::str::Split<'_, char>,
        ) -> Result<PathBuf, ToFilePathError> {
            use percent_encoding::percent_decode;

            if host.is_some() {
                return Err(ToFilePathError);
            }

            let mut bytes = Vec::new();
            bytes
                .try_reserve(estimated_capacity)
                .map_err(|_| ToFilePathError)?;

            for segment in segments {
                bytes.push(b'/');
                bytes.extend(percent_decode(segment.as_bytes()));
            }

            // A windows drive letter must end with a slash.
            if bytes.len() > 2
                && bytes[bytes.len() - 2].is_ascii_alphabetic()
                && matches!(bytes[bytes.len() - 1], b':' | b'|')
            {
                bytes.push(b'/');
            }

            let path = String::from_utf8(bytes).map_err(|_| ToFilePathError)?;
            debug_assert!(
                PathStyle::Unix.is_absolute(&path),
                "to_file_path() failed to produce an absolute Path"
            );

            Ok(PathBuf::from(path))
        }

        fn file_url_segments_to_pathbuf_windows(
            estimated_capacity: usize,
            host: Option<&str>,
            mut segments: std::str::Split<'_, char>,
        ) -> Result<PathBuf, ToFilePathError> {
            use percent_encoding::percent_decode_str;
            let mut string = String::new();
            string
                .try_reserve(estimated_capacity)
                .map_err(|_| ToFilePathError)?;
            if let Some(host) = host {
                string.push_str(r"\\");
                string.push_str(host);
            } else {
                let first = segments.next().ok_or(ToFilePathError)?;

                match first.len() {
                    2 => {
                        if !first.starts_with(|c| char::is_ascii_alphabetic(&c))
                            || first.as_bytes()[1] != b':'
                        {
                            return Err(ToFilePathError);
                        }

                        string.push_str(first);
                    }

                    4 => {
                        if !first.starts_with(|c| char::is_ascii_alphabetic(&c)) {
                            return Err(ToFilePathError);
                        }
                        let bytes = first.as_bytes();
                        if bytes[1] != b'%'
                            || bytes[2] != b'3'
                            || (bytes[3] != b'a' && bytes[3] != b'A')
                        {
                            return Err(ToFilePathError);
                        }

                        string.push_str(&first[0..1]);
                        string.push(':');
                    }

                    _ => return Err(ToFilePathError),
                }
            };

            for segment in segments {
                string.push('\\');

                // Currently non-unicode windows paths cannot be represented
                match percent_decode_str(segment).decode_utf8() {
                    Ok(s) => string.push_str(&s),
                    Err(..) => return Err(ToFilePathError),
                }
            }
            // ensure our estimated capacity was good
            if cfg!(test) {
                debug_assert!(
                    string.len() <= estimated_capacity,
                    "len: {}, capacity: {}",
                    string.len(),
                    estimated_capacity
                );
            }
            debug_assert!(
                PathStyle::Windows.is_absolute(&string),
                "to_file_path() failed to produce an absolute Path"
            );
            let path = PathBuf::from(string);
            Ok(path)
        }
        Err(ToFilePathError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn assert_path_positions(cases: &[(&str, &str, Option<u32>, Option<u32>)]) {
        for &(input, expected, row, column) in cases {
            assert_eq!(
                PathWithPosition::parse_str(input),
                PathWithPosition {
                    path: PathBuf::from(expected),
                    row,
                    column
                },
                "{input}",
            );
        }
    }

    #[track_caller]
    fn assert_path_order(entries: &[(&str, bool)], expected: &[&str]) {
        let mut paths: Vec<_> = entries
            .iter()
            .map(|&(path, is_file)| (Path::new(path), is_file))
            .collect();

        paths.sort_by(|&left, &right| compare_paths(left, right));

        assert_eq!(
            paths
                .iter()
                .map(|(path, _is_file)| path.to_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[track_caller]
    fn assert_rel_path_order(
        mode: SortMode,
        order: SortOrder,
        entries: &[(&str, bool)],
        expected: &[&str],
    ) {
        let mut paths: Vec<_> = entries
            .iter()
            .map(|&(path, is_file)| (RelPath::from_unix_str(path).unwrap(), is_file))
            .collect();

        paths.sort_by(|&left, &right| compare_rel_paths_by(left, right, mode, order));

        assert_eq!(
            paths
                .iter()
                .map(|(path, _is_file)| path.as_unix_str())
                .collect::<Vec<_>>(),
            expected,
            "{mode:?} {order:?}"
        );
    }

    #[test]
    fn test_parse_str_treats_paren_suffix_as_position() {
        // This documents the behavior that causes the folder-drop bug: a name ending in
        // `(N)` is parsed as `name ` + row N. The fix lives in `derive_paths_with_position`,
        // which restores the original path when it exists on disk (file or directory).
        let parsed = PathWithPosition::parse_str("/root/Test (3)");
        assert_eq!(parsed.path, PathBuf::from("/root/Test "));
        assert_eq!(parsed.row, Some(3));
    }

    #[test]
    fn compare_paths_with_dots() {
        assert_path_order(
            &[
                ("test_dirs", false),
                ("test_dirs/1.46", false),
                ("test_dirs/1.46/bar_1", true),
                ("test_dirs/1.46/bar_2", true),
                ("test_dirs/1.45", false),
                ("test_dirs/1.45/foo_2", true),
                ("test_dirs/1.45/foo_1", true),
            ],
            &[
                "test_dirs",
                "test_dirs/1.45",
                "test_dirs/1.45/foo_1",
                "test_dirs/1.45/foo_2",
                "test_dirs/1.46",
                "test_dirs/1.46/bar_1",
                "test_dirs/1.46/bar_2",
            ],
        );
        assert_path_order(
            &[("root1/one.txt", true), ("root1/one.two.txt", true)],
            &["root1/one.txt", "root1/one.two.txt"],
        );
    }

    #[test]
    fn compare_paths_with_same_name_different_extensions() {
        assert_path_order(
            &[
                ("test_dirs/file.rs", true),
                ("test_dirs/file.txt", true),
                ("test_dirs/file.md", true),
                ("test_dirs/file", true),
                ("test_dirs/file.a", true),
            ],
            &[
                "test_dirs/file",
                "test_dirs/file.a",
                "test_dirs/file.md",
                "test_dirs/file.rs",
                "test_dirs/file.txt",
            ],
        );
    }

    #[test]
    fn compare_paths_case_semi_sensitive() {
        assert_path_order(
            &[
                ("test_DIRS", false),
                ("test_DIRS/foo_1", true),
                ("test_DIRS/foo_2", true),
                ("test_DIRS/bar", true),
                ("test_DIRS/BAR", true),
                ("test_dirs", false),
                ("test_dirs/foo_1", true),
                ("test_dirs/foo_2", true),
                ("test_dirs/bar", true),
                ("test_dirs/BAR", true),
            ],
            &[
                "test_dirs",
                "test_dirs/bar",
                "test_dirs/BAR",
                "test_dirs/foo_1",
                "test_dirs/foo_2",
                "test_DIRS",
                "test_DIRS/bar",
                "test_DIRS/BAR",
                "test_DIRS/foo_1",
                "test_DIRS/foo_2",
            ],
        );
    }

    #[test]
    fn compare_paths_mixed_case_numeric_ordering() {
        let mut entries = [
            (Path::new(".config"), false),
            (Path::new("Dir1"), false),
            (Path::new("dir01"), false),
            (Path::new("dir2"), false),
            (Path::new("Dir02"), false),
            (Path::new("dir10"), false),
            (Path::new("Dir10"), false),
        ];

        entries.sort_by(|&a, &b| compare_paths(a, b));

        let ordered: Vec<&str> = entries
            .iter()
            .map(|(path, _)| path.to_str().unwrap())
            .collect();

        assert_eq!(
            ordered,
            vec![
                ".config", "Dir1", "dir01", "dir2", "Dir02", "dir10", "Dir10"
            ]
        );
    }

    #[test]
    fn compare_rel_paths_mixed_case_insensitive() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[
                ("zebra.txt", true),
                ("Apple", false),
                ("banana.rs", true),
                ("Carrot", false),
                ("aardvark.txt", true),
            ],
            &["aardvark.txt", "Apple", "banana.rs", "Carrot", "zebra.txt"],
        );
    }

    #[test]
    fn compare_rel_paths_files_first_basic() {
        assert_rel_path_order(
            SortMode::FilesFirst,
            SortOrder::Default,
            &[
                ("zebra.txt", true),
                ("Apple", false),
                ("banana.rs", true),
                ("Carrot", false),
                ("aardvark.txt", true),
            ],
            &["aardvark.txt", "banana.rs", "zebra.txt", "Apple", "Carrot"],
        );
    }

    #[test]
    fn compare_rel_paths_files_first_case_insensitive() {
        assert_rel_path_order(
            SortMode::FilesFirst,
            SortOrder::Default,
            &[
                ("Zebra.txt", true),
                ("apple", false),
                ("Banana.rs", true),
                ("carrot", false),
                ("Aardvark.txt", true),
            ],
            &["Aardvark.txt", "Banana.rs", "Zebra.txt", "apple", "carrot"],
        );
    }

    #[test]
    fn compare_rel_paths_files_first_numeric() {
        assert_rel_path_order(
            SortMode::FilesFirst,
            SortOrder::Default,
            &[
                ("file10.txt", true),
                ("dir2", false),
                ("file2.txt", true),
                ("dir10", false),
                ("file1.txt", true),
            ],
            &["file1.txt", "file2.txt", "file10.txt", "dir2", "dir10"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_case() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[
                ("README.md", true),
                ("readme.txt", true),
                ("ReadMe.rs", true),
            ],
            &["README.md", "ReadMe.rs", "readme.txt"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_files_and_dirs() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[
                ("file2.txt", true),
                ("Dir1", false),
                ("file1.txt", true),
                ("dir2", false),
            ],
            &["Dir1", "dir2", "file1.txt", "file2.txt"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_same_name_different_case_file_and_dir() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[("Hello.txt", true), ("hello", false)],
            &["hello", "Hello.txt"],
        );

        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[("hello", false), ("Hello.txt", true)],
            &["hello", "Hello.txt"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_with_nested_paths() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[
                ("src/main.rs", true),
                ("Cargo.toml", true),
                ("src", false),
                ("target", false),
            ],
            &["Cargo.toml", "src", "src/main.rs", "target"],
        );
    }

    #[test]
    fn compare_rel_paths_files_first_with_nested() {
        // Files come before directories, even with nested paths
        assert_rel_path_order(
            SortMode::FilesFirst,
            SortOrder::Default,
            &[
                ("src/lib.rs", true),
                ("README.md", true),
                ("src", false),
                ("tests", false),
            ],
            &["README.md", "src", "src/lib.rs", "tests"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_dotfiles() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[
                (".gitignore", true),
                ("README.md", true),
                (".github", false),
                ("src", false),
            ],
            &[".github", ".gitignore", "README.md", "src"],
        );
    }

    #[test]
    fn compare_rel_paths_files_first_dotfiles() {
        assert_rel_path_order(
            SortMode::FilesFirst,
            SortOrder::Default,
            &[
                (".gitignore", true),
                ("README.md", true),
                (".github", false),
                ("src", false),
            ],
            &[".gitignore", "README.md", ".github", "src"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_same_stem_different_extension() {
        // Files with same stem but different extensions should sort by extension
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[("file.rs", true), ("file.md", true), ("file.txt", true)],
            &["file.md", "file.rs", "file.txt"],
        );
    }

    #[test]
    fn compare_rel_paths_files_first_same_stem() {
        // Same stem files should still sort by extension with files_first
        assert_rel_path_order(
            SortMode::FilesFirst,
            SortOrder::Default,
            &[("main.rs", true), ("main.c", true), ("main", false)],
            &["main.c", "main.rs", "main"],
        );
    }

    #[test]
    fn compare_rel_paths_mixed_deep_nesting() {
        assert_rel_path_order(
            SortMode::Mixed,
            SortOrder::Default,
            &[
                ("a/b/c.txt", true),
                ("A/B.txt", true),
                ("a.txt", true),
                ("A.txt", true),
            ],
            &["a/b/c.txt", "A/B.txt", "a.txt", "A.txt"],
        );
    }

    #[test]
    fn compare_rel_paths_case_orders() {
        let directories = [
            ("mixedCase", false),
            ("Zebra", false),
            ("banana", false),
            ("ALLCAPS", false),
            ("Apple", false),
            ("dog", false),
            (".hidden", false),
            ("Carrot", false),
        ];
        let mixed_entries = [
            ("banana", false),
            ("Apple.txt", true),
            ("dog.md", true),
            ("ALLCAPS", false),
            ("file1.txt", true),
            ("File2.txt", true),
            (".hidden", false),
        ];
        let natural_entries = [
            ("file10.txt", true),
            ("file1.txt", true),
            ("file20.txt", true),
            ("file2.txt", true),
        ];
        let accented_entries = [
            ("\u{00C9}something.txt", true),
            ("zebra.txt", true),
            ("Apple.txt", true),
        ];
        let numeric_entries = [
            ("file10.txt", true),
            ("file1.txt", true),
            ("file2.txt", true),
            ("file20.txt", true),
        ];

        for (mode, order, entries, expected) in [
            (
                SortMode::DirectoriesFirst,
                SortOrder::Upper,
                directories.as_slice(),
                &[
                    ".hidden",
                    "ALLCAPS",
                    "Apple",
                    "Carrot",
                    "Zebra",
                    "banana",
                    "dog",
                    "mixedCase",
                ][..],
            ),
            (
                SortMode::DirectoriesFirst,
                SortOrder::Upper,
                mixed_entries.as_slice(),
                &[
                    ".hidden",
                    "ALLCAPS",
                    "banana",
                    "Apple.txt",
                    "File2.txt",
                    "dog.md",
                    "file1.txt",
                ][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Upper,
                mixed_entries.as_slice(),
                &[
                    ".hidden",
                    "ALLCAPS",
                    "Apple.txt",
                    "File2.txt",
                    "banana",
                    "dog.md",
                    "file1.txt",
                ][..],
            ),
            (
                SortMode::FilesFirst,
                SortOrder::Upper,
                mixed_entries.as_slice(),
                &[
                    "Apple.txt",
                    "File2.txt",
                    "dog.md",
                    "file1.txt",
                    ".hidden",
                    "ALLCAPS",
                    "banana",
                ][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Upper,
                natural_entries.as_slice(),
                &["file1.txt", "file2.txt", "file10.txt", "file20.txt"][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Upper,
                accented_entries.as_slice(),
                &["Apple.txt", "\u{00C9}something.txt", "zebra.txt"][..],
            ),
            (
                SortMode::DirectoriesFirst,
                SortOrder::Lower,
                directories.as_slice(),
                &[
                    ".hidden",
                    "banana",
                    "dog",
                    "mixedCase",
                    "ALLCAPS",
                    "Apple",
                    "Carrot",
                    "Zebra",
                ][..],
            ),
            (
                SortMode::DirectoriesFirst,
                SortOrder::Lower,
                mixed_entries.as_slice(),
                &[
                    ".hidden",
                    "banana",
                    "ALLCAPS",
                    "dog.md",
                    "file1.txt",
                    "Apple.txt",
                    "File2.txt",
                ][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Lower,
                mixed_entries.as_slice(),
                &[
                    ".hidden",
                    "banana",
                    "dog.md",
                    "file1.txt",
                    "ALLCAPS",
                    "Apple.txt",
                    "File2.txt",
                ][..],
            ),
            (
                SortMode::FilesFirst,
                SortOrder::Lower,
                mixed_entries.as_slice(),
                &[
                    "dog.md",
                    "file1.txt",
                    "Apple.txt",
                    "File2.txt",
                    ".hidden",
                    "banana",
                    "ALLCAPS",
                ][..],
            ),
            (
                SortMode::DirectoriesFirst,
                SortOrder::Unicode,
                directories.as_slice(),
                &[
                    ".hidden",
                    "ALLCAPS",
                    "Apple",
                    "Carrot",
                    "Zebra",
                    "banana",
                    "dog",
                    "mixedCase",
                ][..],
            ),
            (
                SortMode::DirectoriesFirst,
                SortOrder::Unicode,
                mixed_entries.as_slice(),
                &[
                    ".hidden",
                    "ALLCAPS",
                    "banana",
                    "Apple.txt",
                    "File2.txt",
                    "dog.md",
                    "file1.txt",
                ][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Unicode,
                mixed_entries.as_slice(),
                &[
                    ".hidden",
                    "ALLCAPS",
                    "Apple.txt",
                    "File2.txt",
                    "banana",
                    "dog.md",
                    "file1.txt",
                ][..],
            ),
            (
                SortMode::FilesFirst,
                SortOrder::Unicode,
                mixed_entries.as_slice(),
                &[
                    "Apple.txt",
                    "File2.txt",
                    "dog.md",
                    "file1.txt",
                    ".hidden",
                    "ALLCAPS",
                    "banana",
                ][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Unicode,
                numeric_entries.as_slice(),
                &["file1.txt", "file10.txt", "file2.txt", "file20.txt"][..],
            ),
            (
                SortMode::Mixed,
                SortOrder::Unicode,
                accented_entries.as_slice(),
                &["Apple.txt", "zebra.txt", "\u{00C9}something.txt"][..],
            ),
        ] {
            assert_rel_path_order(mode, order, entries, expected);
        }
    }

    #[test]
    fn path_with_position_parse_posix_path() {
        assert_path_positions(&[
            ("test_file", "test_file", None, None),
            ("a:bc:.zip:1", "a:bc:.zip", Some(1), None),
            ("one.second.zip:1", "one.second.zip", Some(1), None),
            ("test_file:10:1:", "test_file", Some(10), Some(1)),
            ("test_file.rs:", "test_file.rs", None, None),
            ("test_file.rs:1:", "test_file.rs", Some(1), None),
            ("ab\ncd", "ab\ncd", None, None),
            ("👋\nab", "👋\nab", None, None),
            ("Types.hs:(617,9)-(670,28):", "Types.hs", Some(617), Some(9)),
            ("main (1).log", "main (1).log", None, None),
        ]);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn path_with_position_parse_posix_path_with_suffix() {
        assert_path_positions(&[
            ("foo/bar:34:in", "foo/bar", Some(34), None),
            ("foo/bar.rs:1902:::15:", "foo/bar.rs:1902", Some(15), None),
            (
                "app-editors:zed-0.143.6:20240710-201212.log:34:",
                "app-editors:zed-0.143.6:20240710-201212.log",
                Some(34),
                None,
            ),
            (
                "crates/file_finder/src/file_finder.rs:1902:13:",
                "crates/file_finder/src/file_finder.rs",
                Some(1902),
                Some(13),
            ),
            (
                "crate/utils/src/test:today.log:34",
                "crate/utils/src/test:today.log",
                Some(34),
                None,
            ),
            (
                "/testing/out/src/file_finder.odin(7:15)",
                "/testing/out/src/file_finder.odin",
                Some(7),
                Some(15),
            ),
        ]);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn path_with_position_parse_windows_path() {
        assert_path_positions(&[
            (
                "crates\\utils\\paths.rs",
                "crates\\utils\\paths.rs",
                None,
                None,
            ),
            (
                "C:\\Users\\someone\\test_file.rs",
                "C:\\Users\\someone\\test_file.rs",
                None,
                None,
            ),
            (
                "C:\\Users\\someone\\main (1).log",
                "C:\\Users\\someone\\main (1).log",
                None,
                None,
            ),
        ]);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn path_with_position_parse_windows_path_with_suffix() {
        assert_path_positions(&[
            (
                "crates\\utils\\paths.rs:101",
                "crates\\utils\\paths.rs",
                Some(101),
                None,
            ),
            (
                "\\\\?\\C:\\Users\\someone\\test_file.rs:1:20",
                "\\\\?\\C:\\Users\\someone\\test_file.rs",
                Some(1),
                Some(20),
            ),
            (
                "C:\\Users\\someone\\test_file.rs(1902,13)",
                "C:\\Users\\someone\\test_file.rs",
                Some(1902),
                Some(13),
            ),
            (
                "\\\\?\\C:\\Users\\someone\\test_file.rs:1902:13:",
                "\\\\?\\C:\\Users\\someone\\test_file.rs",
                Some(1902),
                Some(13),
            ),
            (
                "\\\\?\\C:\\Users\\someone\\test_file.rs:1902:13:15:",
                "\\\\?\\C:\\Users\\someone\\test_file.rs:1902",
                Some(13),
                Some(15),
            ),
            (
                "\\\\?\\C:\\Users\\someone\\test_file.rs:1902:::15:",
                "\\\\?\\C:\\Users\\someone\\test_file.rs:1902",
                Some(15),
                None,
            ),
            (
                "\\\\?\\C:\\Users\\someone\\test_file.rs(1902,13):",
                "\\\\?\\C:\\Users\\someone\\test_file.rs",
                Some(1902),
                Some(13),
            ),
            (
                "\\\\?\\C:\\Users\\someone\\test_file.rs(1902):",
                "\\\\?\\C:\\Users\\someone\\test_file.rs",
                Some(1902),
                None,
            ),
            (
                "C:\\Users\\someone\\test_file.rs:1902:13:",
                "C:\\Users\\someone\\test_file.rs",
                Some(1902),
                Some(13),
            ),
            (
                "C:\\Users\\someone\\test_file.rs(1902,13):",
                "C:\\Users\\someone\\test_file.rs",
                Some(1902),
                Some(13),
            ),
            (
                "C:\\Users\\someone\\test_file.rs(1902):",
                "C:\\Users\\someone\\test_file.rs",
                Some(1902),
                None,
            ),
            (
                "crates/utils/paths.rs:101",
                "crates\\utils\\paths.rs",
                Some(101),
                None,
            ),
        ]);
    }

    #[test]
    fn test_path_compact() {
        let path: PathBuf = [
            home_dir().to_string_lossy().into_owned(),
            "some_file.txt".to_string(),
        ]
        .iter()
        .collect();
        if cfg!(any(target_os = "linux", target_os = "freebsd")) || cfg!(target_os = "macos") {
            assert_eq!(path.compact().to_str(), Some("~/some_file.txt"));
        } else {
            assert_eq!(path.compact().to_str(), path.to_str());
        }
    }

    #[test]
    fn test_extension_or_hidden_file_name() {
        // No dots in name
        let path = Path::new("/a/b/c/file_name.rs");
        assert_eq!(path.extension_or_hidden_file_name(), Some("rs"));

        // Single dot in name
        let path = Path::new("/a/b/c/file.name.rs");
        assert_eq!(path.extension_or_hidden_file_name(), Some("rs"));

        // Multiple dots in name
        let path = Path::new("/a/b/c/long.file.name.rs");
        assert_eq!(path.extension_or_hidden_file_name(), Some("rs"));

        // Hidden file, no extension
        let path = Path::new("/a/b/c/.gitignore");
        assert_eq!(path.extension_or_hidden_file_name(), Some("gitignore"));

        // Hidden file, with extension
        let path = Path::new("/a/b/c/.eslintrc.js");
        assert_eq!(path.extension_or_hidden_file_name(), Some("eslintrc.js"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_sanitized_path() {
        let path = Path::new("C:\\Users\\someone\\test_file.rs");
        let sanitized_path = SanitizedPath::new(path);
        assert_eq!(
            sanitized_path.to_string(),
            "C:\\Users\\someone\\test_file.rs"
        );

        let path = Path::new("\\\\?\\C:\\Users\\someone\\test_file.rs");
        let sanitized_path = SanitizedPath::new(path);
        assert_eq!(
            sanitized_path.to_string(),
            "C:\\Users\\someone\\test_file.rs"
        );
    }

    #[test]
    fn test_compare_numeric_segments() {
        fn compare(a: &str, b: &str) -> Ordering {
            let mut a_iter = a.chars().peekable();
            let mut b_iter = b.chars().peekable();

            let result = compare_numeric_segments(&mut a_iter, &mut b_iter);

            // Verify iterators advanced correctly
            assert!(
                !a_iter.next().is_some_and(|c| c.is_ascii_digit()),
                "Iterator a should have consumed all digits"
            );
            assert!(
                !b_iter.next().is_some_and(|c| c.is_ascii_digit()),
                "Iterator b should have consumed all digits"
            );

            result
        }

        // Basic numeric comparisons
        assert_eq!(compare("0", "0"), Ordering::Equal);
        assert_eq!(compare("1", "2"), Ordering::Less);
        assert_eq!(compare("9", "10"), Ordering::Less);
        assert_eq!(compare("10", "9"), Ordering::Greater);
        assert_eq!(compare("99", "100"), Ordering::Less);

        // Leading zeros
        assert_eq!(compare("0", "00"), Ordering::Less);
        assert_eq!(compare("00", "0"), Ordering::Greater);
        assert_eq!(compare("01", "1"), Ordering::Greater);
        assert_eq!(compare("001", "1"), Ordering::Greater);
        assert_eq!(compare("001", "01"), Ordering::Greater);

        // Same value different representation
        assert_eq!(compare("000100", "100"), Ordering::Greater);
        assert_eq!(compare("100", "0100"), Ordering::Less);
        assert_eq!(compare("0100", "00100"), Ordering::Less);

        // Large numbers
        assert_eq!(compare("9999999999", "10000000000"), Ordering::Less);
        assert_eq!(
            compare(
                "340282366920938463463374607431768211455", // u128::MAX
                "340282366920938463463374607431768211456"
            ),
            Ordering::Less
        );
        assert_eq!(
            compare(
                "340282366920938463463374607431768211456", // > u128::MAX
                "340282366920938463463374607431768211455"
            ),
            Ordering::Greater
        );

        // Iterator advancement verification
        let mut a_iter = "123abc".chars().peekable();
        let mut b_iter = "456def".chars().peekable();

        compare_numeric_segments(&mut a_iter, &mut b_iter);

        assert_eq!(a_iter.collect::<String>(), "abc");
        assert_eq!(b_iter.collect::<String>(), "def");
    }

    #[test]
    fn test_natural_sort() {
        for (left, right, expected) in [
            ("a", "b", Ordering::Less),
            ("b", "a", Ordering::Greater),
            ("a", "a", Ordering::Equal),
            ("a", "A", Ordering::Less),
            ("A", "a", Ordering::Greater),
            ("aA", "aa", Ordering::Greater),
            ("aa", "aA", Ordering::Less),
            ("1", "2", Ordering::Less),
            ("2", "10", Ordering::Less),
            ("02", "10", Ordering::Less),
            ("02", "2", Ordering::Greater),
            ("a1", "a2", Ordering::Less),
            ("a2", "a10", Ordering::Less),
            ("a02", "a2", Ordering::Greater),
            ("a1b", "a1c", Ordering::Less),
            ("1a2", "1a10", Ordering::Less),
            ("1a10", "1a2", Ordering::Greater),
            ("2a1", "10a1", Ordering::Less),
            ("a-1", "a-2", Ordering::Less),
            ("a_1", "a_2", Ordering::Less),
            ("a.1", "a.2", Ordering::Less),
            ("文1", "文2", Ordering::Less),
            ("文2", "文10", Ordering::Less),
            ("🔤1", "🔤2", Ordering::Less),
            ("", "", Ordering::Equal),
            ("", "a", Ordering::Less),
            ("a", "", Ordering::Greater),
            (" ", "  ", Ordering::Less),
            ("File-1.txt", "File-2.txt", Ordering::Less),
            ("File-02.txt", "File-2.txt", Ordering::Greater),
            ("File-2.txt", "File-10.txt", Ordering::Less),
            ("File_A1", "File_A2", Ordering::Less),
            ("File_a1", "File_A1", Ordering::Less),
            ("A", "A", Ordering::Equal),
            ("aaa", "AAA", Ordering::Less),
            ("AAA", "aaa", Ordering::Greater),
            ("aAa", "AaA", Ordering::Less),
            ("A", "b", Ordering::Less),
            ("a", "B", Ordering::Less),
            ("file1", "file2", Ordering::Less),
            ("file2", "file10", Ordering::Less),
            ("file10", "file2", Ordering::Greater),
            ("1file", "2file", Ordering::Less),
            ("file1text", "file2text", Ordering::Less),
            ("text1file", "text2file", Ordering::Less),
            ("file1-2", "file1-10", Ordering::Less),
            ("2-1file", "10-1file", Ordering::Less),
            ("file002", "file2", Ordering::Greater),
            ("file002", "file10", Ordering::Less),
            (
                "file999999999999999999999",
                "file999999999999999999998",
                Ordering::Greater,
            ),
            (
                "file340282366920938463463374607431768211454",
                "file340282366920938463463374607431768211455",
                Ordering::Less,
            ),
            (
                "file340282366920938463463374607431768211456",
                "file340282366920938463463374607431768211455",
                Ordering::Greater,
            ),
            (
                "file3402823669209384634633746074317682114560",
                "file340282366920938463463374607431768211455",
                Ordering::Greater,
            ),
            (
                "file0340282366920938463463374607431768211455",
                "file340282366920938463463374607431768211455",
                Ordering::Greater,
            ),
            (
                "file999999999999999999999999999999999999999999999999",
                "file9999999999999999999999999999999999999999999999999",
                Ordering::Less,
            ),
            ("File1", "file2", Ordering::Less),
            ("file1", "File2", Ordering::Less),
            ("Dir1", "dir01", Ordering::Less),
            ("dir2", "Dir02", Ordering::Less),
            ("dir2", "dir02", Ordering::Less),
            ("dir1", "Dir1", Ordering::Less),
            ("dir02", "Dir02", Ordering::Less),
            ("dir10", "Dir10", Ordering::Less),
            ("file-1", "file_1", Ordering::Less),
            ("file.1", "file_1", Ordering::Less),
            ("file 1", "file_1", Ordering::Less),
            ("file①", "file②", Ordering::Less),
            ("file⑩", "file②", Ordering::Greater),
            ("file漢", "file字", Ordering::Greater),
            ("file-1a", "file-1b", Ordering::Less),
            ("file-1.2", "file-1.10", Ordering::Less),
            ("file-1.10", "file-1.2", Ordering::Greater),
        ] {
            assert_eq!(natural_sort(left, right), expected, "{left:?} vs {right:?}");
        }
    }

    #[test]
    fn test_compare_paths() {
        fn compare(a: &str, is_a_file: bool, b: &str, is_b_file: bool) -> Ordering {
            compare_paths((Path::new(a), is_a_file), (Path::new(b), is_b_file))
        }

        // Basic path comparison
        assert_eq!(compare("a", true, "b", true), Ordering::Less);
        assert_eq!(compare("b", true, "a", true), Ordering::Greater);
        assert_eq!(compare("a", true, "a", true), Ordering::Equal);

        // Files vs Directories
        assert_eq!(compare("a", true, "a", false), Ordering::Greater);
        assert_eq!(compare("a", false, "a", true), Ordering::Less);
        assert_eq!(compare("b", false, "a", true), Ordering::Less);

        // Extensions
        assert_eq!(compare("a.txt", true, "a.md", true), Ordering::Greater);
        assert_eq!(compare("a.md", true, "a.txt", true), Ordering::Less);
        assert_eq!(compare("a", true, "a.txt", true), Ordering::Less);

        // Nested paths
        assert_eq!(compare("dir/a", true, "dir/b", true), Ordering::Less);
        assert_eq!(compare("dir1/a", true, "dir2/a", true), Ordering::Less);
        assert_eq!(compare("dir/sub/a", true, "dir/a", true), Ordering::Less);

        // Case sensitivity in paths
        assert_eq!(
            compare("Dir/file", true, "dir/file", true),
            Ordering::Greater
        );
        assert_eq!(
            compare("dir/File", true, "dir/file", true),
            Ordering::Greater
        );
        assert_eq!(compare("dir/file", true, "Dir/File", true), Ordering::Less);

        // Hidden files and special names
        assert_eq!(compare(".hidden", true, "visible", true), Ordering::Less);
        assert_eq!(compare("_special", true, "normal", true), Ordering::Less);
        assert_eq!(compare(".config", false, ".data", false), Ordering::Less);

        // Mixed numeric paths
        assert_eq!(
            compare("dir1/file", true, "dir2/file", true),
            Ordering::Less
        );
        assert_eq!(
            compare("dir2/file", true, "dir10/file", true),
            Ordering::Less
        );
        assert_eq!(
            compare("dir02/file", true, "dir2/file", true),
            Ordering::Greater
        );

        // Root paths
        assert_eq!(compare("/a", true, "/b", true), Ordering::Less);
        assert_eq!(compare("/", false, "/a", true), Ordering::Less);

        // Complex real-world examples
        assert_eq!(
            compare("project/src/main.rs", true, "project/src/lib.rs", true),
            Ordering::Greater
        );
        assert_eq!(
            compare(
                "project/tests/test_1.rs",
                true,
                "project/tests/test_2.rs",
                true
            ),
            Ordering::Less
        );
        assert_eq!(
            compare(
                "project/v1.0.0/README.md",
                true,
                "project/v1.10.0/README.md",
                true
            ),
            Ordering::Less
        );
    }

    #[test]
    fn test_multiple_extensions() {
        // No extensions
        let path = Path::new("/a/b/c/file_name");
        assert_eq!(path.multiple_extensions(), None);

        // Only one extension
        let path = Path::new("/a/b/c/file_name.tsx");
        assert_eq!(path.multiple_extensions(), None);

        // Stories sample extension
        let path = Path::new("/a/b/c/file_name.stories.tsx");
        assert_eq!(path.multiple_extensions(), Some("stories.tsx".to_string()));

        // Longer sample extension
        let path = Path::new("/a/b/c/long.app.tar.gz");
        assert_eq!(path.multiple_extensions(), Some("app.tar.gz".to_string()));
    }

    #[test]
    fn test_strip_path_suffix() {
        let base = Path::new("/a/b/c/file_name");
        let suffix = Path::new("file_name");
        assert_eq!(strip_path_suffix(base, suffix), Some(Path::new("/a/b/c")));

        let base = Path::new("/a/b/c/file_name.tsx");
        let suffix = Path::new("file_name.tsx");
        assert_eq!(strip_path_suffix(base, suffix), Some(Path::new("/a/b/c")));

        let base = Path::new("/a/b/c/file_name.stories.tsx");
        let suffix = Path::new("c/file_name.stories.tsx");
        assert_eq!(strip_path_suffix(base, suffix), Some(Path::new("/a/b")));

        let base = Path::new("/a/b/c/long.app.tar.gz");
        let suffix = Path::new("b/c/long.app.tar.gz");
        assert_eq!(strip_path_suffix(base, suffix), Some(Path::new("/a")));

        let base = Path::new("/a/b/c/long.app.tar.gz");
        let suffix = Path::new("/a/b/c/long.app.tar.gz");
        assert_eq!(strip_path_suffix(base, suffix), Some(Path::new("")));

        let base = Path::new("/a/b/c/long.app.tar.gz");
        let suffix = Path::new("/a/b/c/no_match.app.tar.gz");
        assert_eq!(strip_path_suffix(base, suffix), None);

        let base = Path::new("/a/b/c/long.app.tar.gz");
        let suffix = Path::new("app.tar.gz");
        assert_eq!(strip_path_suffix(base, suffix), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_wsl_path() {
        use super::WslPath;
        let path = "/a/b/c";
        assert_eq!(WslPath::from_path(&path), None);

        let path = r"\\wsl.localhost";
        assert_eq!(WslPath::from_path(&path), None);

        let path = r"\\wsl.localhost\Distro";
        assert_eq!(
            WslPath::from_path(&path),
            Some(WslPath {
                distro: "Distro".to_owned(),
                path: "/".into(),
            })
        );

        let path = r"\\wsl.localhost\Distro\blue";
        assert_eq!(
            WslPath::from_path(&path),
            Some(WslPath {
                distro: "Distro".to_owned(),
                path: "/blue".into()
            })
        );

        let path = r"\\wsl$\archlinux\tomato\.\paprika\..\aubergine.txt";
        assert_eq!(
            WslPath::from_path(&path),
            Some(WslPath {
                distro: "archlinux".to_owned(),
                path: "/tomato/paprika/../aubergine.txt".into()
            })
        );

        let path = r"\\windows.localhost\Distro\foo";
        assert_eq!(WslPath::from_path(&path), None);
    }

    #[test]
    fn test_url_to_file_path_ext() {
        for (style, input, expected) in [
            (
                PathStyle::Unix,
                "file:///home/user/file.txt",
                Some("/home/user/file.txt"),
            ),
            (PathStyle::Unix, "file:///", Some("/")),
            (PathStyle::Unix, "file:///a/b/c/d/e", Some("/a/b/c/d/e")),
            (
                PathStyle::Unix,
                "file:///home/user/file%20with%20spaces.txt",
                Some("/home/user/file with spaces.txt"),
            ),
            (
                PathStyle::Unix,
                "file:///path%2Fwith%2Fencoded%2Fslashes",
                Some("/path/with/encoded/slashes"),
            ),
            (
                PathStyle::Unix,
                "file:///special%23chars%3F.txt",
                Some("/special#chars?.txt"),
            ),
            (
                PathStyle::Unix,
                "file://localhost/home/user/file.txt",
                Some("/home/user/file.txt"),
            ),
            (PathStyle::Unix, "file://somehost/home/user/file.txt", None),
            (PathStyle::Unix, "file:///C:", Some("/C:/")),
            (PathStyle::Unix, "file:///D|", Some("/D|/")),
            (
                PathStyle::Windows,
                "file:///C:/Users/user/file.txt",
                Some("C:\\Users\\user\\file.txt"),
            ),
            (
                PathStyle::Windows,
                "file:///D:/folder/subfolder/file.rs",
                Some("D:\\folder\\subfolder\\file.rs"),
            ),
            (PathStyle::Windows, "file:///C:/", Some("C:\\")),
            (
                PathStyle::Windows,
                "file:///C%3A/Users/file.txt",
                Some("C:\\Users\\file.txt"),
            ),
            (
                PathStyle::Windows,
                "file:///c%3a/Users/file.txt",
                Some("c:\\Users\\file.txt"),
            ),
            (
                PathStyle::Windows,
                "file:///D%3A/folder/file.txt",
                Some("D:\\folder\\file.txt"),
            ),
            (
                PathStyle::Windows,
                "file:///d%3A/folder/file.txt",
                Some("d:\\folder\\file.txt"),
            ),
            (
                PathStyle::Windows,
                "file://server/share/path/file.txt",
                Some("\\\\server\\share\\path\\file.txt"),
            ),
            (
                PathStyle::Windows,
                "file://server/share",
                Some("\\\\server\\share"),
            ),
            (
                PathStyle::Windows,
                "file:///C:/Users/user/file%20with%20spaces.txt",
                Some("C:\\Users\\user\\file with spaces.txt"),
            ),
            (
                PathStyle::Windows,
                "file:///C:/special%23chars%3F.txt",
                Some("C:\\special#chars?.txt"),
            ),
            (PathStyle::Windows, "file:///1:/path/file.txt", None),
            (PathStyle::Windows, "file:///CC:/path/file.txt", None),
            (PathStyle::Windows, "file:///C/path/file.txt", None),
            (PathStyle::Windows, "file:///invalid", None),
            (PathStyle::Unix, "http://example.com/path", None),
            (PathStyle::Windows, "http://example.com/path", None),
            (PathStyle::Unix, "https://example.com/path", None),
            (PathStyle::Windows, "https://example.com/path", None),
            (
                PathStyle::Windows,
                "file://localhost/C:/Users/file.txt",
                Some("C:\\Users\\file.txt"),
            ),
        ] {
            let url = url::Url::parse(input).unwrap();
            let expected = expected.map(PathBuf::from).ok_or(ToFilePathError);

            assert_eq!(url.to_file_path_ext(style), expected, "{style:?}: {input}");
        }
    }
}
