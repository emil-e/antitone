//! Device-scoped test fixtures, not part of the application API.
#![allow(dead_code)] // Integration crates exercise different fixture operations.

use antitone::photographic::stages::{display_rendering as drt, look};
use antitone::{
    Error,
    evaluation::{CachePolicy, Context},
};
use std::sync::Arc;

/// Explicit GPU resources for tests that build and record display graphs.
pub struct RenderFixture {
    pub(crate) drt: Arc<drt::Targets>,
    pub(crate) looks: Arc<look::Looks>,
    pub(crate) evaluation: antitone::evaluation::Context,
}

impl RenderFixture {
    /// Shared device-scoped evaluation and its optional calculated-image retention allowance.
    pub fn evaluation(&self) -> &antitone::evaluation::Context {
        &self.evaluation
    }

    pub fn looks(&self) -> &Arc<look::Looks> {
        &self.looks
    }

    pub fn drt(&self, target: antitone::rendering::OutputTarget) -> &drt::Target {
        self.drt.get(target)
    }

    pub async fn new(
        device: &wgpu::Device,
        queue: &antitone::gpu::submission::Queue,
    ) -> Result<Arc<Self>, Error> {
        Self::new_with_cache_policy(device, queue, Default::default()).await
    }

    pub async fn new_with_cache_policy(
        device: &wgpu::Device,
        queue: &antitone::gpu::submission::Queue,
        policy: CachePolicy,
    ) -> Result<Arc<Self>, Error> {
        Self::with_evaluation(Context::new(device, policy), queue).await
    }

    async fn with_evaluation(
        evaluation: Context,
        queue: &antitone::gpu::submission::Queue,
    ) -> Result<Arc<Self>, Error> {
        let device = evaluation.device().clone();
        let looks = Arc::new(look::Looks::new(&device));
        let drt = Arc::new(drt::Targets::new(&device, queue).await?);
        let renderer = Arc::new(Self {
            drt,
            looks,
            evaluation,
        });
        antitone::photographic::graph::prewarm_viewer(&renderer.evaluation, &renderer.drt)?;
        Ok(renderer)
    }
}
