use acheflow_media_api::live::GpuPassTiming;

/// Samples share the compositor's submission and readback. No cross-frame queries are in flight.
pub(crate) struct GpuProfile {
    queries: wgpu::QuerySet,
    encoder_timestamps: bool,
    #[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
    resolve: wgpu::Buffer,
    #[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
    readback: wgpu::Buffer,
    passes: Vec<GpuPassTiming>,
    truncated: bool,
}

const MAX_PASSES: u32 = 128;

impl GpuProfile {
    pub(crate) fn begin(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        stage: &str,
        viewport_id: Option<u64>,
        layer: Option<(u64, u32)>,
    ) -> Option<u32> {
        let index = self.reserve(stage, viewport_id, layer)?;
        self.mark(encoder, index);
        Some(index + 1)
    }

    pub(crate) fn render_pass(
        &mut self,
        stage: &str,
        viewport_id: Option<u64>,
        layer: Option<(u64, u32)>,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let index = self.reserve(stage, viewport_id, layer)?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: Some(index + 1),
        })
    }

    fn reserve(
        &mut self,
        stage: &str,
        viewport_id: Option<u64>,
        layer: Option<(u64, u32)>,
    ) -> Option<u32> {
        if self.passes.len() == MAX_PASSES as usize {
            self.truncated = true;
            return None;
        }
        let index = self.passes.len() as u32 * 2;
        self.passes.push(GpuPassTiming {
            stage: stage.to_owned(),
            viewport_id,
            layer_id: layer.map(|(id, _)| id),
            source_id: layer.map(|(_, source)| source),
            elapsed_ms: 0.,
        });
        Some(index)
    }

    pub(crate) fn end(&self, encoder: &mut wgpu::CommandEncoder, end: Option<u32>) {
        if let Some(index) = end {
            self.mark(encoder, index);
        }
    }

    fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        if self.encoder_timestamps {
            encoder.write_timestamp(&self.queries, index);
        } else {
            // Metal stage-boundary timestamps can delimit work encoded by Vello or a copy.
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Studio timestamp"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &self.queries,
                    beginning_of_pass_write_index: Some(index),
                    end_of_pass_write_index: None,
                }),
            }));
        }
    }
}

#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
impl GpuProfile {
    pub(crate) const FEATURES: wgpu::Features =
        wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

    pub(crate) fn new(device: &wgpu::Device) -> Option<Self> {
        let features = device.features();
        if !features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(MAX_PASSES) * 16,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            encoder_timestamps: features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Studio render timing"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX_PASSES * 2,
            }),
            resolve: buffer(
                "Resolve render timing",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "Read render timing",
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ),
            passes: Vec::new(),
            truncated: false,
        })
    }

    pub(crate) fn reset(&mut self) {
        self.passes.clear();
        self.truncated = false;
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let count = self.passes.len() as u32 * 2;
        encoder.resolve_query_set(&self.queries, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, u64::from(count) * 8);
    }

    pub(crate) fn map(&self) -> std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        rx
    }

    pub(crate) fn finish(
        &mut self,
        scene_id: u32,
        period: f32,
        mapped: bool,
    ) -> Option<acheflow_media_api::live::GpuFrameProfile> {
        if !mapped {
            return None;
        }
        let timings = (|| {
            let data = self.readback.slice(..).get_mapped_range().ok()?;
            let (words, _) = data.as_chunks::<8>();
            let (pairs, _) = words.as_chunks::<2>();
            let passes = self
                .passes
                .iter()
                .zip(pairs)
                .filter_map(|(pass, [start, end])| {
                    let start = u64::from_le_bytes(*start);
                    let end = u64::from_le_bytes(*end);
                    if start == u64::MAX || end == u64::MAX {
                        return None;
                    }
                    let mut pass = pass.clone();
                    pass.elapsed_ms =
                        end.checked_sub(start)? as f64 * f64::from(period) / 1_000_000.;
                    Some(pass)
                })
                .collect();
            Some(acheflow_media_api::live::GpuFrameProfile {
                scene_id,
                passes,
                truncated: self.truncated,
            })
        })();
        self.readback.unmap();
        timings
    }
}
