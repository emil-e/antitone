//! Native source loading. Mutable SDK owners never leave their worker
//! thread; consumers join immutable completion, independently of optional cache
//! ownership and UI tickets. Started work stays pinned to its owner, and at most
//! two owners overlap independent sources, yielding to foreground demand.
use super::{SourceKey, SourceProvider, metadata as source_metadata};
use crate::{
    Error,
    project::Source,
    scan::{DevelopedSource, Scan},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Condvar, Mutex, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};
use std::{path::PathBuf, sync::Arc};
#[cfg(test)]
use std::{sync::mpsc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Foreground,
    Visible,
    Eager,
}

#[derive(Clone)]
pub(crate) struct Request {
    pub path: PathBuf,
    pub wb: Option<[f64; 4]>,
    pub expected: Option<Source>,
    pub key: Option<SourceKey>,
}

impl Request {
    fn joins(&self, other: &Self) -> bool {
        self.key
            .as_ref()
            .zip(other.key.as_ref())
            .is_some_and(|(a, b)| a == b)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum NativePhase {
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
// A step stays on its owner's thread: it holds worker-local mutable native
// handles that are deliberately !Send.
type Step = Box<dyn FnMut(&Arc<Task>) -> Result<Option<Arc<DevelopedSource>>, String>>;

struct Consumer {
    id: u64,
    priority: Priority,
    waker: Option<Waker>,
}

const UNOWNED: usize = usize::MAX;

struct Task {
    id: u64,
    request: Request,
    demands: Mutex<Vec<Consumer>>,
    next: AtomicU64,
    started: std::sync::atomic::AtomicBool,
    owner: AtomicUsize,
    share: AtomicUsize,
    sdk: std::sync::atomic::AtomicBool,
    result: Mutex<Option<Completion>>,
    ready: Condvar,
}

impl Task {
    fn new(id: u64, request: Request) -> Self {
        Self {
            id,
            request,
            demands: Mutex::new(Vec::new()),
            next: AtomicU64::new(0),
            started: std::sync::atomic::AtomicBool::new(false),
            owner: AtomicUsize::new(UNOWNED),
            share: AtomicUsize::new(1),
            sdk: std::sync::atomic::AtomicBool::new(false),
            result: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn wake_waiters(&self) {
        let wakers: Vec<_> = self
            .demands
            .lock()
            .unwrap()
            .iter_mut()
            .filter_map(|consumer| consumer.waker.take())
            .collect();
        self.ready.notify_all();
        for waker in wakers {
            waker.wake();
        }
    }

    fn priority(&self) -> Priority {
        self.demands
            .lock()
            .unwrap()
            .iter()
            .map(|consumer| consumer.priority)
            .min()
            .unwrap_or(Priority::Eager)
    }

    fn abandoned(&self) -> bool {
        !self.started.load(Ordering::Relaxed) && self.demands.lock().unwrap().is_empty()
    }
}

struct State {
    tasks: Vec<Arc<Task>>,
    // Covers delivery/publication handoff without retaining completed pixels.
    live: Vec<(SourceKey, Weak<DevelopedSource>)>,
    queue: Vec<Arc<Task>>,
    next: u64,
    steps: usize,
    // In-flight steps that run an SDK transaction, which the bridge serializes
    // process-wide, so no other native step can make progress beside them.
    sdk_steps: usize,
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn retire_if_unwanted(&self, task: &Arc<Task>) -> bool {
        let mut state = self.state.lock().unwrap();
        if !task.demands.lock().unwrap().is_empty() {
            return false;
        }
        // Exclude new joiners before worker-local native cleanup starts.
        state.tasks.retain(|entry| !Arc::ptr_eq(entry, task));
        true
    }

    fn close(&self) {
        let tasks = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            state.queue.clear();
            std::mem::take(&mut state.tasks)
        };
        self.changed.notify_all();
        for task in tasks {
            task.wake_waiters();
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
pub struct SourceLoader {
    owner: Arc<Owner>,
    #[cfg(test)]
    workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

struct Demand {
    shared: Arc<Shared>,
    task: Arc<Task>,
    id: u64,
}

pub(crate) struct Ticket {
    demand: Demand,
    _owner: Arc<Owner>,
}

impl Future for Ticket {
    type Output = Result<Arc<DevelopedSource>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Register before checking completion. Publication/closure can race the first poll.
        {
            let mut demands = self.demand.task.demands.lock().unwrap();
            if let Some(consumer) = demands
                .iter_mut()
                .find(|consumer| consumer.id == self.demand.id)
            {
                match &mut consumer.waker {
                    Some(waker) => waker.clone_from(cx.waker()),
                    None => consumer.waker = Some(cx.waker().clone()),
                }
            }
        }
        match self.result() {
            Ok(Some(source)) => Poll::Ready(Ok(source)),
            Ok(None) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

impl Ticket {
    pub fn result(&self) -> Result<Option<Arc<DevelopedSource>>, Error> {
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

    #[cfg(test)]
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
        let mut demands = self.task.demands.lock().unwrap();
        if let Some(index) = demands.iter().position(|consumer| consumer.id == self.id) {
            demands.swap_remove(index);
        }
        drop(demands);
        self.shared.changed.notify_all();
    }
}

impl Default for SourceLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceProvider for SourceLoader {
    async fn load_source(
        &self,
        key: SourceKey,
        priority: Priority,
    ) -> Result<Arc<DevelopedSource>, Error> {
        let source = self.submit_key(key.clone(), priority)?.await?;
        if !key.matches(&source) {
            return Err(Error::Project(
                "developed source differs from requested development".into(),
            ));
        }
        Ok(source)
    }
}

impl SourceLoader {
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
    pub(crate) fn new_observed(
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
        Self::start_workers(2, move |_, shared| {
            #[cfg(test)]
            let observe = observe.clone();
            // LibRaw decoded handles and SDK inspected headers stay worker-local across
            // arbitration; each owner keeps its own mutable map and never shares handles.
            let mut unpacked: HashMap<u64, Unpacked> = HashMap::new();

            Box::new(move |task| {
                if shared.retire_if_unwanted(task) {
                    unpacked.remove(&task.id);
                    return Err("source demand retired".into());
                }
                if let Some(input) = unpacked.remove(&task.id) {
                    #[cfg(test)]
                    observe(
                        &task.request.path,
                        NativePhase::DevelopBegin,
                        Some(&input.raw.metadata),
                    )?;
                    let mut raw = input.raw;
                    raw.set_development_share(task.share.load(Ordering::Relaxed));
                    let scan = Scan::from_raw(raw, task.request.wb).map_err(|e| e.to_string())?;
                    #[cfg(test)]
                    observe(
                        &task.request.path,
                        NativePhase::DevelopEnd,
                        Some(&scan.source().metadata()),
                    )?;
                    source_metadata::verify_scan(&input.record, &scan)
                        .map_err(|e| e.to_string())?;
                    Ok(Some(scan.source().clone()))
                } else {
                    #[cfg(test)]
                    observe(&task.request.path, NativePhase::Open, None)?;
                    let (record, mut raw) =
                        source_metadata::open_source(&task.request.path, task.request.wb)
                            .map_err(|e| e.to_string())?;
                    // The bridge serializes SDK transactions process-wide, so the
                    // develop phase must not share native execution with anything.
                    task.sdk.store(raw.metadata.uses_sdk(), Ordering::Relaxed);
                    if let Some(expected) = &task.request.expected {
                        source_metadata::verify_source(expected, &record)
                            .map_err(|e| e.to_string())?;
                    }
                    if task
                        .request
                        .key
                        .as_ref()
                        .is_some_and(|key| key.development() != &record.development)
                    {
                        return Err("source differs from requested development".into());
                    }

                    // Queue priority is not a different pixel policy or a smaller CPU budget.
                    raw.set_development_priority(crate::raw::DevelopmentPriority::Foreground);
                    raw.set_development_share(task.share.load(Ordering::Relaxed));
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
                    unpacked.insert(task.id, Unpacked { record, raw });
                    Ok(None)
                }
            })
        })
    }

    fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State {
                tasks: Vec::new(),
                live: Vec::new(),
                queue: Vec::new(),
                next: 0,
                steps: 0,
                sdk_steps: 0,
                closed: false,
            }),
            changed: Condvar::new(),
        })
    }

    #[cfg(test)]
    fn start(worker: impl FnOnce(Arc<Shared>) + Send + 'static) -> Self {
        let shared = Self::shared();
        let worker_shared = shared.clone();
        let workers = vec![std::thread::spawn(move || {
            // Also wake consumers if a worker unwinds; this is not native/OOM recovery.
            let _exit = Owner(worker_shared.clone());
            worker(worker_shared);
        })];
        #[cfg(test)]
        let workers = Arc::new(Mutex::new(workers));
        #[cfg(not(test))]
        drop(workers);
        Self {
            owner: Arc::new(Owner(shared)),
            #[cfg(test)]
            workers,
        }
    }

    // Each owner runs its own step closure so mutable native ownership stays
    // thread-local; the factory is consulted once per owner on its own thread.
    fn start_workers(
        count: usize,
        build: impl Fn(usize, Arc<Shared>) -> Step + Send + Sync + 'static,
    ) -> Self {
        let shared = Self::shared();
        let build = Arc::new(build);
        let mut workers = Vec::new();
        for slot in 0..count {
            let worker_shared = shared.clone();
            let build = build.clone();
            workers.push(std::thread::spawn(move || {
                // Also wake consumers if a worker unwinds; this is not native/OOM recovery.
                let _exit = Owner(worker_shared.clone());
                let mut step = build(slot, worker_shared.clone());
                run(worker_shared, slot, move |task| step(task));
            }));
        }
        #[cfg(test)]
        let workers = Arc::new(Mutex::new(workers));
        #[cfg(not(test))]
        drop(workers); // Detached: production retires owners through `Owner`/close.
        Self {
            owner: Arc::new(Owner(shared)),
            #[cfg(test)]
            workers,
        }
    }

    #[cfg(test)]
    pub(crate) fn close_and_wait(&self) -> Result<(), String> {
        self.close();
        let workers = std::mem::take(&mut *self.workers.lock().unwrap());
        for worker in workers {
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
    pub(crate) fn test_executor(
        mut step: impl FnMut(&std::path::Path) -> Result<Option<Arc<DevelopedSource>>, String>
        + Send
        + 'static,
    ) -> Self {
        Self::start(move |shared| run(shared, 0, |task| step(&task.request.path)))
    }

    #[cfg(test)]
    pub(crate) fn waiter_count(&self) -> usize {
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

    #[cfg(test)]
    pub(crate) fn load(
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

    pub(crate) fn submit_key(&self, key: SourceKey, priority: Priority) -> Result<Ticket, Error> {
        self.submit(
            Request {
                path: key.path().to_owned(),
                wb: Some(key.development().decode.fixed_wb),
                expected: None,
                key: Some(key),
            },
            priority,
        )
    }

    pub(crate) fn submit(&self, request: Request, priority: Priority) -> Result<Ticket, Error> {
        self.submit_request(request, priority)
    }

    fn submit_request(&self, request: Request, priority: Priority) -> Result<Ticket, Error> {
        let shared = self.owner.0.clone();
        let demand = {
            let mut state = shared.state.lock().unwrap();
            if state.closed {
                return Err(Error::UnusableSample("RAW development worker stopped"));
            }
            state.live.retain(|(_, source)| source.strong_count() != 0);
            let existing = request.key.as_ref().and_then(|key| {
                state
                    .live
                    .iter()
                    .find(|(known, _)| known == key)
                    .and_then(|(_, source)| source.upgrade())
            });
            let task = if let Some(source) = existing {
                state.next += 1;
                let task = Arc::new(Task::new(state.next, request));
                *task.result.lock().unwrap() = Some(Ok(source));
                task
            } else if let Some(task) = state.tasks.iter().find(|task| request.joins(&task.request))
            {
                task.clone()
            } else {
                state.next += 1;
                let task = Arc::new(Task::new(state.next, request));
                state.tasks.push(task.clone());
                state.queue.push(task.clone());
                task
            };
            let id = task.next.fetch_add(1, Ordering::Relaxed);
            task.demands.lock().unwrap().push(Consumer {
                id,
                priority,
                waker: None,
            });
            Demand {
                shared: shared.clone(),
                task,
                id,
            }
        };
        shared.changed.notify_all();
        Ok(Ticket {
            demand,
            _owner: self.owner.clone(),
        })
    }
}

// Started work stays on its owner because worker-local mutable native handles are
// not Send. Unowned starts obey the yield rule — background work begins only when
// no foreground demand lives.
fn eligible(state: &State, task: &Task, slot: usize) -> bool {
    if task.sdk.load(Ordering::Relaxed) {
        if state.steps != 0 {
            return false;
        }
    } else if state.sdk_steps != 0 {
        return false;
    }
    match task.owner.load(Ordering::Relaxed) {
        UNOWNED => {
            task.priority() == Priority::Foreground
                || !state
                    .tasks
                    .iter()
                    .any(|entry| entry.priority() == Priority::Foreground)
        }
        owner => owner == slot,
    }
}

// The in-flight step count feeds each call's thread share at selection.
struct StepCount<'a> {
    shared: &'a Shared,
    sdk: bool,
}
impl Drop for StepCount<'_> {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.steps -= 1;
            if self.sdk {
                state.sdk_steps -= 1;
            }
        }
        // Eligibility reads these counters, and completion notifies before this
        // guard runs, so a waiter woken there re-checks against the old count.
        // Release the capacity here or that waiter sleeps with work queued.
        self.shared.changed.notify_all();
    }
}

fn run(
    shared: Arc<Shared>,
    slot: usize,
    mut step: impl FnMut(&Arc<Task>) -> Result<Option<Arc<DevelopedSource>>, String>,
) {
    loop {
        let (task, sdk) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.closed {
                    return;
                }
                if let Some(index) = state
                    .queue
                    .iter()
                    .enumerate()
                    .filter(|(_, task)| eligible(&state, task, slot))
                    .min_by_key(|(_, task)| (task.priority(), task.id))
                    .map(|(index, _)| index)
                {
                    let task = state.queue.remove(index);
                    // One read decides both the counter and the guard, so the
                    // increment and decrement cannot disagree.
                    let sdk = task.sdk.load(Ordering::Relaxed);
                    if task.abandoned() {
                        state.tasks.retain(|entry| !Arc::ptr_eq(entry, &task));
                    } else {
                        task.started.store(true, Ordering::Relaxed);
                        task.owner.store(slot, Ordering::Relaxed);
                        task.share.store(state.steps + 1, Ordering::Relaxed);
                    }
                    if sdk {
                        state.sdk_steps += 1;
                    }
                    state.steps += 1;
                    break (task, sdk);
                }
                state = shared.changed.wait(state).unwrap();
            }
        };
        let _steps = StepCount {
            shared: &shared,
            sdk,
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
                } else {
                    drop(state);
                    finish(&shared, &task, Err("RAW development worker stopped".into()));
                }
                shared.changed.notify_all();
            }
            result => {
                let result = result.and_then(|source| {
                    let source = source.expect("completed native step returns a source");
                    if shared.state.lock().unwrap().closed {
                        return Err("source demand retired".into());
                    }
                    Ok(source)
                });
                finish(&shared, &task, result);
            }
        }
    }
}

struct Unpacked {
    record: Source,
    raw: crate::raw::RawFile,
}

fn finish(shared: &Shared, task: &Arc<Task>, result: Completion) {
    // Keep the task/result owned outside both metadata locks, including final release.
    let live = result.as_ref().ok().and_then(|source| {
        task.request
            .key
            .as_ref()
            .map(|key| (key.clone(), Arc::downgrade(source)))
    });
    *task.result.lock().unwrap() = Some(result);
    let retired = {
        let mut state = shared.state.lock().unwrap();
        state.live.retain(|(_, source)| source.strong_count() != 0);
        if let Some(live) = live {
            state.live.push(live);
        }
        state
            .tasks
            .iter()
            .position(|entry| Arc::ptr_eq(entry, task))
            .map(|index| state.tasks.remove(index))
    };
    task.wake_waiters();
    drop(retired);
    // Yielding owners sleep until eligibility changes, e.g. a foreground task ends.
    shared.changed.notify_all();
}

#[cfg(test)]

#[path = "loader_tests.rs"]

mod tests;
