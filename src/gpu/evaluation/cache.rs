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
#[derive(Clone)]
pub(crate) struct Retention(Arc<RetentionState>);
struct RetentionState {
    limit: u64,
    bytes: std::sync::atomic::AtomicU64,
}
pub(crate) struct Lease {
    owner: Arc<RetentionState>,
    bytes: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.owner.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
impl Default for Retention {
    fn default() -> Self {
        Self::new(super::CachePolicy::default().generated_bytes)
    }
}
impl Retention {
    pub fn new(limit: u64) -> Self {
        Self(Arc::new(RetentionState {
            limit,
            bytes: 0.into(),
        }))
    }
    pub fn limit(&self) -> u64 {
        self.0.limit
    }
    pub fn bytes(&self) -> u64 {
        self.0.bytes.load(Ordering::Acquire)
    }
    pub fn acquire(&self, bytes: u64) -> Option<Lease> {
        self.0
            .bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .ok()?;
        Some(Lease {
            owner: self.0.clone(),
            bytes,
        })
    }
}
pub(crate) struct RetainedBuffer {
    pub key: RetainedKey,
    pub pixels: wgpu::Buffer,
    ready: AtomicBool,
    lease: Option<Lease>,
}
impl RetainedBuffer {
    pub fn new(device: &wgpu::Device, key: RetainedKey, bytes: u64, lease: Option<Lease>) -> Self {
        Self {
            key,
            pixels: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("evaluation retained generated value"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ready: AtomicBool::new(false),
            lease,
        }
    }
    pub fn ready(&self) -> bool {
        self.lease.is_some() && self.ready.load(Ordering::Acquire)
    }
    pub fn retained_by(&self, retention: &Retention) -> bool {
        self.lease
            .as_ref()
            .is_some_and(|lease| Arc::ptr_eq(&lease.owner, &retention.0))
    }
    pub fn submitted(&self) {
        self.ready.store(true, Ordering::Release);
    }
}
