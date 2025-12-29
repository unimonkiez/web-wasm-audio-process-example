mod utils;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
    #[wasm_bindgen(js_namespace = Date)]
    fn now() -> f64;
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum SingleAudioFileType {
    Wav,
    Mpeg,
    Ogg,
}

#[wasm_bindgen]
pub struct SingleAudioFile {
    #[wasm_bindgen(getter_with_clone)]
    pub bytes: Vec<u8>,
    pub r#type: SingleAudioFileType,
}

#[wasm_bindgen]
impl SingleAudioFile {
    pub fn new(bytes: Vec<u8>, r#type: SingleAudioFileType) -> Self {
        Self { bytes, r#type }
    }
}

fn create_wav_container(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let mut wav = Vec::new();
    let data_size = (samples.len() * 2) as u32;

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let s = (clamped * i16::MAX as f32) as i16;
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

struct AudioCombinerSingleFile {
    samples: Vec<f32>,
}

#[wasm_bindgen]
pub struct AudioCombiner {
    files: Vec<AudioCombinerSingleFile>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

#[wasm_bindgen]
impl AudioCombiner {
    pub async fn new(files: Vec<SingleAudioFile>) -> Result<AudioCombiner, String> {
        let mut processed_files = Vec::with_capacity(files.len());
        for file in files {
            let mut decoded_samples = Vec::new();
            let src = std::io::Cursor::new(file.bytes);
            let mss =
                symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());

            let mut hint = symphonia::core::probe::Hint::new();
            match file.r#type {
                SingleAudioFileType::Wav => {
                    hint.with_extension("wav");
                }
                SingleAudioFileType::Mpeg => {
                    hint.with_extension("mp3");
                }
                SingleAudioFileType::Ogg => {
                    hint.with_extension("ogg");
                }
            }

            let probed = symphonia::default::get_probe()
                .format(&hint, mss, &Default::default(), &Default::default())
                .map_err(|e| e.to_string())?;

            let mut format = probed.format;
            let track = format.default_track().ok_or("No supported audio track")?;
            let mut decoder = symphonia::default::get_codecs()
                .make(&track.codec_params, &Default::default())
                .map_err(|e| e.to_string())?;

            let mut sample_buf = None;
            while let Ok(packet) = format.next_packet() {
                let decoded = decoder.decode(&packet).map_err(|e| e.to_string())?;
                let spec = *decoded.spec();
                let num_channels = spec.channels.count();
                let buf = sample_buf.get_or_insert_with(|| {
                    symphonia::core::audio::SampleBuffer::<f32>::new(
                        decoded.capacity() as u64,
                        spec,
                    )
                });
                buf.copy_interleaved_ref(decoded);

                for frame in buf.samples().chunks(num_channels) {
                    decoded_samples.push(frame[0]);
                    decoded_samples.push(if num_channels > 1 { frame[1] } else { frame[0] });
                }
            }
            processed_files.push(AudioCombinerSingleFile {
                samples: decoded_samples,
            });
        }

        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(|e| format!("Failed to request adapter: {}", e))?; // request_adapter returns Result

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| e.to_string())?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mixer Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
                struct Params { num_files: u32, buffer_len: u32 }
                @group(0) @binding(0) var<uniform> params: Params;
                @group(0) @binding(1) var<storage, read> volumes: array<f32>;
                @group(0) @binding(2) var<storage, read> all_samples: array<f32>;
                @group(0) @binding(3) var<storage, read_write> output: array<f32>;

                @compute @workgroup_size(256)
                fn main(@builtin(global_invocation_id) global_id: vec3<u32>, 
                        @builtin(num_workgroups) num_groups: vec3<u32>) {
                    
                    // Calculate a flat index from the 2D grid of workgroups
                    // global_id.x is the local x + (workgroup_x * 256)
                    // we multiply the y workgroup index by the total width of the x dispatch
                    let idx = global_id.y * (num_groups.x * 256u) + global_id.x;

                    if (idx >= params.buffer_len) { return; }
                    
                    var mixed: f32 = 0.0;
                    for (var i: u32 = 0u; i < params.num_files; i = i + 1u) {
                        mixed += all_samples[(i * params.buffer_len) + idx] * volumes[i];
                    }
                    output[idx] = mixed;
                }
            "#,
            )),
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Mix Pipeline"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            cache: None,
            compilation_options: wgpu::PipelineCompilationOptions::default(), // Added for modern wgpu
        });

        Ok(AudioCombiner {
            files: processed_files,
            device,
            queue,
            pipeline,
        })
    }

    pub async fn combine(&self, volumes: Vec<u8>) -> Result<SingleAudioFile, String> {
        log("Starting combine....");
        let max_len = self
            .files
            .iter()
            .map(|f| f.samples.len())
            .max()
            .unwrap_or(0);
        if max_len == 0 {
            return Err("No data".to_string());
        }

        let mut flat_samples = vec![0.0f32; max_len * self.files.len()];
        for (i, file) in self.files.iter().enumerate() {
            let start = i * max_len;
            let copy_len = file.samples.len();
            flat_samples[start..start + copy_len].copy_from_slice(&file.samples);
        }
        let float_volumes: Vec<f32> = volumes.iter().map(|&v| v as f32 / 100.0).collect();

        let to_bytes = |data: &[f32]| -> Vec<u8> {
            data.iter()
                .flat_map(|&f| f.to_le_bytes().to_vec())
                .collect()
        };

        let param_bytes: [u8; 8] = {
            let mut b = [0u8; 8];
            b[0..4].copy_from_slice(&(self.files.len() as u32).to_le_bytes());
            b[4..8].copy_from_slice(&(max_len as u32).to_le_bytes());
            b
        };

        let param_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&param_buf, 0, &param_bytes);

        let vol_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (float_volumes.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&vol_buf, 0, &to_bytes(&float_volumes));

        let samp_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (flat_samples.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&samp_buf, 0, &to_bytes(&flat_samples));

        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (max_len * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let stage_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (max_len * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: param_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: vol_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: samp_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });
        log("Starting buffers");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            let total_workgroups = (max_len as u32 + 255) / 256;
            let x_groups = 16384; // A safe number well under 65535
            let y_groups = (total_workgroups + x_groups - 1) / x_groups;

            cpass.dispatch_workgroups(x_groups, y_groups, 1);
            // cpass.dispatch_workgroups((max_len as u32 + 255) / 256, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &stage_buf, 0, (max_len * 4) as u64);
        self.queue.submit(Some(encoder.finish()));

        log("Starting submitted");

        // 1. Use a futures oneshot channel instead of std::sync::mpsc
        let (tx, rx) = futures::channel::oneshot::channel();

        stage_buf
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx.send(res);
            });

        // 2. In modern wgpu (v0.20+), use Maintain::poll() or Maintain::Wait
        // On the web, this triggers the device to check for completion.
        self.device
            .poll(wgpu::wgt::PollType::Poll)
            .map_err(|_| ":")?;

        // 3. AWAIT the result. This yields to the browser event loop,
        // allowing the map_async callback to actually fire.
        rx.await
            .map_err(|_| "Channel sender dropped")?
            .map_err(|e| e.to_string())?;

        log("Received");

        let view = stage_buf.slice(..).get_mapped_range();
        let mut master_buffer = vec![0.0f32; max_len];
        for (i, chunk) in view.chunks_exact(4).enumerate() {
            let mut b = [0u8; 4];
            b.copy_from_slice(chunk);
            master_buffer[i] = f32::from_le_bytes(b);
        }
        drop(view);
        stage_buf.unmap();

        Ok(SingleAudioFile {
            bytes: create_wav_container(&master_buffer, 44100),
            r#type: SingleAudioFileType::Wav,
        })
    }
}
