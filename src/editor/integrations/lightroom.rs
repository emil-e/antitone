//! Lightroom's JSON transport and canonical-roll save/return adapter.
use crate::editor::{Error, OutputTarget, document::Document, external_edit, output};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const PROTOCOL_VERSION: u32 = 3;

const PRESENTATION: external_edit::Presentation = external_edit::Presentation {
    action: "Save Back to Lightroom",
    opening: "Opening Lightroom roll…",
    working: "Saving back to Lightroom…",
    completed: "Saved back to Lightroom",
};

pub fn read(file: &Path) -> Result<external_edit::Request, Error> {
    Ok(external_edit::Request::new(Request::read(file)?))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Photo {
    uuid: String,
    path: PathBuf,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    New,
    Open,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    mode: Mode,
    project: PathBuf,
    relocated_from: Option<PathBuf>,
    photos: Vec<Photo>,
    #[serde(skip)]
    receipt: PathBuf,
}

#[derive(Serialize)]
struct Source {
    id: String,
    path: PathBuf,
}

#[derive(Serialize)]
struct Render<'a> {
    uuid: &'a str,
    source: String,
    path: &'a Path,
}

#[derive(Serialize)]
struct Receipt<'a> {
    version: u32,
    project: &'a Path,
    sources: Vec<Source>,
    renders: Vec<Render<'a>>,
}

fn failure(message: impl std::fmt::Display) -> Error {
    Error::Project(message.to_string())
}

impl Request {
    /// Read before starting the native host, not from a paint callback.
    fn read(file: &Path) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(file).map_err(failure)?;
        Self::parse(&contents, file.with_file_name("result.json"))
    }

    fn parse(contents: &str, receipt: PathBuf) -> Result<Self, Error> {
        let mut request: Self = serde_json::from_str(contents).map_err(failure)?;
        if request.version != PROTOCOL_VERSION {
            return Err(failure("unsupported Lightroom request version"));
        }
        if request.photos.is_empty() {
            return Err(failure("no Lightroom photos requested"));
        }
        request.receipt = receipt;
        if !request.project.is_absolute() || !request.receipt.is_absolute() {
            return Err(failure("Lightroom paths must be absolute"));
        }
        if !request.project.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("antitone") || ext.eq_ignore_ascii_case("json")
        }) {
            return Err(failure("Lightroom project must be an Antitone document"));
        }
        if let Some(previous) = &request.relocated_from
            && (request.mode != Mode::Open || !previous.is_absolute())
        {
            return Err(failure(
                "relocated projects require an absolute original path and open mode",
            ));
        }
        let mut uuids = HashSet::new();
        let mut paths = HashSet::new();
        for photo in &request.photos {
            if photo.uuid.is_empty() || !uuids.insert(&photo.uuid) || !paths.insert(&photo.path) {
                return Err(failure("duplicate or empty Lightroom photo identity"));
            }
            if !photo.path.is_absolute() {
                return Err(failure("Lightroom paths must be absolute"));
            }
        }
        Ok(request)
    }

    fn receipt(
        &self,
        document: &Document,
        renders: &[external_edit::Rendered],
    ) -> Result<(), Error> {
        if renders.len() != self.photos.len() {
            return Err(failure("Lightroom render count mismatch"));
        }
        let project = document
            .path
            .as_deref()
            .ok_or_else(|| failure("roll has not been saved"))?;
        let requested_paths = self
            .photos
            .iter()
            .filter_map(|photo| {
                document
                    .find_source(&photo.path)
                    .map(|id| (id, &photo.path))
            })
            .collect::<HashMap<_, _>>();
        let sources = document
            .project
            .order
            .iter()
            .map(|id| {
                let path = document.source_path(*id)?;
                let resolved = path.canonicalize().unwrap_or(path);
                // Preserve the catalogue's locator for requested originals, including aliases.
                let path = requested_paths
                    .get(id)
                    .copied()
                    .unwrap_or(&resolved)
                    .clone();
                Ok(Source {
                    id: id.0.to_string(),
                    path,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let renders = self
            .photos
            .iter()
            .zip(renders)
            .map(|(photo, render)| {
                let source = document
                    .find_source(&photo.path)
                    .ok_or_else(|| failure("requested source was removed"))?;
                if render.source != source {
                    return Err(failure("Lightroom render identity mismatch"));
                }
                Ok(Render {
                    uuid: &photo.uuid,
                    source: source.0.to_string(),
                    path: &render.path,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let receipt = Receipt {
            version: PROTOCOL_VERSION,
            project,
            sources,
            renders,
        };
        let mut file = tempfile::NamedTempFile::new_in(
            self.receipt
                .parent()
                .ok_or_else(|| failure("receipt has no parent"))?,
        )
        .map_err(failure)?;
        serde_json::to_writer_pretty(file.as_file_mut(), &receipt).map_err(failure)?;
        file.as_file().sync_all().map_err(failure)?;
        file.persist_noclobber(&self.receipt).map_err(failure)?;
        Ok(())
    }
}

impl external_edit::Input for Request {
    fn presentation(&self) -> external_edit::Presentation {
        PRESENTATION
    }

    fn prepare(
        self: Box<Self>,
        cancelled: &crate::cancellation::Cancellation,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<external_edit::Opened, Error> {
        if cancelled.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if self.receipt.exists() {
            return Err(failure("this Lightroom session has already returned"));
        }
        let mut document = if self.mode == Mode::Open {
            Document::open(&self.project)?
        } else {
            if self.project.exists() {
                return Err(failure("new Lightroom roll destination already exists"));
            }
            let mut document = Document::default();
            let paths = self
                .photos
                .iter()
                .map(|photo| photo.path.clone())
                .collect::<Vec<_>>();
            document.import_with_progress(&paths, cancelled, progress)?;
            document
        };
        if let Some(previous) = &self.relocated_from
            && self
                .photos
                .iter()
                .any(|photo| document.find_source(&photo.path).is_none())
        {
            // A Finder move does not rebase relative locators as an ordinary Save As does.
            crate::editor::document::rebase_project(
                &mut document.project,
                Some(previous),
                Some(&self.project),
            );
            document.capture_source_files()?;
        }
        let mut requested = Vec::with_capacity(self.photos.len());
        for photo in &self.photos {
            let id = document.find_source(&photo.path).ok_or_else(|| {
                failure(format!(
                    "photo is not in this saved roll: {}",
                    photo.path.display()
                ))
            })?;
            if !document.project.order.contains(&id) {
                return Err(failure("Lightroom photo is not an ordered roll member"));
            }
            requested.push(id);
        }
        let destination = Arc::new(Destination::new(*self));
        let session = external_edit::Session::new(&document, requested, destination)?;
        Ok(external_edit::Opened { document, session })
    }
}

struct Destination {
    request: Request,
    canonical_saved: AtomicBool,
}

impl Destination {
    fn new(request: Request) -> Self {
        Self {
            canonical_saved: AtomicBool::new(request.mode == Mode::Open),
            request,
        }
    }
}

impl external_edit::Destination for Destination {
    fn presentation(&self) -> external_edit::Presentation {
        PRESENTATION
    }

    fn output(&self, document: &Document, target: OutputTarget) -> Result<output::Options, Error> {
        let folders = self
            .request
            .photos
            .iter()
            .map(|photo| {
                let id = document
                    .find_source(&photo.path)
                    .ok_or_else(|| failure("requested source was removed"))?;
                let folder = photo
                    .path
                    .parent()
                    .ok_or_else(|| failure("original has no parent folder"))?;
                Ok((id, folder.to_path_buf()))
            })
            .collect::<Result<HashMap<_, _>, Error>>()?;
        let mut options = output::Options::new(target, output::Folders::PerPhoto(folders));
        options.suffix = "-Antitone".into();
        Ok(options)
    }

    fn save(&self, document: &Document) -> Result<Document, Error> {
        let policy = if self.canonical_saved.load(Ordering::Acquire)
            || document.path.as_ref() == Some(&self.request.project)
        {
            crate::project::Save::Replace
        } else {
            crate::project::Save::New
        };
        let saved = document.save(&self.request.project, policy)?;
        self.canonical_saved.store(true, Ordering::Release);
        Ok(saved)
    }

    fn saved(&self, path: &Path) {
        if path == self.request.project {
            self.canonical_saved.store(true, Ordering::Release);
        }
    }

    fn publish(
        &self,
        document: &Document,
        renders: &[external_edit::Rendered],
    ) -> Result<(), Error> {
        self.request.receipt(document, renders)
    }
}

#[cfg(test)]
mod tests;
