//! Bounded editor native owners. Mutable SDK owners never leave their worker
//! thread; consumers join immutable completion, independently of optional cache
//! ownership and UI tickets. Started work stays pinned to its owner, and at most
//! two owners overlap independent sources, yielding to foreground demand.
use super::*;
use crate::{
    scan::DevelopedSource,
    source_store::{SourceKey, SourceStore},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
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
// A step stays on its owner's thread: it holds worker-local mutable native
// handles that are deliberately !Send.
type Step = Box<dyn FnMut(&Arc<Task>) -> Result<Option<Arc<DevelopedSource>>, String>>;

struct Consumer {
    id: u64,
    priority: Priority,
    store: Option<Arc<SourceStore>>,
    retain: bool,
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
    native_backing: std::sync::atomic::AtomicBool,
    sdk: std::sync::atomic::AtomicBool,
    admissions: Mutex<Vec<crate::blocking_jobs::Admission>>,
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
            native_backing: std::sync::atomic::AtomicBool::new(false),
            sdk: std::sync::atomic::AtomicBool::new(false),
            admissions: Mutex::new(Vec::new()),
            result: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn release_admissions(&self) {
        let retired = std::mem::take(&mut *self.admissions.lock().unwrap());
        drop(retired);
    }

    fn release_unbacked_admissions(&self) {
        if !self.native_backing.load(Ordering::Acquire) {
            self.release_admissions();
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
    fn close(&self) {
        let tasks = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            state.queue.clear();
            std::mem::take(&mut state.tasks)
        };
        self.changed.notify_all();
        for task in tasks {
            if !task.started.load(Ordering::Relaxed) {
                task.release_admissions();
            }
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
pub(super) struct Scheduler {
    owner: Arc<Owner>,
    #[cfg(test)]
    workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
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
    closed: Option<Pin<Box<event_listener::EventListener>>>,
}

impl Future for Ticket {
    type Output = Result<Arc<DevelopedSource>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.closed.is_none() {
            self.closed = self
                .demand
                .store
                .as_ref()
                .map(|store| Box::pin(store.listen_for_close()));
        }
        if let Some(closed) = &mut self.closed {
            let _ = closed.as_mut().poll(cx);
        }
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
        let (retired, admissions) = {
            // Match native dispatch's state → consumer lock order so a last queued
            // waiter cannot release admission as the worker starts its call.
            let _state = self.shared.state.lock().unwrap();
            let mut demands = self.task.demands.lock().unwrap();
            let retired = demands
                .iter()
                .position(|consumer| consumer.id == self.id)
                .map(|index| demands.swap_remove(index));
            let admissions = if demands.is_empty() && !self.task.started.load(Ordering::Relaxed) {
                std::mem::take(&mut *self.task.admissions.lock().unwrap())
            } else {
                Vec::new()
            };
            (retired, admissions)
        };
        drop((retired, admissions));
        self.shared.changed.notify_all();
    }
}

impl Scheduler {
    pub fn new() -> Self {
        #[cfg(test)]
        {
            Self::start_native(Arc::new(|_, _, _| Ok(())), crate::sys::memory_pressure)
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
        Self::start_native(Arc::new(observe), crate::sys::memory_pressure)
    }

    /// The second owner admits background demand only under Normal pressure, so
    /// a scenario that needs both owners running concurrently must inject that
    /// reading; live system pressure during a test run would otherwise turn a
    /// correct schedule into a spurious rendezvous timeout.
    #[cfg(test)]
    pub(super) fn new_observed_with_pressure(
        observe: impl Fn(
            &std::path::Path,
            NativePhase,
            Option<&crate::raw::Metadata>,
        ) -> Result<(), String>
        + Send
        + Sync
        + 'static,
        pressure: fn() -> crate::sys::MemoryPressure,
    ) -> Self {
        Self::start_native(Arc::new(observe), pressure)
    }

    fn start_native(
        #[cfg(test)] observe: NativeObserver,
        #[cfg(test)] pressure: fn() -> crate::sys::MemoryPressure,
    ) -> Self {
        #[cfg(not(test))]
        let pressure = crate::sys::memory_pressure;
        Self::start_workers(2, pressure, move |_| {
            #[cfg(test)]
            let observe = observe.clone();
            // LibRaw decoded handles and SDK inspected headers stay worker-local across
            // arbitration; each owner keeps its own mutable map and never shares handles.
            let mut unpacked: HashMap<u64, Unpacked> = HashMap::new();

            Box::new(move |task| {
                if task.retired() {
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
                    document::verify_scan(&input.record, &scan).map_err(|e| e.to_string())?;
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
                        document::open_source(&task.request.path, task.request.wb)
                            .map_err(|e| e.to_string())?;
                    // The bridge serializes SDK transactions process-wide, so the
                    // develop phase must not share native execution with anything.
                    task.sdk.store(raw.metadata.uses_sdk(), Ordering::Relaxed);
                    if let Some(expected) = &task.request.expected {
                        document::verify_source(expected, &record).map_err(|e| e.to_string())?;
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
                    task.native_backing.store(true, Ordering::Release);
                    unpacked.insert(
                        task.id,
                        Unpacked {
                            record,
                            raw,
                            _backing: NativeBacking(task.clone()),
                        },
                    );
                    Ok(None)
                }
            })
        })
    }

    fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State {
                tasks: Vec::new(),
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
        pressure: fn() -> crate::sys::MemoryPressure,
        build: impl Fn(usize) -> Step + Send + Sync + 'static,
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
                let mut step = build(slot);
                run(worker_shared, slot, pressure, move |task| step(task));
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
    pub(super) fn close_and_wait(&self) -> Result<(), String> {
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
    pub(super) fn test_executor(
        mut step: impl FnMut(&std::path::Path) -> Result<Option<Arc<DevelopedSource>>, String>
        + Send
        + 'static,
    ) -> Self {
        Self::start(move |shared| {
            run(shared, 0, crate::sys::memory_pressure, |task| {
                step(&task.request.path)
            })
        })
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

    #[cfg(test)]
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

    pub fn submit(&self, request: Request, priority: Priority) -> Result<Ticket, Error> {
        self.submit_with_admission(request, priority, None)
    }

    pub fn submit_admitted(
        &self,
        request: Request,
        priority: Priority,
        admission: crate::blocking_jobs::Admission,
    ) -> Result<Ticket, Error> {
        self.submit_with_admission(request, priority, Some(admission))
    }

    fn submit_with_admission(
        &self,
        mut request: Request,
        priority: Priority,
        admission: Option<crate::blocking_jobs::Admission>,
    ) -> Result<Ticket, Error> {
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
                    let task = Arc::new(Task::new(state.next, request));
                    state.tasks.push(task.clone());
                    state.queue.push(task.clone());
                    task
                };
            if let Some(admission) = admission {
                task.admissions.lock().unwrap().push(admission);
            }
            let id = task.next.fetch_add(1, Ordering::Relaxed);
            task.demands.lock().unwrap().push(Consumer {
                id,
                priority,
                store: consumer_store.clone(),
                retain,
                waker: None,
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
            closed: None,
        })
    }
}

// Started work stays on its owner because worker-local mutable native handles are
// not Send. Unowned starts obey the yield rule — background work begins only when
// no foreground demand lives — and the second owner idles under memory pressure.
fn eligible(state: &State, task: &Task, slot: usize, pressure: crate::sys::MemoryPressure) -> bool {
    if task.sdk.load(Ordering::Relaxed) {
        if state.steps != 0 {
            return false;
        }
    } else if state.sdk_steps != 0 {
        return false;
    }
    match task.owner.load(Ordering::Relaxed) {
        UNOWNED => {
            if slot != 0 && !matches!(pressure, crate::sys::MemoryPressure::Normal) {
                return false;
            }
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
    pressure: fn() -> crate::sys::MemoryPressure,
    mut step: impl FnMut(&Arc<Task>) -> Result<Option<Arc<DevelopedSource>>, String>,
) {
    loop {
        let (task, sdk) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.closed {
                    return;
                }
                let level = pressure();
                if let Some(index) = state
                    .queue
                    .iter()
                    .enumerate()
                    .filter(|(_, task)| eligible(&state, task, slot, level))
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
        let _running = Running {
            task: &task,
            shared: &shared,
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
                    if task.retired() || shared.state.lock().unwrap().closed {
                        return Err("source demand retired".into());
                    }
                    let mut source = source;
                    for (store, retain) in task.scopes() {
                        if !retain || store.is_closed() {
                            continue;
                        }
                        let record = document::source_metadata(
                            &task.request.path,
                            &source.metadata(),
                            source.fixed_wb(),
                        )
                        .map_err(|e| e.to_string())?;
                        if let Some(key) = SourceKey::from_revision(
                            &task.request.path,
                            task.request.file.clone(),
                            &record.development,
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

// The worker-local RawFile drops before this final field. Started native backing
// retains admission across arbitration and closure, including late joined demands.
struct Unpacked {
    record: Source,
    raw: crate::raw::RawFile,
    _backing: NativeBacking,
}

struct NativeBacking(Arc<Task>);
impl Drop for NativeBacking {
    fn drop(&mut self) {
        self.0.native_backing.store(false, Ordering::Release);
        self.0.release_admissions();
    }
}

// Unstarted work can retire immediately. Running calls and parked native backing
// retain admission until their physical work and mutable ownership end.
struct Running<'a> {
    task: &'a Task,
    shared: &'a Shared,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() || self.shared.state.lock().unwrap().closed {
            self.task.release_unbacked_admissions();
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
    task.release_unbacked_admissions();
    task.wake_waiters();
    drop(retired);
    // Yielding owners sleep until eligibility changes, e.g. a foreground task ends.
    shared.changed.notify_all();
}

#[cfg(test)]
#[path = "editor_development_tests.rs"]
mod tests;
