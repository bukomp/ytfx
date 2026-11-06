//! GPU backend for the effects: wgpu compute shaders (`effects.wgsl`) that run the same
//! `Step`s as the CPU code in `effects.rs`. Each frame is uploaded once, every step runs
//! as one or a few compute passes on frames that stay in GPU memory, and the result is
//! read back for the renderers.
//!
//! The device starts on a background thread. Until it is ready, and for good if it fails
//! (no adapter, a device error, a lost device, a panic in the driver layer), the effects
//! run on the CPU.

use crate::effects::{FilmPlan, Fx, GlitchPlan, Step, VhsRow, Warp, blur_scale};
use std::cell::Cell;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Invocations per workgroup; matches `@workgroup_size` in the shader.
const GROUP: u32 = 256;
/// Most workgroups per dispatch dimension (the WebGPU minimum limit).
const MAX_GROUPS: u32 = 65535;
/// Size of `Params`, and of its struct in the shader.
const PARAMS_SIZE: u64 = std::mem::size_of::<Params>() as u64;
/// Longest wait for a frame before the GPU counts as hung.
const TIMEOUT: Duration = Duration::from_secs(2);

thread_local! {
    static IN_GPU: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is running GPU code. The panic hook leaves the terminal alone
/// then: such a panic is caught, and the effects go on on the CPU.
pub fn in_gpu_code() -> bool {
    IN_GPU.with(Cell::get)
}

/// Run `f` as GPU code (see `in_gpu_code`), turning a panic into an error.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    IN_GPU.with(|g| g.set(true));
    let result = catch_unwind(AssertUnwindSafe(f));
    IN_GPU.with(|g| g.set(false));
    result.unwrap_or_else(|_| Err("the GPU code panicked".into()))
}

/// The GPU backend as the effects see it: not wanted, starting, ready or failed.
#[derive(Default)]
pub enum GpuSlot {
    #[default]
    Off,
    Starting(mpsc::Receiver<Result<Gpu, String>>),
    Ready(Box<Gpu>),
    Failed(String),
}

impl GpuSlot {
    /// Start the device on a background thread (does nothing if it already started).
    pub fn start(&mut self) {
        if !matches!(self, GpuSlot::Off) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("ytfx-gpu".into())
            .spawn(move || {
                let _ = tx.send(guarded(Gpu::new));
            });
        *self = match spawned {
            Ok(_) => GpuSlot::Starting(rx),
            Err(e) => GpuSlot::Failed(format!("couldn't start the GPU thread: {e}")),
        };
    }

    /// Start the device and wait until it is ready or has failed.
    #[cfg(test)]
    pub fn start_blocking(&mut self) {
        self.start();
        if let GpuSlot::Starting(rx) = self {
            *self = match rx.recv() {
                Ok(Ok(gpu)) => GpuSlot::Ready(Box::new(gpu)),
                Ok(Err(why)) => GpuSlot::Failed(why),
                Err(_) => GpuSlot::Failed("the GPU thread stopped".into()),
            };
        }
    }

    /// The device, once it is ready.
    pub fn get(&mut self) -> Option<&mut Gpu> {
        if let GpuSlot::Starting(rx) = self {
            *self = match rx.try_recv() {
                Ok(Ok(gpu)) => GpuSlot::Ready(Box::new(gpu)),
                Ok(Err(why)) => GpuSlot::Failed(why),
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    GpuSlot::Failed("the GPU thread stopped".into())
                }
            };
        }
        match self {
            GpuSlot::Ready(gpu) => Some(gpu),
            _ => None,
        }
    }

    /// Give up on the GPU: the effects run on the CPU from now on. The device is leaked,
    /// not dropped: dropping it waits for the GPU, which may be what hung.
    pub fn fail(&mut self, why: String) {
        if let GpuSlot::Ready(gpu) = std::mem::replace(self, GpuSlot::Failed(why)) {
            std::mem::forget(gpu);
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, GpuSlot::Ready(_))
    }

    /// Why the GPU isn't used, if it failed.
    pub fn failure(&self) -> Option<&str> {
        match self {
            GpuSlot::Failed(why) => Some(why),
            _ => None,
        }
    }

    /// Where the effects run, and why not on the GPU.
    pub fn status(&self) -> String {
        match self {
            GpuSlot::Off => "cpu (--no-gpu)".into(),
            GpuSlot::Starting(_) => "cpu (gpu starting)".into(),
            GpuSlot::Ready(gpu) => format!("gpu ({})", gpu.name),
            GpuSlot::Failed(why) => format!("cpu ({why})"),
        }
    }

    /// Drop the history (trails, tunnel, echo), so it starts over from the next frame.
    pub fn forget(&mut self) {
        if let GpuSlot::Ready(gpu) = self {
            gpu.history = History::default();
        }
    }

    /// Drop the history of effects that are off.
    pub fn free_unused(&mut self, enabled: &[bool; Fx::ALL.len()]) {
        if let GpuSlot::Ready(gpu) = self {
            let h = &mut gpu.history;
            h.feedback &= enabled[Fx::Tunnel as usize];
            h.prev &= enabled[Fx::Trails as usize];
            h.lag &= enabled[Fx::Echo as usize];
        }
    }
}

/// Which history regions hold the previous frame's state.
#[derive(Default, Clone, Copy)]
struct History {
    feedback: bool,
    prev: bool,
    lag: bool,
}

/// Frame-sized regions of the frames buffer, in order. Each holds `Gpu::region` words:
/// one per pixel, or packed RGB24 bytes in `Io`.
#[derive(Clone, Copy)]
enum Region {
    Io,
    A,
    B,
    Tmp,
    Feedback,
    Prev,
    Lag1,
    Lag2,
}
const REGIONS: u64 = 8;

/// Compute passes, in the order of `ENTRY`.
#[derive(Clone, Copy)]
enum Op {
    Unpack,
    Pack,
    Copy,
    Warp,
    Shift,
    Blend,
    Echo,
    HBlur,
    VBlur,
    Down,
    Up,
    Split,
    Grade,
    Hue,
    Swap,
    Glitch,
    Snow,
    Edges,
    Pixel,
    Dither,
    Vhs,
    Crt,
    Film,
    Darken,
    Cinema,
    Tone,
}

/// Shader entry point of each `Op`.
const ENTRY: [&str; 26] = [
    "unpack", "pack", "copy", "warp", "shift", "blend", "echo", "hblur", "vblur", "down", "up",
    "split", "grade", "hue", "swap", "glitch", "snow", "edges", "pixel", "dither", "vhs", "crt",
    "film", "darken", "cinema", "tone",
];

/// One pass's uniforms; the layout matches `struct P` in the shader. Region fields are
/// word offsets into the frames buffer, `data` a word offset into the data buffer.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    src: u32,
    dst: u32,
    aux: u32,
    aux2: u32,
    w: u32,
    h: u32,
    sw: u32,
    sh: u32,
    data: u32,
    count: u32,
    mode: u32,
    r: u32,
    i: [i32; 12],
    f: [f32; 8],
}

/// Buffers sized for one frame size.
struct Frames {
    w: usize,
    h: usize,
    /// Every region (see `Region`).
    buf: wgpu::Buffer,
    /// The result, mapped for reading.
    readback: wgpu::Buffer,
}

pub struct Gpu {
    name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::ComputePipeline>,
    limits: wgpu::Limits,
    /// Distance between the passes' uniforms in `params`.
    stride: u64,
    frames: Option<Frames>,
    data: wgpu::Buffer,
    params: wgpu::Buffer,
    bind: Option<wgpu::BindGroup>,
    history: History,
    /// The first device error, from wgpu's error callback.
    error: Arc<Mutex<Option<String>>>,
    lost: Arc<AtomicBool>,
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            // No validation layers: they print to the terminal.
            flags: wgpu::InstanceFlags::empty(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|_| "no GPU found".to_string())?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(format!("only a software renderer ({})", info.name));
        }
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ytfx effects"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("couldn't open the GPU: {e}"))?;

        let error = Arc::new(Mutex::new(None));
        let slot = error.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let mut slot = slot.lock().unwrap_or_else(|p| p.into_inner());
            slot.get_or_insert_with(|| e.to_string());
        }));
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::Relaxed));

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("effects.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effects"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(PARAMS_SIZE),
                    },
                    count: None,
                },
                storage(1, false),
                storage(2, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effects"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = ENTRY
            .iter()
            .map(|entry| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        let stride =
            PARAMS_SIZE.next_multiple_of(limits.min_uniform_buffer_offset_alignment as u64);
        let gpu = Gpu {
            name: info.name,
            data: buffer(&device, 1 << 16, wgpu::BufferUsages::STORAGE),
            params: buffer(&device, stride * 64, wgpu::BufferUsages::UNIFORM),
            device,
            queue,
            layout,
            pipelines,
            limits,
            stride,
            frames: None,
            bind: None,
            history: History::default(),
            error,
            lost,
        };
        gpu.check()?;
        Ok(gpu)
    }

    /// The first device error since the start, if any.
    fn check(&self) -> Result<(), String> {
        if self.lost.load(Ordering::Relaxed) {
            return Err("the GPU device was lost".into());
        }
        match self
            .error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            Some(e) => Err(e.lines().next().unwrap_or(e).to_string()),
            None => Ok(()),
        }
    }

    /// Run `steps` on `src` (w x h RGB24) into `out`. Ok(false) means this frame size
    /// doesn't fit the GPU's limits, and the caller should use the CPU for it. An error
    /// means the GPU can't be used any more.
    pub fn run(
        &mut self,
        src: &[u8],
        w: usize,
        h: usize,
        steps: &[Step],
        noise: &[i16],
        out: &mut [u8],
    ) -> Result<bool, String> {
        guarded(|| self.run_inner(src, w, h, steps, noise, out))
    }

    fn run_inner(
        &mut self,
        src: &[u8],
        w: usize,
        h: usize,
        steps: &[Step],
        noise: &[i16],
        out: &mut [u8],
    ) -> Result<bool, String> {
        self.check()?;
        if !self.fit(w, h) {
            return Ok(false);
        }
        let n = w * h;
        let region = n as u32;
        let mut chain = Chain::new(w, h, region, noise, self.history);
        for step in steps {
            chain.step(step);
        }
        let words = (n * 3).div_ceil(4) as u32;
        let io = chain.at(Region::Io);
        let mut pack = chain.params(chain.cur, io);
        pack.count = words;
        pack.r = (n * 3) as u32;
        chain.passes.push((Op::Pack, pack));
        self.history = chain.history;

        // Uploads: the frame, the tables, and every pass's uniforms.
        self.grow_data((chain.data.len() * 4) as u64);
        self.grow_params(chain.passes.len() as u64 * self.stride);
        let bind = self.bind_group();
        let Some(frames) = &self.frames else {
            return Err("no frame buffers".into());
        };
        let aligned = src.len() & !3;
        self.queue.write_buffer(&frames.buf, 0, &src[..aligned]);
        if aligned < src.len() {
            let mut tail = [0u8; 4];
            tail[..src.len() - aligned].copy_from_slice(&src[aligned..]);
            self.queue.write_buffer(&frames.buf, aligned as u64, &tail);
        }
        self.queue
            .write_buffer(&self.data, 0, bytemuck::cast_slice(&chain.data));
        let mut uniforms = vec![0u8; chain.passes.len() * self.stride as usize];
        for (k, (_, p)) in chain.passes.iter().enumerate() {
            let at = k * self.stride as usize;
            uniforms[at..at + PARAMS_SIZE as usize].copy_from_slice(bytemuck::bytes_of(p));
        }
        self.queue.write_buffer(&self.params, 0, &uniforms);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("effects"),
                timestamp_writes: None,
            });
            for (k, (op, p)) in chain.passes.iter().enumerate() {
                let groups = p.count.div_ceil(GROUP).max(1);
                let x = groups.min(MAX_GROUPS);
                pass.set_pipeline(&self.pipelines[*op as usize]);
                pass.set_bind_group(0, &bind, &[(k as u64 * self.stride) as u32]);
                pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
            }
        }
        let bytes = words as u64 * 4;
        encoder.copy_buffer_to_buffer(&frames.buf, 0, &frames.readback, 0, bytes);
        let index = self.queue.submit([encoder.finish()]);

        let slice = frames.readback.slice(..bytes);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(TIMEOUT),
            })
            .map_err(|e| format!("GPU: {e}"))?;
        match rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("GPU readback: {e}")),
            Err(_) => return Err("the GPU didn't finish a frame in time".into()),
        }
        {
            let view = slice
                .get_mapped_range()
                .map_err(|e| format!("GPU readback: {e}"))?;
            out.copy_from_slice(&view[..n * 3]);
        }
        frames.readback.unmap();
        self.check()?;
        Ok(true)
    }

    /// Make the frame buffers fit w x h, unless that is beyond the GPU's limits.
    fn fit(&mut self, w: usize, h: usize) -> bool {
        if self.frames.as_ref().is_some_and(|f| (f.w, f.h) == (w, h)) {
            return true;
        }
        let n = (w * h) as u64;
        let size = n * 4 * REGIONS;
        let groups = n.div_ceil(GROUP as u64);
        if size > self.limits.max_storage_buffer_binding_size
            || size > self.limits.max_buffer_size
            || groups > MAX_GROUPS as u64 * MAX_GROUPS as u64
        {
            return false;
        }
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let readback = (n * 3).div_ceil(4) * 4;
        self.frames = Some(Frames {
            w,
            h,
            buf: buffer(&self.device, size, usage),
            readback: buffer(&self.device, readback, wgpu::BufferUsages::MAP_READ),
        });
        self.bind = None;
        self.history = History::default();
        true
    }

    fn grow_data(&mut self, bytes: u64) {
        if bytes > self.data.size() {
            let size = bytes.next_power_of_two();
            self.data = buffer(&self.device, size, wgpu::BufferUsages::STORAGE);
            self.bind = None;
        }
    }

    fn grow_params(&mut self, bytes: u64) {
        if bytes > self.params.size() {
            let size = bytes.next_power_of_two();
            self.params = buffer(&self.device, size, wgpu::BufferUsages::UNIFORM);
            self.bind = None;
        }
    }

    fn bind_group(&mut self) -> wgpu::BindGroup {
        if let Some(bind) = &self.bind {
            return bind.clone();
        }
        let frames = self.frames.as_ref().map(|f| &f.buf);
        let frames = frames.expect("frame buffers before the bind group");
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effects"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.params,
                        offset: 0,
                        size: NonZeroU64::new(PARAMS_SIZE),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frames.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.data.as_entire_binding(),
                },
            ],
        });
        self.bind = Some(bind.clone());
        bind
    }
}

/// A buffer that can also be written to (by the queue, or by a copy).
fn buffer(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// One frame's passes, built from its `Step`s. Steps read the current frame (`cur`) and
/// write the other one, then the two swap.
struct Chain {
    w: u32,
    h: u32,
    region: u32,
    cur: u32,
    other: u32,
    passes: Vec<(Op, Params)>,
    /// The data buffer's contents: the grain table, then each pass's tables.
    data: Vec<u32>,
    history: History,
}

impl Chain {
    fn new(w: usize, h: usize, region: u32, noise: &[i16], history: History) -> Chain {
        let mut chain = Chain {
            w: w as u32,
            h: h as u32,
            region,
            cur: 0,
            other: 0,
            passes: Vec::new(),
            data: noise.iter().map(|&v| v as i32 as u32).collect(),
            history,
        };
        chain.cur = chain.at(Region::A);
        chain.other = chain.at(Region::B);
        let unpack = chain.params(chain.at(Region::Io), chain.cur);
        chain.passes.push((Op::Unpack, unpack));
        chain
    }

    fn at(&self, r: Region) -> u32 {
        r as u32 * self.region
    }

    /// A full-frame pass from `src` to `dst`.
    fn params(&self, src: u32, dst: u32) -> Params {
        Params {
            src,
            dst,
            w: self.w,
            h: self.h,
            count: self.w * self.h,
            ..Params::default()
        }
    }

    /// A full-frame pass from the current frame to the other one, which becomes current.
    fn then(&mut self, op: Op, f: impl FnOnce(&mut Params)) {
        let mut p = self.params(self.cur, self.other);
        f(&mut p);
        self.passes.push((op, p));
        std::mem::swap(&mut self.cur, &mut self.other);
    }

    /// Append words to the data buffer; returns where they start.
    fn put(&mut self, words: impl IntoIterator<Item = u32>) -> u32 {
        let at = self.data.len() as u32;
        self.data.extend(words);
        at
    }

    fn copy(&mut self, src: u32, dst: u32) {
        let p = self.params(src, dst);
        self.passes.push((Op::Copy, p));
    }

    fn step(&mut self, step: &Step) {
        match step {
            Step::Warp(warp) => self.then(Op::Warp, |p| warp_params(warp, p)),
            Step::Shift(offs) => {
                let data = self.put(offs.iter().map(|&o| o as u32));
                self.then(Op::Shift, |p| p.data = data);
            }
            Step::Tunnel { zoom, k } => {
                let (feedback, tmp) = (self.at(Region::Feedback), self.at(Region::Tmp));
                if !self.history.feedback {
                    self.copy(self.cur, feedback);
                    self.history.feedback = true;
                }
                let mut p = self.params(feedback, tmp);
                let fb = Warp {
                    zoom: *zoom,
                    ..Warp::default()
                };
                warp_params(&fb, &mut p);
                self.passes.push((Op::Warp, p));
                self.then(Op::Blend, |p| {
                    (p.aux, p.aux2, p.mode, p.r) = (tmp, feedback, 1, mix(*k));
                });
            }
            Step::Trails { k } => {
                let prev = self.at(Region::Prev);
                if !self.history.prev {
                    self.copy(self.cur, prev);
                    self.history.prev = true;
                }
                self.then(Op::Blend, |p| {
                    (p.aux, p.aux2, p.mode, p.r) = (prev, prev, 1, mix(*k));
                });
            }
            Step::Echo { k } => {
                let (lag1, lag2) = (self.at(Region::Lag1), self.at(Region::Lag2));
                if !self.history.lag {
                    self.copy(self.cur, lag1);
                    self.copy(self.cur, lag2);
                    self.history.lag = true;
                }
                self.then(Op::Echo, |p| (p.aux, p.aux2, p.r) = (lag1, lag2, *k));
            }
            Step::Blur { radius } => self.blur(*radius, None),
            Step::Glow { radius, gain } => self.blur(*radius, Some(*gain)),
            Step::Split { off } => self.then(Op::Split, |p| p.i[0] = *off as i32),
            Step::Grade { lut, mix, vignette } => {
                let data = self.put(lut.iter().map(|&c| pack(c)));
                self.then(Op::Grade, |p| {
                    (p.data, p.r, p.mode) = (data, (*mix).min(256), *vignette as u32);
                });
            }
            Step::Hue(m) => self.then(Op::Hue, |p| {
                for (row, k) in m.iter().zip([0, 4, 8]) {
                    p.i[k..k + 3].copy_from_slice(row);
                }
            }),
            Step::Swap(perm) => self.then(Op::Swap, |p| p.r = *perm as u32),
            Step::Glitch(plan) => {
                let data = self.glitch_data(plan);
                self.then(Op::Glitch, |p| p.data = data);
            }
            Step::Snow { a, rows } => {
                let data = self.put(rows.iter().copied());
                self.then(Op::Snow, |p| (p.data, p.i[0]) = (data, *a));
            }
            Step::Edges { gain } => self.then(Op::Edges, |p| p.i[0] = *gain),
            Step::Pixel { block } => self.then(Op::Pixel, |p| p.r = *block as u32),
            Step::Dither { levels, cell } => {
                self.then(Op::Dither, |p| (p.r, p.i[0]) = (*levels, *cell as i32));
            }
            Step::Vhs { line, grain, rows } => {
                let data = self.put(
                    rows.iter()
                        .flat_map(|r: &VhsRow| [r.off as u32, r.start, r.lift as u32]),
                );
                self.then(Op::Vhs, |p| {
                    (p.data, p.r, p.i[0]) = (data, *line as u32, *grain)
                });
            }
            Step::Crt { line, cell, boost } => self.then(Op::Crt, |p| {
                (p.r, p.i[0], p.i[1]) = (*line as u32, *cell as i32, *boost as i32);
            }),
            Step::Film(plan) => self.film(plan),
            Step::Darken { k } => self.then(Op::Darken, |p| p.r = *k),
            Step::Cinema { bar } => self.then(Op::Cinema, |p| p.r = *bar as u32),
            Step::Tone(lut) => {
                let data = self.put(lut.iter().map(|&v| v as u32));
                self.then(Op::Tone, |p| p.data = data);
            }
        }
    }

    /// `soft_blur`, or with a gain `glow`: downsample (keeping only the bright part for
    /// glow), box blur in two passes, and scale back up (added on top for glow).
    fn blur(&mut self, radius: usize, glow: Option<f32>) {
        let (w, h) = (self.w, self.h);
        let tmp = self.at(Region::Tmp);
        let f = blur_scale(radius);
        let window = |r: u32| (1u32 << 16).div_ceil(2 * r + 1);
        if f == 1 && glow.is_none() {
            let r = radius as u32;
            let mut p = self.params(self.cur, tmp);
            (p.r, p.mode) = (r, window(r));
            self.passes.push((Op::HBlur, p));
            self.then(Op::VBlur, |p| (p.src, p.r, p.mode) = (tmp, r, window(r)));
            return;
        }
        let f32 = f as u32;
        let (sw, sh) = (w.div_ceil(f32), h.div_ceil(f32));
        let r = (radius / f).max(1) as u32;
        // Downsample into tmp, blur through the other frame and back into tmp.
        let mut p = self.params(self.cur, tmp);
        (p.sw, p.sh, p.count, p.r, p.mode) = (sw, sh, sw * sh, f32, glow.is_some() as u32);
        self.passes.push((Op::Down, p));
        for (op, src, dst) in [(Op::HBlur, tmp, self.other), (Op::VBlur, self.other, tmp)] {
            let mut p = self.params(src, dst);
            (p.w, p.h, p.count, p.r, p.mode) = (sw, sh, sw * sh, r, window(r));
            self.passes.push((op, p));
        }
        self.then(Op::Up, |p| {
            (p.aux, p.sw, p.sh) = (tmp, sw, sh);
            if let Some(gain) = glow {
                (p.mode, p.i[0]) = (1, (gain * 256.0) as u32 as i32);
            }
        });
    }

    fn glitch_data(&mut self, plan: &GlitchPlan) -> u32 {
        let counts = [plan.slices.len(), plan.blocks.len(), plan.rows.len()];
        let at = self.put(counts.map(|c| c as u32));
        for s in &plan.slices {
            self.put([
                s.y0 as u32,
                s.y1 as u32,
                s.shift as i32 as u32,
                s.perm as u32,
            ]);
        }
        for b in &plan.blocks {
            self.put(b.map(|v| v as u32));
        }
        for (y, values) in &plan.rows {
            self.put(std::iter::once(*y as u32).chain(values.iter().map(|&v| v as u32)));
        }
        at
    }

    fn film(&mut self, plan: &FilmPlan) {
        let lut = self.put(plan.lut.iter().map(|&c| pack(c)));
        let rows = self.put(plan.rows.iter().copied());
        let scratches = self.put(
            std::iter::once(plan.scratches.len() as u32)
                .chain(plan.scratches.iter().map(|&x| x as u32)),
        );
        let dust = self.put(
            std::iter::once(plan.dust.len() as u32)
                .chain(plan.dust.iter().flat_map(|&(x, y)| [x as u32, y as u32])),
        );
        self.then(Op::Film, |p| {
            p.data = lut;
            p.i[..6].copy_from_slice(&[
                plan.mix,
                plan.grain,
                rows as i32,
                scratches as i32,
                dust as i32,
                plan.size as i32,
            ]);
        });
    }
}

/// `blend`'s weight of the history, of 256.
fn mix(k: f32) -> u32 {
    (k.clamp(0.0, 1.0) * 256.0) as u32
}

fn pack(c: [u8; 3]) -> u32 {
    c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16
}

/// The shader's view of a `Warp` (see `warp` in effects.wgsl).
fn warp_params(warp: &Warp, p: &mut Params) {
    let flags = [
        warp.mirror,
        warp.mirror_y,
        warp.per_pixel(),
        warp.identity_x(),
        warp.bulge > 1e-4,
    ];
    p.mode = flags
        .iter()
        .enumerate()
        .map(|(bit, &on)| (on as u32) << bit)
        .sum();
    let inv = 1.0 / warp.zoom.max(1e-3);
    let (sin, cos) = warp.angle.sin_cos();
    let (cx, cy) = (p.w as f32 * 0.5, p.h as f32 * 0.5);
    let (rx2, ry2) = (1.0 / (cx * cx).max(0.25), 1.0 / (cy * cy).max(0.25));
    p.f = [
        inv,
        warp.dx,
        warp.dy,
        warp.bulge,
        sin * inv,
        cos * inv,
        rx2,
        ry2,
    ];
}
