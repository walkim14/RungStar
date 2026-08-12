//! Small shader-generated textures used by the SDL renderer.
//!
//! The GPU is borrowed only during startup. Two tiny WGSL compute passes produce an ambient
//! stage wash and a soft capsule glow, the pixels are uploaded to SDL textures, and the wgpu
//! device is dropped. A deterministic CPU implementation is the fallback, so effects can never
//! be the reason the game does not start.

use std::borrow::Cow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const AMBIENT_WIDTH: u32 = 320;
pub const AMBIENT_HEIGHT: u32 = 160;
pub const AMBIENT_VIEW_WIDTH: u32 = 256;
pub const AMBIENT_VIEW_HEIGHT: u32 = 128;
pub const GLOW_WIDTH: u32 = 128;
pub const GLOW_HEIGHT: u32 = 64;

const AMBIENT_SHADER: &str = include_str!("../../../assets/shaders/ambient.wgsl");
const GLOW_SHADER: &str = include_str!("../../../assets/shaders/glow.wgsl");

pub struct Generated {
    pub ambient: Vec<u8>,
    pub glow: Vec<u8>,
    pub backend: String,
    pub elapsed: Duration,
}

pub fn generate() -> Generated {
    let started = Instant::now();
    match pollster::block_on(generate_gpu()) {
        Ok((ambient, glow, backend)) => Generated {
            ambient,
            glow,
            backend: format!("WGSL/{backend:?}"),
            elapsed: started.elapsed(),
        },
        Err(error) => {
            tracing::warn!("shader effects unavailable ({error}); using the CPU fallback");
            Generated {
                ambient: ambient_cpu(),
                glow: glow_cpu(),
                backend: "CPU fallback".to_owned(),
                elapsed: started.elapsed(),
            }
        }
    }
}

async fn generate_gpu() -> Result<(Vec<u8>, Vec<u8>, wgpu::Backend), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: shader_backends(),
        flags: wgpu::InstanceFlags::empty(),
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .map_err(|error| error.to_string())?;
    let backend = adapter.get_info().backend;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("RungStar startup effects"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .map_err(|error| error.to_string())?;

    let ambient = run_shader(
        &device,
        &queue,
        "ambient stage wash",
        AMBIENT_SHADER,
        AMBIENT_WIDTH,
        AMBIENT_HEIGHT,
    )?;
    let glow = run_shader(
        &device,
        &queue,
        "bubble glow",
        GLOW_SHADER,
        GLOW_WIDTH,
        GLOW_HEIGHT,
    )?;
    Ok((ambient, glow, backend))
}

fn shader_backends() -> wgpu::Backends {
    if cfg!(windows) {
        wgpu::Backends::DX12
    } else if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    }
}

fn run_shader(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    source: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let byte_len = u64::from(width) * u64::from(height) * 4;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: byte_len,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("effect readback"),
        size: byte_len,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, byte_len);
    queue.submit(Some(encoder.finish()));

    let (sent, received) = mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
    device
        .poll(wgpu::PollType::Wait)
        .map_err(|error| error.to_string())?;
    received
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let bytes = readback.slice(..).get_mapped_range().to_vec();
    readback.unmap();
    Ok(bytes)
}

fn ambient_cpu() -> Vec<u8> {
    rgba(AMBIENT_WIDTH, AMBIENT_HEIGHT, |x, y| {
        let u = (x as f32 + 0.5) / AMBIENT_WIDTH as f32;
        let v = (y as f32 + 0.5) / AMBIENT_HEIGHT as f32;
        let diagonal = u * 0.72 + v * 0.28;
        let beam_a = (1.0 - (diagonal - 0.34).abs() * 4.2).max(0.0).powf(2.4);
        let beam_b = (1.0 - (diagonal - 0.76).abs() * 5.0).max(0.0).powf(2.8);
        let curtain = 0.5 + 0.5 * ((u * 2.4 + v * 0.8) * std::f32::consts::TAU).sin();
        let falloff = (1.0 - v).powf(1.35);
        let grain = (hash(x, y) - 0.5) * 0.012;
        0.014 + beam_a * 0.075 + beam_b * 0.048 + curtain * falloff * 0.014 + grain
    })
}

fn glow_cpu() -> Vec<u8> {
    rgba(GLOW_WIDTH, GLOW_HEIGHT, |x, y| {
        let u = ((x as f32 + 0.5) / GLOW_WIDTH as f32 * 2.0 - 1.0).abs();
        let v = ((y as f32 + 0.5) / GLOW_HEIGHT as f32 * 2.0 - 1.0).abs();
        let horizontal = (u - 0.34).max(0.0) * 0.78;
        let distance = horizontal.hypot(v);
        let halo = 1.0 - smoothstep(0.12, 1.0, distance);
        let core = 1.0 - smoothstep(0.04, 0.30, distance);
        halo * halo * 0.48 + core * 0.16
    })
}

fn rgba(width: u32, height: u32, alpha: impl Fn(u32, u32) -> f32) -> Vec<u8> {
    let mut pixels = vec![255; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset + 3] = (alpha(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    pixels
}

fn hash(x: u32, y: u32) -> f32 {
    let mut value = x
        .wrapping_mul(1973)
        .wrapping_add(y.wrapping_mul(9277))
        .wrapping_add(0x68bc_21eb);
    value = (value ^ (value >> 15)).wrapping_mul(2_246_822_519);
    value = (value ^ (value >> 13)).wrapping_mul(3_266_489_917);
    (value ^ (value >> 16)) as f32 / u32::MAX as f32
}

fn smoothstep(from: f32, to: f32, value: f32) -> f32 {
    let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_textures_have_detail_and_a_fading_glow() {
        let ambient = ambient_cpu();
        let alphas: Vec<u8> = ambient.chunks_exact(4).map(|pixel| pixel[3]).collect();
        assert!(alphas.iter().max().unwrap() > alphas.iter().min().unwrap());
        assert!(alphas.iter().copied().max().unwrap() < 64);

        let glow = glow_cpu();
        let alpha_at = |x: u32, y: u32| glow[((y * GLOW_WIDTH + x) * 4 + 3) as usize];
        assert!(alpha_at(GLOW_WIDTH / 2, GLOW_HEIGHT / 2) > alpha_at(0, 0));
        assert_eq!(alpha_at(0, 0), 0);
    }
}
