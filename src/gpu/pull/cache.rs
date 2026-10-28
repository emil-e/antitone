//! Value-keyed generated resources. Only submitted initializers make a value reusable.
use std::{
    any::Any,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
trait Value: Any + fmt::Debug + Send + Sync {}
impl<T: Any + fmt::Debug + Send + Sync> Value for T {}
#[derive(Clone)]
pub struct RetainedKey {
    value: Arc<dyn Value>,
    equals: fn(&dyn Value, &dyn Value) -> bool,
}
impl RetainedKey {
    pub fn new<T: Any + fmt::Debug + Send + Sync + PartialEq>(value: T) -> Self {
        Self {
            value: Arc::new(value),
            equals: |a, b| {
                (a as &dyn Any).downcast_ref::<T>() == (b as &dyn Any).downcast_ref::<T>()
            },
        }
    }
    pub(crate) fn same_type(&self, other: &Self) -> bool {
        self.value.as_ref().type_id() == other.value.as_ref().type_id()
    }
    pub fn get<T: Any>(&self) -> Option<&T> {
        (self.value.as_ref() as &dyn Any).downcast_ref()
    }
}
impl fmt::Debug for RetainedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}
impl PartialEq for RetainedKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value)
            || (self.equals)(self.value.as_ref(), other.value.as_ref())
    }
}
pub(crate) struct RetainedBuffer {
    pub key: RetainedKey,
    pub pixels: wgpu::Buffer,
    ready: AtomicBool,
}
impl RetainedBuffer {
    pub fn new(device: &wgpu::Device, key: RetainedKey, bytes: u64) -> Self {
        Self {
            key,
            pixels: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pull retained generated value"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ready: AtomicBool::new(false),
        }
    }
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn submitted(&self) {
        self.ready.store(true, Ordering::Release);
    }
}
