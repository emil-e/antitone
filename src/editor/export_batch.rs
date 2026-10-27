use super::{Editor, Error, ExrPrecision, OutputFormat, OutputTarget, document, source};
use crate::{image_recipe::ImageRecipe, project::SourceId};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Selected,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Collision {
    KeepBoth,
    Skip,
    Replace,
    #[cfg(test)]
    Refuse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Tiff,
    Exr,
}

impl Kind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff => "tif",
            Self::Exr => "exr",
        }
    }
}

pub(super) struct Draft {
    pub scope: Scope,
    pub folder: PathBuf,
    pub kind: Kind,
    pub target: OutputTarget,
    pub precision: ExrPrecision,
    pub suffix: String,
    pub collision: Collision,
    pub confirming_replace: bool,
    pub error: Option<String>,
}

impl Draft {
    pub fn new(target: OutputTarget, folder: PathBuf) -> Self {
        Self {
            scope: Scope::Selected,
            folder,
            kind: Kind::Tiff,
            target,
            precision: ExrPrecision::Half,
            suffix: String::new(),
            collision: Collision::KeepBoth,
            confirming_replace: false,
            error: None,
        }
    }

    pub fn format(&self) -> OutputFormat {
        match self.kind {
            Kind::Tiff => OutputFormat::Tiff(self.target),
            Kind::Exr => OutputFormat::Exr(self.precision),
        }
    }
}

pub(super) enum Input {
    Ready(source::Context),
    Acquire(Box<source::Request>),
}

pub(super) struct Photo {
    pub input: Input,
    pub recipe: ImageRecipe,
    pub path: PathBuf,
}

pub(super) struct Request {
    pub photos: Vec<Photo>,
    pub format: OutputFormat,
    pub collision: Collision,
    pub protected: Vec<PathBuf>,
}

pub(super) enum Outcome {
    Exported(PathBuf),
    Skipped(PathBuf),
    Failed { path: PathBuf, error: String },
}

#[derive(Default)]
pub(super) struct Summary {
    pub total: usize,
    pub outcomes: Vec<Outcome>,
    pub stopped: bool,
}

impl Summary {
    pub fn exported(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Exported(_)))
            .count()
    }
    pub fn skipped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Skipped(_)))
            .count()
    }
    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Failed { .. }))
            .count()
    }
    pub fn unattempted(&self) -> usize {
        self.total - self.outcomes.len()
    }
    pub fn text(&self) -> String {
        format!(
            "{} exported · {} skipped · {} failed · {} remaining",
            self.exported(),
            self.skipped(),
            self.failed(),
            self.unattempted()
        )
    }
}

pub(super) fn filename(source: &Path, suffix: &str, kind: Kind) -> Result<PathBuf, Error> {
    if suffix
        .chars()
        .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
    {
        return Err(Error::InvalidParameter(
            "filename suffix contains a path separator or control character",
        ));
    }
    let mut name = source
        .file_stem()
        .filter(|s| !s.is_empty())
        .ok_or(Error::InvalidParameter("source has no filename"))?
        .to_os_string();
    name.push(suffix);
    name.push(".");
    name.push(kind.extension());
    Ok(PathBuf::from(name))
}

pub(super) fn numbered(path: &Path, index: usize) -> PathBuf {
    let mut stem = path.file_stem().unwrap_or_default().to_os_string();
    stem.push(format!(" ({index})"));
    if let Some(extension) = path.extension() {
        stem.push(".");
        stem.push(extension);
    }
    path.with_file_name(stem)
}

pub(super) fn destination(
    path: &Path,
    policy: Collision,
    protected: &[PathBuf],
    published: &[PathBuf],
) -> Result<Option<PathBuf>, Error> {
    let mut candidate = path.to_owned();
    let mut number = 2;
    loop {
        require_safe(&candidate, protected)?;
        // The destination filesystem decides name equivalence. In particular,
        // normalization/case aliases must not replace a previous batch output.
        if published
            .iter()
            .any(|path| same_destination(&candidate, path))
        {
            candidate = numbered(path, number);
            number += 1;
            continue;
        }
        let exists = match std::fs::symlink_metadata(&candidate) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(Error::Export(error.to_string())),
        };
        if !exists || policy == Collision::Replace {
            return Ok(Some(candidate));
        }
        if policy == Collision::Skip {
            return Ok(None);
        }
        if policy != Collision::KeepBoth {
            return Ok(Some(candidate));
        }
        candidate = numbered(path, number);
        number += 1;
    }
}

fn same_destination(a: &Path, b: &Path) -> bool {
    if document::same_path(a, b) {
        return true;
    }
    crate::sys::same_file(a, b)
}

pub(super) fn require_safe(path: &Path, protected: &[PathBuf]) -> Result<(), Error> {
    if protected.iter().any(|input| same_destination(path, input)) {
        return Err(Error::Export(
            "destination is a project document or source file".into(),
        ));
    }
    if path.is_dir() {
        return Err(Error::Export("destination is a directory".into()));
    }
    Ok(())
}

impl Editor {
    pub(super) fn export_ids(&self, scope: Scope) -> Vec<SourceId> {
        self.document
            .project
            .order
            .iter()
            .copied()
            .filter(|id| scope == Scope::All || self.roll_selection.contains(*id))
            .collect()
    }

    pub(super) fn batch_input(&mut self, draft: &Draft) -> Result<Request, Error> {
        self.sync_document();
        self.validate_document_candidate(&self.document)?;
        if !draft.folder.is_absolute() || !draft.folder.is_dir() {
            return Err(Error::InvalidParameter(
                "choose an existing destination folder",
            ));
        }
        let ids = self.export_ids(draft.scope);
        if ids.is_empty() {
            return Err(Error::InvalidParameter("no photos selected for export"));
        }
        let mut protected = self
            .document
            .project
            .sources
            .iter()
            .map(|s| {
                self.document.source_path(s.id).and_then(|path| {
                    std::path::absolute(path).map_err(|error| Error::Export(error.to_string()))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(path) = &self.document.path {
            protected
                .push(std::path::absolute(path).map_err(|error| Error::Export(error.to_string()))?);
        }
        let mut names = HashSet::new();
        let mut photos = Vec::with_capacity(ids.len());
        for id in ids {
            let source_path = std::path::absolute(self.document.source_path(id)?)
                .map_err(|error| Error::Export(error.to_string()))?;
            let base = draft
                .folder
                .join(filename(&source_path, &draft.suffix, draft.kind)?);
            // Duplicate basenames must never replace an earlier file from this batch.
            let mut path = base.clone();
            let mut number = 2;
            while !names.insert(path.to_string_lossy().to_lowercase()) {
                path = numbered(&base, number);
                number += 1;
            }
            require_safe(&path, &protected)?;
            let expected = self.document.project.source(id)?;
            let input = if self.current_source == Some(id) {
                self.source_context().map(Input::Ready)
            } else {
                None
            }
            .unwrap_or_else(|| {
                let expected = expected.clone();
                let wb = Some(expected.development.decode.fixed_wb);
                Input::Acquire(Box::new(source::Request {
                    reference: None,

                    input: self.input(),

                    file: self.document.file_key(&source_path),
                    cache: Some(self.document.cache.clone()),

                    path: source_path,
                    wb,
                    expected: Some(expected),
                    correction: self.correction.clone(),
                }))
            });
            photos.push(Photo {
                input,
                path,
                recipe: self.document.project.image_recipe(id)?,
            });
        }
        Ok(Request {
            photos,
            format: draft.format(),
            collision: draft.collision,
            protected,
        })
    }
}

#[cfg(test)]
mod tests;
