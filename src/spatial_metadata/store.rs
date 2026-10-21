use super::*;
use std::sync::{Arc, Weak};

/// Document-owned publication authority. Cloned snapshots carry data, never this authority.
#[derive(Default)]
pub struct Store {
    epoch: Arc<()>,

    closed: bool,
    ready: Arc<Metadata>,
}

#[derive(Clone, Debug)]
pub struct Request(Weak<()>);

impl Store {
    pub fn restored(metadata: Metadata) -> Self {
        Self {
            ready: Arc::new(if metadata.validate().is_ok() {
                metadata
            } else {
                Metadata::default()
            }),
            ..Self::default()
        }
    }

    /// Captures ready data without transferring the live document's publication authority.
    /// Consumers still verify its inputs before using it; this accessor performs no file I/O.
    pub fn ready(&self) -> Arc<Metadata> {
        self.ready.clone()
    }

    pub fn begin(&mut self) -> Option<Request> {
        if self.closed {
            return None;
        }
        self.epoch = Arc::new(());
        Some(Request(Arc::downgrade(&self.epoch)))
    }

    /// Publication never accepts sampling, grades or crops. The caller supplies fresh input
    /// identities captured by its normal service, not a lease on live UI/source state.
    pub fn publish(
        &mut self,
        request: Request,
        metadata: Metadata,
        project: &Project,
        current: &[Input],
    ) -> Result<bool, Error> {
        if self.closed
            || !request
                .0
                .upgrade()
                .is_some_and(|epoch| Arc::ptr_eq(&epoch, &self.epoch))
        {
            return Ok(false);
        }
        metadata.validate()?;
        if metadata.matching(project, current) != metadata {
            return Ok(false);
        }
        self.ready = Arc::new(metadata);
        Ok(true)
    }

    /// A save/consumer snapshot includes only ready, compatible results; it never awaits work.
    pub fn snapshot(&self, project: &Project, current: &[Input]) -> Metadata {
        self.ready.matching(project, current)
    }

    pub fn cancel(&mut self) {
        self.epoch = Arc::new(());
    }

    pub fn close(&mut self) {
        self.closed = true;
    }
}
