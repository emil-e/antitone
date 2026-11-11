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
pub(crate) struct Allowance(Arc<AllowanceState>);

struct AllowanceState {
    limit: std::sync::atomic::AtomicU64,
    bytes: std::sync::atomic::AtomicU64,
}

pub(crate) struct Lease {
    owner: Arc<AllowanceState>,
    bytes: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.owner.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl Default for Allowance {
    fn default() -> Self {
        Self::new(super::CachePolicy::default().generated_bytes)
    }
}

impl Allowance {
    pub fn new(limit: u64) -> Self {
        Self(Arc::new(AllowanceState {
            limit: limit.into(),
            bytes: 0.into(),
        }))
    }

    pub fn limit(&self) -> u64 {
        self.0.limit.load(Ordering::Acquire)
    }

    #[cfg(feature = "editor")]
    pub(super) fn set_limit(&self, limit: u64) {
        self.0.limit.store(limit, Ordering::Release);
    }

    pub fn bytes(&self) -> u64 {
        self.0.bytes.load(Ordering::Acquire)
    }

    pub fn acquire(&self, bytes: u64) -> Option<Lease> {
        self.0
            .bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit())
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
    pub descriptor: Arc<dyn Any + Send + Sync>,
    image: Arc<super::ImageValue>,
    ready: AtomicBool,
    admitted: bool,
    publication: std::sync::Mutex<Option<super::materializations::Publication>>,
}
impl RetainedBuffer {
    pub(super) fn pending(
        key: RetainedKey,
        descriptor: Arc<dyn Any + Send + Sync>,
        image: Arc<super::ImageValue>,
        publication: Option<super::materializations::Publication>,
    ) -> Self {
        Self {
            key,
            pixels: image
                .as_buffer()
                .expect("generated values use buffer storage")
                .clone(),
            image,
            descriptor,
            ready: AtomicBool::new(false),
            admitted: publication.is_some(),
            publication: std::sync::Mutex::new(publication),
        }
    }
    pub(super) fn reused(
        key: RetainedKey,
        descriptor: Arc<dyn Any + Send + Sync>,
        image: Arc<super::ImageValue>,
    ) -> Self {
        Self {
            key,
            pixels: image
                .as_buffer()
                .expect("generated values use buffer storage")
                .clone(),
            image,
            descriptor,
            ready: AtomicBool::new(true),
            admitted: true,
            publication: std::sync::Mutex::new(None),
        }
    }
    pub fn ready(&self) -> bool {
        self.admitted && self.ready.load(Ordering::Acquire)
    }
    pub fn submitted(&self) {
        self.ready.store(true, Ordering::Release);
        if let Some(publication) = self.publication.lock().unwrap().take() {
            publication.submitted(&self.image);
        }
    }
}
