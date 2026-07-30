//! Optional GPU acceleration for exact integer cross-correlation.
//!
//! The GPU only replaces the inner dot-product plane. Score normalisation,
//! thresholds, peak selection, and native verification stay in `corr.rs`, so
//! enabling an adapter cannot change the detector's decisions. Any failure
//! returns `None` and the caller runs the existing Rayon implementation.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use wgpu::util::DeviceExt;

const WORKGROUP_SIDE: u32 = 8;
const GPU_MIN_MULTIPLIES: u64 = 24_000_000;
const GPU_TIMEOUT: Duration = Duration::from_secs(2);

const SHADER: &str = r#"
struct Params {
    image_width: u32,
    image_height: u32,
    kernel_width: u32,
    kernel_height: u32,
    output_width: u32,
    output_height: u32,
    wide_accumulator: u32,
    _padding_1: u32,
};

@group(0) @binding(0)
var<storage, read> image: array<u32>;

@group(0) @binding(1)
var<storage, read> kernel: array<u32>;

@group(0) @binding(2)
var<storage, read> mask: array<u32>;

@group(0) @binding(3)
var<storage, read_write> output: array<u32>;

@group(0) @binding(4)
var<uniform> params: Params;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    if (x >= params.output_width || y >= params.output_height) {
        return;
    }

    var low = 0u;
    if (params.wide_accumulator == 0u) {
        for (var ky = 0u; ky < params.kernel_height; ky = ky + 1u) {
            let image_row = (y + ky) * params.image_width + x;
            let kernel_row = ky * params.kernel_width;
            for (var kx = 0u; kx < params.kernel_width; kx = kx + 1u) {
                low = low + image[image_row + kx] * kernel[kernel_row + kx];
            }
        }
        let output_index = (y * params.output_width + x) * 2u;
        output[output_index] = low;
        output[output_index + 1u] = 0u;
        return;
    }

    var high = 0u;
    for (var ky = 0u; ky < params.kernel_height; ky = ky + 1u) {
        let image_row = (y + ky) * params.image_width + x;
        let kernel_row = ky * params.kernel_width;
        for (var kx = 0u; kx < params.kernel_width; kx = kx + 1u) {
            let product = image[image_row + kx] * kernel[kernel_row + kx];
            let previous = low;
            low = low + product;
            if (low < previous) {
                high = high + 1u;
            }
        }
    }

    let output_index = (y * params.output_width + x) * 2u;
    output[output_index] = low;
    output[output_index + 1u] = high;
}

@compute @workgroup_size(8, 8, 1)
fn masked(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    if (x >= params.output_width || y >= params.output_height) {
        return;
    }

    var raw_low = 0u;
    var sum_low = 0u;
    var square_low = 0u;
    if (params.wide_accumulator == 0u) {
        for (var ky = 0u; ky < params.kernel_height; ky = ky + 1u) {
            let image_row = (y + ky) * params.image_width + x;
            let kernel_row = ky * params.kernel_width;
            for (var kx = 0u; kx < params.kernel_width; kx = kx + 1u) {
                let kernel_index = kernel_row + kx;
                if (mask[kernel_index] != 0u) {
                    let pixel = image[image_row + kx];
                    raw_low = raw_low + pixel * kernel[kernel_index];
                    sum_low = sum_low + pixel;
                    square_low = square_low + pixel * pixel;
                }
            }
        }
        let output_index = (y * params.output_width + x) * 6u;
        output[output_index] = raw_low;
        output[output_index + 1u] = 0u;
        output[output_index + 2u] = sum_low;
        output[output_index + 3u] = 0u;
        output[output_index + 4u] = square_low;
        output[output_index + 5u] = 0u;
        return;
    }

    var raw_high = 0u;
    var sum_high = 0u;
    var square_high = 0u;
    for (var ky = 0u; ky < params.kernel_height; ky = ky + 1u) {
        let image_row = (y + ky) * params.image_width + x;
        let kernel_row = ky * params.kernel_width;
        for (var kx = 0u; kx < params.kernel_width; kx = kx + 1u) {
            let kernel_index = kernel_row + kx;
            if (mask[kernel_index] != 0u) {
                let pixel = image[image_row + kx];

                let raw_previous = raw_low;
                raw_low = raw_low + pixel * kernel[kernel_index];
                if (raw_low < raw_previous) {
                    raw_high = raw_high + 1u;
                }

                let sum_previous = sum_low;
                sum_low = sum_low + pixel;
                if (sum_low < sum_previous) {
                    sum_high = sum_high + 1u;
                }

                let square_previous = square_low;
                square_low = square_low + pixel * pixel;
                if (square_low < square_previous) {
                    square_high = square_high + 1u;
                }
            }
        }
    }

    let output_index = (y * params.output_width + x) * 6u;
    output[output_index] = raw_low;
    output[output_index + 1u] = raw_high;
    output[output_index + 2u] = sum_low;
    output[output_index + 3u] = sum_high;
    output[output_index + 4u] = square_low;
    output[output_index + 5u] = square_high;
}
"#;

/// GPU copy of the pixel plane owned by one `Searched`.
pub struct Search {
    pixels: wgpu::Buffer,
    pixel_max: u32,
    square_max: u32,
    w: usize,
    h: usize,
}

struct Context {
    device: wgpu::Device,
    queue: wgpu::Queue,
    plain_pipeline: wgpu::ComputePipeline,
    masked_pipeline: wgpu::ComputePipeline,
    failed: Arc<AtomicBool>,
    max_storage_binding: u64,
    max_buffer: u64,
}

enum State {
    Uninitialised,
    Initialising,
    Ready(Context),
    Disabled,
}

struct Backend {
    state: Mutex<State>,
    ready: Condvar,
}

fn backend() -> &'static Backend {
    static BACKEND: OnceLock<Backend> = OnceLock::new();
    BACKEND.get_or_init(|| Backend {
        state: Mutex::new(State::Uninitialised),
        ready: Condvar::new(),
    })
}

/// Begin adapter discovery and shader compilation away from the UI and first
/// detection paths. Calls made while this is still running simply use CPU.
pub fn warm_up() {
    let should_start = {
        let Ok(mut state) = backend().state.lock() else {
            return;
        };
        if matches!(*state, State::Uninitialised) {
            *state = State::Initialising;
            true
        } else {
            false
        }
    };
    if !should_start {
        return;
    }

    let spawned = std::thread::Builder::new()
        .name("clawmation-gpu-detection".into())
        .spawn(|| {
            let result = Context::new();
            if let Ok(mut state) = backend().state.lock() {
                if matches!(*state, State::Initialising) {
                    *state = result.map_or(State::Disabled, State::Ready);
                }
                backend().ready.notify_all();
            }
        });
    if spawned.is_err() {
        if let Ok(mut state) = backend().state.lock() {
            if matches!(*state, State::Initialising) {
                *state = State::Disabled;
            }
            backend().ready.notify_all();
        }
    }
}

/// GPU dispatch loses to Rayon below this amount of arithmetic because it must
/// still copy the result plane back for the exact CPU normalisation.
pub fn worth_accelerating(iw: usize, ih: usize, kw: usize, kh: usize) -> bool {
    if kw == 0 || kh == 0 || kw > iw || kh > ih {
        return false;
    }
    let ow = iw - kw + 1;
    let oh = ih - kh + 1;
    (ow as u64)
        .saturating_mul(oh as u64)
        .saturating_mul(kw as u64)
        .saturating_mul(kh as u64)
        >= GPU_MIN_MULTIPLIES
}

/// Upload the search plane once. `None` means the caller should keep using
/// CPU correlation; it is not an application error.
pub fn prepare(px: &[i32], px2: &[i32], w: usize, h: usize) -> Option<Search> {
    if px.len() != w.checked_mul(h)? || px2.len() != px.len() || px.is_empty() {
        return None;
    }
    let pixels: Vec<u32> = px
        .iter()
        .map(|&value| u32::try_from(value).ok())
        .collect::<Option<_>>()?;
    let byte_len = (pixels.len() as u64).checked_mul(4)?;
    let pixel_max = pixels.iter().copied().max().unwrap_or(0);
    let square_max = px2.iter().try_fold(0u32, |largest, &value| {
        Some(largest.max(u32::try_from(value).ok()?))
    })?;

    with_context(|context| {
        if byte_len > context.max_storage_binding || byte_len > context.max_buffer {
            return None;
        }
        let pixels = context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clawmation vision pixels"),
                contents: bytemuck::cast_slice(&pixels),
                usage: wgpu::BufferUsages::STORAGE,
            });
        Some(Search {
            pixels,
            pixel_max,
            square_max,
            w,
            h,
        })
    })
    .flatten()
}

/// Run one exact correlation plane. The two returned words are reconstructed
/// into the same non-negative `i64` sums produced by the CPU implementation.
pub fn correlate(search: &Search, kernel: &[i32], kw: usize, kh: usize) -> Option<Vec<i64>> {
    if kw == 0 || kh == 0 || kw > search.w || kh > search.h || kernel.len() != kw.checked_mul(kh)? {
        return None;
    }
    let kernel: Vec<u32> = kernel
        .iter()
        .map(|&value| u32::try_from(value).ok())
        .collect::<Option<_>>()?;

    with_context(|context| context.correlate(search, &kernel, kw, kh)).flatten()
}

/// Fused masked correlation. One pass produces `Σ(I·T·M)`, `Σ(I·M)`, and
/// `Σ(I²·M)` instead of submitting and reading back three independent jobs.
pub fn correlate_masked(
    search: &Search,
    template_masked: &[i32],
    mask: &[i32],
    kw: usize,
    kh: usize,
) -> Option<(Vec<i64>, Vec<i64>, Vec<i64>)> {
    if kw == 0
        || kh == 0
        || kw > search.w
        || kh > search.h
        || template_masked.len() != kw.checked_mul(kh)?
        || mask.len() != template_masked.len()
    {
        return None;
    }
    let template_masked: Vec<u32> = template_masked
        .iter()
        .map(|&value| u32::try_from(value).ok())
        .collect::<Option<_>>()?;
    let mask: Vec<u32> = mask
        .iter()
        .map(|&value| u32::try_from(value).ok())
        .collect::<Option<_>>()?;

    with_context(|context| context.correlate_masked(search, &template_masked, &mask, kw, kh))
        .flatten()
}

#[cfg(test)]
pub fn available() -> bool {
    warm_up();
    let Ok(state) = backend().state.lock() else {
        return false;
    };
    let Ok((state, _)) = backend()
        .ready
        .wait_timeout_while(state, GPU_TIMEOUT, |state| {
            matches!(state, State::Initialising)
        })
    else {
        return false;
    };
    matches!(*state, State::Ready(_))
}

fn with_context<T>(operation: impl FnOnce(&mut Context) -> T) -> Option<T> {
    warm_up();
    let mut guard = backend().state.lock().ok()?;

    let result = match &mut *guard {
        State::Ready(context) if !context.failed.load(Ordering::Acquire) => {
            Some(operation(context))
        }
        _ => None,
    };
    let failed = match &*guard {
        State::Ready(context) => context.failed.load(Ordering::Acquire),
        _ => false,
    };
    if failed {
        *guard = State::Disabled;
    }
    result
}

impl Context {
    fn new() -> Option<Self> {
        Self::for_backend(wgpu::Backends::VULKAN)
            .or_else(|| Self::for_backend(wgpu::Backends::DX12))
    }

    fn for_backend(backend: wgpu::Backends) -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: backend,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .ok()?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return None;
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Clawmation vision compute"),
            ..Default::default()
        }))
        .ok()?;
        let failed = Arc::new(AtomicBool::new(false));
        let error_flag = Arc::clone(&failed);
        device.on_uncaptured_error(Arc::new(move |_| {
            error_flag.store(true, Ordering::Release);
        }));

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Clawmation exact correlation"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let plain_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Clawmation exact correlation"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let masked_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Clawmation exact masked correlation"),
            layout: None,
            module: &module,
            entry_point: Some("masked"),
            compilation_options: Default::default(),
            cache: None,
        });
        let limits = device.limits();
        Some(Self {
            device,
            queue,
            plain_pipeline,
            masked_pipeline,
            failed,
            max_storage_binding: u64::from(limits.max_storage_buffer_binding_size),
            max_buffer: limits.max_buffer_size,
        })
    }

    fn correlate(
        &mut self,
        search: &Search,
        kernel: &[u32],
        kw: usize,
        kh: usize,
    ) -> Option<Vec<i64>> {
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        let ow = search.w - kw + 1;
        let oh = search.h - kh + 1;
        let output_len = ow.checked_mul(oh)?;
        let output_bytes = (output_len as u64).checked_mul(8)?;
        let kernel_bytes = (kernel.len() as u64).checked_mul(4)?;
        if output_bytes == 0
            || output_bytes > self.max_storage_binding
            || output_bytes > self.max_buffer
            || kernel_bytes > self.max_storage_binding
            || kernel_bytes > self.max_buffer
        {
            return None;
        }

        let kernel_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clawmation vision kernel"),
                contents: bytemuck::cast_slice(kernel),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clawmation vision correlation"),
            size: output_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clawmation vision readback"),
            size: output_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let params = [
            u32::try_from(search.w).ok()?,
            u32::try_from(search.h).ok()?,
            u32::try_from(kw).ok()?,
            u32::try_from(kh).ok()?,
            u32::try_from(ow).ok()?,
            u32::try_from(oh).ok()?,
            u32::from(
                kernel
                    .iter()
                    .map(|&value| u64::from(value))
                    .sum::<u64>()
                    .saturating_mul(u64::from(search.pixel_max))
                    > u64::from(u32::MAX),
            ),
            0,
        ];
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clawmation vision correlation parameters"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layout = self.plain_pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clawmation vision correlation"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: search.pixels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: kernel_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clawmation vision correlation"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("clawmation vision correlation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.plain_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                (ow as u32).div_ceil(WORKGROUP_SIDE),
                (oh as u32).div_ceil(WORKGROUP_SIDE),
                1,
            );
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
        let submission = self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        if self
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(GPU_TIMEOUT),
            })
            .is_err()
            || !matches!(receiver.recv_timeout(GPU_TIMEOUT), Ok(Ok(())))
            || self.failed.load(Ordering::Acquire)
        {
            self.failed.store(true, Ordering::Release);
            return None;
        }

        let view = slice.get_mapped_range();
        let words: &[u32] = bytemuck::cast_slice(&view);
        if words.len() != output_len * 2 {
            self.failed.store(true, Ordering::Release);
            return None;
        }
        let mut result = Vec::with_capacity(output_len);
        for pair in words.chunks_exact(2) {
            let value = (u64::from(pair[1]) << 32) | u64::from(pair[0]);
            result.push(i64::try_from(value).ok()?);
        }
        drop(view);
        readback.unmap();
        Some(result)
    }

    fn correlate_masked(
        &mut self,
        search: &Search,
        template_masked: &[u32],
        mask: &[u32],
        kw: usize,
        kh: usize,
    ) -> Option<(Vec<i64>, Vec<i64>, Vec<i64>)> {
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        let ow = search.w - kw + 1;
        let oh = search.h - kh + 1;
        let output_len = ow.checked_mul(oh)?;
        let output_bytes = (output_len as u64).checked_mul(24)?;
        let kernel_bytes = (template_masked.len() as u64).checked_mul(4)?;
        if output_bytes == 0
            || output_bytes > self.max_storage_binding
            || output_bytes > self.max_buffer
            || kernel_bytes > self.max_storage_binding
            || kernel_bytes > self.max_buffer
        {
            return None;
        }

        let template_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clawmation vision masked template"),
                contents: bytemuck::cast_slice(template_masked),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let mask_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clawmation vision template mask"),
                contents: bytemuck::cast_slice(mask),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clawmation vision masked correlation"),
            size: output_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clawmation vision masked readback"),
            size: output_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mask_count = mask.iter().map(|&value| u64::from(value != 0)).sum::<u64>();
        let raw_limit = template_masked
            .iter()
            .map(|&value| u64::from(value))
            .sum::<u64>()
            .saturating_mul(u64::from(search.pixel_max));
        let sum_limit = mask_count.saturating_mul(u64::from(search.pixel_max));
        let square_limit = mask_count.saturating_mul(u64::from(search.square_max));
        let params = [
            u32::try_from(search.w).ok()?,
            u32::try_from(search.h).ok()?,
            u32::try_from(kw).ok()?,
            u32::try_from(kh).ok()?,
            u32::try_from(ow).ok()?,
            u32::try_from(oh).ok()?,
            u32::from(
                raw_limit > u64::from(u32::MAX)
                    || sum_limit > u64::from(u32::MAX)
                    || square_limit > u64::from(u32::MAX),
            ),
            0,
        ];
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clawmation vision masked correlation parameters"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layout = self.masked_pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clawmation vision masked correlation"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: search.pixels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: template_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: mask_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clawmation vision masked correlation"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("clawmation vision masked correlation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.masked_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                (ow as u32).div_ceil(WORKGROUP_SIDE),
                (oh as u32).div_ceil(WORKGROUP_SIDE),
                1,
            );
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
        let submission = self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        if self
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(GPU_TIMEOUT),
            })
            .is_err()
            || !matches!(receiver.recv_timeout(GPU_TIMEOUT), Ok(Ok(())))
            || self.failed.load(Ordering::Acquire)
        {
            self.failed.store(true, Ordering::Release);
            return None;
        }

        let view = slice.get_mapped_range();
        let words: &[u32] = bytemuck::cast_slice(&view);
        if words.len() != output_len * 6 {
            self.failed.store(true, Ordering::Release);
            return None;
        }
        let mut raw = Vec::with_capacity(output_len);
        let mut sums = Vec::with_capacity(output_len);
        let mut squares = Vec::with_capacity(output_len);
        for values in words.chunks_exact(6) {
            raw.push(words_to_i64(values[0], values[1])?);
            sums.push(words_to_i64(values[2], values[3])?);
            squares.push(words_to_i64(values[4], values[5])?);
        }
        drop(view);
        readback.unmap();
        Some((raw, sums, squares))
    }
}

fn words_to_i64(low: u32, high: u32) -> Option<i64> {
    i64::try_from((u64::from(high) << 32) | u64::from(low)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_threshold_rejects_small_and_invalid_jobs() {
        assert!(!worth_accelerating(100, 100, 3, 3));
        assert!(!worth_accelerating(10, 10, 11, 1));
        assert!(worth_accelerating(400, 300, 20, 20));
    }
}
