//! One editor native worker. Mutable SDK owners never leave this thread; consumers
//! join immutable completion, independently of optional cache ownership and UI tickets.
use super::*;
use crate::{
    scan::DevelopedSource,
    source_store::{SourceKey, SourceStore},
};
use std::{
    collections::HashMap,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Priority {
    Foreground,
    Visible,
    Eager,
}

#[derive(Clone)]
pub(super) struct Request {
    pub path: PathBuf,
    pub wb: Option<[f64; 4]>,
    pub expected: Option<Source>,
    pub basis: Option<crate::project::CameraBasis>,
    pub file: thumbnails::FileKey,
    pub key: Option<SourceKey>,
    pub store: Option<Arc<SourceStore>>,
    pub parent: Option<Arc<SourceStore>>,
    pub retain: bool,
}

impl Request {
    fn joins(&self, other: &Self) -> bool {
        self.key
            .as_ref()
            .zip(other.key.as_ref())
            .is_some_and(|(a, b)| a == b)
            && self
                .parent
                .as_ref()
                .or(self.store.as_ref())
                .zip(other.parent.as_ref().or(other.store.as_ref()))
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
    }

    fn retired(&self) -> bool {
        self.store.as_ref().is_some_and(|store| store.is_closed())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum NativePhase {
    Open,
    UnpackBegin,
    UnpackEnd,
    DevelopBegin,
    DevelopEnd,
}
#[cfg(test)]
type NativeObserver = Arc<
    dyn Fn(&std::path::Path, NativePhase, Option<&crate::raw::Metadata>) -> Result<(), String>
        + Send
        + Sync,
>;
type Completion = Result<Arc<DevelopedSource>, String>;

struct Consumer {
    id: u64,
    priority: Priority,
    store: Option<Arc<SourceStore>>,
    retain: bool,
}

struct Task {
    id: u64,
    request: Request,
    demands: Mutex<Vec<Consumer>>,
    next: AtomicU64,
    started: std::sync::atomic::AtomicBool,
    result: Mutex<Option<Completion>>,
    ready: Condvar,
}

impl Task {
    fn priority(&self) -> Priority {
        self.demands
            .lock()
            .unwrap()
            .iter()
            .map(|consumer| consumer.priority)
            .min()
            .unwrap_or(Priority::Eager)
    }

    fn scopes(&self) -> Vec<(Arc<SourceStore>, bool)> {
        let mut scopes: Vec<_> = self
            .request
            .store
            .iter()
            .map(|store| (store.clone(), self.request.retain))
            .collect();
        scopes.extend(self.demands.lock().unwrap().iter().filter_map(|consumer| {
            consumer
                .store
                .as_ref()
                .map(|store| (store.clone(), consumer.retain))
        }));
        scopes
    }

    fn retired(&self) -> bool {
        self.request.retired() && self.scopes().iter().all(|(store, _)| store.is_closed())
    }

    fn abandoned(&self) -> bool {
        !self.started.load(Ordering::Relaxed) && self.demands.lock().unwrap().is_empty()
    }
}

struct State {
    tasks: Vec<Arc<Task>>,
    queue: Vec<Arc<Task>>,
    next: u64,
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn close(&self) {
        let tasks = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            state.queue.clear();
            std::mem::take(&mut state.tasks)
        };
        self.changed.notify_all();
        for task in tasks {
            task.ready.notify_all();
        }
    }
}

struct Owner(Arc<Shared>);

impl Drop for Owner {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[derive(Clone)]
pub(super) struct Scheduler {
    owner: Arc<Owner>,
    #[cfg(all(test, target_os = "macos"))]
    worker: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

struct Demand {
    shared: Arc<Shared>,
    task: Arc<Task>,
    id: u64,
    store: Option<Arc<SourceStore>>,
}

pub(super) struct Ticket {
    demand: Demand,
    _owner: Arc<Owner>,
}

impl Ticket {
    pub fn result(&self) -> Result<Option<Arc<DevelopedSource>>, Error> {
        if self
            .demand
            .store
            .as_ref()
            .is_some_and(|store| store.is_closed())
        {
            return Err(Error::UnusableSample("source demand retired"));
        }
        if self.demand.shared.state.lock().unwrap().closed {
            return Err(Error::UnusableSample("RAW development worker stopped"));
        }
        self.demand
            .task
            .result
            .lock()
            .unwrap()
            .clone()
            .transpose()
            .map_err(Error::Project)
    }

    fn wait(&self, cancelled: impl Fn() -> bool) -> Result<Arc<DevelopedSource>, Error> {
        loop {
            if cancelled() {
                return Err(Error::UnusableSample("source demand retired"));
            }
            if let Some(source) = self.result()? {
                return Ok(source);
            }
            let result = self.demand.task.result.lock().unwrap();
            if result.is_none() {
                drop(
                    self.demand
                        .task
                        .ready
                        .wait_timeout(result, Duration::from_millis(20))
                        .unwrap(),
                );
            }
        }
    }
}

impl Drop for Demand {
    fn drop(&mut self) {
        let retired = {
            let mut demands = self.task.demands.lock().unwrap();
            demands
                .iter()
                .position(|consumer| consumer.id == self.id)
                .map(|index| demands.swap_remove(index))
        };
        drop(retired);
        self.shared.changed.notify_all();
    }
}

impl Scheduler {
    pub fn new() -> Self {
        #[cfg(test)]
        {
            Self::start_native(Arc::new(|_, _, _| Ok(())))
        }
        #[cfg(not(test))]
        {
            Self::start_native()
        }
    }

    #[cfg(test)]
    pub(super) fn new_observed(
        observe: impl Fn(
            &std::path::Path,
            NativePhase,
            Option<&crate::raw::Metadata>,
        ) -> Result<(), String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self::start_native(Arc::new(observe))
    }

    fn start_native(#[cfg(test)] observe: NativeObserver) -> Self {
        Self::start(move |shared| {
            // Paused unpacked handles remain worker-local across queue arbitration.
            let mut unpacked: HashMap<u64, (Source, crate::raw::RawFile)> = HashMap::new();

            run(shared, |task| {
                if task.retired() {
                    unpacked.remove(&task.id);
                    return Err("source demand retired".into());
                }
                if let Some((record, raw)) = unpacked.remove(&task.id) {
                    #[cfg(test)]
                    observe(
                        &task.request.path,
                        NativePhase::DevelopBegin,
                        Some(&raw.metadata),
                    )?;
                    let scan = Scan::from_raw(raw, task.request.wb).map_err(|e| e.to_string())?;
                    #[cfg(test)]
                    observe(
                        &task.request.path,
                        NativePhase::DevelopEnd,
                        Some(&scan.source().metadata()),
                    )?;
                    document::verify_scan(&record, &scan).map_err(|e| e.to_string())?;
                    Ok(Some(scan.source().clone()))
                } else {
                    if let Some(key) = &task.request.key
                        && let Some(source) = task
                            .request
                            .store
                            .as_ref()
                            .and_then(|store| store.get(key))
                            .or_else(|| {
                                task.request
                                    .parent
                                    .as_ref()
                                    .and_then(|store| store.get(key))
                            })
                    {
                        return Ok(Some(source));
                    }

                    #[cfg(test)]
                    observe(&task.request.path, NativePhase::Open, None)?;
                    let (record, mut raw) =
                        document::open_source(&task.request.path).map_err(|e| e.to_string())?;
                    if let Some(basis) = &task.request.basis {
                        basis
                            .require_source(&record.development)
                            .map_err(|e| e.to_string())?;
                    }
                    if let Some(expected) = &task.request.expected {
                        document::verify_source(expected, &record).map_err(|e| e.to_string())?;
                    }

                    // Queue priority is not a different pixel policy or a smaller CPU budget.
                    raw.set_development_priority(crate::raw::DevelopmentPriority::Foreground);
                    #[cfg(test)]
                    observe(
                        &task.request.path,
                        NativePhase::UnpackBegin,
                        Some(&raw.metadata),
                    )?;
                    raw.ensure_unpacked().map_err(|e| e.to_string())?;
                    #[cfg(test)]
                    observe(
                        &task.request.path,
                        NativePhase::UnpackEnd,
                        Some(&raw.metadata),
                    )?;
                    unpacked.insert(task.id, (record, raw));
                    Ok(None)
                }
            });
        })
    }

    fn start(worker: impl FnOnce(Arc<Shared>) + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                tasks: Vec::new(),
                queue: Vec::new(),
                next: 0,
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let _worker = std::thread::spawn(move || {
            // Also wake consumers if the worker unwinds; this is not native/OOM recovery.
            let _exit = Owner(worker_shared.clone());
            worker(worker_shared);
        });
        Self {
            owner: Arc::new(Owner(shared)),
            #[cfg(all(test, target_os = "macos"))]
            worker: Arc::new(Mutex::new(Some(_worker))),
        }
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn close_and_wait(&self) -> Result<(), String> {
        self.close();
        let worker = self.worker.lock().unwrap().take();
        if let Some(worker) = worker {
            worker
                .join()
                .map_err(|_| "native diagnostic worker panicked".to_owned())?;
        }
        Ok(())
    }

    pub fn close(&self) {
        self.owner.0.close();
    }

    #[cfg(test)]
    pub(super) fn test_executor(
        mut step: impl FnMut(&std::path::Path) -> Result<Option<Arc<DevelopedSource>>, String>
        + Send
        + 'static,
    ) -> Self {
        Self::start(move |shared| run(shared, |task| step(&task.request.path)))
    }

    #[cfg(test)]
    pub(super) fn waiter_count(&self) -> usize {
        self.owner
            .0
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .map(|task| task.demands.lock().unwrap().len())
            .sum()
    }

    pub fn load(
        &self,
        request: Request,
        priority: Priority,
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<DevelopedSource>, Error> {
        if cancelled() {
            return Err(Error::UnusableSample("source demand retired"));
        }
        self.submit(request, priority)?.wait(cancelled)
    }

    pub fn submit(&self, mut request: Request, priority: Priority) -> Result<Ticket, Error> {
        if request.retired() {
            return Err(Error::UnusableSample("source demand retired"));
        }
        // Missing/stale stats disable joining, not authoritative opening. Never refresh
        // a supplied pre-work revision after work or relabel the worker's result.
        if request.key.as_ref().is_some_and(|key| !key.current()) {
            request.key = None;
        }
        let retain = request.retain;
        let consumer_store = request.store.clone();
        let shared = self.owner.0.clone();
        let demand = {
            let mut state = shared.state.lock().unwrap();
            if state.closed {
                return Err(Error::UnusableSample("RAW development worker stopped"));
            }
            let task =
                if let Some(task) = state.tasks.iter().find(|task| request.joins(&task.request)) {
                    task.clone()
                } else {
                    state.next += 1;
                    let task = Arc::new(Task {
                        id: state.next,
                        request,
                        demands: Mutex::new(Vec::new()),
                        next: AtomicU64::new(0),
                        started: std::sync::atomic::AtomicBool::new(false),
                        result: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    state.tasks.push(task.clone());
                    state.queue.push(task.clone());
                    task
                };
            let id = task.next.fetch_add(1, Ordering::Relaxed);
            task.demands.lock().unwrap().push(Consumer {
                id,
                priority,
                store: consumer_store.clone(),
                retain,
            });
            Demand {
                shared: shared.clone(),
                task,
                id,
                store: consumer_store.clone(),
            }
        };
        shared.changed.notify_all();
        Ok(Ticket {
            demand,
            _owner: self.owner.clone(),
        })
    }
}

fn run(
    shared: Arc<Shared>,
    mut step: impl FnMut(&Task) -> Result<Option<Arc<DevelopedSource>>, String>,
) {
    loop {
        let task = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.closed {
                    return;
                }
                if let Some(index) = state
                    .queue
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, task)| (task.priority(), task.id))
                    .map(|(index, _)| index)
                {
                    let task = state.queue.remove(index);
                    if task.abandoned() {
                        state.tasks.retain(|entry| !Arc::ptr_eq(entry, &task));
                    } else {
                        task.started.store(true, Ordering::Relaxed);
                    }
                    break task;
                }
                state = shared.changed.wait(state).unwrap();
            }
        };
        if task.abandoned() {
            finish(&shared, &task, Err("source demand retired".into()));
            continue;
        }
        match step(&task) {
            Ok(None) => {
                let mut state = shared.state.lock().unwrap();
                if !state.closed {
                    state.queue.push(task.clone());
                }
                shared.changed.notify_all();
            }
            result => {
                let result = result.and_then(|source| {
                    let source = source.expect("completed native step returns a source");
                    if task.retired() || shared.state.lock().unwrap().closed {
                        return Err("source demand retired".into());
                    }
                    let mut source = source;
                    for (store, retain) in task.scopes() {
                        if !retain || store.is_closed() {
                            continue;
                        }
                        let record =
                            document::source_metadata(&task.request.path, &source.metadata())
                                .map_err(|e| e.to_string())?;
                        if let Some(key) = SourceKey::from_revision(
                            &task.request.path,
                            task.request.file.clone(),
                            &record.development,
                            &Development::new(crate::raw::version(), source.fixed_wb()),
                        ) && let Some(canonical) = store
                            .publish_source(key, source.clone())
                            .map_err(|e| e.to_string())?
                        {
                            source = canonical;
                        }
                    }
                    Ok(source)
                });
                finish(&shared, &task, result);
            }
        }
    }
}

fn finish(shared: &Shared, task: &Arc<Task>, result: Completion) {
    // Keep the task/result owned outside both metadata locks, including final release.
    *task.result.lock().unwrap() = Some(result);
    let retired = {
        let mut state = shared.state.lock().unwrap();
        state
            .tasks
            .iter()
            .position(|entry| Arc::ptr_eq(entry, task))
            .map(|index| state.tasks.remove(index))
    };
    task.ready.notify_all();
    drop(retired);
}

#[cfg(test)]
#[path = "editor_development_tests.rs"]
mod tests;
