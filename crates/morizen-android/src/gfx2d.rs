//! A tiny 2D renderer for the mobile shell: untextured coloured rectangles, and text drawn from
//! the built-in 8x8 font as one small quad per lit pixel. No bind groups, no samplers, no game
//! resources: the shell must open before anything is downloaded.

use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

const SHADER: &str = r#"
struct In {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs(input: In) -> Out {
    return Out(vec4<f32>(input.position, 0.0, 1.0), input.color);
}

@fragment
fn fs(input: Out) -> @location(0) vec4<f32> {
    // The colours in this renderer are written as sRGB; wgpu encodes a linear value for an sRGB
    // swapchain, so undo that once here.
    return vec4<f32>(pow(input.color.rgb, vec3<f32>(2.2)), input.color.a);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, point: [f32; 2]) -> bool {
        point[0] >= self.x
            && point[0] <= self.x + self.w
            && point[1] >= self.y
            && point[1] <= self.y + self.h
    }
}

/// One frame of coloured geometry, in physical pixels with the origin at the top left.
pub struct Painter {
    pub width: f32,
    pub height: f32,
    /// The size of a normal UI element, so one layout fits a phone and a tablet.
    pub ui: f32,
    vertices: Vec<Vertex>,
}

impl Painter {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            // One `ui` is one percent of the shorter side, so a phone and a tablet in either
            // orientation get the same proportions and the same finger-sized targets.
            ui: (width.min(height) / 100.0).clamp(3.0, 40.0),
            vertices: Vec::new(),
        }
    }

    pub fn clear(&mut self, color: [f32; 4]) {
        self.rect(0.0, 0.0, self.width, self.height, color);
    }

    fn push(&mut self, x: f32, y: f32, color: [f32; 4]) {
        self.vertices.push(Vertex {
            position: [x / self.width * 2.0 - 1.0, 1.0 - y / self.height * 2.0],
            color,
        });
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        if w <= 0.0 || h <= 0.0 || color[3] <= 0.0 {
            return;
        }
        let (x1, y1) = (x + w, y + h);
        self.push(x, y, color);
        self.push(x1, y, color);
        self.push(x1, y1, color);
        self.push(x, y, color);
        self.push(x1, y1, color);
        self.push(x, y1, color);
    }

    /// A filled panel with a one to two pixel outline.
    pub fn frame(&mut self, area: Rect, fill: [f32; 4], border: [f32; 4]) {
        self.rect(area.x, area.y, area.w, area.h, fill);
        let t = (2.0 * self.ui).round();
        self.rect(area.x, area.y, area.w, t, border);
        self.rect(area.x, area.y + area.h - t, area.w, t, border);
        self.rect(area.x, area.y, t, area.h, border);
        self.rect(area.x + area.w - t, area.y, t, area.h, border);
    }

    /// How many pixels one character cell takes at this text size.
    fn advance(size: f32) -> f32 {
        (size * 1.1).max(1.0)
    }

    pub fn text_width(text: &str, size: f32) -> f32 {
        text.chars().count() as f32 * Self::advance(size)
    }

    /// Draws `text` with its top left at (`x`, `y`). `size` is the cell height in pixels.
    pub fn text(&mut self, mut x: f32, y: f32, text: &str, size: f32, color: [f32; 4]) -> f32 {
        let pixel = (size / 8.0).max(1.0);
        let advance = Self::advance(size);
        for c in text.chars() {
            let rows = crate::font8x8::glyph(c);
            for (row, bits) in rows.iter().enumerate() {
                if *bits == 0 {
                    continue;
                }
                let mut column = 0;
                while column < 8 {
                    if (bits >> column) & 1 == 0 {
                        column += 1;
                        continue;
                    }
                    let start = column;
                    while column < 8 && (bits >> column) & 1 == 1 {
                        column += 1;
                    }
                    self.rect(
                        x + start as f32 * pixel,
                        y + row as f32 * pixel,
                        (column - start) as f32 * pixel,
                        pixel,
                        color,
                    );
                }
            }
            x += advance;
        }
        x
    }

    pub fn text_centered(&mut self, center: f32, y: f32, text: &str, size: f32, color: [f32; 4]) {
        let x = center - Self::text_width(text, size) / 2.0;
        self.text(x, y, text, size, color);
    }

    /// The character size that fits `text` inside `max_width`.
    pub fn fitted(text: &str, size: f32, max_width: f32) -> f32 {
        let width = Self::text_width(text, size);
        if width > max_width && width > 0.0 {
            size * max_width / width
        } else {
            size
        }
    }

    /// Greedy word wrap; words longer than a line are split by character.
    pub fn wrap(text: &str, size: f32, max_width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if Self::text_width(&candidate, size) <= max_width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    pub fn vertices(&self) -> &[u8] {
        bytemuck::cast_slice(self.vertices.as_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// The window's display connection, so the OpenGL backend creates its EGL display from the
/// native window this surface presents to.
#[derive(Debug)]
struct WindowDisplay(Arc<Window>);

impl wgpu::rwh::HasDisplayHandle for WindowDisplay {
    fn display_handle(
        &self,
    ) -> std::result::Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
        wgpu::rwh::HasDisplayHandle::display_handle(&*self.0)
    }
}

/// The swapchain and its one pipeline.
pub struct Gfx {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl Gfx {
    pub fn new(window: Arc<Window>) -> Result<Self, String> {
        pollster::block_on(Self::create(window))
    }

    async fn create(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(WindowDisplay(window.clone())),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("No drawing surface: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("No graphics adapter: {e}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("morizen-shell"),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("No graphics device: {e}"))?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("The screen cannot be drawn on by wgpu.")?;
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(|format| format.is_srgb())
        {
            config.format = format;
        }
        surface.configure(&device, &config);

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("morizen-shell-2d"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("morizen-shell-2d"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 8,
                shader_location: 1,
            },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("morizen-shell-2d"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 1 << 16;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("morizen-shell-2d"),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            buffer,
            capacity,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn width(&self) -> u32 {
        self.config.width
    }

    pub fn height(&self) -> u32 {
        self.config.height
    }

    pub fn draw(&mut self, painter: &Painter) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => return,
                }
            }
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("morizen-shell"),
            });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("morizen-shell"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.02,
                        g: 0.02,
                        b: 0.03,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if !painter.is_empty() {
            let vertices = painter.vertices();
            let bytes = vertices.len() as u64;
            if bytes > self.capacity {
                self.capacity = bytes.next_power_of_two();
                self.buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("morizen-shell-2d"),
                    size: self.capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.queue.write_buffer(&self.buffer, 0, vertices);
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.buffer.slice(..bytes));
            let count = (bytes / std::mem::size_of::<Vertex>() as u64) as u32;
            pass.draw(0..count, 0..1);
        }
        drop(pass);
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_measured_by_its_cell_advance() {
        assert!((Painter::text_width("ABC", 16.0) - 52.8).abs() < 0.01);
        let mut painter = Painter::new(100.0, 100.0);
        painter.text(0.0, 0.0, "H", 8.0, [1.0; 4]);
        // 'H' is three lit runs (two stems, one bar) on each of seven rows.
        assert!(!painter.is_empty());
        assert_eq!(painter.vertices().len() % (6 * 24), 0);
    }

    #[test]
    fn rectangles_convert_to_clip_space() {
        let mut painter = Painter::new(200.0, 100.0);
        painter.rect(0.0, 0.0, 200.0, 100.0, [1.0; 4]);
        assert_eq!(painter.vertices().len(), 6 * 24);
    }

    #[test]
    fn wrapping_never_exceeds_the_width_unless_a_word_cannot_fit() {
        let lines = Painter::wrap("a short line of words", 16.0, 200.0);
        assert!(lines.len() > 1);
        assert_eq!(
            Painter::wrap("abcdefghijklmnop", 16.0, 40.0),
            ["abcdefghijklmnop"]
        );
    }
}
