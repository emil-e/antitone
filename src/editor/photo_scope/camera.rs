//! Photo-owned camera preparation; views observe readiness without owning its pixels.
use crate::{
    application::UiContext,
    cancellation::{CancelOnDrop, Cancellation},
    editor::{Editor, memory, source},
    gpu::evaluation::{ActiveInput, Value},
};
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
pub(in crate::editor) struct State {
    preparation: Option<Preparation>,
    #[cfg(test)]
    pressure: Option<memory::Pressure>,
}

struct Preparation {
    source: Value,
    shared: Arc<Shared>,
    _task: async_executor::Task<()>,
    _watch: memory::PressureWatch,
    _retire: CancelOnDrop,
}

struct Shared {
    state: Mutex<Progress>,
    changed: event_listener::Event,
    stop: Cancellation,
}

struct Progress {
    image: Option<ActiveInput>,
    completed: Option<Result<(), String>>,
    pressure: memory::Pressure,
}

impl Shared {
    fn pressure(&self, pressure: memory::Pressure) {
        let mut state = self.state.lock().unwrap();
        state.pressure = pressure;
        if pressure != memory::Pressure::Normal {
            self.stop.cancel();
            state.image = None;
            state.completed = Some(Ok(()));
        }
        drop(state);
        self.changed.notify(usize::MAX);
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.changed.notify(usize::MAX);
    }
}

#[derive(Clone)]
pub(in crate::editor) struct Ready(Weak<Shared>);

impl Ready {
    #[cfg(test)]
    pub(in crate::editor) fn finished(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|shared| shared.state.lock().unwrap().completed.is_some())
    }

    #[cfg(test)]
    pub(in crate::editor) fn same_preparation(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }

    pub async fn wait(&self) -> Result<bool, String> {
        loop {
            let changed = {
                let Some(shared) = self.0.upgrade() else {
                    return Ok(false);
                };
                let changed = shared.changed.listen();
                if let Some(result) = &shared.state.lock().unwrap().completed {
                    return result.clone().map(|()| true);
                }
                changed
            };
            changed.await;
        }
    }
}

impl State {
    #[cfg(test)]
    pub(in crate::editor) fn ready(&self) -> Option<Ready> {
        self.preparation
            .as_ref()
            .map(|preparation| Ready(Arc::downgrade(&preparation.shared)))
    }

    #[cfg(test)]
    pub(in crate::editor) fn pressure(&mut self, pressure: memory::Pressure) {
        self.pressure = Some(pressure);
        if let Some(preparation) = &self.preparation {
            preparation.shared.pressure(pressure);
        }
    }

    pub(in crate::editor) fn prepare(
        &mut self,
        pixels: &source::Context,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
        cx: &UiContext<'_, Editor>,
    ) -> Result<Ready, String> {
        let source = crate::gpu::evaluation::camera_source(
            &pixels.scan().map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if let Some(preparation) = &self.preparation
            && preparation.source.same_definition(&source)
        {
            let progress = preparation.shared.state.lock().unwrap();
            if progress.completed.is_none()
                || progress
                    .image
                    .as_ref()
                    .is_some_and(|image| image.matches(&source))
                || progress.pressure != memory::Pressure::Normal
            {
                return Ok(Ready(Arc::downgrade(&preparation.shared)));
            }
        }
        // Source replacement retires the old owner before admitting its replacement.
        self.preparation = None;
        let pressure = crate::sys::memory_pressure();
        #[cfg(test)]
        let pressure = self.pressure.unwrap_or(pressure);
        let shared = Arc::new(Shared {
            state: Mutex::new(Progress {
                image: None,
                completed: None,
                pressure,
            }),
            changed: event_listener::Event::new(),
            stop: Cancellation::default(),
        });
        shared.pressure(pressure);
        let weak = Arc::downgrade(&shared);
        let watch = memory::PressureWatch::observe(move |pressure| {
            if let Some(shared) = weak.upgrade() {
                shared.pressure(pressure);
            }
        });
        let ready = Ready(Arc::downgrade(&shared));
        let weak = Arc::downgrade(&shared);
        let stop = shared.stop.clone();
        let retire = stop.cancel_on_drop();
        let input = source.clone();
        let task = cx
            .spawn(move |_| async move {
                let result = async {
                    if stop.is_cancelled() {
                        return Ok(None);
                    }
                    let device = services.device.clone();
                    let queue = services.queue.clone();
                    let renderer =
                        crate::background::run(move |_| services.renderers.get(&device, &queue))
                            .await
                            .map_err(|_| "Camera preparation worker panicked".to_owned())?
                            .map_err(|error| error.to_string())?;
                    renderer
                        .evaluation()
                        .prepare_input(
                            &services.queue,
                            &submissions,
                            &renderer.evaluation().materializations(),
                            input,
                            || stop.is_cancelled(),
                        )
                        .await
                        .map_err(|error| error.to_string())
                }
                .await;
                if let Some(shared) = weak.upgrade() {
                    let mut progress = shared.state.lock().unwrap();
                    if !stop.is_cancelled() {
                        match result {
                            Ok(image) => {
                                progress.image = image;
                                progress.completed = Some(Ok(()));
                            }
                            Err(error) => progress.completed = Some(Err(error)),
                        }
                    }
                    drop(progress);
                    shared.changed.notify(usize::MAX);
                }
            })
            .ok_or_else(|| "Application stopped".to_owned())?;
        self.preparation = Some(Preparation {
            source,
            shared,
            _task: task,
            _watch: watch,
            _retire: retire,
        });
        Ok(ready)
    }
}
