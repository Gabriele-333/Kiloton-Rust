//! The mushroom cloud on the screen (Java: `VolumeRenderer`): each cloud ray marched as one volume
//! (`kiloton_cloud.wgsl`) at half resolution, farthest first, then laid over the frame. Runs in
//! Horizon's `world_drawn` render hook, after the world and before the hand and the HUD, with the
//! world's depth so the terrain in front hides the cloud.

use crate::client;
use horizon_client::gfx::native_render::wgpu;
use horizon_client::mod_api::{RenderHooks, RenderNeedsEvent, WorldDrawnEvent};
use kiloton_core::fx::mushroom::VOLUME_PARAMS;
use kiloton_core::fx::wilson;
use std::sync::{Arc, Mutex};

/// Clouds drawn at most per frame (the Java edition's `VolumeRenderer.MAX`).
const MAX: usize = 8;
/// The clouds are ray marched at this fraction of the screen's width and height.
const DIVIDE: u32 = 2;
/// One cloud's uniforms: 14 vec4, the inverse matrix, the sizes (304 bytes, rounded up).
const UBO_SIZE: u64 = 320;

pub struct CloudRender;

pub static RENDER: CloudRender = CloudRender;

struct State {
    format: wgpu::TextureFormat,
    volume: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    linear: wgpu::Sampler,
    noise: Option<(Arc<(u32, u32, Vec<u8>)>, wgpu::TextureView)>,
    half: Option<([u32; 2], wgpu::Texture, wgpu::TextureView)>,
    ubos: Vec<wgpu::Buffer>,
    flash: wgpu::RenderPipeline,
    flash_ubo: wgpu::Buffer,
    shadow: wgpu::RenderPipeline,
    shadow_ubos: Vec<wgpu::Buffer>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The composite pass, for a target that takes linear colour or not.
fn composite_source(linear: bool) -> String {
    let s = include_str!("kiloton_cloud_composite.wgsl");
    if linear {
        s.replacen("const LINEAR: bool = false;", "const LINEAR: bool = true;", 1)
    } else {
        s.to_owned()
    }
}

fn premultiplied() -> wgpu::BlendState {
    let part = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState { color: part, alpha: part }
}

fn pipeline(device: &wgpu::Device, source: &str, label: &str, fs: &str, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(premultiplied()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl State {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let linear_target = format.is_srgb() || matches!(format, wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float);
        log::info!("Kiloton: cloud volume drawn over a {format:?} frame (linear: {linear_target})");
        let volume = pipeline(device, include_str!("kiloton_cloud.wgsl"), "Kiloton cloud volume", "fs_volume", wgpu::TextureFormat::Rgba16Float);
        let composite = pipeline(device, &composite_source(linear_target), "Kiloton cloud composite", "fs", format);
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Kiloton cloud"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ubos = (0..MAX)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Kiloton cloud uniforms"),
                    size: UBO_SIZE,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let flash = pipeline(device, include_str!("kiloton_flash.wgsl"), "Kiloton flash", "fs", format);
        let flash_ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Kiloton flash uniforms"),
            size: 160,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow = pipeline(device, include_str!("kiloton_shadow.wgsl"), "Kiloton nuclear shadow", "fs", format);
        let shadow_ubos = (0..16)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Kiloton shadow uniforms"),
                    size: 112,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self { format, volume, composite, linear, noise: None, half: None, ubos, flash, flash_ubo, shadow, shadow_ubos }
    }
}

/// The clouds of this frame: each one's 14 vec4 and its distance (for the order).
fn clouds(camera: [f64; 3]) -> (Vec<([f32; 56], f64)>, Option<Arc<(u32, u32, Vec<u8>)>>) {
    client::with(|c| {
        let mut out = Vec::new();
        for b in &c.blasts {
            let t = c.age(b);
            if t < 0.0 || b.cloud_off || !b.cloud.alive(t) {
                continue;
            }
            let (cx, cy, cz) = (camera[0] - b.x, camera[1] - b.y, camera[2] - b.z);
            let mut sky = c.sky;
            sky.humidity = b.humidity;
            let Some(v) = b.cloud.volume(t, cx, cy, cz, &sky) else { continue };
            let mut p = [0f32; 56];
            p[..VOLUME_PARAMS].copy_from_slice(&v);
            // P12: no skyline texture here; z = -10, no fade past the drawn terrain.
            p[48] = 0.0;
            p[49] = 0.0;
            p[50] = -10.0;
            p[51] = 1.0;
            let w = wilson::params(&b.cloud.model, t, b.humidity as f64, c.config.wilson_cloud);
            p[52..56].copy_from_slice(&w);
            out.push((p, cx * cx + cz * cz));
            if out.len() >= MAX {
                break;
            }
        }
        // Farthest first.
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        (out, c.noise.clone())
    })
}

impl RenderHooks for CloudRender {
    fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
        let any = client::with(|c| !c.blasts.is_empty() || !c.shadows.is_empty());
        if any {
            event.needs.world_depth = true;
        }
    }

    fn world_drawn(&self, event: &mut WorldDrawnEvent<'_, '_>) {
        let w = &mut *event.world;
        let camera = [w.camera.x, w.camera.y, w.camera.z];
        let (list, noise) = clouds(camera);
        let (glares, afters, wobble) = flash_spots(camera, w.view_proj.to_cols_array(), w.size);
        let shadows: Vec<(u32, [f64; 3], f32, f32)> = client::with(|c| {
            c.shadows
                .iter()
                .filter(|s| (s.1[0] - camera[0]).powi(2) + (s.1[2] - camera[2]).powi(2) < 256.0 * 256.0)
                .take(16)
                .copied()
                .collect()
        });
        if list.is_empty() && glares.is_empty() && afters.is_empty() && shadows.is_empty() {
            return;
        }
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_none_or(|s| s.format != w.color_format) {
            *guard = Some(State::new(w.device, w.color_format));
        }
        let s = guard.as_mut().unwrap();
        if !shadows.is_empty() {
            let vp = w.view_proj.to_cols_array();
            let mut groups = Vec::new();
            for (k, sh) in shadows.iter().enumerate() {
                let mut b = Vec::with_capacity(112);
                for v in vp {
                    b.extend_from_slice(&v.to_le_bytes());
                }
                let o = [(sh.1[0] - camera[0]) as f32, (sh.1[1] - camera[1]) as f32, (sh.1[2] - camera[2]) as f32, sh.2];
                let rest = [sh.3, 0.85, w.depth_size[0] as f32, w.depth_size[1] as f32, w.size[0] as f32, w.size[1] as f32, 0.0, 0.0];
                for v in o.iter().chain(rest.iter()) {
                    b.extend_from_slice(&v.to_le_bytes());
                }
                w.queue.write_buffer(&s.shadow_ubos[k], 0, &b);
                groups.push(w.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Kiloton shadow"),
                    layout: &s.shadow.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: s.shadow_ubos[k].as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(w.depth) },
                    ],
                }));
            }
            let mut pass = w.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kiloton nuclear shadows"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: w.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&s.shadow);
            for g in &groups {
                pass.set_bind_group(0, g, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        if let (false, Some(noise)) = (list.is_empty(), noise) {
            draw_clouds(s, w, &list, noise);
        }
        if !glares.is_empty() || !afters.is_empty() {
            let mut bytes = Vec::with_capacity(160);
            for v in [w.size[0] as f32, w.size[1] as f32, glares.len() as f32, afters.len() as f32] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for list in [&glares, &afters] {
                for k in 0..4 {
                    let v = list.get(k).copied().unwrap_or([0.0; 4]);
                    for x in v {
                        bytes.extend_from_slice(&x.to_le_bytes());
                    }
                }
            }
            for v in [wobble, 0.0, 0.0, 0.0] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            w.queue.write_buffer(&s.flash_ubo, 0, &bytes);
            let group = w.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Kiloton flash"),
                layout: &s.flash.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: s.flash_ubo.as_entire_binding() }],
            });
            let mut pass = w.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kiloton flash"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: w.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&s.flash);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// The glares (centre, size in pixels, intensity) and after-images (centre, size, alpha) of this
/// frame, and the after-image's wobble in pixels (Java: `FlashOverlay`).
fn flash_spots(camera: [f64; 3], view_proj: [f32; 16], size: [u32; 2]) -> (Vec<[f32; 4]>, Vec<[f32; 4]>, f32) {
    let (wpx, hpx) = (size[0] as f32, size[1] as f32);
    client::with(|c| {
        let strength = c.config.flash_strength;
        let (mut glares, mut afters) = (Vec::new(), Vec::new());
        for b in &c.blasts {
            let t = c.age(b);
            if t < 0.0 || b.cloud_off {
                continue;
            }
            if b.sight >= 0.5 && t <= b.cloud.life && glares.len() < 4 {
                let fy = b.y + b.cloud.fireball_centre(t);
                let rel = [b.x - camera[0], fy - camera[1], b.z - camera[2]];
                let d = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
                let i = (kiloton_core::fx::viewer::glare(&b.cloud, t, d) * strength) as f32;
                if i >= 0.01 {
                    let m = &view_proj;
                    let v = [rel[0] as f32, rel[1] as f32, rel[2] as f32];
                    let clip: Vec<f32> = (0..4).map(|r| m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2] + m[12 + r]).collect();
                    if clip[3] > 0.01 {
                        let (nx, ny) = (clip[0] / clip[3], clip[1] / clip[3]);
                        if nx.abs() <= 1.6 && ny.abs() <= 1.6 {
                            let side = wpx.max(hpx) * (0.25 + 1.1 * i);
                            glares.push([(nx * 0.5 + 0.5) * wpx, (1.0 - (ny * 0.5 + 0.5)) * hpx, side, i]);
                        }
                    }
                }
            }
            if b.after_strength > 0.01 && afters.len() < 4 {
                let a = ((0.6 * b.after_strength).min(0.45) * strength) as f32;
                afters.push([b.after_pos[0] * wpx, b.after_pos[1] * hpx, (b.after_size * hpx).max(4.0), a]);
            }
        }
        let gt = c.clock.map_or(0, |(g, _)| g);
        (glares, afters, (gt as f32 * 0.05).sin() * 2.0 * hpx / 360.0)
    })
}

/// The clouds of this frame into the half-size target, then over the frame.
fn draw_clouds(s: &mut State, w: &mut horizon_client::mod_api::WorldDrawn<'_>, list: &[([f32; 56], f64)], noise: Arc<(u32, u32, Vec<u8>)>) {
    {
        if s.noise.as_ref().is_none_or(|(n, _)| !Arc::ptr_eq(n, &noise)) {
            let (nw, nh, rgba) = (noise.0, noise.1, &noise.2);
            let tex = w.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Kiloton cloud noise"),
                size: wgpu::Extent3d { width: nw, height: nh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            w.queue.write_texture(
                tex.as_image_copy(),
                rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * nw), rows_per_image: Some(nh) },
                tex.size(),
            );
            s.noise = Some((noise.clone(), tex.create_view(&Default::default())));
        }
        let half_size = [(w.size[0] / DIVIDE).max(1), (w.size[1] / DIVIDE).max(1)];
        if s.half.as_ref().is_none_or(|(sz, _, _)| *sz != half_size) {
            let tex = w.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Kiloton cloud volume (half size)"),
                size: wgpu::Extent3d { width: half_size[0], height: half_size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            s.half = Some((half_size, tex, view));
        }
        let inv = w.view_proj.inverse().to_cols_array();
        let mut groups = Vec::with_capacity(list.len());
        for (k, (p, _)) in list.iter().enumerate() {
            let mut bytes = Vec::with_capacity(UBO_SIZE as usize);
            for v in p.iter().chain(inv.iter()) {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for v in [w.depth_size[0] as f32, w.depth_size[1] as f32, half_size[0] as f32, half_size[1] as f32] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            bytes.resize(UBO_SIZE as usize, 0);
            w.queue.write_buffer(&s.ubos[k], 0, &bytes);
            groups.push(w.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Kiloton cloud"),
                layout: &s.volume.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: s.ubos[k].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&s.noise.as_ref().unwrap().1) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&s.linear) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(w.depth) },
                ],
            }));
        }
        let half_view = &s.half.as_ref().unwrap().2;
        {
            let mut pass = w.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kiloton mushroom cloud volume"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: half_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&s.volume);
            for g in &groups {
                pass.set_bind_group(0, g, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        let composite_group = w.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Kiloton cloud composite"),
            layout: &s.composite.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(half_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&s.linear) },
            ],
        });
        let mut pass = w.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Kiloton mushroom cloud composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: w.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&s.composite);
        pass.set_bind_group(0, &composite_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
